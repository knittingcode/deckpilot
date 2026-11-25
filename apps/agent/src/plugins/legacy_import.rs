//! The old `deckpilot.github` and `deckpilot.gitlab` plugins' server and
//! token as a Git Providers connection (design §11.2 and §11.4, PR-10,
//! phase 1: side by side, an opt-in import).
//!
//! This module only **plans** an import from what the old plugin has on
//! this computer; the plugin store writes it, in one transaction
//! ([`deckpilot_plugin_store::PluginManager::import_legacy_connection`]):
//! idempotent (a connection whose `legacy_source` is the old plugin
//! already is kept as it is), all or nothing, and the old plugin's
//! settings, credential row and consents are never changed or deleted.
//! Its keys and workflows keep running on the old plugin: nothing here
//! touches a stored document, and routing them to Git Providers is phase 2
//! (PR-11, `PLUGIN_SUCCESSION` and `InjectConnection`).
//!
//! * **The server.** `apiUrl` (or the old plugin's default) becomes a
//!   cloud connection when it is the public service's API, else a
//!   self-hosted one through the same normalization as a connection the
//!   user adds ([`super::connections::address`]). A GitHub Enterprise
//!   Cloud data-residency API (`api.<x>.ghe.com`, OD-7) is reported as not
//!   importable, never guessed (A10).
//! * **A private network** is carried over only from the old plugin's
//!   recorded consent to exactly that server with its private network;
//!   otherwise such a server is not imported (the user adds it in
//!   Settings › Connections, where they are asked).
//! * **The token** is copied host-side only when the user approved that
//!   item ([`Approval::token`]); never derived, never through IPC, never
//!   to the plugin. Without it the connection waits for one
//!   (`needsReconnect`).
//! * **Read-only**, as the old plugins were: `GET` only.
//! * **Settings beyond the server**: see [`successor_settings`] (open
//!   decision A9).

use std::collections::BTreeMap;

use deckpilot_ipc::{ErrorCode, IpcError, Response};
use deckpilot_plugin_broker::UserOrigin;
use deckpilot_plugin_manifest::{HttpMethod, Manifest, PluginId};
use deckpilot_plugin_store::{Deployment, LegacyImport, NewConnection, StoredUserOrigin};
use serde_json::Value;
use tracing::info;

use super::{PluginService, error, invalid, store_error};

/// The plugin the old ones are imported into.
pub(crate) const SUCCESSOR: &str = "deckpilot.git-providers";

/// One old plugin, as its released contract
/// (`plugins/first-party/contracts/deckpilot.{github,gitlab}`) has it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LegacySource {
    pub plugin: &'static str,
    /// The successor's connection type.
    pub type_id: &'static str,
    /// For the connection's name.
    pub label: &'static str,
    /// The old plugin's default `apiUrl`: its public service.
    pub cloud_api: &'static str,
    /// The setting with the server, and the origin slot of its consent.
    pub server_setting: &'static str,
    /// The credential slot of its token.
    pub token_slot: &'static str,
}

pub(crate) const SOURCES: [LegacySource; 2] = [
    LegacySource {
        plugin: "deckpilot.github",
        type_id: "github",
        label: "GitHub",
        cloud_api: "https://api.github.com",
        server_setting: "apiUrl",
        token_slot: "token",
    },
    LegacySource {
        plugin: "deckpilot.gitlab",
        type_id: "gitlab",
        label: "GitLab",
        cloud_api: "https://gitlab.com/api/v4",
        server_setting: "apiUrl",
        token_slot: "token",
    },
];

/// The old plugin `plugin`, if it is one the successor imports from.
pub(crate) fn source(plugin: &str) -> Option<&'static LegacySource> {
    SOURCES.iter().find(|s| s.plugin == plugin)
}

/// What the user approved for one import. The import itself (its server)
/// is the approval of the item that names the server; the token is a
/// separate item, never derived.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Approval {
    pub token: bool,
}

/// Why an old plugin's settings are not imported. Each has a reason code
/// for the app ([`NotImportable::code`]); none is guessed around.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NotImportable {
    /// The successor declares no connection type for it.
    NoConnectionType,
    /// `apiUrl` is not a server a connection can name (the reason).
    Server(String),
    /// A GitHub Enterprise Cloud data-residency API (`api.<x>.ghe.com`):
    /// not expressible until OD-7 (A10).
    DataResidency,
    /// A server on a private network without the old plugin's recorded
    /// consent to exactly it with its private network.
    PrivateNetwork,
    /// The token was asked for, but the old plugin never had the user's
    /// consent to send it to this server (the server may still be imported
    /// without it).
    TokenConsent,
    /// Imported before without its token; the token is added in Settings ›
    /// Connections, never by a second import.
    AlreadyImported,
}

impl NotImportable {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::NoConnectionType => "no-connection-type",
            Self::Server(_) => "invalid-server",
            Self::DataResidency => "data-residency",
            Self::PrivateNetwork => "private-network-consent",
            Self::TokenConsent => "token-consent",
            Self::AlreadyImported => "already-imported",
        }
    }
}

/// An import, ready for the plugin store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Planned {
    pub connection: NewConnection,
    /// The old plugin's slot whose secret is copied: only when approved.
    pub secret_slot: Option<&'static str>,
    /// Settings of the successor's that the old plugin's become (A9).
    pub successor_settings: BTreeMap<String, Value>,
}

/// Plans the import of `source`'s server (its saved `settings`, its
/// recorded user-origin `consents`) into the successor (its active
/// `manifest`) as the connection `id`.
pub(crate) fn plan(
    source: &LegacySource,
    manifest: &Manifest,
    settings: &BTreeMap<String, Value>,
    consents: &[StoredUserOrigin],
    approval: Approval,
    id: String,
) -> Result<Planned, NotImportable> {
    let ty = manifest
        .contributes
        .connection_types()
        .iter()
        .find(|t| t.id == source.type_id)
        .ok_or(NotImportable::NoConnectionType)?;
    let api_url = settings
        .get(source.server_setting)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .unwrap_or(source.cloud_api)
        .trim_end_matches('/');
    // Parsed first: case, a default port or a trailing dot never decide.
    let parsed = UserOrigin::parse(api_url).map_err(|_| {
        NotImportable::Server(format!(
            "`{}` is not a server address DeckPilot can connect to",
            source.server_setting
        ))
    })?;
    if data_residency(&parsed.origin().to_string()) {
        return Err(NotImportable::DataResidency);
    }
    let cloud = UserOrigin::parse(source.cloud_api)
        .map_err(|_| NotImportable::Server("the public API does not parse".into()))?;
    let deployment =
        if parsed.origin() == cloud.origin() && parsed.path_prefix() == cloud.path_prefix() {
            Deployment::Cloud
        } else {
            Deployment::SelfHosted
        };
    let address = super::connections::address(ty, deployment, Some(api_url))
        .map_err(NotImportable::Server)?;
    // The old plugin's recorded consent to exactly this server: its
    // origin, a path prefix the connection's lies within, reading.
    let consented: Vec<_> = consents
        .iter()
        .filter(|c| {
            let prefix = c.consent.path_prefix.as_deref().unwrap_or("");
            let ours = address.path_prefix.as_deref().unwrap_or("");
            c.consent.setting == source.server_setting
                && c.consent.origin == address.origin
                && (ours == prefix || ours.starts_with(&format!("{prefix}/")) || prefix.is_empty())
                && c.consent.methods.contains(&HttpMethod::Get)
        })
        .collect();
    let private_network = deployment == Deployment::SelfHosted
        && address.private_host
        && consented.iter().any(|c| c.consent.private_network);
    if address.private_host && !private_network {
        return Err(NotImportable::PrivateNetwork);
    }
    // The old plugin sent its token to a server the user entered only
    // after they allowed exactly that server for it; so does the import.
    let token_allowed = deployment == Deployment::Cloud
        || consented
            .iter()
            .any(|c| c.consent.credentials.iter().any(|k| k == source.token_slot));
    if approval.token && !token_allowed {
        return Err(NotImportable::TokenConsent);
    }
    let display_name = match deployment {
        Deployment::Cloud => source.label.to_owned(),
        Deployment::SelfHosted => {
            let host = address
                .web_origin
                .strip_prefix("https://")
                .unwrap_or(&address.web_origin);
            let name = format!("{} {host}", source.label);
            // A connection's name is at most 64 characters.
            name.chars().take(64).collect()
        }
    };
    Ok(Planned {
        connection: NewConnection {
            id,
            type_id: source.type_id.to_owned(),
            display_name,
            deployment,
            origin: address.origin,
            path_prefix: address.path_prefix,
            web_origin: address.web_origin,
            methods: vec![HttpMethod::Get],
            private_network,
            legacy_source: Some(source.plugin.to_owned()),
        },
        secret_slot: approval.token.then_some(source.token_slot),
        successor_settings: successor_settings(settings),
    })
}

/// `api.<x>.ghe.com`: GitHub Enterprise Cloud with data residency (OD-7).
/// Given a parsed origin; checked again on the lower-case host without a
/// trailing dot.
fn data_residency(api_url: &str) -> bool {
    let lower = api_url.to_ascii_lowercase();
    let host = lower
        .strip_prefix("https://")
        .unwrap_or(&lower)
        .split(['/', ':'])
        .next()
        .unwrap_or_default()
        .trim_end_matches('.')
        .to_owned();
    host.strip_prefix("api.")
        .and_then(|rest| rest.strip_suffix(".ghe.com"))
        .is_some_and(|tenant| !tenant.is_empty() && !tenant.contains('.'))
}

/// The time limit the legacy wrappers (deckpilot-plugins #101) use for
/// every request of one action: fixed, whatever the old plugin's
/// `timeoutSeconds` says.
pub(crate) const LEGACY_TIMEOUT_SECONDS: u64 = 20;

/// The successor's settings that the old plugin's settings become, besides
/// the server (which becomes the connection).
///
/// **TODO(A9): open owner decision, do not resolve here.** Design §11.4
/// says the old `timeoutSeconds` is copied into the successor's timeout
/// setting; the legacy wrappers instead use a fixed
/// [`LEGACY_TIMEOUT_SECONDS`] (20 s). Until the owner decides A9 this
/// keeps the current plugin behaviour: nothing is carried over, and the
/// wrappers stay at 20 s. Carrying it would be this function returning
/// `{"<successor's timeout setting>": timeoutSeconds}` (validated against
/// the successor's declared bounds), applied in the same import; nothing
/// else changes.
pub(crate) fn successor_settings(_legacy: &BTreeMap<String, Value>) -> BTreeMap<String, Value> {
    BTreeMap::new()
}

/// Who decided an import: the user, in the migration's item for the old
/// plugin (`user:migration:plugin:<id>`), never derived.
fn decided_by(source: &LegacySource) -> String {
    format!("user:migration:plugin:{}", source.plugin)
}

/// A refusal of an import as the app gets it: its reason code and why.
fn refusal(legacy: &LegacySource, why: &NotImportable) -> IpcError {
    let message = match why {
        NotImportable::NoConnectionType => {
            format!("Git Providers has no {} connections", legacy.label)
        }
        NotImportable::Server(problem) => format!("the {} server {problem}", legacy.label),
        NotImportable::DataResidency => format!(
            "{} Enterprise Cloud with data residency cannot be connected yet; add the \
                 connection in Settings › Connections once it can",
            legacy.label
        ),
        NotImportable::PrivateNetwork => format!(
            "the {} server is on a private network; add it in Settings › Connections, \
                 where you are asked to allow that",
            legacy.label
        ),
        NotImportable::TokenConsent | NotImportable::AlreadyImported => format!(
            "the {} plugin's token was never allowed for this server; import the \
                 server without it and add the token in Settings › Connections",
            legacy.label
        ),
    };
    error(ErrorCode::Failed, why.code(), message)
}

impl PluginService {
    /// `importLegacyConnection`: the user asked to use the old plugin
    /// `source`'s server in Git Providers, and with `token` also its saved
    /// token. Answers the connection (new, or the one made before:
    /// importing again changes nothing). A server that cannot be imported
    /// is refused with its reason code ([`NotImportable::code`]), and
    /// nothing is written.
    pub(super) fn import_legacy_connection(
        &mut self,
        source: &PluginId,
        token: bool,
    ) -> Result<Response, IpcError> {
        let legacy = self::source(source.as_str())
            .ok_or_else(|| invalid(format!("Git Providers does not import from {source}")))?;
        let successor = PluginId::parse(SUCCESSOR).map_err(|e| invalid(e.to_string()))?;
        let manifest = self
            .record(&successor)?
            .active()
            .map(|a| a.manifest.clone())
            .ok_or_else(|| {
                store_error(&deckpilot_plugin_store::StoreError::NotInstalled(
                    successor.clone(),
                ))
            })?;
        let store = self.manager.store();
        if store.plugin(source).map_err(|e| store_error(&e))?.is_none() {
            return Err(store_error(
                &deckpilot_plugin_store::StoreError::NotInstalled(source.clone()),
            ));
        }
        let settings = store.settings(source).map_err(|e| store_error(&e))?;
        let consents = store.user_origins(source).map_err(|e| store_error(&e))?;
        let planned = plan(
            legacy,
            &manifest,
            &settings,
            &consents,
            Approval { token },
            super::connections::new_id()?,
        )
        .map_err(|why| refusal(legacy, &why))?;
        // TODO(A9): nothing is carried today (`successor_settings`); when
        // it is, it is written in the same transaction as the connection.
        debug_assert!(planned.successor_settings.is_empty());
        let outcome = self
            .manager
            .import_legacy_connection(
                &successor,
                source,
                &planned.connection,
                planned.secret_slot,
                &decided_by(legacy),
            )
            .map_err(|e| store_error(&e))?;
        let connection = match &outcome {
            LegacyImport::Created {
                connection,
                with_secret,
            } => {
                // The ids and the server only; never the token.
                info!(
                    plugin = %successor,
                    from = %source,
                    %connection,
                    origin = %planned.connection.origin,
                    with_token = with_secret,
                    // A9 open: the wrappers keep this time limit.
                    timeout_seconds = LEGACY_TIMEOUT_SECONDS,
                    "connection imported from an old plugin"
                );
                self.sync_grants(&successor);
                self.changed(&successor);
                connection.clone()
            }
            LegacyImport::Existing {
                connection,
                has_secret,
            } => {
                if token && !has_secret {
                    let why = NotImportable::AlreadyImported;
                    return Err(error(
                        ErrorCode::Failed,
                        why.code(),
                        format!(
                            "the {} server was imported without its token; add the token in \
                             Settings › Connections",
                            legacy.label
                        ),
                    ));
                }
                connection.clone()
            }
        };
        self.connection_response(&successor, &connection)
    }
}

#[cfg(test)]
mod tests {
    use deckpilot_plugin_store::UserOriginConsent;
    use serde_json::json;

    use super::*;

    const ID: &str = "cn_01JD7Q0000000000000000000A";

    fn manifest() -> Manifest {
        Manifest::from_json(include_str!(
            "../../../../crates/deckpilot-plugin-manifest/tests/fixtures/connections-manifest.json"
        ))
        .unwrap()
    }

    fn github() -> &'static LegacySource {
        source("deckpilot.github").unwrap()
    }

    fn gitlab() -> &'static LegacySource {
        source("deckpilot.gitlab").unwrap()
    }

    fn settings(api_url: Option<&str>, timeout: i64) -> BTreeMap<String, Value> {
        let mut out = BTreeMap::from([("timeoutSeconds".to_owned(), json!(timeout))]);
        if let Some(url) = api_url {
            out.insert("apiUrl".into(), json!(url));
        }
        out
    }

    fn consent(origin: &str, private_network: bool) -> StoredUserOrigin {
        StoredUserOrigin {
            consent: UserOriginConsent {
                setting: "apiUrl".into(),
                origin: origin.into(),
                path_prefix: Some("/api/v4".into()),
                methods: vec![HttpMethod::Get],
                private_network,
                credentials: vec!["token".into()],
            },
            decided_by: "user".into(),
            decided_at: 1,
            granted_for_version: "0.1.2".into(),
        }
    }

    fn planned(
        source: &LegacySource,
        settings: &BTreeMap<String, Value>,
        consents: &[StoredUserOrigin],
        token: bool,
    ) -> Result<Planned, NotImportable> {
        plan(
            source,
            &manifest(),
            settings,
            consents,
            Approval { token },
            ID.into(),
        )
    }

    /// The defaults (nothing saved, or the public API) become the public
    /// service's connection, read-only, named after the old plugin, with
    /// its source recorded; the token only when approved.
    #[test]
    fn the_public_services_are_cloud_connections() {
        for (source, saved, origin, prefix, web) in [
            (
                github(),
                None,
                "https://api.github.com",
                None,
                "https://github.com",
            ),
            (
                github(),
                Some("https://api.github.com/"),
                "https://api.github.com",
                None,
                "https://github.com",
            ),
            (
                gitlab(),
                None,
                "https://gitlab.com",
                Some("/api/v4"),
                "https://gitlab.com",
            ),
        ] {
            let p = planned(source, &settings(saved, 20), &[], true).unwrap();
            let c = &p.connection;
            assert_eq!(c.deployment, Deployment::Cloud, "{saved:?}");
            assert_eq!(
                (
                    c.origin.as_str(),
                    c.path_prefix.as_deref(),
                    c.web_origin.as_str()
                ),
                (origin, prefix, web)
            );
            assert_eq!(c.display_name, source.label);
            assert_eq!(c.methods, vec![HttpMethod::Get]);
            assert!(!c.private_network);
            assert_eq!(c.legacy_source.as_deref(), Some(source.plugin));
            assert_eq!(p.secret_slot, Some("token"));
        }
        let p = planned(gitlab(), &settings(None, 20), &[], false).unwrap();
        assert_eq!(p.secret_slot, None, "the token is never copied unapproved");
    }

    /// A self-managed GitLab (with a path prefix and a port) or GitHub
    /// Enterprise Server becomes a self-hosted connection; a private
    /// network only with the old plugin's consent to exactly that server
    /// with its private network.
    #[test]
    fn servers_the_user_entered_are_self_hosted() {
        let p = planned(
            gitlab(),
            &settings(Some("https://code.example.com:8443/gitlab/api/v4"), 20),
            &[],
            false,
        )
        .unwrap();
        let c = &p.connection;
        assert_eq!(c.deployment, Deployment::SelfHosted);
        assert_eq!(c.origin, "https://code.example.com:8443");
        assert_eq!(c.path_prefix.as_deref(), Some("/gitlab/api/v4"));
        assert_eq!(c.web_origin, "https://code.example.com:8443");
        assert_eq!(c.display_name, "GitLab code.example.com:8443");
        assert!(!c.private_network);

        let p = planned(
            github(),
            &settings(Some("https://ghe.example.com/api/v3"), 20),
            &[],
            false,
        )
        .unwrap();
        assert_eq!(p.connection.path_prefix.as_deref(), Some("/api/v3"));
        assert_eq!(p.connection.deployment, Deployment::SelfHosted);

        // A private address: only with the consent to exactly it, with its
        // private network.
        let private = settings(Some("https://gitlab.internal/api/v4"), 20);
        assert_eq!(
            planned(gitlab(), &private, &[], true).unwrap_err(),
            NotImportable::PrivateNetwork
        );
        for other in [
            consent("https://gitlab.internal", false),
            consent("https://other.internal", true),
        ] {
            assert_eq!(
                planned(gitlab(), &private, &[other], true).unwrap_err(),
                NotImportable::PrivateNetwork
            );
        }
        let p = planned(
            gitlab(),
            &private,
            &[consent("https://gitlab.internal", true)],
            true,
        )
        .unwrap();
        assert!(p.connection.private_network);
    }

    /// Nothing is guessed: a data-residency GitHub (A10, OD-7), a plain
    /// http server, a user name in the address, a type the successor does
    /// not declare.
    #[test]
    fn what_a_connection_cannot_name_is_not_imported() {
        for url in [
            "https://api.acme.ghe.com",
            "https://API.Acme.GHE.com/",
            "https://api.acme.ghe.com/api/v3",
        ] {
            assert_eq!(
                planned(github(), &settings(Some(url), 20), &[], true).unwrap_err(),
                NotImportable::DataResidency,
                "{url}"
            );
        }
        // Not data residency: a GHES host that merely ends like it.
        assert!(!data_residency("https://api.acme.ghe.com.example.com"));
        for url in [
            "http://code.example.com/api/v4",
            "https://user:pw@code.example.com/api/v4",
            "https://code.example.com/api/v4?private_token=x",
        ] {
            let refused = planned(gitlab(), &settings(Some(url), 20), &[], true).unwrap_err();
            assert_eq!(refused.code(), "invalid-server", "{url}");
            assert!(!format!("{refused:?}").contains("pw"), "{refused:?}");
        }
        let mut json: Value = serde_json::from_str(include_str!(
            "../../../../crates/deckpilot-plugin-manifest/tests/fixtures/connections-manifest.json"
        ))
        .unwrap();
        json["contributes"]["connectionTypes"]
            .as_array_mut()
            .unwrap()
            .retain(|t| t["id"] != "github");
        json["permissions"][0]["types"] = json!(["gitlab"]);
        let narrow = Manifest::from_json(&json.to_string()).unwrap();
        assert_eq!(
            plan(
                github(),
                &narrow,
                &settings(None, 20),
                &[],
                Approval::default(),
                ID.into()
            )
            .unwrap_err(),
            NotImportable::NoConnectionType
        );
    }

    /// A9 is open: the old `timeoutSeconds` is not carried over, whatever
    /// it is; the wrappers keep their fixed 20 s.
    #[test]
    fn the_time_limit_stays_the_wrappers_fixed_one_until_a9_is_decided() {
        for timeout in [1, 20, 120] {
            let p = planned(github(), &settings(None, timeout), &[], false).unwrap();
            assert!(p.successor_settings.is_empty(), "{timeout}");
        }
    }

    fn consent_to(origin: &str, prefix: Option<&str>, credentials: &[&str]) -> StoredUserOrigin {
        let mut c = consent(origin, false);
        c.consent.path_prefix = prefix.map(str::to_owned);
        c.consent.credentials = credentials.iter().map(|k| (*k).to_owned()).collect();
        c
    }

    /// The token of a server the user entered is copied only when the old
    /// plugin had the consent to send it there: the same origin, a prefix
    /// the connection's lies within, the token slot. The public services
    /// need none (the old plugin's manifest names them).
    #[test]
    fn a_token_follows_only_the_old_plugins_consent() {
        let server = settings(Some("https://code.example.com/gitlab/api/v4"), 20);
        for allowed in [
            consent_to(
                "https://code.example.com",
                Some("/gitlab/api/v4"),
                &["token"],
            ),
            consent_to("https://code.example.com", Some("/gitlab"), &["token"]),
            consent_to("https://code.example.com", None, &["token"]),
        ] {
            let p = planned(gitlab(), &server, &[allowed], true).unwrap();
            assert_eq!(p.secret_slot, Some("token"));
        }
        for refused in [
            Vec::new(),
            vec![consent_to(
                "https://code.example.com",
                Some("/gitlab/api/v4"),
                &[],
            )],
            vec![consent_to(
                "https://code.example.com",
                Some("/other"),
                &["token"],
            )],
            vec![consent_to(
                "https://code.example.com",
                Some("/git"),
                &["token"],
            )],
            vec![consent_to(
                "https://evil.example.com",
                Some("/gitlab/api/v4"),
                &["token"],
            )],
        ] {
            assert_eq!(
                planned(gitlab(), &server, &refused, true).unwrap_err(),
                NotImportable::TokenConsent,
                "{refused:?}"
            );
            // The server alone still imports.
            assert!(planned(gitlab(), &server, &refused, false).is_ok());
        }
        assert_eq!(
            planned(github(), &settings(None, 20), &[], true)
                .unwrap()
                .secret_slot,
            Some("token")
        );
    }

    /// Case, a default port or a trailing dot never change what is
    /// decided: the public API stays the cloud, data residency stays
    /// refused; a private network only on a private host.
    #[test]
    fn spelling_never_decides() {
        for url in ["https://api.github.com:443", "HTTPS://API.GITHUB.COM/"] {
            let p = planned(github(), &settings(Some(url), 20), &[], true).unwrap();
            assert_eq!(p.connection.deployment, Deployment::Cloud, "{url}");
            assert_eq!(p.connection.path_prefix, None, "{url}");
        }
        assert_eq!(
            planned(
                github(),
                &settings(Some("HTTPS://api.acme.ghe.com"), 20),
                &[],
                false
            )
            .unwrap_err(),
            NotImportable::DataResidency
        );
        // A trailing dot: refused (never imported as a guess), whichever
        // check stops it; the check itself ignores the dot too.
        assert!(
            planned(
                github(),
                &settings(Some("https://api.acme.ghe.com./"), 20),
                &[],
                false
            )
            .is_err()
        );
        assert!(data_residency("https://api.acme.ghe.com."));
        let mut public = consent_to("https://code.example.com", Some("/api/v4"), &["token"]);
        public.consent.private_network = true;
        let p = planned(
            gitlab(),
            &settings(Some("https://code.example.com/api/v4"), 20),
            &[public],
            true,
        )
        .unwrap();
        assert!(!p.connection.private_network);
    }
}
