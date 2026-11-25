//! Plugin API 1.14, connections: the broker knows the operations but no
//! agent carries them out yet, so each is refused as unsupported whatever
//! the grants, and a fetch that names a connection is refused as a whole,
//! never checked as an ordinary fetch to a granted origin.

use std::sync::Arc;

use deckpilot_plugin_abi::capability::{HttpFetch, RunContext, RunTrigger};
use deckpilot_plugin_abi::connections::{
    ConnectionRef, ConnectionStatus, ConnectionStatusReport, Sleep,
};
use deckpilot_plugin_abi::{CallErrorCode, CapabilityRequest, HttpMethod};
use deckpilot_plugin_broker::*;

const ID: &str = "cn_01JD7Q0000000000000000000A";

fn session() -> Session {
    let broker = Broker::new(
        Arc::new(MemoryAudit::new(1_000)),
        Arc::new(ManualClock::default()),
        RateLimits::default(),
    );
    let plugin = PluginId::new("deckpilot.git-providers").unwrap();
    broker
        .set_grants(
            &plugin,
            &ApprovedPermissions::from_json(
                r#"[{"kind":"network","origin":"https://gitlab.example.com","methods":["GET","POST"]},
                    {"kind":"dialogs"}]"#,
            )
            .unwrap(),
        )
        .unwrap();
    broker.session(plugin)
}

fn key() -> RunContext {
    RunContext::direct(RunTrigger::Key)
}

#[test]
fn connection_operations_are_unsupported_whatever_the_grants() {
    let s = session();
    for request in [
        CapabilityRequest::ConnectionGet(ConnectionRef {
            connection: ID.into(),
        }),
        CapabilityRequest::ConnectionReportStatus(ConnectionStatusReport {
            connection: ID.into(),
            status: ConnectionStatus::Ok,
            detail: None,
        }),
        CapabilityRequest::TimeSleep(Sleep { ms: 1_000 }),
        CapabilityRequest::HttpFetch(HttpFetch {
            method: HttpMethod::Get,
            // A granted origin: the connection still decides, and there is
            // none yet.
            url: "https://gitlab.example.com/api/v4/projects".into(),
            headers: vec![],
            body: None,
            auth: None,
            connection: Some(ID.into()),
        }),
    ] {
        let d = s.authorize_in(&request, Some(&key()));
        assert_eq!(
            d.denial(),
            Some(DenyReason::NotImplemented),
            "{}",
            request.op()
        );
        assert_eq!(
            DenyReason::NotImplemented.call_error_code(),
            CallErrorCode::UnsupportedOp
        );
    }
}

#[test]
fn malformed_connection_requests_stay_bad_requests() {
    let s = session();
    let d = s.authorize_in(
        &CapabilityRequest::ConnectionGet(ConnectionRef {
            connection: "github".into(),
        }),
        Some(&key()),
    );
    assert_eq!(d.denial(), Some(DenyReason::TooLarge));
    let d = s.authorize_in(
        &CapabilityRequest::TimeSleep(Sleep { ms: 600_000 }),
        Some(&key()),
    );
    assert_eq!(d.denial(), Some(DenyReason::TooLarge));
}

#[test]
fn connections_are_not_a_grant_record_yet() {
    assert!(ApprovedPermissions::from_json(r#"[{"kind":"connections"}]"#).is_err());
}

#[test]
fn sleeping_is_not_available_in_a_state_call() {
    let s = session();
    let d = s.authorize_state(&CapabilityRequest::TimeSleep(Sleep { ms: 1_000 }));
    assert_eq!(d.denial(), Some(DenyReason::NotInStateCall));
}
