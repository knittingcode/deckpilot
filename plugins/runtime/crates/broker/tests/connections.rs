//! Plugin API 1.14, connections: every operation on a connection is
//! allowed only for a connection the invocation is bound to, that the user
//! created (a grant record of its own) and has not turned off; a request on
//! one goes to exactly its origin, within its path prefix, with its
//! methods, never with the header its secret travels in; a redirect stays
//! on it or ends the request (`redirect-refused`); and a fetch naming a
//! connection is never checked as an ordinary fetch to a granted origin.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use deckpilot_plugin_abi::capability::{Header, HttpFetch, OpenUrl, RunContext, RunTrigger};
use deckpilot_plugin_abi::connections::{
    ConnectionRef, ConnectionStatus, ConnectionStatusReport, Sleep,
};
use deckpilot_plugin_abi::{CallErrorCode, CapabilityRequest, HttpMethod};
use deckpilot_plugin_broker::*;

const GITLAB: &str = "cn_01JD7Q0000000000000000000A";
const OTHER: &str = "cn_01JD7Q0000000000000000000B";
const OFF: &str = "cn_01JD7Q0000000000000000000C";
const LAN: &str = "cn_01JD7Q0000000000000000000D";

/// A self-hosted GitLab with a port and a path prefix (GET only), a second
/// GitHub-like connection that may POST, one turned off, and one on a
/// private network; plus an ordinary network grant of the same host as the
/// first, which a connection request never uses.
const GRANTS: &str = r#"[
    {"kind":"network","origin":"https://code.example.com:8443","methods":["GET","POST"]},
    {"kind":"dialogs"},
    {"kind":"connection","id":"cn_01JD7Q0000000000000000000A",
     "origin":"https://code.example.com:8443","pathPrefix":"/gitlab/api/v4",
     "webOrigin":"https://code.example.com:8443","methods":["GET"],"enabled":true,
     "openUrl":true,"headerName":"PRIVATE-TOKEN"},
    {"kind":"connection","id":"cn_01JD7Q0000000000000000000B",
     "origin":"https://api.github.com","webOrigin":"https://github.com",
     "methods":["GET","POST"],"enabled":true,"openUrl":true},
    {"kind":"connection","id":"cn_01JD7Q0000000000000000000C",
     "origin":"https://ghe.example.com","pathPrefix":"/api/v3",
     "webOrigin":"https://ghe.example.com","methods":["GET"],"enabled":false,"openUrl":true},
    {"kind":"connection","id":"cn_01JD7Q0000000000000000000D",
     "origin":"https://10.0.0.5","pathPrefix":"/api/v4","webOrigin":"https://10.0.0.5",
     "methods":["GET"],"privateNetwork":true,"enabled":true,"openUrl":true}
]"#;

fn fixture(limits: RateLimits) -> (Broker, Session, Arc<MemoryAudit>) {
    let audit = Arc::new(MemoryAudit::new(1_000));
    let broker = Broker::new(audit.clone(), Arc::new(ManualClock::default()), limits);
    let plugin = PluginId::new("deckpilot.git-providers").unwrap();
    broker
        .set_grants(&plugin, &ApprovedPermissions::from_json(GRANTS).unwrap())
        .unwrap();
    let session = broker.session(plugin);
    (broker, session, audit)
}

fn session() -> Session {
    fixture(RateLimits::default()).1
}

fn bound(ids: &[&str]) -> BTreeSet<String> {
    ids.iter().map(|s| (*s).to_owned()).collect()
}

fn key() -> RunContext {
    RunContext::direct(RunTrigger::Key)
}

fn fetch(method: HttpMethod, url: &str, connection: &str) -> CapabilityRequest {
    CapabilityRequest::HttpFetch(HttpFetch {
        method,
        url: url.into(),
        headers: vec![Header {
            name: "Accept".into(),
            value: "application/json".into(),
        }],
        body: None,
        auth: None,
        connection: Some(connection.into()),
    })
}

fn get(url: &str, connection: &str) -> CapabilityRequest {
    fetch(HttpMethod::Get, url, connection)
}

fn allowed(d: &Decision) -> &AllowedFetch {
    match d {
        Decision::Allow(Allowed::Fetch(a)) => a,
        other => panic!("{other:?}"),
    }
}

const PROJECTS: &str = "https://code.example.com:8443/gitlab/api/v4/projects?per_page=20";

#[test]
fn a_request_on_a_bound_connection_goes_to_its_server_only() {
    let s = session();
    let d = s.authorize_bound(&get(PROJECTS, GITLAB), Some(&key()), &bound(&[GITLAB]));
    let a = allowed(&d);
    assert_eq!(a.connection.as_deref(), Some(GITLAB));
    assert_eq!(a.auth, None, "the connection's secret, never a slot's");
    assert!(!a.private_network);
    assert_eq!(a.url.as_str(), PROJECTS);

    for (url, reason) in [
        // Another host, port, scheme or a path outside the prefix.
        ("https://api.github.com/user", DenyReason::OriginNotGranted),
        (
            "https://code.example.com/gitlab/api/v4/projects",
            DenyReason::OriginNotGranted,
        ),
        (
            "https://code.example.com:8443/gitlab/api/v40",
            DenyReason::PathNotGranted,
        ),
        (
            "https://code.example.com:8443/admin",
            DenyReason::PathNotGranted,
        ),
        (
            "https://code.example.com:8443/gitlab/api/v4/..%2f..%2fadmin",
            DenyReason::PathNotGranted,
        ),
    ] {
        let d = s.authorize_bound(&get(url, GITLAB), Some(&key()), &bound(&[GITLAB]));
        assert_eq!(d.denial(), Some(reason), "{url}");
    }
    for url in [
        "http://code.example.com:8443/gitlab/api/v4/projects",
        "https://user:pw@code.example.com:8443/gitlab/api/v4/projects",
        "https://127.0.0.1/gitlab/api/v4",
        "https://169.254.169.254/latest/meta-data",
    ] {
        let d = s.authorize_bound(&get(url, GITLAB), Some(&key()), &bound(&[GITLAB]));
        assert!(
            matches!(
                d.denial(),
                Some(DenyReason::Url(_) | DenyReason::OriginNotGranted)
            ),
            "{url}: {d:?}"
        );
    }
    // Read-only: no POST, whatever the token could do.
    let d = s.authorize_bound(
        &fetch(HttpMethod::Post, PROJECTS, GITLAB),
        Some(&key()),
        &bound(&[GITLAB]),
    );
    assert_eq!(d.denial(), Some(DenyReason::MethodNotGranted));
    // The other connection may.
    let d = s.authorize_bound(
        &fetch(
            HttpMethod::Post,
            "https://api.github.com/repos/o/r/pulls",
            OTHER,
        ),
        Some(&key()),
        &bound(&[OTHER]),
    );
    allowed(&d);
}

/// Isolation (T6): a connection the invocation was not given is refused
/// exactly like one that does not exist, so a plugin can neither use nor
/// discover another connection.
#[test]
fn only_connections_the_invocation_is_bound_to_are_reachable() {
    let s = session();
    let github = "https://api.github.com/user";
    for (request, ids) in [
        (get(github, OTHER), bound(&[GITLAB])),
        (get(github, OTHER), bound(&[])),
        (
            get(github, "cn_01JD7Q0000000000000000000Z"),
            bound(&["cn_01JD7Q0000000000000000000Z"]),
        ),
        (
            CapabilityRequest::ConnectionGet(ConnectionRef {
                connection: OTHER.into(),
            }),
            bound(&[GITLAB]),
        ),
        (
            CapabilityRequest::ConnectionReportStatus(ConnectionStatusReport {
                connection: OTHER.into(),
                status: ConnectionStatus::Unauthorized,
                detail: None,
            }),
            bound(&[GITLAB]),
        ),
    ] {
        let d = s.authorize_bound(&request, Some(&key()), &ids);
        assert_eq!(
            d.denial(),
            Some(DenyReason::ConnectionMissing),
            "{}",
            request.op()
        );
    }
    // The plain `authorize_in` binds nothing.
    let d = s.authorize_in(&get(PROJECTS, GITLAB), Some(&key()));
    assert_eq!(d.denial(), Some(DenyReason::ConnectionMissing));
    // The refusal names its code, which the plugin reads.
    assert!(
        DenyReason::ConnectionMissing
            .to_string()
            .starts_with("connection-missing")
    );
    assert_eq!(
        DenyReason::ConnectionMissing.call_error_code(),
        CallErrorCode::NotPermitted
    );

    let d = s.authorize_bound(
        &CapabilityRequest::ConnectionGet(ConnectionRef {
            connection: GITLAB.into(),
        }),
        Some(&key()),
        &bound(&[GITLAB]),
    );
    assert_eq!(
        d,
        Decision::Allow(Allowed::ConnectionGet {
            connection: GITLAB.into()
        })
    );
    let d = s.authorize_bound(
        &CapabilityRequest::ConnectionReportStatus(ConnectionStatusReport {
            connection: GITLAB.into(),
            status: ConnectionStatus::Unauthorized,
            detail: Some("token-expired".into()),
        }),
        Some(&key()),
        &bound(&[GITLAB]),
    );
    assert_eq!(
        d,
        Decision::Allow(Allowed::ConnectionReportStatus {
            connection: GITLAB.into(),
            status: ConnectionStatus::Unauthorized,
            detail: Some("token-expired".into()),
        })
    );
}

/// A connection the user turned off is withheld at once (T11), with its
/// own code.
#[test]
fn a_disabled_connection_is_refused_with_its_code() {
    let s = session();
    for request in [
        get("https://ghe.example.com/api/v3/user", OFF),
        CapabilityRequest::ConnectionGet(ConnectionRef {
            connection: OFF.into(),
        }),
    ] {
        let d = s.authorize_bound(&request, Some(&key()), &bound(&[OFF]));
        assert_eq!(d.denial(), Some(DenyReason::ConnectionDisabled));
    }
    assert!(
        DenyReason::ConnectionDisabled
            .to_string()
            .starts_with("connection-disabled")
    );
}

/// The secret's own header (`PRIVATE-TOKEN`) and every forbidden header are
/// refused, so a guest can neither inject a secret nor overwrite it.
#[test]
fn the_secret_header_cannot_be_set_by_the_plugin() {
    let s = session();
    for name in ["PRIVATE-TOKEN", "private-token", "Authorization", "Cookie"] {
        let request = CapabilityRequest::HttpFetch(HttpFetch {
            method: HttpMethod::Get,
            url: PROJECTS.into(),
            headers: vec![Header {
                name: name.into(),
                value: "dp-test-forged".into(),
            }],
            body: None,
            auth: None,
            connection: Some(GITLAB.into()),
        });
        let d = s.authorize_bound(&request, Some(&key()), &bound(&[GITLAB]));
        assert_eq!(d.denial(), Some(DenyReason::ForbiddenHeader), "{name}");
    }
}

/// Redirects (T1): within the connection's origin and prefix the request
/// goes on (as the same connection); anything else, the second
/// connection's server included, ends it with `redirect-refused`, and
/// https to http is refused as before.
#[test]
fn a_redirect_stays_on_the_connection_or_ends_the_request() {
    let s = session();
    let d = s.authorize_bound(&get(PROJECTS, GITLAB), Some(&key()), &bound(&[GITLAB]));
    let first = allowed(&d).clone();
    let d = s.authorize_redirect(&first, "/gitlab/api/v4/projects?page=2", 1);
    assert_eq!(allowed(&d).connection.as_deref(), Some(GITLAB));
    for location in [
        "https://api.github.com/user",
        "https://code.example.com/gitlab/api/v4/projects",
        "https://code.example.com:8443/elsewhere",
        "https://evil.example/collect",
        "https://169.254.169.254/latest/meta-data",
        "https://127.0.0.1:8443/gitlab/api/v4",
    ] {
        let d = s.authorize_redirect(&first, location, 1);
        assert_eq!(
            d.denial(),
            Some(DenyReason::RedirectRefused),
            "{location}: {d:?}"
        );
    }
    let d = s.authorize_redirect(&first, "http://code.example.com:8443/gitlab/api/v4", 1);
    assert_eq!(d.denial(), Some(DenyReason::InsecureRedirect));
    let d = s.authorize_redirect(&first, "/gitlab/api/v4/x", MAX_REDIRECTS + 1);
    assert_eq!(d.denial(), Some(DenyReason::TooManyRedirects));
    assert!(
        DenyReason::RedirectRefused
            .to_string()
            .starts_with("redirect-refused")
    );
}

/// A private-network connection (SSRF, T2): its private address only
/// because the user consented, never loopback or link-local; a connection
/// without the consent cannot even be granted on a private host.
#[test]
fn a_private_network_connection_needs_its_consent() {
    let s = session();
    let d = s.authorize_bound(
        &get("https://10.0.0.5/api/v4/projects", LAN),
        Some(&key()),
        &bound(&[LAN]),
    );
    assert!(allowed(&d).private_network);
    // Its consent covers its own server only.
    let d = s.authorize_bound(
        &get("https://10.0.0.6/api/v4/projects", LAN),
        Some(&key()),
        &bound(&[LAN]),
    );
    assert!(d.denial().is_some());
    let without = r#"[{"kind":"connection","id":"cn_01JD7Q0000000000000000000D",
        "origin":"https://10.0.0.5","webOrigin":"https://10.0.0.5","methods":["GET"],
        "enabled":true}]"#;
    assert_eq!(
        Grants::derive(&ApprovedPermissions::from_json(without).unwrap()),
        Err(GrantError::PrivateNetworkNotGranted)
    );
    for bad in [
        // Loopback and link-local are never a connection's server.
        r#"[{"kind":"connection","id":"cn_01JD7Q0000000000000000000D","origin":"https://127.0.0.1","webOrigin":"https://127.0.0.1","methods":["GET"],"privateNetwork":true,"enabled":true}]"#,
        r#"[{"kind":"connection","id":"cn_01JD7Q0000000000000000000D","origin":"https://169.254.169.254","webOrigin":"https://169.254.169.254","methods":["GET"],"privateNetwork":true,"enabled":true}]"#,
        r#"[{"kind":"connection","id":"cn_01JD7Q0000000000000000000D","origin":"http://code.example.com","webOrigin":"https://code.example.com","methods":["GET"],"enabled":true}]"#,
        r#"[{"kind":"connection","id":"github","origin":"https://api.github.com","webOrigin":"https://github.com","methods":["GET"],"enabled":true}]"#,
        r#"[{"kind":"connection","id":"cn_01JD7Q0000000000000000000D","origin":"https://api.github.com","webOrigin":"https://github.com","methods":[],"enabled":true}]"#,
        r#"[{"kind":"connection","id":"cn_01JD7Q0000000000000000000D","origin":"https://api.github.com","webOrigin":"https://github.com/x","methods":["GET"],"enabled":true}]"#,
        r#"[{"kind":"connection","id":"cn_01JD7Q0000000000000000000D","origin":"https://api.github.com","webOrigin":"https://github.com","methods":["GET"],"headerName":"bad name","enabled":true}]"#,
    ] {
        assert!(
            Grants::derive(&ApprovedPermissions::from_json(bad).unwrap()).is_err(),
            "{bad}"
        );
    }
    // The same id twice is malformed.
    let twice = r#"[
        {"kind":"connection","id":"cn_01JD7Q0000000000000000000D","origin":"https://api.github.com","webOrigin":"https://github.com","methods":["GET"],"enabled":true},
        {"kind":"connection","id":"cn_01JD7Q0000000000000000000D","origin":"https://api.github.com","webOrigin":"https://github.com","methods":["GET"],"enabled":true}]"#;
    assert_eq!(
        Grants::derive(&ApprovedPermissions::from_json(twice).unwrap()),
        Err(GrantError::Duplicate)
    );
}

/// `openUrl.connections`: pages on the web origin of a bound, enabled
/// connection, in a run the user started; nothing else.
#[test]
fn pages_of_a_bound_connection_may_be_opened() {
    let plenty = Rate::new(100, Duration::from_secs(1));
    let (_, s, _) = fixture(RateLimits {
        open_url: plenty,
        open_url_window: plenty,
        ..RateLimits::default()
    });
    let open = |url: &str| CapabilityRequest::OpenUrl(OpenUrl { url: url.into() });
    let d = s.authorize_bound(
        &open("https://github.com/octo/site/pulls"),
        Some(&key()),
        &bound(&[OTHER]),
    );
    assert!(
        matches!(d, Decision::Allow(Allowed::OpenUrl { .. })),
        "{d:?}"
    );
    let d = s.authorize_bound(
        &open("https://github.com/octo/site/pulls"),
        Some(&key()),
        &bound(&[GITLAB]),
    );
    assert_eq!(d.denial(), Some(DenyReason::OriginNotGranted));
    let d = s.authorize_bound(
        &open("https://ghe.example.com/x"),
        Some(&key()),
        &bound(&[OFF]),
    );
    assert_eq!(d.denial(), Some(DenyReason::OriginNotGranted));
    let d = s.authorize_bound(
        &open("https://10.0.0.5/group/project"),
        Some(&key()),
        &bound(&[LAN]),
    );
    assert!(
        matches!(d, Decision::Allow(Allowed::OpenUrl { .. })),
        "{d:?}"
    );
}

/// Rate limits per connection (60 a minute each by default) and per plugin
/// for all of its connections: one busy connection cannot starve another.
#[test]
fn each_connection_has_its_own_rate() {
    let (_, s, _) = fixture(RateLimits {
        connection_http: Rate::new(2, Duration::from_secs(60)),
        connections_http: Rate::new(3, Duration::from_secs(60)),
        ..RateLimits::default()
    });
    let ids = bound(&[GITLAB, OTHER]);
    let gitlab = || s.authorize_bound(&get(PROJECTS, GITLAB), Some(&key()), &ids);
    let github = || {
        s.authorize_bound(
            &get("https://api.github.com/user", OTHER),
            Some(&key()),
            &ids,
        )
    };
    assert!(gitlab().is_allowed());
    assert!(gitlab().is_allowed());
    assert_eq!(gitlab().denial(), Some(DenyReason::RateLimited));
    assert!(github().is_allowed(), "another connection is not starved");
    assert_eq!(
        github().denial(),
        Some(DenyReason::RateLimited),
        "plugin-wide"
    );
}

/// `time.sleep`: bounded per call (the ABI), rate limited, never in a
/// state call.
#[test]
fn sleeping_is_bounded() {
    let s = session();
    let d = s.authorize_in(
        &CapabilityRequest::TimeSleep(Sleep { ms: 1_000 }),
        Some(&key()),
    );
    assert_eq!(d, Decision::Allow(Allowed::Sleep { ms: 1_000 }));
    let d = s.authorize_in(
        &CapabilityRequest::TimeSleep(Sleep { ms: 600_000 }),
        Some(&key()),
    );
    assert_eq!(d.denial(), Some(DenyReason::TooLarge));
    let d = s.authorize_state(&CapabilityRequest::TimeSleep(Sleep { ms: 1_000 }));
    assert_eq!(d.denial(), Some(DenyReason::NotInStateCall));
}

#[test]
fn malformed_connection_requests_stay_bad_requests() {
    let s = session();
    let d = s.authorize_bound(
        &CapabilityRequest::ConnectionGet(ConnectionRef {
            connection: "github".into(),
        }),
        Some(&key()),
        &bound(&["github"]),
    );
    assert_eq!(d.denial(), Some(DenyReason::TooLarge));
}

/// The audit names the connection's origin only, never the path or query.
#[test]
fn the_audit_names_the_origin_only() {
    let (_, s, audit) = fixture(RateLimits::default());
    let d = s.authorize_bound(&get(PROJECTS, GITLAB), Some(&key()), &bound(&[GITLAB]));
    assert!(d.is_allowed());
    let records = audit.records();
    let last = records.last().unwrap();
    assert_eq!(
        last.target.as_deref(),
        Some("https://code.example.com:8443")
    );
    assert!(!format!("{records:?}").contains("projects"));
}

#[test]
fn removing_the_plugin_removes_its_connections() {
    let (broker, s, _) = fixture(RateLimits::default());
    broker.remove_plugin(s.plugin());
    let d = s.authorize_bound(&get(PROJECTS, GITLAB), Some(&key()), &bound(&[GITLAB]));
    assert_eq!(d.denial(), Some(DenyReason::UnknownPlugin));
}
