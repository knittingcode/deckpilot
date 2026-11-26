//! Plugin API 1.14, connections (`docs/marketplace/sdk/capabilities.md`
//! "Connections"): what the broker allowed for a connection the invocation
//! is bound to.
//!
//! * `http.fetch` with `connection`: the secret is the connection's, read
//!   from plugins.db for every hop and injected with the scheme its type
//!   declares (a bearer token, or the type's own header such as
//!   `PRIVATE-TOKEN`); never a credential slot's. It is redacted from the
//!   answer like every injected secret (`Executor::fetch`). A connection
//!   without a secret sends nothing.
//! * `connections.get`: the connection's record without its secret.
//! * `connections.reportStatus`: the status and detail code are stored on
//!   the connection, which the app shows.
//! * `time.sleep`: at most what the broker allowed, never closer than a
//!   second to the invocation's deadline, woken at once when the
//!   invocation is interrupted (then an error, `cancelled`).

use std::time::{Duration, Instant};

use deckpilot_plugin_abi::connections::{ConnectionInfo, ConnectionStatus, Deployment};
use deckpilot_plugin_abi::{CallErrorCode, CallResult};
use deckpilot_plugin_broker::PluginId;
use deckpilot_plugin_manifest::CredentialScheme;
use deckpilot_plugin_store::{ConnectionStatus as Stored, Deployment as StoredDeployment};
use deckpilot_plugin_supervisor::CallInterrupt;
use serde_json::Value;

use super::{Auth, Executor};

/// What a sleep keeps free of the invocation's deadline, so the plugin can
/// still answer.
const DEADLINE_MARGIN: Duration = Duration::from_secs(1);
/// How often a sleep looks at its invocation's interrupt.
const SLEEP_POLL: Duration = Duration::from_millis(20);

fn missing() -> CallResult {
    CallResult::error(
        CallErrorCode::NotPermitted,
        "connection-missing: the connection was removed",
    )
}

fn abi_method(method: deckpilot_plugin_manifest::HttpMethod) -> deckpilot_plugin_abi::HttpMethod {
    use deckpilot_plugin_abi::HttpMethod as A;
    use deckpilot_plugin_manifest::HttpMethod as M;
    match method {
        M::Get => A::Get,
        M::Head => A::Head,
        M::Post => A::Post,
        M::Put => A::Put,
        M::Patch => A::Patch,
        M::Delete => A::Delete,
    }
}

/// The abi's status of a stored one.
pub(super) fn abi_status(status: Stored, enabled: bool) -> ConnectionStatus {
    if !enabled {
        return ConnectionStatus::Disabled;
    }
    match status {
        Stored::Ok => ConnectionStatus::Ok,
        Stored::Unauthorized => ConnectionStatus::Unauthorized,
        Stored::InsufficientScope => ConnectionStatus::InsufficientScope,
        Stored::Unreachable => ConnectionStatus::Unreachable,
        Stored::TlsFailed => ConnectionStatus::TlsFailed,
        Stored::NeedsReconnect => ConnectionStatus::NeedsReconnect,
        Stored::Untested => ConnectionStatus::Untested,
    }
}

/// The stored status of a reportable one (the broker passes only those).
fn stored_status(status: ConnectionStatus) -> Option<Stored> {
    Some(match status {
        ConnectionStatus::Ok => Stored::Ok,
        ConnectionStatus::Unauthorized => Stored::Unauthorized,
        ConnectionStatus::InsufficientScope => Stored::InsufficientScope,
        ConnectionStatus::Unreachable => Stored::Unreachable,
        ConnectionStatus::TlsFailed => Stored::TlsFailed,
        _ => return None,
    })
}

impl Executor {
    /// The secret of `connection` as the HTTP client injects it, in the
    /// scheme of the connection's type.
    pub(super) fn connection_auth(
        &self,
        plugin: &PluginId,
        connection: &str,
        fetch: &deckpilot_plugin_broker::AllowedFetch,
    ) -> Result<Auth, CallResult> {
        let (record, secret, auth) = self.with_store(plugin, |data, id, manifest| {
            let record = data.connection(id, connection);
            let secret = data.connection_secret(id, connection);
            let auth = record.as_ref().ok().and_then(Option::as_ref).and_then(|r| {
                manifest
                    .contributes
                    .connection_types()
                    .iter()
                    .find(|t| t.id == r.type_id)
                    .and_then(|t| t.auth.first().cloned())
            });
            (record, secret, auth)
        })?;
        let failed = |e: deckpilot_plugin_store::StoreError| {
            CallResult::error(CallErrorCode::Failed, e.to_string())
        };
        let record = record.map_err(failed)?.ok_or_else(missing)?;
        if !record.enabled {
            return Err(CallResult::error(
                CallErrorCode::Unavailable,
                "connection-disabled: the connection is turned off in DeckPilot",
            ));
        }
        // Defense in depth: the token goes only to the server stored with
        // it now, whatever the broker's copy of the grant said.
        if fetch.origin.to_string() != record.origin {
            return Err(CallResult::error(
                CallErrorCode::NotPermitted,
                "connection-missing: the request is not for the connection's server",
            ));
        }
        let secret = secret.map_err(failed)?.ok_or_else(|| {
            CallResult::error(
                CallErrorCode::Unavailable,
                "the connection has no token yet; add it to the connection in DeckPilot",
            )
        })?;
        let auth = auth.ok_or_else(|| {
            CallResult::error(
                CallErrorCode::Failed,
                "the connection's type declares no way to sign in",
            )
        })?;
        let value = deckpilot_core::Secret::new(secret.expose());
        match auth.scheme {
            CredentialScheme::Bearer => Ok(Auth::Bearer(value)),
            CredentialScheme::Header => match auth.header_name.filter(|n| !n.is_empty()) {
                Some(name) => Ok(Auth::Header(name, value)),
                None => Err(CallResult::error(
                    CallErrorCode::Failed,
                    "the connection's type declares no header name",
                )),
            },
            // `basic` needs a user name stored with the connection, which
            // no connection has yet; a secret never goes in a URL.
            CredentialScheme::Basic | CredentialScheme::Query => Err(CallResult::error(
                CallErrorCode::NotPermitted,
                "this way of signing in is not supported for connections",
            )),
        }
    }

    /// `connections.get`: the record without its secret.
    pub(super) fn connection_info(&self, plugin: &PluginId, connection: &str) -> CallResult {
        let record = self
            .with_store(plugin, |data, id, _| data.connection(id, connection))
            .and_then(|r| r.map_err(|e| CallResult::error(CallErrorCode::Failed, e.to_string())));
        let record = match record {
            Ok(Some(record)) => record,
            Ok(None) => return missing(),
            Err(refused) => return refused,
        };
        let info = ConnectionInfo {
            id: record.id.clone(),
            type_id: record.type_id.clone(),
            deployment: match record.deployment {
                StoredDeployment::Cloud => Deployment::Cloud,
                StoredDeployment::SelfHosted => Deployment::SelfHosted,
            },
            api_base: record.api_base(),
            web_base: record.web_origin.clone(),
            methods: record.methods.iter().copied().map(abi_method).collect(),
            status: abi_status(record.status, record.enabled),
            capabilities: record.capabilities.clone(),
        };
        serde_json::to_value(&info).map_or_else(
            |e| CallResult::error(CallErrorCode::Failed, e.to_string()),
            CallResult::Ok,
        )
    }

    /// `connections.reportStatus`: stored on the connection.
    pub(super) fn report_connection_status(
        &self,
        plugin: &PluginId,
        connection: &str,
        status: ConnectionStatus,
        detail: Option<&str>,
    ) -> CallResult {
        let Some(status) = stored_status(status) else {
            return CallResult::error(CallErrorCode::BadRequest, "not a status a plugin reports");
        };
        let written = self.with_store(plugin, |data, id, _| {
            data.report_connection_status(id, connection, status, detail)
        });
        match written {
            Ok(Ok(())) => CallResult::Ok(Value::Null),
            Ok(Err(e)) => CallResult::error(CallErrorCode::BadRequest, e.to_string()),
            Err(refused) => refused,
        }
    }
}

/// `time.sleep`: `ms`, but never closer than [`DEADLINE_MARGIN`] to the
/// invocation's deadline; an interrupt ends it at once.
pub(super) fn sleep(ms: u32, interrupt: Option<&CallInterrupt>) -> CallResult {
    let wanted = Instant::now() + Duration::from_millis(u64::from(ms));
    let until = interrupt.map_or(wanted, |i| {
        wanted.min(
            i.deadline()
                .checked_sub(DEADLINE_MARGIN)
                .unwrap_or_else(Instant::now),
        )
    });
    loop {
        if interrupt.is_some_and(CallInterrupt::is_raised) {
            return CallResult::error(
                CallErrorCode::Unavailable,
                "cancelled: the action was cancelled or ran out of time",
            );
        }
        let now = Instant::now();
        if now >= until {
            return CallResult::Ok(Value::Null);
        }
        std::thread::sleep(SLEEP_POLL.min(until - now));
    }
}

#[cfg(all(test, feature = "test-hooks"))]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use deckpilot_plugin_abi::CapabilityRequest;
    use deckpilot_plugin_abi::capability::{RunContext, RunTrigger};
    use deckpilot_plugin_broker::{ApprovedPermissions, Broker, Decision};
    use deckpilot_plugin_manifest::Manifest;
    use deckpilot_plugin_store::{DEFAULT_BUSY_TIMEOUT, Database, PluginData, StoreKey};
    use serde_json::{Value, json};
    use tokio::sync::broadcast;

    use super::super::test_transport::Canned;
    use super::super::{DataAccess, Executor, Opener, OwnVariables, Transport};
    use super::*;

    const GITLAB: &str = "cn_01JD7Q0000000000000000000A";
    const GITHUB: &str = "cn_01JD7Q0000000000000000000B";
    const TOKEN: &str = "dp-test-gitlab-secret-0001";
    const GITHUB_TOKEN: &str = "dp-test-github-secret-0002";

    struct Fixture {
        broker: Broker,
        plugin: PluginId,
        data: Arc<Mutex<Option<PluginData>>>,
        manifest: Arc<Manifest>,
        log: std::path::PathBuf,
        dir: tempfile::TempDir,
    }

    fn manifest() -> Manifest {
        Manifest::from_json(include_str!(
            "../../../../../crates/deckpilot-plugin-manifest/tests/fixtures/connections-manifest.json"
        ))
        .unwrap()
    }

    /// plugins.db with two connections (a self-hosted GitLab with its
    /// token, a GitHub.com with its own) and the broker's records of them.
    fn fixture(gitlab_enabled_in_store: bool) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plugins.db");
        let key = StoreKey::from_bytes([0x55; 32]);
        let manifest = manifest();
        let plugin = PluginId::new(manifest.id.as_str()).unwrap();
        {
            let mut db = Database::open(&path, &key, DEFAULT_BUSY_TIMEOUT).unwrap();
            let tx = db.write().unwrap();
            for (id, ty, origin, prefix, web, enabled, secret) in [
                (
                    GITLAB,
                    "gitlab",
                    "https://code.example.com:8443",
                    Some("/gitlab/api/v4"),
                    "https://code.example.com:8443",
                    gitlab_enabled_in_store,
                    TOKEN,
                ),
                (
                    GITHUB,
                    "github",
                    "https://api.github.com",
                    None,
                    "https://github.com",
                    true,
                    GITHUB_TOKEN,
                ),
            ] {
                tx.execute(
                    "INSERT INTO plugin_connections (plugin_id, connection_id, type_id,
                         display_name, deployment, origin, path_prefix, web_origin, auth_method,
                         methods_json, private_network, enabled, status, consent_by, consent_at,
                         created_at, updated_at)
                     VALUES (?1, ?2, ?3, 'n', 'selfHosted', ?4, ?5, ?6, 'pat', '[\"GET\"]', 0,
                         ?7, 'untested', 'user', 1, 1, 1)",
                    rusqlite::params![manifest.id.as_str(), id, ty, origin, prefix, web, enabled],
                )
                .unwrap();
                tx.execute(
                    "INSERT INTO plugin_connection_secrets VALUES (?1, ?2, ?3, 1)",
                    rusqlite::params![manifest.id.as_str(), id, secret],
                )
                .unwrap();
            }
            tx.commit().unwrap();
        }
        let data = PluginData::open(&path, &key, DEFAULT_BUSY_TIMEOUT).unwrap();
        let broker = Broker::new(
            Arc::new(deckpilot_plugin_broker::MemoryAudit::new(100)),
            Arc::new(deckpilot_plugin_broker::ManualClock::default()),
            deckpilot_plugin_broker::RateLimits::default(),
        );
        broker
            .set_grants(
                &plugin,
                &ApprovedPermissions::from_json(
                    r#"[
                    {"kind":"connection","id":"cn_01JD7Q0000000000000000000A",
                     "origin":"https://code.example.com:8443","pathPrefix":"/gitlab/api/v4",
                     "webOrigin":"https://code.example.com:8443","methods":["GET"],
                     "enabled":true,"headerName":"PRIVATE-TOKEN"},
                    {"kind":"connection","id":"cn_01JD7Q0000000000000000000B",
                     "origin":"https://api.github.com","webOrigin":"https://github.com",
                     "methods":["GET"],"enabled":true}]"#,
                )
                .unwrap(),
            )
            .unwrap();
        Fixture {
            broker,
            plugin,
            data: Arc::new(Mutex::new(Some(data))),
            manifest: Arc::new(manifest),
            log: dir.path().join("http.log"),
            dir,
        }
    }

    impl Fixture {
        fn executor(&self, routes: &Value, bound: &[&str]) -> Executor {
            let (events, _) = broadcast::channel(4);
            Executor::new(
                self.broker.clone(),
                "Git Providers".into(),
                events,
                Transport::Canned(Arc::new(Canned::new(routes, self.log.clone()))),
                OwnVariables {
                    store: crate::plugins::variables::PublishedVariables::default(),
                    namespace: "plugins.deckpilot.gitProviders".into(),
                    generation: 0,
                    declared: Vec::new(),
                },
                Opener::System,
            )
            .with_data(DataAccess {
                data: self.data.clone(),
                manifest: self.manifest.clone(),
            })
            .with_run(RunContext::direct(RunTrigger::Key))
            .with_connections(bound.iter().map(|s| (*s).to_owned()).collect())
        }

        /// What the supervisor does with one call.
        fn call(&self, executor: &mut Executor, request: &Value) -> CallResult {
            use deckpilot_plugin_supervisor::CapabilityExecutor as _;
            let request: CapabilityRequest = serde_json::from_value(request.clone()).unwrap();
            let interrupt = CallInterrupt::new(None, Instant::now() + Duration::from_secs(60));
            let session = self.broker.session(self.plugin.clone());
            match session.authorize_bound(
                &request,
                executor.run_context(),
                executor.bound_connections(),
            ) {
                Decision::Allow(allowed) => {
                    executor.execute_interruptible(&self.plugin, &allowed, &interrupt)
                }
                Decision::Deny(reason) => {
                    CallResult::error(reason.call_error_code(), reason.to_string())
                }
            }
        }

        fn sent(&self) -> String {
            std::fs::read_to_string(&self.log).unwrap_or_default()
        }
    }

    fn fetch(url: &str, connection: &str) -> Value {
        json!({"op": "http.fetch", "args": {"method": "GET", "url": url, "connection": connection}})
    }

    fn ok(result: &CallResult) -> &Value {
        match result {
            CallResult::Ok(v) => v,
            other @ CallResult::Error(_) => panic!("{other:?}"),
        }
    }

    fn message(result: &CallResult) -> String {
        serde_json::to_string(result).unwrap()
    }

    const PROJECTS: &str = "https://code.example.com:8443/gitlab/api/v4/projects";

    /// The connection's own token, in its type's header, on its own server
    /// only; an echo of it is redacted before the plugin sees the answer.
    #[test]
    fn the_connections_secret_is_injected_and_redacted() {
        let f = fixture(true);
        let routes = json!([
            {"url": PROJECTS, "status": 200,
             "headers": [["x-total", "1"], ["x-echo", TOKEN]],
             "body": format!(r#"[{{"id":4711,"token":"{TOKEN}"}}]"#)},
            {"url": "https://api.github.com/user", "status": 200, "body": "{\"login\":\"octo\"}"}
        ]);
        let mut executor = f.executor(&routes, &[GITLAB, GITHUB]);
        let result = f.call(&mut executor, &fetch(PROJECTS, GITLAB));
        let answer = ok(&result).to_string();
        assert!(!answer.contains(TOKEN), "the answer holds the token");
        assert!(answer.contains("[redacted]"));
        assert!(
            f.sent()
                .contains(&format!("{PROJECTS}\tPRIVATE-TOKEN: {TOKEN}"))
        );

        let result = f.call(&mut executor, &fetch("https://api.github.com/user", GITHUB));
        ok(&result);
        let sent = f.sent();
        assert!(sent.contains(&format!("\tauthorization: Bearer {GITHUB_TOKEN}")));
        // Each secret went to its own server only.
        for line in sent.lines() {
            if line.starts_with("https://api.github.com") {
                assert!(!line.contains(TOKEN), "the GitLab token went to GitHub");
            } else {
                assert!(
                    !line.contains(GITHUB_TOKEN),
                    "the GitHub token went elsewhere"
                );
            }
        }
    }

    /// Redirects: within the connection the token goes along; off it the
    /// request ends with `redirect-refused` and nothing is sent there.
    #[test]
    fn a_redirect_off_the_connection_sends_nothing_there() {
        let f = fixture(true);
        let routes = json!([
            {"url": format!("{PROJECTS}?page=2"), "status": 200, "body": "[]"},
            {"url": format!("{PROJECTS}?moved"), "status": 302,
             "headers": [["location", "/gitlab/api/v4/projects?page=2"]]},
            {"url": format!("{PROJECTS}?leak"), "status": 302,
             "headers": [["location", "https://collector.example/steal"]]},
            {"url": format!("{PROJECTS}?github"), "status": 307,
             "headers": [["location", "https://api.github.com/user"]]},
            {"url": "https://collector.example/", "status": 200, "body": "got it"},
            {"url": "https://api.github.com/", "status": 200, "body": "{}"}
        ]);
        let mut executor = f.executor(&routes, &[GITLAB, GITHUB]);
        let result = f.call(&mut executor, &fetch(&format!("{PROJECTS}?moved"), GITLAB));
        ok(&result);
        assert!(
            f.sent()
                .contains(&format!("{PROJECTS}?page=2\tPRIVATE-TOKEN: {TOKEN}"))
        );
        for target in ["leak", "github"] {
            let result = f.call(
                &mut executor,
                &fetch(&format!("{PROJECTS}?{target}"), GITLAB),
            );
            // The code comes first, as plugins read it.
            match &result {
                CallResult::Error(error) => {
                    assert!(
                        error.message.starts_with("redirect-refused: "),
                        "{result:?}"
                    );
                }
                other @ CallResult::Ok(_) => panic!("{other:?}"),
            }
        }
        let sent = f.sent();
        assert!(
            !sent.contains("collector.example"),
            "a request went to the collector"
        );
        assert!(
            !sent.contains("https://api.github.com"),
            "a request went to GitHub"
        );
    }

    /// A connection that is off in the store is refused even if the broker
    /// still had it; one without a token sends nothing.
    #[test]
    fn a_connection_turned_off_or_without_a_token_sends_nothing() {
        let f = fixture(false);
        let mut executor = f.executor(&json!([{"url": PROJECTS, "status": 200}]), &[GITLAB]);
        let result = f.call(&mut executor, &fetch(PROJECTS, GITLAB));
        assert!(
            message(&result).contains("connection-disabled"),
            "{result:?}"
        );
        assert_eq!(f.sent(), "");

        let f = fixture(true);
        {
            let path = f.dir.path().join("plugins.db");
            let mut db = Database::open(
                &path,
                &StoreKey::from_bytes([0x55; 32]),
                DEFAULT_BUSY_TIMEOUT,
            )
            .unwrap();
            let tx = db.write().unwrap();
            tx.execute("DELETE FROM plugin_connection_secrets", [])
                .unwrap();
            tx.commit().unwrap();
        }
        let mut executor = f.executor(&json!([{"url": PROJECTS, "status": 200}]), &[GITLAB]);
        let result = f.call(&mut executor, &fetch(PROJECTS, GITLAB));
        assert!(message(&result).contains("no token"), "{result:?}");
        assert_eq!(f.sent(), "");
    }

    /// A connection's responses carry its token's scopes and expiry; a
    /// fetch that is not through a connection never does, even from the
    /// same server (least privilege).
    #[test]
    fn scope_headers_reach_the_plugin_on_a_connection_only() {
        let f = fixture(true);
        f.broker
            .set_grants(
                &f.plugin,
                &ApprovedPermissions::from_json(
                    r#"[{"kind":"network","origin":"https://api.github.com","methods":["GET"]},
                    {"kind":"connection","id":"cn_01JD7Q0000000000000000000B",
                     "origin":"https://api.github.com","webOrigin":"https://github.com",
                     "methods":["GET"],"enabled":true}]"#,
                )
                .unwrap(),
            )
            .unwrap();
        let routes = json!([{"url": "https://api.github.com/user", "status": 200,
            "headers": [["x-oauth-scopes", "repo, read:org"],
                        ["X-Accepted-OAuth-Scopes", "repo"],
                        ["github-authentication-token-expiration", "2026-12-31 00:00:00 UTC"],
                        ["x-ratelimit-remaining", "4999"]],
            "body": "{}"}]);
        let names = |result: &CallResult| -> Vec<String> {
            ok(result)["headers"]
                .as_array()
                .unwrap()
                .iter()
                .map(|h| h["name"].as_str().unwrap().to_owned())
                .collect()
        };
        let mut executor = f.executor(&routes, &[GITHUB]);
        let on_connection = f.call(&mut executor, &fetch("https://api.github.com/user", GITHUB));
        assert_eq!(
            names(&on_connection),
            [
                "x-oauth-scopes",
                "x-accepted-oauth-scopes",
                "github-authentication-token-expiration",
                "x-ratelimit-remaining"
            ]
        );
        let plain = f.call(
            &mut executor,
            &json!({"op": "http.fetch", "args": {"method": "GET", "url": "https://api.github.com/user"}}),
        );
        assert_eq!(names(&plain), ["x-ratelimit-remaining"]);
    }

    /// The token goes only to the server stored with the connection now:
    /// a broker grant that disagrees with the store (a stale copy) sends
    /// nothing.
    #[test]
    fn the_token_follows_the_stored_server_only() {
        let f = fixture(true);
        f.broker
            .set_grants(
                &f.plugin,
                &ApprovedPermissions::from_json(
                    r#"[{"kind":"connection","id":"cn_01JD7Q0000000000000000000A",
                     "origin":"https://stale.example.com","pathPrefix":"/gitlab/api/v4",
                     "webOrigin":"https://stale.example.com","methods":["GET"],
                     "enabled":true,"headerName":"PRIVATE-TOKEN"}]"#,
                )
                .unwrap(),
            )
            .unwrap();
        let stale = "https://stale.example.com/gitlab/api/v4/projects";
        let mut executor = f.executor(&json!([{"url": stale, "status": 200}]), &[GITLAB]);
        let result = f.call(&mut executor, &fetch(stale, GITLAB));
        assert!(
            message(&result).contains("connection-missing"),
            "{result:?}"
        );
        assert_eq!(f.sent(), "");
    }

    /// `connections.get` answers the record without any secret;
    /// `connections.reportStatus` stores the status on the connection.
    #[test]
    fn get_and_report_status() {
        let f = fixture(true);
        let mut executor = f.executor(&json!([]), &[GITLAB]);
        let result = f.call(
            &mut executor,
            &json!({"op": "connections.get", "args": {"connection": GITLAB}}),
        );
        let info = ok(&result);
        assert_eq!(
            info["apiBase"],
            "https://code.example.com:8443/gitlab/api/v4"
        );
        assert_eq!(info["type"], "gitlab");
        assert_eq!(info["methods"], json!(["GET"]));
        assert!(!info.to_string().contains(TOKEN));
        // Another connection: not bound.
        let result = f.call(
            &mut executor,
            &json!({"op": "connections.get", "args": {"connection": GITHUB}}),
        );
        assert!(message(&result).contains("connection-missing"));

        let result = f.call(
            &mut executor,
            &json!({"op": "connections.reportStatus",
                    "args": {"connection": GITLAB, "status": "unauthorized", "detail": "token-expired"}}),
        );
        ok(&result);
        let row = f
            .data
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .connection(
                &deckpilot_plugin_manifest::PluginId::parse(f.plugin.as_str()).unwrap(),
                GITLAB,
            )
            .unwrap()
            .unwrap();
        assert_eq!(row.status, Stored::Unauthorized);
        assert_eq!(row.status_detail.as_deref(), Some("token-expired"));
    }

    /// `time.sleep` waits, ends at once when interrupted, and never runs
    /// into the last second of the deadline.
    #[test]
    fn sleeping_is_interruptible_and_bounded_by_the_deadline() {
        let started = Instant::now();
        assert!(matches!(sleep(50, None), CallResult::Ok(_)));
        assert!(started.elapsed() >= Duration::from_millis(50));

        let abort = deckpilot_plugin_supervisor::AbortSignal::new();
        let interrupt = CallInterrupt::new(
            Some(abort.clone()),
            Instant::now() + Duration::from_secs(60),
        );
        let waker = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            abort.abort();
        });
        let started = Instant::now();
        let result = sleep(30_000, Some(&interrupt));
        waker.join().unwrap();
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(message(&result).contains("cancelled"), "{result:?}");

        let near = CallInterrupt::new(None, Instant::now() + Duration::from_millis(1_200));
        let started = Instant::now();
        assert!(matches!(sleep(30_000, Some(&near)), CallResult::Ok(_)));
        assert!(started.elapsed() < Duration::from_millis(1_000));
    }
}
