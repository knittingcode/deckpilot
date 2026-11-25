//! `call.state` and `call.control` through the broker and the executor:
//! the grant, the plugin id, the run's origin, the agent's refusals and
//! the audit (T-P1).

use std::sync::{Arc, Mutex};

use deckpilot_plugin_abi::CapabilityRequest;
use deckpilot_plugin_abi::calls::{CallAction, CallControlReply, CallStateReply};
use deckpilot_plugin_abi::capability::{RunCause, RunContext, RunTrigger};
use deckpilot_plugin_broker::{Broker, Decision};
use deckpilot_plugin_supervisor::CapabilityExecutor;
use serde_json::{Value, json};
use tokio::sync::broadcast;

use super::super::{
    Executor, Opener, OwnVariables, PrivilegedAudit, PrivilegedEntry, PublishedVariables, Transport,
};
use super::*;
use crate::service::plugin_calls::CallsReply;

/// The agent's side, recorded: every call it received. It answers a state
/// with an active call, and a control as detection-only Teams does.
#[derive(Debug, Default)]
struct FakeAgent {
    calls: Mutex<Vec<(String, CallsCall)>>,
}

impl CallsHost for FakeAgent {
    fn call(&self, plugin: &str, call: CallsCall) -> CallsReply {
        self.calls.lock().unwrap().push((plugin.to_owned(), call));
        match call {
            CallsCall::State => Ok(CallsAnswer::State(Box::new(CallStateReply {
                state: "active".into(),
                provider: Some("os-signals".into()),
                connected: true,
                mic_muted: None,
                camera_on: None,
                pending: false,
                unconfirmed: false,
                controls: std::collections::BTreeMap::default(),
                title: None,
                generation: 7,
            }))),
            CallsCall::Control(CallAction::MicSet { muted: true }) => {
                Ok(CallsAnswer::Control(CallControlReply::AlreadyThere))
            }
            CallsCall::Control(_) => {
                Err(CallsRefusal::new(codes::UNSUPPORTED, "provider-has-no-api"))
            }
        }
    }
}

struct Fixture {
    broker: Broker,
    plugin: PluginId,
    agent: Arc<FakeAgent>,
    audit: Arc<Mutex<Vec<PrivilegedEntry>>>,
}

impl Fixture {
    fn of(plugin: &str, grants: &str) -> Self {
        let broker = Broker::new(
            Arc::new(deckpilot_plugin_broker::MemoryAudit::new(100)),
            Arc::new(deckpilot_plugin_broker::ManualClock::default()),
            deckpilot_plugin_broker::RateLimits::default(),
        );
        let plugin = PluginId::new(plugin).unwrap();
        broker
            .set_grants(
                &plugin,
                &deckpilot_plugin_broker::ApprovedPermissions::from_json(grants).unwrap(),
            )
            .unwrap();
        Self {
            broker,
            plugin,
            agent: Arc::new(FakeAgent::default()),
            audit: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn new(grants: &str) -> Self {
        Self::of(CALLS_PLUGIN_ID, grants)
    }

    fn executor(&self, run: RunContext, host: bool) -> Executor {
        let (events, _) = broadcast::channel(4);
        let sink = self.audit.clone();
        let audit: PrivilegedAudit = Arc::new(move |entry: &PrivilegedEntry| {
            sink.lock().unwrap().push(entry.clone());
        });
        Executor::new(
            self.broker.clone(),
            "Calls".into(),
            events,
            Transport::Network,
            OwnVariables {
                store: PublishedVariables::default(),
                namespace: "plugins.deckpilot.calls".into(),
                generation: 0,
                declared: Vec::new(),
            },
            Opener::System,
        )
        .with_audit(audit)
        .with_calls(host.then(|| self.agent.clone() as Arc<dyn CallsHost>))
        .with_run(run)
    }

    /// What the supervisor does with one call.
    fn call_with(&self, run: RunContext, host: bool, request: &Value) -> CallResult {
        let mut executor = self.executor(run, host);
        let request: CapabilityRequest = serde_json::from_value(request.clone()).unwrap();
        let session = self.broker.session(self.plugin.clone());
        match session.authorize_in(&request, executor.run_context()) {
            Decision::Allow(allowed) => executor.execute(&self.plugin, &allowed),
            Decision::Deny(reason) => {
                CallResult::error(reason.call_error_code(), reason.to_string())
            }
        }
    }

    fn call(&self, run: RunContext, request: &Value) -> CallResult {
        self.call_with(run, true, request)
    }

    fn reached(&self) -> usize {
        self.agent.calls.lock().unwrap().len()
    }
}

fn key() -> RunContext {
    RunContext::direct(RunTrigger::Key)
}

fn state() -> Value {
    json!({"op": "call.state", "args": {}})
}

fn control(kind: &str) -> Value {
    json!({"op": "call.control", "args": {"action": {"kind": kind}}})
}

fn refused(result: &CallResult) -> (CallErrorCode, String) {
    match result {
        CallResult::Error(e) => (e.code, e.message.clone()),
        CallResult::Ok(value) => panic!("allowed: {value}"),
    }
}

#[test]
fn without_the_grant_nothing_reaches_the_agent() {
    let f = Fixture::new(r#"[{"kind":"notifications"}]"#);
    for request in [state(), control("end")] {
        let (code, _) = refused(&f.call(key(), &request));
        assert_eq!(code, CallErrorCode::NotPermitted);
    }
    assert_eq!(f.reached(), 0);
}

#[test]
fn observing_reads_the_state_and_controls_nothing() {
    let f = Fixture::new(r#"[{"kind":"calls"}]"#);
    let CallResult::Ok(value) = f.call(key(), &state()) else {
        panic!("the state")
    };
    let reply: CallStateReply = serde_json::from_value(value).unwrap();
    assert_eq!((reply.state.as_str(), reply.generation), ("active", 7));
    for kind in ["answer", "end", "micToggle", "cameraToggle"] {
        let (code, _) = refused(&f.call(key(), &control(kind)));
        assert_eq!(code, CallErrorCode::NotPermitted, "{kind}");
    }
    assert_eq!(f.reached(), 1, "only the state reached the agent");
}

/// T-P1: another plugin with the same grant record gets nothing.
#[test]
fn another_plugin_is_refused_with_the_same_grant() {
    let f = Fixture::of(
        "deckpilot.workspace",
        r#"[{"kind":"calls","control":true}]"#,
    );
    for request in [state(), control("end")] {
        let (code, _) = refused(&f.call(key(), &request));
        assert_eq!(code, CallErrorCode::NotPermitted);
    }
    assert_eq!(f.reached(), 0);
}

#[test]
fn a_control_needs_a_run_the_user_started() {
    let f = Fixture::new(r#"[{"kind":"calls","control":true}]"#);
    for run in [
        RunContext::workflow(RunCause::Schedule, None),
        RunContext::workflow(RunCause::Event, None),
        RunContext::workflow(RunCause::Startup, None),
        RunContext::unknown(),
    ] {
        let (code, _) = refused(&f.call(run.clone(), &control("end")));
        assert_eq!(code, CallErrorCode::NotPermitted, "{run:?}");
    }
    assert_eq!(f.reached(), 0);
}

/// The agent's refusal reaches the plugin with its code first and its
/// reason (detection-only Teams: `provider-has-no-api`), and a set of the
/// state the call has answers `alreadyThere`.
#[test]
fn the_agents_answer_reaches_the_plugin_and_is_audited() {
    let f = Fixture::new(r#"[{"kind":"calls","control":true}]"#);
    let (code, message) = refused(&f.call(key(), &control("end")));
    assert_eq!(code, CallErrorCode::Failed);
    assert_eq!(message, "calls-unsupported: provider-has-no-api");
    let done = f.call(
        key(),
        &json!({"op": "call.control", "args": {"action": {"kind": "micSet", "muted": true}}}),
    );
    assert!(
        matches!(done, CallResult::Ok(ref v) if v == "alreadyThere"),
        "{done:?}"
    );
    assert!(matches!(f.call(key(), &state()), CallResult::Ok(_)));
    let calls: Vec<(String, CallsCall)> = f.agent.calls.lock().unwrap().clone();
    assert_eq!(
        calls,
        vec![
            (
                CALLS_PLUGIN_ID.to_owned(),
                CallsCall::Control(CallAction::End)
            ),
            (
                CALLS_PLUGIN_ID.to_owned(),
                CallsCall::Control(CallAction::MicSet { muted: true })
            ),
            (CALLS_PLUGIN_ID.to_owned(), CallsCall::State),
        ]
    );
    let audit = f.audit.lock().unwrap();
    let records: Vec<_> = audit
        .iter()
        .map(|e| (e.record.op, e.record.grant, e.record.outcome))
        .collect();
    assert_eq!(
        records,
        [
            ("call.control", "calls:control", Some(codes::UNSUPPORTED)),
            ("call.control", "calls:control", None),
            ("call.state", "calls", None),
        ]
    );
    // The action's kind only; never the state or anything of the call.
    assert_eq!(audit[0].record.details, vec![("action", "end".to_owned())]);
    assert_eq!(audit[2].record.details, []);
}

#[test]
fn an_agent_without_calls_says_so() {
    let f = Fixture::new(r#"[{"kind":"calls","control":true}]"#);
    let (code, message) = refused(&f.call_with(key(), false, &state()));
    assert_eq!(code, CallErrorCode::Unavailable);
    assert!(message.starts_with("calls-provider-offline: "), "{message}");
}

#[test]
fn one_run_controls_at_most_twice() {
    let f = Fixture::new(r#"[{"kind":"calls","control":true}]"#);
    let mut executor = f.executor(key(), true);
    let request: CapabilityRequest = serde_json::from_value(control("end")).unwrap();
    let session = f.broker.session(f.plugin.clone());
    let mut answers = Vec::new();
    for _ in 0..3 {
        let Decision::Allow(allowed) = session.authorize_in(&request, executor.run_context())
        else {
            panic!("allowed by the broker")
        };
        answers.push(refused(&executor.execute(&f.plugin, &allowed)).1);
    }
    assert_eq!(answers[0], "calls-unsupported: provider-has-no-api");
    assert_eq!(answers[1], "calls-unsupported: provider-has-no-api");
    assert!(
        answers[2].starts_with("calls-consent: at most 2"),
        "{}",
        answers[2]
    );
    assert_eq!(f.reached(), 2, "the third never reached the agent");
}
