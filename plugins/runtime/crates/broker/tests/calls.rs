//! Plugin API 1.16's operations (`docs/calls.md`): `call.state` and
//! `call.control` exist for the plugin `deckpilot.calls` only, each needs
//! its own grant, `call.control` only in a run the user started, and a
//! Calls grant never comes with a way out of the agent.

use std::sync::Arc;

use deckpilot_plugin_abi::CapabilityRequest;
use deckpilot_plugin_abi::calls::{CallAction, CallControl};
use deckpilot_plugin_abi::capability::{NoArgs, RunCause, RunContext, RunTrigger};
use deckpilot_plugin_broker::*;

fn session_of(plugin: &str, perms: &str) -> (Session, Arc<MemoryAudit>) {
    let audit = Arc::new(MemoryAudit::new(10_000));
    let broker = Broker::new(
        audit.clone(),
        Arc::new(ManualClock::default()),
        RateLimits::default(),
    );
    let plugin = PluginId::new(plugin).unwrap();
    broker
        .set_grants(&plugin, &ApprovedPermissions::from_json(perms).unwrap())
        .unwrap();
    (broker.session(plugin), audit)
}

fn session(perms: &str) -> Session {
    session_of(CALLS_PLUGIN, perms).0
}

fn state() -> CapabilityRequest {
    CapabilityRequest::CallState(NoArgs {})
}

fn control(action: CallAction) -> CapabilityRequest {
    CapabilityRequest::CallControl(CallControl { action })
}

fn key() -> RunContext {
    RunContext::direct(RunTrigger::Key)
}

#[test]
fn nothing_without_the_grant() {
    let none = session(r#"[{"kind":"notifications"}]"#);
    for request in [state(), control(CallAction::End)] {
        assert_eq!(
            none.authorize_in(&request, Some(&key())).denial(),
            Some(DenyReason::NotGranted),
            "{}",
            request.op()
        );
    }
}

#[test]
fn observing_does_not_control() {
    let observe = session(r#"[{"kind":"calls"}]"#);
    assert_eq!(
        observe.authorize_in(&state(), Some(&key())),
        Decision::Allow(Allowed::CallState)
    );
    for action in [
        CallAction::Answer,
        CallAction::End,
        CallAction::MicToggle,
        CallAction::CameraSet { on: false },
    ] {
        assert_eq!(
            observe
                .authorize_in(&control(action), Some(&key()))
                .denial(),
            Some(DenyReason::NotGranted),
            "{action:?}"
        );
    }
    let control_grant = session(r#"[{"kind":"calls","control":true}]"#);
    assert_eq!(
        control_grant.authorize_in(&control(CallAction::End), Some(&key())),
        Decision::Allow(Allowed::CallControl {
            action: CallAction::End
        })
    );
}

/// T-P1: another plugin is refused even with the same grant record (the
/// manifest rules and the policy keep it from getting one at all).
#[test]
fn only_the_calls_plugin_may_use_them() {
    let (other, audit) = session_of(
        "deckpilot.workspace",
        r#"[{"kind":"calls","control":true}]"#,
    );
    for request in [state(), control(CallAction::MicToggle)] {
        assert_eq!(
            other.authorize_in(&request, Some(&key())).denial(),
            Some(DenyReason::NotGranted),
            "{}",
            request.op()
        );
    }
    let ops: Vec<&str> = audit.records().iter().map(|r| r.op).collect();
    assert_eq!(
        ops,
        ["call.state", "call.control"],
        "every refusal is audited"
    );
}

#[test]
fn control_only_in_a_run_the_user_started() {
    let control_grant = session(r#"[{"kind":"calls","control":true}]"#);
    for refused in [
        None,
        Some(RunContext::unknown()),
        Some(RunContext::workflow(RunCause::Schedule, None)),
        Some(RunContext::workflow(RunCause::Event, None)),
        Some(RunContext::workflow(RunCause::Startup, None)),
        Some(RunContext::workflow(RunCause::WorkSession, None)),
    ] {
        assert_eq!(
            control_grant
                .authorize_in(&control(CallAction::End), refused.as_ref())
                .denial(),
            Some(DenyReason::PrivilegedNotUserInitiated),
            "{refused:?}"
        );
    }
    for allowed in [
        RunContext::direct(RunTrigger::Key),
        RunContext::direct(RunTrigger::Gesture),
        RunContext::direct(RunTrigger::App),
        RunContext::workflow(RunCause::User, None),
    ] {
        assert!(
            control_grant
                .authorize_in(&control(CallAction::End), Some(&allowed))
                .is_allowed(),
            "{allowed:?}"
        );
    }
    // Reading the state needs no run at all (a key's face).
    assert!(control_grant.authorize_in(&state(), None).is_allowed());
}

#[test]
fn the_state_is_read_in_a_state_call_and_control_is_not() {
    let control_grant = session(r#"[{"kind":"calls","control":true}]"#);
    assert!(control_grant.authorize_state(&state()).is_allowed());
    assert_eq!(
        control_grant
            .authorize_state_in(&control(CallAction::End), Some(&key()))
            .denial(),
        Some(DenyReason::NotInStateCall)
    );
}

#[test]
fn control_is_rate_limited() {
    let control_grant = session(r#"[{"kind":"calls","control":true}]"#);
    let allowed = (0..10)
        .filter(|_| {
            control_grant
                .authorize_in(&control(CallAction::MicToggle), Some(&key()))
                .is_allowed()
        })
        .count();
    assert_eq!(allowed, 4, "4 per second");
    assert_eq!(
        control_grant
            .authorize_in(&control(CallAction::MicToggle), Some(&key()))
            .denial(),
        Some(DenyReason::RateLimited)
    );
}

/// What a Calls plugin learns of a call cannot leave the agent: a Calls
/// grant together with a way out is refused as a whole.
#[test]
fn a_calls_grant_never_comes_with_a_way_out() {
    for way_out in [
        r#"{"kind":"network","origin":"https://example.com","methods":["GET"]}"#,
        r#"{"kind":"openUrl","origin":"https://example.com"}"#,
        r#"{"kind":"credential","key":"token","origins":["https://example.com"],"schemes":["bearer"]}"#,
        r#"{"kind":"tool","tool":"git"}"#,
        r#"{"kind":"openUrlFromParams"}"#,
        r#"{"kind":"resources","open":true}"#,
        r#"{"kind":"connection","id":"cn_01JD7Q0000000000000000000B","origin":"https://api.github.com","webOrigin":"https://github.com","methods":["GET"],"enabled":true}"#,
        r#"{"kind":"scripts","fs":true}"#,
        r#"{"kind":"userOrigin","origin":"https://example.com","methods":["GET"]}"#,
    ] {
        let json = format!(r#"[{{"kind":"calls"}},{way_out}]"#);
        let approved = ApprovedPermissions::from_json(&json).unwrap();
        assert_eq!(
            Grants::derive(&approved),
            Err(GrantError::CallsWithWayOut),
            "{way_out}"
        );
    }
    // Its own storage, settings and notifications are no way out.
    let fine = ApprovedPermissions::from_json(
        r#"[{"kind":"calls","control":true},{"kind":"notifications"},{"kind":"keyState"},{"kind":"storage","quotaKb":512}]"#,
    )
    .unwrap();
    assert!(Grants::derive(&fine).is_ok());
    let twice =
        ApprovedPermissions::from_json(r#"[{"kind":"calls"},{"kind":"calls","control":true}]"#)
            .unwrap();
    assert_eq!(Grants::derive(&twice), Err(GrantError::Duplicate));
}
