//! Connections in the agent (plugin API 1.14,
//! `docs/marketplace/sdk/permissions.md` "Connections"): the IPC that
//! creates, tests, changes and removes them, the checks of the addresses
//! the user enters, the grant records the broker gets, and the binding of
//! an invocation to the connections its parameters name.
//!
//! **Addresses.** What the user enters is the address they open in the
//! browser. It is parsed and normalized by the broker's own parser
//! (`UserOrigin::parse`): `https` only, no user name or password, no query
//! or fragment, no `.`/`..` or encoded `/` in the path, punycode host,
//! default port dropped. The API base is that address plus the type's
//! `apiPath` (`/api/v4`); a pasted API address (one already ending in it)
//! is taken as the API. A cloud connection always gets the manifest's own
//! addresses. A server named by an IP literal or a local name is on a
//! private network and needs the user's separate consent; this computer
//! (`localhost`, loopback), link-local and metadata addresses are never a
//! connection's server, consent or not. A public name that resolves to a
//! private address is refused at request time unless the consent was given
//! (the executor's address policy). TLS is the operating system's trust
//! store; nothing here or anywhere else turns verification off.
//!
//! **Grants.** A connection reaches the broker only while the plugin is
//! enabled, its `connections` grant is granted, the active manifest still
//! declares the connection's type in that permission, and with the methods
//! and private network that both the user and the grant allow now.
//!
//! **Binding.** An invocation is bound to the connection ids its
//! `connection` parameters hold (never a dynamic reference: the manifest
//! rules refuse one), or to the one the app names for a test. The broker
//! refuses every other id.
//!
//! **Secrets.** A token arrives over the authenticated local IPC, is
//! stored in the encrypted plugin database and is never sent back, logged
//! or handed to the plugin.

use std::collections::{BTreeMap, BTreeSet};

use deckpilot_ipc::connections as wire;
use deckpilot_ipc::{ErrorCode, IpcError, Request, Response};
use deckpilot_plugin_abi::Params;
use deckpilot_plugin_abi::value::ParamValue;
use deckpilot_plugin_broker::{ApprovedPermission, UserOrigin};
use deckpilot_plugin_manifest::{
    Action, CredentialScheme, HttpMethod, Manifest, ParameterKind, PermissionKind, PluginId,
};
use deckpilot_plugin_store::{
    ConnectionProbe, ConnectionStatus, Deployment, GrantKey, GrantScope, NewConnection,
    OPEN_URL_CONNECTIONS_KEY, StoredConnection,
};
use tracing::info;

use super::{PluginService, error, invalid, store_error};

/// The id of the internal action that tests a connection: it takes the
/// `connection` parameter and answers with what it found.
pub(super) const PROBE_ACTION: &str = "probe";

/// A new connection id: `cn_` and a ULID (48 bits of milliseconds, 80
/// random bits from the operating system) in Crockford base32.
pub(super) fn new_id() -> Result<String, IpcError> {
    const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let mut random = [0u8; 10];
    getrandom::fill(&mut random).map_err(|_| {
        IpcError::new(
            ErrorCode::Failed,
            "the system's random source is not available",
        )
    })?;
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis())
        & ((1u128 << 48) - 1);
    let mut value = millis << 80;
    for (i, byte) in random.iter().enumerate() {
        value |= u128::from(*byte) << (8 * (9 - i));
    }
    let mut body = [0u8; 26];
    for (i, slot) in body.iter_mut().enumerate() {
        let shift = 5 * (25 - i);
        *slot = ALPHABET[usize::try_from((value >> shift) & 31).unwrap_or(0)];
    }
    Ok(format!(
        "cn_{}",
        std::str::from_utf8(&body).unwrap_or_default()
    ))
}

/// Where a new connection's requests go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Address {
    pub origin: String,
    pub path_prefix: Option<String>,
    pub web_origin: String,
    /// The host is an IP literal or a local name.
    pub private_host: bool,
}

fn problem(error: deckpilot_plugin_broker::OriginError) -> String {
    use deckpilot_plugin_broker::OriginError as E;
    match error {
        E::SchemeNotAllowed => "must start with https:// (DeckPilot never sends a token over \
                                plain http)"
            .into(),
        E::UserinfoNotAllowed => "must not contain a user name or password".into(),
        E::NotAnOrigin => "must not have a query or fragment".into(),
        E::IpLiteralNotAllowed => "is this computer, a link-local or a special address, which \
                                   DeckPilot never connects to"
            .into(),
        E::InvalidHost => "does not name a server DeckPilot can reach".into(),
        E::InvalidPath => "has a path with `.` or `..`, empty segments or encoded slashes".into(),
        E::TooLong => "is too long".into(),
        E::IllegalCharacters => "contains spaces, control characters or `\\`".into(),
        E::Unparsable => "is not an address".into(),
    }
}

/// The addresses of a new connection of `ty`: the manifest's own for the
/// cloud; for a self-hosted server, `url` (the address the user opens in
/// the browser, or its API address) normalized, with the type's API path.
pub(super) fn address(
    ty: &deckpilot_plugin_manifest::ConnectionType,
    deployment: Deployment,
    url: Option<&str>,
) -> Result<Address, String> {
    match deployment {
        Deployment::Cloud => {
            let cloud = ty
                .cloud
                .as_ref()
                .ok_or_else(|| format!("{} has no public service", ty.id))?;
            let api = UserOrigin::parse(&cloud.api).map_err(problem)?;
            let web = UserOrigin::parse(&cloud.web).map_err(problem)?;
            Ok(Address {
                origin: api.origin().to_string(),
                path_prefix: api.path_prefix().map(str::to_owned),
                web_origin: web.origin().to_string(),
                private_host: false,
            })
        }
        Deployment::SelfHosted => {
            let hosted = ty
                .self_hosted
                .as_ref()
                .ok_or_else(|| format!("{} has no self-hosted servers", ty.id))?;
            let url = url
                .map(str::trim)
                .filter(|u| !u.is_empty())
                .ok_or("enter the server's address")?;
            let entered = UserOrigin::parse(url.trim_end_matches('/'))
                .map_err(|e| format!("the address {}", problem(e)))?;
            let base = entered.path_prefix().unwrap_or("");
            // A pasted API address is the API; anything else is where the
            // pages are, with the API under it.
            let prefix = if base.ends_with(hosted.api_path.as_str()) {
                base.to_owned()
            } else {
                format!("{base}{}", hosted.api_path)
            };
            // Read back through the parser, so the stored prefix is in
            // exactly the form the broker compares.
            let api = UserOrigin::parse(&format!("{}{prefix}", entered.origin()))
                .map_err(|e| format!("the address {}", problem(e)))?;
            Ok(Address {
                origin: api.origin().to_string(),
                path_prefix: api.path_prefix().map(str::to_owned),
                web_origin: api.origin().to_string(),
                private_host: api.private_host(),
            })
        }
    }
}

/// The connection ids an invocation of `action` with `params` is bound to:
/// the values of its `connection` parameters that are connection ids.
pub(super) fn bound(action: &Action, params: &Params) -> BTreeSet<String> {
    action
        .parameters
        .iter()
        .filter(|p| matches!(p.kind, ParameterKind::Connection { .. }))
        .filter_map(|p| match params.get(&p.key) {
            Some(ParamValue::String(id)) if deckpilot_plugin_store::is_connection_id(id) => {
                Some(id.clone())
            }
            _ => None,
        })
        .collect()
}

/// The header a connection type's secret travels in, if it has one.
fn header_name(manifest: &Manifest, type_id: &str) -> Option<String> {
    manifest
        .contributes
        .connection_types()
        .iter()
        .find(|t| t.id == type_id)
        .and_then(|t| t.auth.first())
        .filter(|a| a.scheme == CredentialScheme::Header)
        .and_then(|a| a.header_name.clone())
}

fn abi_method(method: HttpMethod) -> deckpilot_plugin_abi::HttpMethod {
    use deckpilot_plugin_abi::HttpMethod as A;
    match method {
        HttpMethod::Get => A::Get,
        HttpMethod::Head => A::Head,
        HttpMethod::Post => A::Post,
        HttpMethod::Put => A::Put,
        HttpMethod::Patch => A::Patch,
        HttpMethod::Delete => A::Delete,
    }
}

/// The broker records of the plugin's connections: none unless the
/// `connections` grant is effective; each within what the grant allows
/// now (its types, methods and private network).
pub(super) fn approved(
    manifest: &Manifest,
    effective: &[(GrantKey, GrantScope)],
    rows: &[StoredConnection],
) -> Vec<ApprovedPermission> {
    let Some((types, methods, private)) = effective.iter().find_map(|(key, scope)| match scope {
        GrantScope::Connections {
            types,
            methods,
            private_network,
        } if key.kind == PermissionKind::Connections => Some((types, methods, *private_network)),
        _ => None,
    }) else {
        return Vec::new();
    };
    let declared = deckpilot_plugin_store::declared_connections(manifest);
    let open_url = effective
        .iter()
        .any(|(k, _)| k.kind == PermissionKind::OpenUrl && k.key == OPEN_URL_CONNECTIONS_KEY);
    rows.iter()
        .filter(|row| {
            types.contains(&row.type_id)
                && declared
                    .as_ref()
                    .is_some_and(|(declared, _, _)| declared.contains(&row.type_id))
        })
        .filter(|row| private || !row.private_network)
        .filter_map(|row| {
            let methods: BTreeSet<_> = row
                .methods
                .iter()
                .filter(|m| methods.contains(m))
                .copied()
                .map(abi_method)
                .collect();
            (!methods.is_empty()).then(|| ApprovedPermission::Connection {
                id: row.id.clone(),
                origin: row.origin.clone(),
                path_prefix: row.path_prefix.clone(),
                web_origin: row.web_origin.clone(),
                methods,
                private_network: row.private_network,
                enabled: row.enabled,
                open_url,
                header_name: header_name(manifest, &row.type_id),
            })
        })
        .collect()
}

fn state(row: &StoredConnection) -> wire::ConnectionState {
    if !row.enabled {
        return wire::ConnectionState::Disabled;
    }
    match row.status {
        ConnectionStatus::Ok => wire::ConnectionState::Ok,
        ConnectionStatus::Unauthorized => wire::ConnectionState::Unauthorized,
        ConnectionStatus::InsufficientScope => wire::ConnectionState::InsufficientScope,
        ConnectionStatus::Unreachable => wire::ConnectionState::Unreachable,
        ConnectionStatus::TlsFailed => wire::ConnectionState::TlsFailed,
        ConnectionStatus::NeedsReconnect => wire::ConnectionState::NeedsReconnect,
        ConnectionStatus::Untested => wire::ConnectionState::Untested,
    }
}

/// A row as the app sees it: never the secret.
pub(super) fn view(row: &StoredConnection) -> wire::PluginConnection {
    wire::PluginConnection {
        id: row.id.clone(),
        type_id: row.type_id.clone(),
        display_name: row.display_name.clone(),
        deployment: match row.deployment {
            Deployment::Cloud => wire::ConnectionDeployment::Cloud,
            Deployment::SelfHosted => wire::ConnectionDeployment::SelfHosted,
        },
        api_url: row.api_base(),
        web_url: row.web_origin.clone(),
        allow_changes: row.methods.iter().any(|m| *m != HttpMethod::Get),
        private_network: row.private_network,
        enabled: row.enabled,
        state: state(row),
        state_detail: row.status_detail.clone(),
        account: row.account_label.clone(),
        capabilities: row.capabilities.clone(),
        is_default: row.is_default,
        has_secret: row.has_secret(),
        secret_updated_at: row.secret_updated_at,
        created_at: row.created_at,
    }
}

fn types(manifest: &Manifest) -> Vec<wire::PluginConnectionType> {
    manifest
        .contributes
        .connection_types()
        .iter()
        .map(|t| wire::PluginConnectionType {
            id: t.id.clone(),
            label: t.label.clone(),
            cloud_api_url: t.cloud.as_ref().map(|c| c.api.clone()),
            cloud_web_url: t.cloud.as_ref().map(|c| c.web.clone()),
            self_hosted: t.self_hosted.is_some(),
            self_hosted_label: t.self_hosted.as_ref().and_then(|h| h.label.clone()),
            self_hosted_api_path: t.self_hosted.as_ref().map(|h| h.api_path.clone()),
            auth: t
                .auth
                .iter()
                .map(|a| wire::ConnectionTypeAuth {
                    method: "pat".into(),
                    cloud_setup_url: a.setup_url.as_ref().and_then(|u| u.cloud.clone()),
                    self_hosted_setup_url: a.setup_url.as_ref().and_then(|u| u.self_hosted.clone()),
                })
                .collect(),
        })
        .collect()
}

/// The methods a connection gets: `GET`, or with changes allowed every
/// method the plugin's permission lists.
fn methods(manifest: &Manifest, allow_changes: bool) -> Vec<HttpMethod> {
    if allow_changes {
        deckpilot_plugin_store::declared_connections(manifest)
            .map_or_else(|| vec![HttpMethod::Get], |(_, methods, _)| methods)
    } else {
        vec![HttpMethod::Get]
    }
}

/// What a failed test's code means for the connection.
fn failed_status(code: &str, message: &str) -> ConnectionStatus {
    let lower = message.to_ascii_lowercase();
    match code {
        "unauthorized" => ConnectionStatus::Unauthorized,
        "insufficient-scope" | "forbidden" => ConnectionStatus::InsufficientScope,
        "tls-failed" => ConnectionStatus::TlsFailed,
        _ if lower.contains("certificate") || lower.contains("tls") => ConnectionStatus::TlsFailed,
        "unreachable" | "timed-out" | "dns-failed" | "connection-failed" => {
            ConnectionStatus::Unreachable
        }
        _ => ConnectionStatus::Untested,
    }
}

/// A detail code from a failure code: lower case, digits and `-` only.
fn detail(code: &str) -> Option<String> {
    let ok = code.len() <= 48
        && code.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
        && code
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    ok.then(|| code.to_owned())
}

/// What a successful probe answered, as the stored probe: the account and
/// what the server and token can do (no secrets; only the scalar outputs
/// and the record's capabilities object).
fn probe_of(data: &BTreeMap<String, serde_json::Value>) -> ConnectionProbe {
    let text = |key: &str| data.get(key).and_then(|v| v.as_str()).map(str::to_owned);
    let account = text("account").filter(|a| {
        let chars = a.chars().count();
        chars > 0
            && chars <= deckpilot_plugin_store::MAX_ACCOUNT_LABEL
            && !a.chars().any(char::is_control)
            && !deckpilot_plugin_store::looks_like_secret(a)
    });
    let mut capabilities = serde_json::Map::new();
    if let Some(version) = text("version") {
        capabilities.insert("version".into(), version.into());
    }
    if let Some(can_write) = data.get("canWrite").and_then(serde_json::Value::as_bool) {
        capabilities.insert("canWrite".into(), can_write.into());
    }
    if let Some(excess) = text("excessScopes").filter(|s| !s.is_empty()) {
        capabilities.insert("excessScopes".into(), excess.into());
    }
    // The record's own capabilities (a JSON object as text).
    let record = data
        .get("items")
        .and_then(|v| v.as_array())
        .and_then(|items| items.first())
        .and_then(|first| match first {
            serde_json::Value::String(text) => serde_json::from_str(text).ok(),
            other => Some(other.clone()),
        });
    if let Some(serde_json::Value::Object(record)) = record {
        if let Some(found) = record
            .get("capabilities")
            .and_then(|c| c.as_str())
            .and_then(|c| serde_json::from_str::<serde_json::Value>(c).ok())
            .filter(serde_json::Value::is_object)
        {
            capabilities.insert("found".into(), found);
        }
        if let Some(expires) = record.get("expiresAt").filter(|v| v.is_number()) {
            capabilities.insert("expiresAt".into(), expires.clone());
        }
    }
    let capabilities = serde_json::Value::Object(capabilities);
    let fits = serde_json::to_string(&capabilities)
        .is_ok_and(|t| t.len() <= deckpilot_plugin_store::MAX_CAPABILITIES_BYTES);
    ConnectionProbe {
        status: ConnectionStatus::Ok,
        detail: None,
        account_label: account,
        capabilities: fits.then_some(capabilities),
    }
}

/// What a probe's answer means for the connection, and for the app.
fn test_result(
    result: Result<deckpilot_plugin_abi::ActionOutcome, super::LaneError>,
    outputs: &BTreeSet<String>,
) -> (ConnectionProbe, wire::ConnectionTestOutcome) {
    let untested = || ConnectionProbe {
        status: ConnectionStatus::Untested,
        detail: None,
        account_label: None,
        capabilities: None,
    };
    let failed = |code: String, message: Option<String>| wire::ConnectionTestOutcome {
        ok: false,
        code: Some(code),
        message,
    };
    match result {
        Ok(deckpilot_plugin_abi::ActionOutcome::Ok { data, .. }) => {
            let values: BTreeMap<String, serde_json::Value> = data
                .iter()
                .filter(|(k, _)| outputs.contains(*k))
                .map(|(k, v)| (k.clone(), super::convert::json(v)))
                .collect();
            (
                probe_of(&values),
                wire::ConnectionTestOutcome {
                    ok: true,
                    code: None,
                    message: None,
                },
            )
        }
        Ok(deckpilot_plugin_abi::ActionOutcome::Failed { code, message, .. }) => {
            let message = message.unwrap_or_default();
            let probe = ConnectionProbe {
                status: failed_status(&code, &message),
                detail: detail(&code),
                ..untested()
            };
            (probe, failed(code, Some(message)))
        }
        Ok(deckpilot_plugin_abi::ActionOutcome::Skipped { message }) => {
            (untested(), failed("skipped".into(), message))
        }
        Err(e) => (
            untested(),
            failed("plugin-failed".into(), Some(super::lane_error(&e).message)),
        ),
    }
}

/// The answer of a test once the plugin's probe finished: what it found is
/// stored on the connection, then the connection is answered as it stands.
fn tested(
    data: super::SharedData,
    plugin: PluginId,
    connection: String,
    outputs: BTreeSet<String>,
    reply: tokio::sync::oneshot::Sender<Result<Response, IpcError>>,
) -> super::Done {
    Box::new(move |result| {
        let (probe, outcome) = test_result(result, &outputs);
        let row = data.lock().ok().and_then(|mut guard| {
            let data = guard.as_mut()?;
            data.record_connection_probe(&plugin, &connection, &probe)
                .ok()?;
            data.connection(&plugin, &connection).ok().flatten()
        });
        let response = match row {
            Some(row) => Ok(Response::PluginConnectionTested {
                plugin: plugin.to_string(),
                connection: Box::new(view(&row)),
                outcome,
            }),
            None => Err(IpcError::new(
                ErrorCode::Failed,
                "the connection's test result could not be stored",
            )),
        };
        let _ = reply.send(response);
    })
}

/// The `records` outputs of `action` (plugin API 1.14), each with its
/// schema.
pub(super) fn record_outputs(
    manifest: &Manifest,
    action: &Action,
) -> Vec<(String, deckpilot_plugin_manifest::RecordSchema)> {
    action
        .node
        .as_ref()
        .and_then(|n| n.outputs.as_ref())
        .into_iter()
        .flatten()
        .filter(|o| o.kind == Some(deckpilot_plugin_manifest::OutputKind::Records))
        .filter_map(|o| {
            let schema = o.schema.as_deref()?;
            manifest
                .contributes
                .record_schemas()
                .iter()
                .find(|s| s.id == schema)
                .map(|s| (o.key.clone(), s.clone()))
        })
        .collect()
}

/// One record of `schema` from its JSON text: an object with only the
/// schema's fields, each of its kind.
fn record(
    schema: &deckpilot_plugin_manifest::RecordSchema,
    text: &str,
) -> Result<serde_json::Value, String> {
    use deckpilot_plugin_manifest::RecordFieldKind as K;
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|_| "a record is not a JSON object".to_owned())?;
    let object = value
        .as_object()
        .ok_or_else(|| "a record is not a JSON object".to_owned())?;
    for (key, field) in object {
        let declared = schema
            .fields
            .iter()
            .find(|f| f.key == *key)
            .ok_or_else(|| format!("`{key}` is not a field of `{}`", schema.id))?;
        let fits = match declared.kind {
            K::Text | K::State => field.is_string(),
            K::Integer | K::Timestamp => field.is_i64() || field.is_u64(),
            K::Boolean => field.is_boolean(),
            K::Url => field.as_str().is_some_and(|u| u.starts_with("https://")),
        };
        if !fits && !field.is_null() {
            return Err(format!("`{key}` is not of its kind in `{}`", schema.id));
        }
    }
    Ok(value)
}

/// `outcome` with each `records` output as a list of objects, checked
/// against its schema: at most [`MAX_RECORDS`] records and
/// [`MAX_RECORDS_BYTES`]. One that does not fit fails the outcome
/// (`invalid-output`), so the app never shows what the plugin got wrong.
///
/// [`MAX_RECORDS`]: deckpilot_plugin_manifest::MAX_RECORDS
/// [`MAX_RECORDS_BYTES`]: deckpilot_plugin_manifest::MAX_RECORDS_BYTES
pub(super) fn checked_records(
    outcome: deckpilot_ipc::plugins::PluginActionOutcome,
    records: &[(String, deckpilot_plugin_manifest::RecordSchema)],
) -> deckpilot_ipc::plugins::PluginActionOutcome {
    use deckpilot_ipc::plugins::PluginActionOutcome as O;
    let O::Ok { message, mut data } = outcome else {
        return outcome;
    };
    for (key, schema) in records {
        let Some(value) = data.get_mut(key) else {
            continue;
        };
        let checked = (|| {
            let items = value
                .as_array()
                .ok_or_else(|| format!("`{key}` is not a list of records"))?;
            if items.len() > deckpilot_plugin_manifest::MAX_RECORDS {
                return Err(format!(
                    "`{key}` has more than {} records",
                    deckpilot_plugin_manifest::MAX_RECORDS
                ));
            }
            let size: usize = items
                .iter()
                .map(|i| i.as_str().map_or(usize::MAX / 512, str::len))
                .sum();
            if size > deckpilot_plugin_manifest::MAX_RECORDS_BYTES {
                return Err(format!("`{key}` is larger than its limit"));
            }
            items
                .iter()
                .map(|item| {
                    item.as_str()
                        .ok_or_else(|| format!("`{key}` holds a record that is not text"))
                        .and_then(|text| record(schema, text))
                })
                .collect::<Result<Vec<_>, _>>()
        })();
        match checked {
            Ok(items) => *value = serde_json::Value::Array(items),
            Err(problem) => {
                return O::Failed {
                    code: "invalid-output".into(),
                    message: Some(format!("the plugin's answer does not fit: {problem}")),
                    retryable: false,
                };
            }
        }
    }
    O::Ok { message, data }
}

impl PluginService {
    /// The connection requests of plugin API 1.14.
    pub(super) fn handle_connections(&mut self, request: Request) -> Result<Response, IpcError> {
        match request {
            Request::ListPluginConnections { plugin } => {
                self.list_connections(&super::plugin_id(&plugin)?)
            }
            Request::CreatePluginConnection {
                plugin,
                connection_type,
                deployment,
                url,
                display_name,
                allow_changes,
                private_network,
                secret,
            } => self.create_connection(
                &super::plugin_id(&plugin)?,
                &connection_type,
                deployment,
                url.as_deref(),
                &display_name,
                allow_changes,
                private_network,
                secret.as_ref(),
            ),
            Request::SetPluginConnectionSecret {
                plugin,
                connection,
                secret,
            } => self.set_connection_secret(&super::plugin_id(&plugin)?, &connection, &secret),
            Request::UpdatePluginConnection {
                plugin,
                connection,
                display_name,
                enabled,
                allow_changes,
                make_default,
            } => self.update_connection(
                &super::plugin_id(&plugin)?,
                &connection,
                display_name.as_deref(),
                enabled,
                allow_changes,
                make_default,
            ),
            Request::RemovePluginConnection { plugin, connection } => {
                self.remove_connection(&super::plugin_id(&plugin)?, &connection)
            }
            other => Err(invalid(format!(
                "{} is not a connection request",
                other.name()
            ))),
        }
    }

    fn active_manifest_localized(&self, id: &PluginId) -> Result<Manifest, IpcError> {
        let record = self.record(id)?;
        let active = record.active().ok_or_else(|| {
            store_error(&deckpilot_plugin_store::StoreError::NotInstalled(
                id.clone(),
            ))
        })?;
        let dir = self.manager.layout().resolve(&active.install_dir);
        Ok(self.locales.manifest(id.as_str(), active, &dir))
    }

    fn connection_row(
        &self,
        id: &PluginId,
        connection: &str,
    ) -> Result<StoredConnection, IpcError> {
        self.manager
            .store()
            .connection_record(id, connection)
            .map_err(|e| store_error(&e))?
            .ok_or_else(|| {
                error(
                    ErrorCode::NotFound,
                    deckpilot_ipc::plugins::reason::NOT_INSTALLED,
                    format!("{id} has no connection {connection}"),
                )
            })
    }

    fn connection_response(&self, id: &PluginId, connection: &str) -> Result<Response, IpcError> {
        Ok(Response::PluginConnection {
            plugin: id.to_string(),
            connection: Box::new(view(&self.connection_row(id, connection)?)),
        })
    }

    /// `listPluginConnections`.
    pub(super) fn list_connections(&self, id: &PluginId) -> Result<Response, IpcError> {
        let manifest = self.active_manifest_localized(id)?;
        let declared = deckpilot_plugin_store::declared_connections(&manifest);
        let granted = self
            .manager
            .store()
            .effective_grants(id)
            .map_err(|e| store_error(&e))?
            .iter()
            .any(|(k, _)| k.kind == PermissionKind::Connections);
        let rows = self
            .manager
            .store()
            .connections(id)
            .map_err(|e| store_error(&e))?;
        Ok(Response::PluginConnections {
            connections: Box::new(wire::PluginConnections {
                plugin: id.to_string(),
                granted,
                private_network_allowed: declared.as_ref().is_some_and(|(_, _, p)| *p),
                changes_allowed: declared
                    .as_ref()
                    .is_some_and(|(_, m, _)| m.iter().any(|m| *m != HttpMethod::Get)),
                types: types(&manifest),
                connections: rows.iter().map(view).collect(),
            }),
        })
    }

    /// `createPluginConnection`.
    #[expect(
        clippy::too_many_arguments,
        reason = "one IPC request, handed over field by field"
    )]
    pub(super) fn create_connection(
        &mut self,
        id: &PluginId,
        type_id: &str,
        deployment: wire::ConnectionDeployment,
        url: Option<&str>,
        display_name: &str,
        allow_changes: bool,
        private_network: bool,
        secret: Option<&deckpilot_ipc::plugins::PluginSecret>,
    ) -> Result<Response, IpcError> {
        let manifest = self
            .record(id)?
            .active()
            .map(|a| a.manifest.clone())
            .ok_or_else(|| {
                store_error(&deckpilot_plugin_store::StoreError::NotInstalled(
                    id.clone(),
                ))
            })?;
        let ty = manifest
            .contributes
            .connection_types()
            .iter()
            .find(|t| t.id == type_id)
            .ok_or_else(|| invalid(format!("{id} has no connection type `{type_id}`")))?;
        if ty.auth.first().is_none_or(|a| {
            !matches!(
                a.scheme,
                CredentialScheme::Bearer | CredentialScheme::Header
            )
        }) {
            return Err(invalid(format!(
                "connections of `{type_id}` sign in in a way DeckPilot does not support yet"
            )));
        }
        let deployment = match deployment {
            wire::ConnectionDeployment::Cloud => Deployment::Cloud,
            wire::ConnectionDeployment::SelfHosted => Deployment::SelfHosted,
        };
        let address = address(ty, deployment, url).map_err(invalid)?;
        if address.private_host && !private_network {
            return Err(error(
                ErrorCode::Failed,
                deckpilot_ipc::plugins::reason::CONSENT_REQUIRED,
                format!(
                    "{} is on a private network (a company or home network); confirm that \
                     DeckPilot may connect there",
                    address.origin
                ),
            ));
        }
        let new = NewConnection {
            id: new_id()?,
            type_id: type_id.to_owned(),
            display_name: display_name.trim().to_owned(),
            deployment,
            origin: address.origin.clone(),
            path_prefix: address.path_prefix.clone(),
            web_origin: address.web_origin.clone(),
            methods: methods(&manifest, allow_changes),
            private_network: private_network && deployment == Deployment::SelfHosted,
            legacy_source: None,
        };
        self.manager
            .create_connection(id, &new, super::DECIDED_BY)
            .map_err(|e| store_error(&e))?;
        let stored = secret.map(|secret| {
            self.manager
                .set_connection_secret(id, &new.id, secret.expose())
        });
        if let Some(Err(e)) = stored {
            // Never half made: no connection without the token the user
            // meant to give it.
            let _ = self.manager.remove_connection(id, &new.id);
            return Err(store_error(&e));
        }
        // The id and the server only; never the token.
        info!(plugin = %id, connection = %new.id, origin = %new.origin, "plugin connection created");
        self.sync_grants(id);
        self.changed(id);
        self.connection_response(id, &new.id)
    }

    /// `setPluginConnectionSecret`: also reconnect.
    pub(super) fn set_connection_secret(
        &mut self,
        id: &PluginId,
        connection: &str,
        secret: &deckpilot_ipc::plugins::PluginSecret,
    ) -> Result<Response, IpcError> {
        self.connection_row(id, connection)?;
        self.manager
            .set_connection_secret(id, connection, secret.expose())
            .map_err(|e| store_error(&e))?;
        info!(plugin = %id, connection, "plugin connection token stored");
        self.changed(id);
        self.connection_response(id, connection)
    }

    /// `updatePluginConnection`.
    pub(super) fn update_connection(
        &mut self,
        id: &PluginId,
        connection: &str,
        display_name: Option<&str>,
        enabled: Option<bool>,
        allow_changes: Option<bool>,
        make_default: bool,
    ) -> Result<Response, IpcError> {
        self.connection_row(id, connection)?;
        let map = |e: deckpilot_plugin_store::StoreError| store_error(&e);
        if let Some(name) = display_name {
            self.manager
                .rename_connection(id, connection, name.trim())
                .map_err(map)?;
        }
        if let Some(enabled) = enabled {
            self.manager
                .set_connection_enabled(id, connection, enabled)
                .map_err(map)?;
        }
        if let Some(allow) = allow_changes {
            let manifest = self
                .record(id)?
                .active()
                .map(|a| a.manifest.clone())
                .ok_or_else(|| invalid(format!("{id} has no active version")))?;
            self.manager
                .set_connection_methods(id, connection, &methods(&manifest, allow))
                .map_err(map)?;
        }
        if make_default {
            self.manager
                .set_default_connection(id, connection)
                .map_err(map)?;
        }
        self.sync_grants(id);
        self.changed(id);
        self.connection_response(id, connection)
    }

    /// `removePluginConnection`: the row and its secret, at once out of the
    /// broker.
    pub(super) fn remove_connection(
        &mut self,
        id: &PluginId,
        connection: &str,
    ) -> Result<Response, IpcError> {
        let removed = self
            .manager
            .remove_connection(id, connection)
            .map_err(|e| store_error(&e))?;
        if removed {
            info!(plugin = %id, connection, "plugin connection removed");
        }
        self.sync_grants(id);
        self.changed(id);
        self.list_connections(id)
    }

    /// `testPluginConnection`: runs the plugin's internal `probe` action
    /// bound to exactly this connection, in a run the user started from
    /// the app, and stores what it found. Answered on the plugin's lane.
    /// A test that did not run: stored and answered as such.
    fn refused_test(
        &mut self,
        id: &PluginId,
        connection: &str,
        code: &str,
        message: &str,
        status: ConnectionStatus,
    ) -> Result<Response, IpcError> {
        let probe = ConnectionProbe {
            status,
            detail: detail(code),
            account_label: None,
            capabilities: None,
        };
        let _ = self.manager.record_connection_probe(id, connection, &probe);
        let row = self.connection_row(id, connection)?;
        Ok(Response::PluginConnectionTested {
            plugin: id.to_string(),
            connection: Box::new(view(&row)),
            outcome: wire::ConnectionTestOutcome {
                ok: false,
                code: Some(code.to_owned()),
                message: Some(message.to_owned()),
            },
        })
    }

    pub(crate) fn test_connection(
        &mut self,
        plugin: &str,
        connection: &str,
        reply: tokio::sync::oneshot::Sender<Result<Response, IpcError>>,
    ) {
        let answer = |reply: tokio::sync::oneshot::Sender<_>, r| {
            let _ = reply.send(r);
        };
        let started = (|| {
            let id = super::plugin_id(plugin)?;
            let row = self.connection_row(&id, connection)?;
            Ok::<_, IpcError>((id, row))
        })();
        let (id, row) = match started {
            Ok(v) => v,
            Err(e) => return answer(reply, Err(e)),
        };
        let refusal = if !row.has_secret() {
            Some((
                "no-token",
                "add a token to the connection first",
                ConnectionStatus::NeedsReconnect,
            ))
        } else if !row.enabled {
            Some((
                "connection-disabled",
                "the connection is turned off",
                row.status,
            ))
        } else {
            None
        };
        if let Some((code, message, status)) = refusal {
            return answer(
                reply,
                self.refused_test(&id, &row.id, code, message, status),
            );
        }
        let prepared = self.prepare(&id, PROBE_ACTION, false);
        let (record, prepared) = match prepared {
            Ok(v) => v,
            Err(e) => return answer(reply, Err(e)),
        };
        let super::Prepared {
            timeout,
            outputs,
            executor,
            definition,
            ..
        } = prepared;
        let mut params = BTreeMap::new();
        params.insert(
            "connection".to_owned(),
            serde_json::Value::String(row.id.clone()),
        );
        let params = match super::convert::params(&params)
            .map_err(invalid)
            .and_then(|p| super::data::normalize_params(&definition, &p).map_err(invalid))
        {
            Ok(p) => p,
            Err(e) => return answer(reply, Err(e)),
        };
        let bound = BTreeSet::from([row.id.clone()]);
        let done = tested(
            self.data.clone(),
            id.clone(),
            row.id.clone(),
            outputs,
            reply,
        );
        let done = self.health.watch(id.to_string(), done);
        match self.lane(&record) {
            Ok(lane) => lane.submit(super::Job {
                action: PROBE_ACTION.to_owned(),
                params,
                timeout,
                executor: executor
                    .with_run(deckpilot_plugin_broker::RunContext::direct(
                        deckpilot_plugin_abi::capability::RunTrigger::Test,
                    ))
                    .with_connections(bound),
                abort: deckpilot_plugin_supervisor::AbortSignal::new(),
                reply: super::Reply::Action(done),
            }),
            Err(e) => done(Err(super::LaneError::Refused(e))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> Manifest {
        Manifest::from_json(include_str!(
            "../../../../crates/deckpilot-plugin-manifest/tests/fixtures/connections-manifest.json"
        ))
        .unwrap()
    }

    fn ty(id: &str) -> deckpilot_plugin_manifest::ConnectionType {
        manifest()
            .contributes
            .connection_types()
            .iter()
            .find(|t| t.id == id)
            .unwrap()
            .clone()
    }

    #[test]
    fn ids_are_ulids_and_unique() {
        let a = new_id().unwrap();
        let b = new_id().unwrap();
        assert!(deckpilot_plugin_store::is_connection_id(&a), "{a}");
        assert!(deckpilot_plugin_abi::connections::is_connection_id(&a));
        assert_ne!(a, b);
    }

    /// The cloud always gets the manifest's addresses, whatever the app
    /// sends; a self-hosted server is the address the user opens, with the
    /// type's API path, a port and a path prefix kept.
    #[test]
    fn addresses_are_normalized_from_what_the_user_opens() {
        let cloud = address(
            &ty("gitlab"),
            Deployment::Cloud,
            Some("https://evil.example"),
        )
        .unwrap();
        assert_eq!(cloud.origin, "https://gitlab.com");
        assert_eq!(cloud.path_prefix.as_deref(), Some("/api/v4"));
        assert_eq!(cloud.web_origin, "https://gitlab.com");
        let github = address(&ty("github"), Deployment::Cloud, None).unwrap();
        assert_eq!(github.origin, "https://api.github.com");
        assert_eq!(github.path_prefix, None);
        assert_eq!(github.web_origin, "https://github.com");

        for (entered, origin, prefix) in [
            (
                "https://code.example.com",
                "https://code.example.com",
                "/api/v4",
            ),
            (
                "https://Code.Example.com:8443/gitlab/",
                "https://code.example.com:8443",
                "/gitlab/api/v4",
            ),
            (
                "https://code.example.com:443/",
                "https://code.example.com",
                "/api/v4",
            ),
            (
                "https://code.example.com/gitlab/api/v4",
                "https://code.example.com",
                "/gitlab/api/v4",
            ),
            (
                "  https://code.example.com  ",
                "https://code.example.com",
                "/api/v4",
            ),
        ] {
            let a = address(&ty("gitlab"), Deployment::SelfHosted, Some(entered)).unwrap();
            assert_eq!(a.origin, origin, "{entered}");
            assert_eq!(a.path_prefix.as_deref(), Some(prefix), "{entered}");
            assert_eq!(a.web_origin, origin, "{entered}");
            assert!(!a.private_host);
        }
        let ghes = address(
            &ty("github"),
            Deployment::SelfHosted,
            Some("https://ghe.example.com"),
        )
        .unwrap();
        assert_eq!(ghes.path_prefix.as_deref(), Some("/api/v3"));
    }

    /// https only, no user name or password, no query; this computer,
    /// link-local and metadata never; a private address only as a private
    /// host (which then needs the user's consent).
    #[test]
    fn unsafe_addresses_are_refused() {
        for bad in [
            "",
            "http://code.example.com",
            "ftp://code.example.com",
            "https://user:pass@code.example.com",
            "https://user-token@code.example.com",
            "https://code.example.com/?private_token=x",
            "https://code.example.com/#x",
            "https://localhost",
            "https://127.0.0.1",
            "https://[::1]",
            "https://169.254.169.254",
            "https://[fe80::1]",
            "https://0.0.0.0",
            "https://code.example.com/a/../b",
            "https://code.example.com/a%2fb",
            "https://code example.com",
        ] {
            assert!(
                address(&ty("gitlab"), Deployment::SelfHosted, Some(bad)).is_err(),
                "{bad}"
            );
        }
        for private in [
            "https://10.0.0.5",
            "https://192.168.1.10:8443",
            "https://gitlab.lan",
            "https://git",
        ] {
            let a = address(&ty("gitlab"), Deployment::SelfHosted, Some(private)).unwrap();
            assert!(a.private_host, "{private}");
        }
    }

    /// Only the `connection` parameters bind, and only connection ids.
    #[test]
    fn an_invocation_is_bound_to_its_connection_parameters() {
        let m = manifest();
        let action = m
            .contributes
            .actions()
            .iter()
            .find(|a| a.id == "list-repositories")
            .unwrap();
        let mut params = Params::new();
        params.insert(
            "connection".into(),
            ParamValue::String("cn_01JD7Q0000000000000000000A".into()),
        );
        params.insert(
            "search".into(),
            ParamValue::String("cn_01JD7Q0000000000000000000B".into()),
        );
        assert_eq!(
            bound(action, &params),
            BTreeSet::from(["cn_01JD7Q0000000000000000000A".to_owned()])
        );
        params.insert("connection".into(), ParamValue::String("github".into()));
        assert_eq!(bound(action, &params), BTreeSet::new());
    }

    fn row(id: &str, type_id: &str, methods: Vec<HttpMethod>, private: bool) -> StoredConnection {
        StoredConnection {
            id: id.into(),
            type_id: type_id.into(),
            display_name: "x".into(),
            deployment: Deployment::SelfHosted,
            origin: "https://code.example.com".into(),
            path_prefix: Some("/api/v4".into()),
            web_origin: "https://code.example.com".into(),
            methods,
            private_network: private,
            enabled: true,
            status: ConnectionStatus::Ok,
            status_detail: None,
            capabilities: None,
            account_label: None,
            is_default: true,
            legacy_source: None,
            consent_by: "user".into(),
            consent_at: 1,
            created_at: 1,
            updated_at: 1,
            secret_updated_at: Some(1),
        }
    }

    /// Without the `connections` grant nothing reaches the broker; with
    /// it, each connection within what the grant allows now.
    #[test]
    fn connections_reach_the_broker_only_within_the_grant() {
        let m = manifest();
        let rows = [
            row(
                "cn_01JD7Q0000000000000000000A",
                "gitlab",
                vec![HttpMethod::Get, HttpMethod::Post],
                false,
            ),
            row(
                "cn_01JD7Q0000000000000000000B",
                "github",
                vec![HttpMethod::Get],
                true,
            ),
            row(
                "cn_01JD7Q0000000000000000000C",
                "bitbucket",
                vec![HttpMethod::Get],
                false,
            ),
        ];
        assert_eq!(approved(&m, &[], &rows), Vec::new());
        let key = GrantKey::new(PermissionKind::Connections, "");
        let narrow = [(
            key.clone(),
            GrantScope::Connections {
                types: vec!["gitlab".into(), "github".into()],
                methods: vec![HttpMethod::Get],
                private_network: false,
            },
        )];
        let out = approved(&m, &narrow, &rows);
        assert_eq!(out.len(), 1, "{out:?}");
        match &out[0] {
            ApprovedPermission::Connection {
                id,
                methods,
                header_name,
                open_url,
                ..
            } => {
                assert_eq!(id, "cn_01JD7Q0000000000000000000A");
                assert_eq!(methods.len(), 1, "POST is not granted now");
                assert_eq!(header_name.as_deref(), Some("PRIVATE-TOKEN"));
                assert!(!open_url);
            }
            other => panic!("{other:?}"),
        }
        let wide = [
            (
                key,
                GrantScope::Connections {
                    types: vec!["gitlab".into(), "github".into()],
                    methods: vec![HttpMethod::Get, HttpMethod::Post],
                    private_network: true,
                },
            ),
            (
                GrantKey::new(PermissionKind::OpenUrl, OPEN_URL_CONNECTIONS_KEY),
                GrantScope::OpenUrl,
            ),
        ];
        let out = approved(&m, &wide, &rows);
        assert_eq!(out.len(), 2, "an undeclared type never: {out:?}");
        // What the broker derives from them is valid.
        deckpilot_plugin_broker::Grants::derive(&deckpilot_plugin_broker::ApprovedPermissions(out))
            .unwrap();
    }

    /// `records` outputs reach the app as objects of their schema, within
    /// the limits; anything else fails the outcome.
    #[test]
    fn records_are_checked_against_their_schema() {
        use deckpilot_ipc::plugins::PluginActionOutcome as O;
        let m = manifest();
        let action = m
            .contributes
            .actions()
            .iter()
            .find(|a| a.id == "list-repositories")
            .unwrap();
        let records = record_outputs(&m, action);
        assert_eq!(records.len(), 1);
        let schema = &records[0].1;
        let field = schema.fields.first().unwrap().key.clone();
        let ok = |items: serde_json::Value| O::Ok {
            message: None,
            data: BTreeMap::from([("items".to_owned(), items), ("count".to_owned(), 1.into())]),
        };
        let good = serde_json::json!([serde_json::json!({ field.clone(): "gl:4711" }).to_string()]);
        match checked_records(ok(good), &records) {
            O::Ok { data, .. } => assert!(data["items"][0].is_object(), "{data:?}"),
            other => panic!("{other:?}"),
        }
        for bad in [
            serde_json::json!("not a list"),
            serde_json::json!([r#"{"unknownField":1}"#]),
            serde_json::json!(["not json"]),
            serde_json::json!([1]),
            serde_json::Value::Array(vec![serde_json::json!("{}"); 201]),
            serde_json::json!(["x".repeat(70 * 1024)]),
        ] {
            assert!(
                matches!(checked_records(ok(bad.clone()), &records), O::Failed { ref code, .. } if code == "invalid-output"),
                "{bad}"
            );
        }
    }

    /// A test's answer: the account (never one shaped like a token) and
    /// the capabilities, bounded.
    #[test]
    fn a_probe_keeps_the_account_and_capabilities_only() {
        let mut data = BTreeMap::new();
        data.insert("account".into(), serde_json::json!("octo"));
        data.insert("canWrite".into(), serde_json::json!(false));
        data.insert("version".into(), serde_json::json!("GitLab 16.11"));
        data.insert(
            "items".into(),
            serde_json::json!([r#"{"account":"octo","capabilities":"{\"pipelines\":\"read\"}","expiresAt":1767225600000}"#]),
        );
        let probe = probe_of(&data);
        assert_eq!(probe.account_label.as_deref(), Some("octo"));
        let caps = probe.capabilities.unwrap();
        assert_eq!(caps["found"]["pipelines"], "read");
        assert_eq!(caps["version"], "GitLab 16.11");
        data.insert(
            "account".into(),
            serde_json::json!(format!("{}{}", "glp", "at-0123456789abcdefghij")),
        );
        assert_eq!(probe_of(&data).account_label, None);
        assert_eq!(
            failed_status("unauthorized", ""),
            ConnectionStatus::Unauthorized
        );
        assert_eq!(
            failed_status("unreachable", "the certificate is not trusted"),
            ConnectionStatus::TlsFailed
        );
        assert_eq!(
            failed_status("unreachable", "dns"),
            ConnectionStatus::Unreachable
        );
        assert_eq!(detail("Bad credentials"), None);
        assert_eq!(
            detail("insufficient-scope").as_deref(),
            Some("insufficient-scope")
        );
    }
}
