//! Connections (plugin API 1.14, `docs/marketplace/sdk/permissions.md`
//! "Connections"): servers the user connects a plugin to, one row each in
//! `plugin_connections`, and the secret of each in
//! `plugin_connection_secrets`.
//!
//! A connection is an instance of one of the plugin's declared
//! `contributes.connectionTypes`: its own origin (and path prefix), the
//! user's consent to that origin (creating the connection is the consent),
//! a separate consent to a private network, the methods it may send (`GET`
//! until the user allows changes), a status and a secret. Two connections
//! to the same server are two rows with two secrets, never merged.
//!
//! The rules this module keeps, whoever calls it:
//!
//! - the type is declared by the active manifest and listed by its
//!   `connections` permission; the methods are within the permission's, and
//!   a private network only where the permission allows one;
//! - a cloud connection has exactly the manifest's fixed origins; a
//!   self-hosted one has its API under the type's `apiPath` and its web
//!   pages on the same scheme, host and port;
//! - the display name and the account label never look like a token (a
//!   name is shown on cards, in dialogs and in logs);
//! - the secret is written and read only here, returned only by
//!   [`crate::PluginData::connection_secret`] for injection, and deleted in
//!   the same transaction as its connection.
//!
//! The agent normalizes and checks URLs strictly before it calls in
//! (`apps/agent/src/plugins/connections.rs`); this module re-checks their
//! shape, so a malformed row is never written.

use deckpilot_plugin_manifest::{HttpMethod, Manifest, Permission, PluginId};
use rusqlite::{Connection, OptionalExtension as _, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use zeroize::Zeroizing;

use crate::data::{MAX_SECRET_BYTES, Secret};
use crate::error::{Result, StoreError};

/// The prefix of a connection id.
pub const CONNECTION_ID_PREFIX: &str = "cn_";
/// Characters after [`CONNECTION_ID_PREFIX`] (a ULID in Crockford base32).
pub const CONNECTION_ID_BODY: usize = 26;
/// Longest display name, in characters.
pub const MAX_CONNECTION_NAME: usize = 64;
/// Longest account label, in characters.
pub const MAX_ACCOUNT_LABEL: usize = 64;
/// Most connections one plugin may hold.
pub const MAX_CONNECTIONS: usize = 32;
/// Longest status detail code.
pub const MAX_STATUS_DETAIL: usize = 48;
/// Largest capabilities record, as JSON.
pub const MAX_CAPABILITIES_BYTES: usize = 8 * 1024;
/// Longest origin with its path prefix.
pub const MAX_CONNECTION_URL: usize = 1024;

/// Whether `id` is a connection id: `cn_` and 26 characters of Crockford
/// base32 (digits and upper-case letters without `I`, `L`, `O`, `U`).
pub fn is_connection_id(id: &str) -> bool {
    id.strip_prefix(CONNECTION_ID_PREFIX).is_some_and(|body| {
        body.len() == CONNECTION_ID_BODY
            && body
                .bytes()
                .all(|b| b.is_ascii_digit() || (b.is_ascii_uppercase() && !b"ILOU".contains(&b)))
    })
}

/// Whether `text` looks like a token a user may paste by mistake into a
/// name: the known GitHub and GitLab token shapes, or a long run of base64
/// or hex without spaces. Names are shown on cards and in confirmations;
/// a secret must never become one.
pub fn looks_like_secret(text: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "ghp_",
        "gho_",
        "ghu_",
        "ghs_",
        "ghr_",
        "github_pat_",
        "glpat-",
        "gloas-",
        "glptt-",
        "glrt-",
        "gldt-",
        "glsoat-",
        "glcbt-",
    ];
    text.split_whitespace().any(|word| {
        let lower = word.to_ascii_lowercase();
        PREFIXES.iter().any(|p| lower.contains(p))
            || (word.len() >= 32
                && word
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"+/=_-".contains(&b)))
    })
}

/// Where a connection's server runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Deployment {
    /// The provider's public service, at the manifest's fixed origins.
    Cloud,
    /// A server the user entered.
    SelfHosted,
}

impl Deployment {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cloud => "cloud",
            Self::SelfHosted => "selfHosted",
        }
    }

    fn parse(text: &str) -> Result<Self> {
        match text {
            "cloud" => Ok(Self::Cloud),
            "selfHosted" => Ok(Self::SelfHosted),
            other => Err(StoreError::record(format!("unknown deployment `{other}`"))),
        }
    }
}

/// A connection's status as the app shows it: what the last test or
/// request found, or the host's own state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionStatus {
    Ok,
    Unauthorized,
    InsufficientScope,
    Unreachable,
    TlsFailed,
    NeedsReconnect,
    Untested,
}

impl ConnectionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Unauthorized => "unauthorized",
            Self::InsufficientScope => "insufficientScope",
            Self::Unreachable => "unreachable",
            Self::TlsFailed => "tlsFailed",
            Self::NeedsReconnect => "needsReconnect",
            Self::Untested => "untested",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        [
            Self::Ok,
            Self::Unauthorized,
            Self::InsufficientScope,
            Self::Unreachable,
            Self::TlsFailed,
            Self::NeedsReconnect,
            Self::Untested,
        ]
        .into_iter()
        .find(|s| s.as_str() == text)
    }
}

/// A connection to create. The caller generated the id and normalized the
/// URLs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewConnection {
    pub id: String,
    pub type_id: String,
    pub display_name: String,
    pub deployment: Deployment,
    /// `https://host[:port]`, normalized.
    pub origin: String,
    /// `/api/v4` and the like; `None` for an API at the origin's root.
    pub path_prefix: Option<String>,
    /// `https://host[:port]` the pages are opened on.
    pub web_origin: String,
    /// `GET` only until the user allows changes.
    pub methods: Vec<HttpMethod>,
    /// The user's separate consent to a server on a private network.
    pub private_network: bool,
    /// The connection the old plugin's settings became (§11 of the design),
    /// if any.
    pub legacy_source: Option<String>,
}

/// One row of `plugin_connections`, with whether a secret is stored (never
/// the secret).
#[derive(Debug, Clone, PartialEq)]
pub struct StoredConnection {
    pub id: String,
    pub type_id: String,
    pub display_name: String,
    pub deployment: Deployment,
    pub origin: String,
    pub path_prefix: Option<String>,
    pub web_origin: String,
    pub methods: Vec<HttpMethod>,
    pub private_network: bool,
    pub enabled: bool,
    pub status: ConnectionStatus,
    /// A code (`dns-failed`), never text of the server's.
    pub status_detail: Option<String>,
    /// What the plugin's last test reported (no secrets).
    pub capabilities: Option<Value>,
    pub account_label: Option<String>,
    pub is_default: bool,
    pub legacy_source: Option<String>,
    pub consent_by: String,
    pub consent_at: i64,
    pub created_at: i64,
    pub updated_at: i64,
    /// When the secret was stored; `None` without one.
    pub secret_updated_at: Option<i64>,
}

impl StoredConnection {
    /// The API base: the origin and its path prefix.
    pub fn api_base(&self) -> String {
        format!(
            "{}{}",
            self.origin,
            self.path_prefix.as_deref().unwrap_or("")
        )
    }

    pub fn has_secret(&self) -> bool {
        self.secret_updated_at.is_some()
    }
}

/// What a test of a connection found, written by the host.
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectionProbe {
    pub status: ConnectionStatus,
    pub detail: Option<String>,
    pub account_label: Option<String>,
    pub capabilities: Option<Value>,
}

/// The `connections` permission of `manifest`: its types, methods and
/// whether a private network may be consented to.
pub fn declared(manifest: &Manifest) -> Option<(&[String], Vec<HttpMethod>, bool)> {
    manifest.permissions.iter().find_map(|p| match p {
        Permission::Connections {
            types,
            private_network,
            ..
        } => Some((
            types.as_slice(),
            p.connection_methods(),
            *private_network == Some(true),
        )),
        _ => None,
    })
}

fn invalid<T>(id: &PluginId, why: impl std::fmt::Display) -> Result<T> {
    Err(StoreError::InvalidData(format!("{id}: {why}")))
}

/// Whether `text` is a normalized `https://host[:port]`.
fn is_origin(text: &str) -> bool {
    let host = text.strip_prefix("https://").unwrap_or_default();
    !host.is_empty()
        && !host.contains(['/', '?', '#', '@', '\\', ' '])
        && !host.chars().any(|c| c.is_control() || c.is_whitespace())
        && host == host.to_ascii_lowercase()
}

fn is_prefix(text: &str) -> bool {
    text.starts_with('/')
        && !text.ends_with('/')
        && !text.contains(['?', '#', '\\', '%'])
        && !text.contains("//")
        && !text.chars().any(|c| c.is_control() || c.is_whitespace())
        && !text
            .split('/')
            .any(|segment| segment == "." || segment == "..")
}

/// Splits an `https` URL of the manifest (`https://gitlab.com/api/v4`) into
/// its origin and path prefix, as the agent normalizes them.
fn split_url(url: &str) -> (String, Option<String>) {
    let url = url.trim_end_matches('/');
    let rest = url.strip_prefix("https://").unwrap_or(url);
    match rest.find('/') {
        Some(at) => (
            format!("https://{}", rest[..at].to_ascii_lowercase()),
            Some(rest[at..].to_owned()),
        ),
        None => (format!("https://{}", rest.to_ascii_lowercase()), None),
    }
}

pub(crate) fn check_name(id: &PluginId, name: &str) -> Result<()> {
    let chars = name.chars().count();
    if chars == 0 || chars > MAX_CONNECTION_NAME || name.trim() != name {
        return invalid(
            id,
            format!("a connection's name is 1 to {MAX_CONNECTION_NAME} characters"),
        );
    }
    if name.chars().any(char::is_control) {
        return invalid(id, "a connection's name has no control characters");
    }
    if looks_like_secret(name) {
        return invalid(
            id,
            "a connection's name looks like a token; tokens go only in the token field",
        );
    }
    Ok(())
}

/// Checks a new connection against the active manifest.
pub(crate) fn check(id: &PluginId, manifest: &Manifest, new: &NewConnection) -> Result<()> {
    if !is_connection_id(&new.id) {
        return invalid(id, "not a connection id");
    }
    let Some((types, methods, private_allowed)) = declared(manifest) else {
        return invalid(id, "the plugin does not ask for connections");
    };
    if !types.contains(&new.type_id) {
        return invalid(
            id,
            format!("`{}` is not one of its connection types", new.type_id),
        );
    }
    let Some(declared) = manifest
        .contributes
        .connection_types()
        .iter()
        .find(|t| t.id == new.type_id)
    else {
        return invalid(id, format!("`{}` is not declared", new.type_id));
    };
    check_name(id, &new.display_name)?;
    if new.methods.is_empty() || new.methods.iter().any(|m| !methods.contains(m)) {
        return invalid(id, "the methods are not those the plugin may be allowed");
    }
    if new.private_network && !private_allowed {
        return invalid(id, "the plugin may not reach a private network");
    }
    if !is_origin(&new.origin) || !is_origin(&new.web_origin) {
        return invalid(id, "not a normalized https origin");
    }
    if let Some(prefix) = new.path_prefix.as_ref().filter(|p| !is_prefix(p)) {
        return invalid(id, format!("`{prefix}` is not a normalized path prefix"));
    }
    if new.origin.len() + new.path_prefix.as_deref().map_or(0, str::len) > MAX_CONNECTION_URL {
        return invalid(id, "the server address is too long");
    }
    match new.deployment {
        Deployment::Cloud => {
            let Some(cloud) = &declared.cloud else {
                return invalid(id, format!("`{}` has no public service", new.type_id));
            };
            let (origin, prefix) = split_url(&cloud.api);
            let (web, _) = split_url(&cloud.web);
            if new.origin != origin || new.path_prefix != prefix || new.web_origin != web {
                return invalid(id, "a cloud connection uses the provider's own addresses");
            }
            if new.private_network {
                return invalid(id, "a cloud connection is never on a private network");
            }
        }
        Deployment::SelfHosted => {
            let Some(hosted) = &declared.self_hosted else {
                return invalid(id, format!("`{}` has no self-hosted servers", new.type_id));
            };
            let under = new
                .path_prefix
                .as_deref()
                .is_some_and(|p| p.ends_with(hosted.api_path.as_str()));
            if !under {
                return invalid(
                    id,
                    format!(
                        "the API of a self-hosted server is under `{}`",
                        hosted.api_path
                    ),
                );
            }
            if new.web_origin != new.origin {
                return invalid(id, "the pages and the API of a server share its address");
            }
        }
    }
    Ok(())
}

/// Checks a secret before it is stored: 1 to [`MAX_SECRET_BYTES`] bytes
/// without line breaks or NUL characters (it travels in one header).
pub(crate) fn check_secret(secret: &str) -> Result<()> {
    if secret.is_empty() || secret.len() > MAX_SECRET_BYTES {
        return Err(StoreError::InvalidData(format!(
            "a secret is 1 to {MAX_SECRET_BYTES} bytes"
        )));
    }
    if secret
        .chars()
        .any(|c| c == '\r' || c == '\n' || c == '\0' || c.is_control())
    {
        return Err(StoreError::InvalidData(
            "a secret cannot contain line breaks or control characters".into(),
        ));
    }
    Ok(())
}

fn check_detail(detail: Option<&str>) -> Result<()> {
    let Some(detail) = detail else {
        return Ok(());
    };
    let mut bytes = detail.bytes();
    let ok = detail.len() <= MAX_STATUS_DETAIL
        && bytes.next().is_some_and(|b| b.is_ascii_lowercase())
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if ok {
        Ok(())
    } else {
        Err(StoreError::InvalidData(
            "a status detail is a short code".into(),
        ))
    }
}

pub(crate) fn check_probe(probe: &ConnectionProbe) -> Result<()> {
    check_detail(probe.detail.as_deref())?;
    if let Some(label) = &probe.account_label {
        let chars = label.chars().count();
        if chars == 0
            || chars > MAX_ACCOUNT_LABEL
            || label.chars().any(char::is_control)
            || looks_like_secret(label)
        {
            return Err(StoreError::InvalidData(
                "an account label is a short name".into(),
            ));
        }
    }
    if let Some(capabilities) = &probe.capabilities {
        let size = serde_json::to_string(capabilities)
            .map_err(StoreError::record)?
            .len();
        if !capabilities.is_object() || size > MAX_CAPABILITIES_BYTES {
            return Err(StoreError::InvalidData(format!(
                "capabilities are an object of at most {MAX_CAPABILITIES_BYTES} bytes"
            )));
        }
    }
    Ok(())
}

const COLUMNS: &str = "c.connection_id, c.type_id, c.display_name, c.deployment, c.origin,
    c.path_prefix, c.web_origin, c.methods_json, c.private_network, c.enabled, c.status,
    c.status_detail, c.capabilities_json, c.account_label, c.is_default, c.legacy_source,
    c.consent_by, c.consent_at, c.created_at, c.updated_at, s.updated_at";

fn row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawRow> {
    Ok(RawRow {
        id: row.get(0)?,
        type_id: row.get(1)?,
        display_name: row.get(2)?,
        deployment: row.get(3)?,
        origin: row.get(4)?,
        path_prefix: row.get(5)?,
        web_origin: row.get(6)?,
        methods: row.get(7)?,
        private_network: row.get(8)?,
        enabled: row.get(9)?,
        status: row.get(10)?,
        status_detail: row.get(11)?,
        capabilities: row.get(12)?,
        account_label: row.get(13)?,
        is_default: row.get(14)?,
        legacy_source: row.get(15)?,
        consent_by: row.get(16)?,
        consent_at: row.get(17)?,
        created_at: row.get(18)?,
        updated_at: row.get(19)?,
        secret_updated_at: row.get(20)?,
    })
}

struct RawRow {
    id: String,
    type_id: String,
    display_name: String,
    deployment: String,
    origin: String,
    path_prefix: Option<String>,
    web_origin: String,
    methods: String,
    private_network: bool,
    enabled: bool,
    status: String,
    status_detail: Option<String>,
    capabilities: Option<String>,
    account_label: Option<String>,
    is_default: bool,
    legacy_source: Option<String>,
    consent_by: String,
    consent_at: i64,
    created_at: i64,
    updated_at: i64,
    secret_updated_at: Option<i64>,
}

impl RawRow {
    fn parse(self) -> Result<StoredConnection> {
        Ok(StoredConnection {
            id: self.id,
            type_id: self.type_id,
            display_name: self.display_name,
            deployment: Deployment::parse(&self.deployment)?,
            origin: self.origin,
            path_prefix: self.path_prefix,
            web_origin: self.web_origin,
            methods: serde_json::from_str(&self.methods).map_err(StoreError::record)?,
            private_network: self.private_network,
            enabled: self.enabled,
            // A status a later build wrote: shown as untested.
            status: ConnectionStatus::parse(&self.status).unwrap_or(ConnectionStatus::Untested),
            status_detail: self.status_detail,
            capabilities: self
                .capabilities
                .map(|c| serde_json::from_str(&c))
                .transpose()
                .map_err(StoreError::record)?,
            account_label: self.account_label,
            is_default: self.is_default,
            legacy_source: self.legacy_source,
            consent_by: self.consent_by,
            consent_at: self.consent_at,
            created_at: self.created_at,
            updated_at: self.updated_at,
            secret_updated_at: self.secret_updated_at,
        })
    }
}

/// The plugin's connections, oldest first.
pub(crate) fn load(conn: &Connection, id: &PluginId) -> Result<Vec<StoredConnection>> {
    let mut statement = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM plugin_connections c
         LEFT JOIN plugin_connection_secrets s
             ON s.plugin_id = c.plugin_id AND s.connection_id = c.connection_id
         WHERE c.plugin_id = ?1 ORDER BY c.created_at, c.connection_id"
    ))?;
    let rows = statement
        .query_map([id.as_str()], row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter().map(RawRow::parse).collect()
}

/// One connection of the plugin.
pub(crate) fn get(
    conn: &Connection,
    id: &PluginId,
    connection: &str,
) -> Result<Option<StoredConnection>> {
    conn.query_row(
        &format!(
            "SELECT {COLUMNS} FROM plugin_connections c
             LEFT JOIN plugin_connection_secrets s
                 ON s.plugin_id = c.plugin_id AND s.connection_id = c.connection_id
             WHERE c.plugin_id = ?1 AND c.connection_id = ?2"
        ),
        params![id.as_str(), connection],
        row,
    )
    .optional()?
    .map(RawRow::parse)
    .transpose()
}

pub(crate) fn insert(
    conn: &Connection,
    id: &PluginId,
    new: &NewConnection,
    consent_by: &str,
    now: i64,
) -> Result<()> {
    let count: i64 = conn.query_row(
        "SELECT count(*) FROM plugin_connections WHERE plugin_id = ?1",
        [id.as_str()],
        |row| row.get(0),
    )?;
    if usize::try_from(count).unwrap_or(usize::MAX) >= MAX_CONNECTIONS {
        return invalid(
            id,
            format!("a plugin holds at most {MAX_CONNECTIONS} connections"),
        );
    }
    // The first connection of a type is its default.
    let has_default: bool = conn.query_row(
        "SELECT count(*) > 0 FROM plugin_connections
         WHERE plugin_id = ?1 AND type_id = ?2 AND is_default = 1",
        params![id.as_str(), new.type_id],
        |row| row.get(0),
    )?;
    let inserted = conn.execute(
        "INSERT INTO plugin_connections (plugin_id, connection_id, type_id, display_name,
             deployment, origin, path_prefix, web_origin, auth_method, methods_json,
             private_network, enabled, status, is_default, legacy_source, consent_by,
             consent_at, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'pat', ?9, ?10, 1, 'untested', ?11, ?12,
             ?13, ?14, ?14, ?14)
         ON CONFLICT (plugin_id, connection_id) DO NOTHING",
        params![
            id.as_str(),
            new.id,
            new.type_id,
            new.display_name,
            new.deployment.as_str(),
            new.origin,
            new.path_prefix,
            new.web_origin,
            serde_json::to_string(&new.methods).map_err(StoreError::record)?,
            new.private_network,
            !has_default,
            new.legacy_source,
            consent_by,
            now,
        ],
    )?;
    if inserted == 0 {
        // Ids are never reused.
        return invalid(id, "that connection id is taken");
    }
    Ok(())
}

fn touched(changed: usize, id: &PluginId, connection: &str) -> Result<()> {
    if changed == 0 {
        Err(StoreError::InvalidData(format!(
            "{id} has no connection {connection}"
        )))
    } else {
        Ok(())
    }
}

pub(crate) fn rename(
    conn: &Connection,
    id: &PluginId,
    connection: &str,
    name: &str,
    now: i64,
) -> Result<()> {
    check_name(id, name)?;
    let changed = conn.execute(
        "UPDATE plugin_connections SET display_name = ?3, updated_at = ?4
         WHERE plugin_id = ?1 AND connection_id = ?2",
        params![id.as_str(), connection, name, now],
    )?;
    touched(changed, id, connection)
}

pub(crate) fn set_enabled(
    conn: &Connection,
    id: &PluginId,
    connection: &str,
    enabled: bool,
    now: i64,
) -> Result<()> {
    let changed = conn.execute(
        "UPDATE plugin_connections SET enabled = ?3, updated_at = ?4
         WHERE plugin_id = ?1 AND connection_id = ?2",
        params![id.as_str(), connection, enabled, now],
    )?;
    touched(changed, id, connection)
}

pub(crate) fn set_methods(
    conn: &Connection,
    id: &PluginId,
    connection: &str,
    methods: &[HttpMethod],
    now: i64,
) -> Result<()> {
    let changed = conn.execute(
        "UPDATE plugin_connections SET methods_json = ?3, updated_at = ?4
         WHERE plugin_id = ?1 AND connection_id = ?2",
        params![
            id.as_str(),
            connection,
            serde_json::to_string(methods).map_err(StoreError::record)?,
            now
        ],
    )?;
    touched(changed, id, connection)
}

pub(crate) fn set_default(
    conn: &Connection,
    id: &PluginId,
    connection: &str,
    now: i64,
) -> Result<()> {
    let type_id: Option<String> = conn
        .query_row(
            "SELECT type_id FROM plugin_connections WHERE plugin_id = ?1 AND connection_id = ?2",
            params![id.as_str(), connection],
            |row| row.get(0),
        )
        .optional()?;
    let Some(type_id) = type_id else {
        return touched(0, id, connection);
    };
    conn.execute(
        "UPDATE plugin_connections SET is_default = 0
         WHERE plugin_id = ?1 AND type_id = ?2 AND is_default = 1",
        params![id.as_str(), type_id],
    )?;
    conn.execute(
        "UPDATE plugin_connections SET is_default = 1, updated_at = ?3
         WHERE plugin_id = ?1 AND connection_id = ?2",
        params![id.as_str(), connection, now],
    )?;
    Ok(())
}

/// Writes what a test or a request found. A plugin's report never touches
/// the account or the capabilities (`capabilities: None` keeps them).
pub(crate) fn set_probe(
    conn: &Connection,
    id: &PluginId,
    connection: &str,
    probe: &ConnectionProbe,
    now: i64,
) -> Result<()> {
    check_probe(probe)?;
    let capabilities = probe
        .capabilities
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(StoreError::record)?;
    let changed = conn.execute(
        "UPDATE plugin_connections SET status = ?3, status_detail = ?4,
             account_label = coalesce(?5, account_label),
             capabilities_json = coalesce(?6, capabilities_json), updated_at = ?7
         WHERE plugin_id = ?1 AND connection_id = ?2",
        params![
            id.as_str(),
            connection,
            probe.status.as_str(),
            probe.detail,
            probe.account_label,
            capabilities,
            now
        ],
    )?;
    touched(changed, id, connection)
}

/// Deletes the connection and its secret; whether it existed. A default
/// passes to the oldest other connection of its type.
pub(crate) fn delete(conn: &Connection, id: &PluginId, connection: &str) -> Result<bool> {
    let type_id: Option<(String, bool)> = conn
        .query_row(
            "SELECT type_id, is_default FROM plugin_connections
             WHERE plugin_id = ?1 AND connection_id = ?2",
            params![id.as_str(), connection],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    conn.execute(
        "DELETE FROM plugin_connection_secrets WHERE plugin_id = ?1 AND connection_id = ?2",
        params![id.as_str(), connection],
    )?;
    let deleted = conn.execute(
        "DELETE FROM plugin_connections WHERE plugin_id = ?1 AND connection_id = ?2",
        params![id.as_str(), connection],
    )? > 0;
    if let Some((type_id, true)) = type_id {
        conn.execute(
            "UPDATE plugin_connections SET is_default = 1
             WHERE plugin_id = ?1 AND connection_id = (
                 SELECT connection_id FROM plugin_connections
                 WHERE plugin_id = ?1 AND type_id = ?2
                 ORDER BY created_at, connection_id LIMIT 1)",
            params![id.as_str(), type_id],
        )?;
    }
    Ok(deleted)
}

/// Stores the secret and marks the connection untested: a new secret is
/// tested before it counts as working.
pub(crate) fn set_secret(
    conn: &Connection,
    id: &PluginId,
    connection: &str,
    secret: &str,
    now: i64,
) -> Result<()> {
    check_secret(secret)?;
    let changed = conn.execute(
        "UPDATE plugin_connections SET status = 'untested', status_detail = NULL,
             updated_at = ?3
         WHERE plugin_id = ?1 AND connection_id = ?2",
        params![id.as_str(), connection, now],
    )?;
    touched(changed, id, connection)?;
    conn.execute(
        "INSERT INTO plugin_connection_secrets (plugin_id, connection_id, secret, updated_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (plugin_id, connection_id) DO UPDATE SET secret = excluded.secret,
             updated_at = excluded.updated_at",
        params![id.as_str(), connection, secret, now],
    )?;
    Ok(())
}

/// What importing an old plugin's server and token as a connection did
/// (Git Providers design §11.2, PR-10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyImport {
    /// The connection was made. `with_secret`: the old plugin's token was
    /// copied into it; without one it waits for a token (`needsReconnect`).
    Created {
        connection: String,
        with_secret: bool,
    },
    /// The old plugin's settings are a connection already (its
    /// `legacy_source`): nothing was written. `has_secret`: it has a token
    /// (a later import never adds one).
    Existing {
        connection: String,
        has_secret: bool,
    },
}

/// The oldest connection of `id` that `source`'s settings became.
pub(crate) fn by_legacy_source(
    conn: &Connection,
    id: &PluginId,
    source: &PluginId,
) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT connection_id FROM plugin_connections
             WHERE plugin_id = ?1 AND legacy_source = ?2
             ORDER BY created_at, connection_id LIMIT 1",
            params![id.as_str(), source.as_str()],
            |row| row.get(0),
        )
        .optional()?)
}

/// Makes `new` the connection `source`'s settings become, in the caller's
/// transaction: nothing when there is one already (idempotent), else the
/// connection, read-only as the old plugin was, and, only with
/// `secret_slot` (the user approved the copy, never derived), the old
/// plugin's token copied into it. The old plugin's credential row is only
/// read, never changed or deleted. Any failure leaves the transaction to
/// be rolled back, so nothing of the import is kept.
pub(crate) fn import_legacy(
    conn: &Connection,
    id: &PluginId,
    source: &PluginId,
    new: &NewConnection,
    secret_slot: Option<&str>,
    consent_by: &str,
    now: i64,
) -> Result<LegacyImport> {
    if new.legacy_source.as_deref() != Some(source.as_str()) {
        return invalid(
            id,
            format!("an import from {source} names it as its source"),
        );
    }
    if new.methods != [HttpMethod::Get] {
        return invalid(id, "an import reads only, as the old plugin did");
    }
    if let Some(connection) = by_legacy_source(conn, id, source)? {
        let has_secret = secret(conn, id, &connection)?.is_some();
        return Ok(LegacyImport::Existing {
            connection,
            has_secret,
        });
    }
    insert(conn, id, new, consent_by, now)?;
    let secret = match secret_slot {
        Some(slot) => crate::data::credential_secret(conn, source, slot)?,
        None => None,
    };
    if let Some(secret) = secret {
        set_secret(conn, id, &new.id, secret.expose(), now)?;
        return Ok(LegacyImport::Created {
            connection: new.id.clone(),
            with_secret: true,
        });
    }
    conn.execute(
        "UPDATE plugin_connections SET status = 'needsReconnect', updated_at = ?3
         WHERE plugin_id = ?1 AND connection_id = ?2",
        params![id.as_str(), new.id, now],
    )?;
    Ok(LegacyImport::Created {
        connection: new.id.clone(),
        with_secret: false,
    })
}

/// The secret of `connection`, for injection into one request.
pub(crate) fn secret(conn: &Connection, id: &PluginId, connection: &str) -> Result<Option<Secret>> {
    let secret: Option<Zeroizing<String>> = conn
        .query_row(
            "SELECT secret FROM plugin_connection_secrets
             WHERE plugin_id = ?1 AND connection_id = ?2",
            params![id.as_str(), connection],
            |row| row.get::<_, String>(0).map(Zeroizing::new),
        )
        .optional()?;
    Ok(secret.map(|s| Secret::new(s.as_str())))
}

/// Deletes every connection and secret of the plugin (an uninstall that
/// does not keep its credentials).
pub(crate) fn delete_all(conn: &Connection, id: &PluginId) -> Result<()> {
    conn.execute(
        "DELETE FROM plugin_connection_secrets WHERE plugin_id = ?1",
        [id.as_str()],
    )?;
    conn.execute(
        "DELETE FROM plugin_connections WHERE plugin_id = ?1",
        [id.as_str()],
    )?;
    Ok(())
}

#[cfg(all(test, feature = "sqlcipher"))]
mod tests {
    use super::*;
    use crate::db::{DEFAULT_BUSY_TIMEOUT, Database, StoreKey};

    type Edit = Box<dyn Fn(&mut NewConnection)>;

    const ID: &str = "cn_01JD7Q0000000000000000000A";
    const OTHER: &str = "cn_01JD7Q0000000000000000000B";

    fn manifest() -> Manifest {
        Manifest::from_json(include_str!(
            "../../deckpilot-plugin-manifest/tests/fixtures/connections-manifest.json"
        ))
        .unwrap()
    }

    fn plugin() -> PluginId {
        manifest().id
    }

    fn open(dir: &tempfile::TempDir) -> Database {
        Database::open(
            &dir.path().join("plugins.db"),
            &StoreKey::from_bytes([0x44; 32]),
            DEFAULT_BUSY_TIMEOUT,
        )
        .unwrap()
    }

    fn self_hosted(id: &str) -> NewConnection {
        NewConnection {
            id: id.into(),
            type_id: "gitlab".into(),
            display_name: "Work GitLab".into(),
            deployment: Deployment::SelfHosted,
            origin: "https://code.example.com:8443".into(),
            path_prefix: Some("/gitlab/api/v4".into()),
            web_origin: "https://code.example.com:8443".into(),
            methods: vec![HttpMethod::Get],
            private_network: true,
            legacy_source: None,
        }
    }

    fn cloud(id: &str) -> NewConnection {
        NewConnection {
            id: id.into(),
            type_id: "github".into(),
            display_name: "octo @ github.com".into(),
            deployment: Deployment::Cloud,
            origin: "https://api.github.com".into(),
            path_prefix: None,
            web_origin: "https://github.com".into(),
            methods: vec![HttpMethod::Get],
            private_network: false,
            legacy_source: None,
        }
    }

    #[test]
    fn ids_names_and_secrets_have_their_shapes() {
        assert!(is_connection_id(ID));
        for bad in [
            "",
            "cn_",
            "cn_01jd7q0000000000000000000a",
            "xx_01JD7Q0000000000000000000A",
        ] {
            assert!(!is_connection_id(bad), "{bad}");
        }
        // Built from pieces, so no literal here has a real token's shape.
        let shaped = |prefix: [&str; 2], rest: &str| format!("{}{}{rest}", prefix[0], prefix[1]);
        for (case, secret) in [
            &shaped(["gh", "p_"], "0123456789abcdefghijABCDEFGHIJ012345"),
            &shaped(["github", "_pat_"], "11AAAA"),
            &format!(
                "my {} token",
                shaped(["gl", "pat-"], "abcdefghijklmnopqrst")
            ),
            "0123456789abcdef0123456789abcdef",
            "QUJDREVGR0hJSktMTU5PUFFSU1RVVldYWVo=",
        ]
        .iter()
        .enumerate()
        {
            assert!(looks_like_secret(secret), "secret shape {case}");
        }
        for (case, name) in [
            "octo @ github.com",
            "Work GitLab",
            "ana.kowalska @ code.example.com",
        ]
        .iter()
        .enumerate()
        {
            assert!(!looks_like_secret(name), "name {case}");
        }
        check_secret("dp-test-abc").unwrap();
        for bad in [
            "",
            "a\nb",
            "a\rb",
            "a\0b",
            "a\tb",
            &"x".repeat(MAX_SECRET_BYTES + 1),
        ] {
            assert!(check_secret(bad).is_err());
        }
    }

    /// A connection is checked against the manifest: declared and listed
    /// types, methods within the permission, cloud at the fixed origins
    /// only, a self-hosted API under the type's path on the same server,
    /// a private network only where allowed, and a name that is no token.
    #[test]
    fn a_connection_must_fit_the_manifest() {
        let m = manifest();
        let p = plugin();
        check(&p, &m, &self_hosted(ID)).unwrap();
        check(&p, &m, &cloud(ID)).unwrap();
        let refused: Vec<(&str, Edit)> = vec![
            ("bad id", Box::new(|c| c.id = "cn_1".into())),
            (
                "undeclared type",
                Box::new(|c| c.type_id = "bitbucket".into()),
            ),
            (
                "token as a name",
                Box::new(|c| c.display_name = format!("{}{}", "glp", "at-abcdefghijklmnopqrstuv")),
            ),
            ("empty name", Box::new(|c| c.display_name = String::new())),
            ("long name", Box::new(|c| c.display_name = "n".repeat(65))),
            (
                "newline in name",
                Box::new(|c| c.display_name = "a\nb".into()),
            ),
            (
                "DELETE",
                Box::new(|c| c.methods = vec![HttpMethod::Get, HttpMethod::Delete]),
            ),
            ("no methods", Box::new(|c| c.methods = Vec::new())),
            (
                "http",
                Box::new(|c| c.origin = "http://code.example.com".into()),
            ),
            (
                "userinfo",
                Box::new(|c| c.origin = "https://u@code.example.com".into()),
            ),
            (
                "path in origin",
                Box::new(|c| c.origin = "https://code.example.com/x".into()),
            ),
            (
                "upper case",
                Box::new(|c| c.origin = "https://Code.example.com".into()),
            ),
            (
                "other web host",
                Box::new(|c| c.web_origin = "https://evil.example".into()),
            ),
            (
                "API not under apiPath",
                Box::new(|c| c.path_prefix = Some("/api/v3".into())),
            ),
            ("no API path", Box::new(|c| c.path_prefix = None)),
            (
                "dot segment",
                Box::new(|c| c.path_prefix = Some("/a/../api/v4".into())),
            ),
            (
                "encoded",
                Box::new(|c| c.path_prefix = Some("/a%2Fb/api/v4".into())),
            ),
        ];
        for (what, edit) in &refused {
            let mut c = self_hosted(ID);
            edit(&mut c);
            assert!(check(&p, &m, &c).is_err(), "{what}");
        }
        for (what, edit) in [
            (
                "other cloud host",
                Box::new(|c: &mut NewConnection| c.origin = "https://api.evil.example".into())
                    as Box<dyn Fn(&mut NewConnection)>,
            ),
            (
                "other cloud web",
                Box::new(|c| c.web_origin = "https://evil.example".into()),
            ),
            (
                "cloud prefix",
                Box::new(|c| c.path_prefix = Some("/api".into())),
            ),
            ("private cloud", Box::new(|c| c.private_network = true)),
        ] {
            let mut c = cloud(ID);
            edit(&mut c);
            assert!(check(&p, &m, &c).is_err(), "{what}");
        }
        // Without the private network in the permission, never.
        let mut json: serde_json::Value = serde_json::from_str(include_str!(
            "../../deckpilot-plugin-manifest/tests/fixtures/connections-manifest.json"
        ))
        .unwrap();
        json["permissions"][0]
            .as_object_mut()
            .unwrap()
            .remove("privateNetwork");
        let narrow = Manifest::from_json(&json.to_string()).unwrap();
        assert!(check(&p, &narrow, &self_hosted(ID)).is_err());
    }

    /// Rows and secrets: the first of a type is the default, the secret is
    /// never read back with the row (only when it was stored), a new
    /// secret makes the connection untested, removing deletes both and
    /// passes the default on, and an id is never reused.
    #[test]
    fn connections_and_their_secrets_are_stored_apart() {
        let dir = tempfile::tempdir().unwrap();
        let mut db = open(&dir);
        let p = plugin();
        let tx = db.write().unwrap();
        insert(&tx, &p, &self_hosted(ID), "user", 10).unwrap();
        let mut second = self_hosted(OTHER);
        second.display_name = "Second GitLab".into();
        insert(&tx, &p, &second, "user", 11).unwrap();
        assert!(insert(&tx, &p, &self_hosted(ID), "user", 12).is_err());
        tx.commit().unwrap();

        let rows = load(db.connection(), &p).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows[0].is_default && !rows[1].is_default);
        assert_eq!(rows[0].status, ConnectionStatus::Untested);
        assert!(!rows[0].has_secret());
        assert_eq!(
            rows[0].api_base(),
            "https://code.example.com:8443/gitlab/api/v4"
        );

        let tx = db.write().unwrap();
        set_probe(
            &tx,
            &p,
            ID,
            &ConnectionProbe {
                status: ConnectionStatus::Ok,
                detail: None,
                account_label: Some("ana".into()),
                capabilities: Some(serde_json::json!({"pipelines": true})),
            },
            13,
        )
        .unwrap();
        set_secret(&tx, &p, ID, "dp-test-secret-value", 14).unwrap();
        tx.commit().unwrap();
        let row = get(db.connection(), &p, ID).unwrap().unwrap();
        assert!(row.has_secret());
        // A new secret is tested again; the account stays until then.
        assert_eq!(row.status, ConnectionStatus::Untested);
        assert_eq!(row.account_label.as_deref(), Some("ana"));
        assert!(!format!("{row:?}").contains("dp-test-secret-value"));
        assert_eq!(
            secret(db.connection(), &p, ID).unwrap().unwrap().expose(),
            "dp-test-secret-value"
        );
        assert!(secret(db.connection(), &p, OTHER).unwrap().is_none());

        // A plugin's report never sets an account or a server's text.
        let tx = db.write().unwrap();
        for bad in [
            ConnectionProbe {
                status: ConnectionStatus::Unauthorized,
                detail: Some("Bad credentials".into()),
                account_label: None,
                capabilities: None,
            },
            ConnectionProbe {
                status: ConnectionStatus::Ok,
                detail: None,
                account_label: Some(format!(
                    "{}{}",
                    "gh", "p_0123456789abcdefghijABCDEFGHIJ0123"
                )),
                capabilities: None,
            },
        ] {
            assert!(set_probe(&tx, &p, ID, &bad, 15).is_err());
        }
        set_default(&tx, &p, OTHER, 16).unwrap();
        assert!(delete(&tx, &p, OTHER).unwrap());
        assert!(!delete(&tx, &p, OTHER).unwrap());
        tx.commit().unwrap();
        let rows = load(db.connection(), &p).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].is_default, "the default passes on");

        let tx = db.write().unwrap();
        assert!(delete(&tx, &p, ID).unwrap());
        tx.commit().unwrap();
        assert!(secret(db.connection(), &p, ID).unwrap().is_none());
        let leftover: i64 = db
            .connection()
            .query_row("SELECT count(*) FROM plugin_connection_secrets", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(leftover, 0);
    }

    /// An uninstall that does not keep the credentials deletes the
    /// connections and their secrets; one that keeps them keeps both.
    #[test]
    fn connections_go_with_the_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let mut db = open(&dir);
        let p = plugin();
        let tx = db.write().unwrap();
        insert(&tx, &p, &self_hosted(ID), "user", 10).unwrap();
        set_secret(&tx, &p, ID, "dp-test-secret-value", 11).unwrap();
        crate::data::delete_plugin_data(
            &tx,
            &p,
            crate::data::Retention {
                settings: false,
                credentials: true,
            },
        )
        .unwrap();
        tx.commit().unwrap();
        assert!(secret(db.connection(), &p, ID).unwrap().is_some());
        let tx = db.write().unwrap();
        crate::data::delete_plugin_data(&tx, &p, crate::data::Retention::default()).unwrap();
        tx.commit().unwrap();
        assert_eq!(load(db.connection(), &p).unwrap().len(), 0);
        assert!(secret(db.connection(), &p, ID).unwrap().is_none());
    }

    fn legacy(id: &str) -> NewConnection {
        let mut new = self_hosted(id);
        new.legacy_source = Some("deckpilot.gitlab".into());
        new
    }

    fn source() -> PluginId {
        PluginId::parse("deckpilot.gitlab").unwrap()
    }

    /// The old plugin's token is copied into the new connection in the
    /// same transaction, its own row kept; a second import changes
    /// nothing; without the approved slot (or without a token there) the
    /// connection waits for one.
    #[test]
    fn an_import_copies_the_token_once_and_keeps_the_old_one() {
        let dir = tempfile::tempdir().unwrap();
        let mut db = open(&dir);
        let p = plugin();
        let tx = db.write().unwrap();
        crate::data::set_credential(&tx, &source(), "token", "dp-test-legacy-token", 5).unwrap();
        let first = import_legacy(
            &tx,
            &p,
            &source(),
            &legacy(ID),
            Some("token"),
            "user:migration",
            10,
        )
        .unwrap();
        assert_eq!(
            first,
            LegacyImport::Created {
                connection: ID.into(),
                with_secret: true
            }
        );
        // Idempotent: the same or another new connection, nothing written.
        for again in [legacy(ID), legacy(OTHER)] {
            let outcome = import_legacy(
                &tx,
                &p,
                &source(),
                &again,
                Some("token"),
                "user:migration",
                20,
            )
            .unwrap();
            assert_eq!(
                outcome,
                LegacyImport::Existing {
                    connection: ID.into(),
                    has_secret: true
                }
            );
        }
        tx.commit().unwrap();
        let rows = load(db.connection(), &p).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].legacy_source.as_deref(), Some("deckpilot.gitlab"));
        assert_eq!(rows[0].methods, vec![HttpMethod::Get]);
        assert_eq!(rows[0].status, ConnectionStatus::Untested);
        assert_eq!(
            secret(db.connection(), &p, ID)
                .unwrap()
                .map(|s| s.expose().to_owned())
                .as_deref(),
            Some("dp-test-legacy-token")
        );
        // The old plugin's token stays where it was.
        assert!(
            crate::data::credential_secret(db.connection(), &source(), "token")
                .unwrap()
                .is_some()
        );

        // Not approved, or nothing stored: the connection waits for a token.
        let dir = tempfile::tempdir().unwrap();
        let mut db = open(&dir);
        let tx = db.write().unwrap();
        crate::data::set_credential(&tx, &source(), "token", "dp-test-legacy-token", 5).unwrap();
        let outcome =
            import_legacy(&tx, &p, &source(), &legacy(ID), None, "user:migration", 10).unwrap();
        assert_eq!(
            outcome,
            LegacyImport::Created {
                connection: ID.into(),
                with_secret: false
            }
        );
        tx.commit().unwrap();
        let rows = load(db.connection(), &p).unwrap();
        assert_eq!(rows[0].status, ConnectionStatus::NeedsReconnect);
        assert!(secret(db.connection(), &p, ID).unwrap().is_none());
        // A later import never adds the token, and says it has none.
        let tx = db.write().unwrap();
        let again = import_legacy(
            &tx,
            &p,
            &source(),
            &legacy(OTHER),
            Some("token"),
            "user:migration",
            30,
        )
        .unwrap();
        assert_eq!(
            again,
            LegacyImport::Existing {
                connection: ID.into(),
                has_secret: false
            }
        );
        drop(tx);
        assert!(secret(db.connection(), &p, ID).unwrap().is_none());
    }

    /// A failure after the connection was written (a token that cannot
    /// travel in a header) rolls the whole import back: no connection, no
    /// secret, the old token untouched; a later import starts afresh.
    #[test]
    fn a_failed_import_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let mut db = open(&dir);
        let p = plugin();
        let tx = db.write().unwrap();
        crate::data::set_credential(&tx, &source(), "token", "dp-test\tlegacy", 5).unwrap();
        tx.commit().unwrap();
        {
            let tx = db.write().unwrap();
            assert!(
                import_legacy(
                    &tx,
                    &p,
                    &source(),
                    &legacy(ID),
                    Some("token"),
                    "user:migration",
                    10
                )
                .is_err()
            );
            // Dropped without a commit: rolled back.
        }
        assert_eq!(load(db.connection(), &p).unwrap(), Vec::new());
        assert!(secret(db.connection(), &p, ID).unwrap().is_none());
        assert!(
            crate::data::credential_secret(db.connection(), &source(), "token")
                .unwrap()
                .is_some()
        );
        // Wrong source, or anything but reading: refused before writing.
        let tx = db.write().unwrap();
        let mut other = legacy(ID);
        other.legacy_source = Some("deckpilot.github".into());
        assert!(import_legacy(&tx, &p, &source(), &other, None, "user:migration", 10).is_err());
        let mut writes = legacy(ID);
        writes.methods = vec![HttpMethod::Get, HttpMethod::Post];
        assert!(import_legacy(&tx, &p, &source(), &writes, None, "user:migration", 10).is_err());
        assert_eq!(load(&tx, &p).unwrap(), Vec::new());
    }
}
