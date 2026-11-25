//! The Scripts host bridge in the wasm layer (plugin API 1.13,
//! `docs/scripts-bridge.md`): the per-invocation call and poll budgets the
//! supervisor gives `deckpilot.scripts` (`runtime.hostCalls`), and the
//! op-aware continuous-CPU deadline: `script.poll` and `script.log` do not
//! reset it, calls that wait on the agent do.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use deckpilot_plugin_abi::protocol::GuestLimits;
use deckpilot_plugin_abi::scripts::{ScriptFault, codes};
use deckpilot_plugin_abi::{
    ActionOutcome, CallErrorCode, CallResult, CapabilityRequest, ParamValue, Params,
};
use deckpilot_plugin_host::engine::{
    ChannelError, EffectiveLimits, Fault, HostCallHandler, LoadedPlugin, PluginEngine,
};
use deckpilot_plugin_test_fixtures::TEST_GUEST;
use serde_json::json;

const POLL: &str = r#"{"op":"script.poll","args":{}}"#;
const LOG: &str = r#"{"op":"script.log","args":{"level":"info","message":"tick"}}"#;
const TIME: &str = r#"{"op":"time.now","args":{}}"#;
const RUN: &str = r#"{"op":"process.run","args":{"command":"git","args":["status"],"env":{}}}"#;
const REMOVE: &str = r#"{"op":"fs.remove","args":{"path":{"repository":"cms","relative":"x"},"recursive":true,"dryRun":false}}"#;

/// Answers every call; `process.run` after `delay`, like a child process.
struct Agent {
    delay: Duration,
    seen: Rc<RefCell<Vec<&'static str>>>,
}

impl HostCallHandler for Agent {
    fn call(
        &mut self,
        _inv: u64,
        _call: u64,
        request: CapabilityRequest,
    ) -> Result<CallResult, ChannelError> {
        self.seen.borrow_mut().push(request.op());
        Ok(match request {
            CapabilityRequest::ProcessRun(_) => {
                std::thread::sleep(self.delay);
                CallResult::Ok(json!({"ok": true, "exitCode": 0}))
            }
            CapabilityRequest::ScriptPoll(_) => {
                CallResult::Ok(json!({"cancelled": false, "remainingMs": 1000, "nowUs": 1}))
            }
            CapabilityRequest::FsRemove(_) => CallResult::script_error(
                CallErrorCode::NotPermitted,
                ScriptFault {
                    confirmation: Some(deckpilot_plugin_abi::scripts::Confirmation {
                        token: "0123456789abcdef0123456789abcdef".into(),
                    }),
                    ..ScriptFault::new(codes::NEEDS_CONFIRMATION, "confirm")
                },
            ),
            _ => CallResult::Ok(serde_json::Value::Null),
        })
    }
}

/// The Scripts plugin's limits: a 200 ms continuous deadline, fuel to
/// spare, and the given budgets.
fn scripts_limits(host_calls: Option<u32>, polls: Option<u32>) -> GuestLimits {
    GuestLimits {
        deadline_ms: 200,
        fuel_per_invocation: 50_000_000_000,
        host_calls,
        polls,
        ..GuestLimits::default()
    }
}

fn load(
    limits: GuestLimits,
    delay: Duration,
) -> (LoadedPlugin, Rc<RefCell<Vec<&'static str>>>, PluginEngine) {
    let engine = PluginEngine::new().unwrap();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let plugin = engine
        .load(
            TEST_GUEST,
            &limits,
            Box::new(Agent {
                delay,
                seen: seen.clone(),
            }),
        )
        .unwrap();
    (plugin, seen, engine)
}

fn raw_loop(json: &str, n: i64, spin: i64) -> Params {
    Params::from([
        ("json".into(), ParamValue::String(json.into())),
        ("n".into(), ParamValue::Integer(n)),
        ("spin".into(), ParamValue::Integer(spin)),
    ])
}

fn counts(outcome: &ActionOutcome) -> (i64, i64) {
    let ActionOutcome::Ok { data, .. } = outcome else {
        panic!("expected ok, got {outcome:?}")
    };
    let int = |k: &str| match data.get(k) {
        Some(ParamValue::Integer(i)) => *i,
        other => panic!("{k}: {other:?}"),
    };
    (int("ok"), int("err"))
}

/// A busy loop that polls between bursts of work still meets the deadline:
/// polls never give it back the time it used.
#[test]
fn a_poll_loop_traps_at_the_continuous_deadline() {
    let (mut p, seen, _e) = load(scripts_limits(Some(4096), Some(8192)), Duration::ZERO);
    let started = Instant::now();
    let r = p.invoke(1, "raw_loop", &raw_loop(POLL, 0, 300_000));
    let took = started.elapsed();
    assert!(matches!(r, Err(Fault::Deadline)), "{r:?}");
    assert!(
        took >= Duration::from_millis(150) && took < Duration::from_secs(3),
        "{took:?}"
    );
    assert!(seen.borrow().iter().all(|op| *op == "script.poll"));
    assert!(!seen.borrow().is_empty(), "the polls reached the agent");
}

#[test]
fn a_log_loop_traps_at_the_continuous_deadline() {
    let (mut p, seen, _e) = load(scripts_limits(Some(4096), Some(8192)), Duration::ZERO);
    let r = p.invoke(1, "raw_loop", &raw_loop(LOG, 0, 300_000));
    assert!(matches!(r, Err(Fault::Deadline)), "{r:?}");
    assert!(seen.borrow().iter().all(|op| *op == "script.log"));
}

/// A malformed poll is answered by the host itself and gives no time back
/// either, so it is no way around the deadline.
#[test]
fn a_malformed_poll_does_not_reset_the_deadline() {
    let (mut p, seen, _e) = load(scripts_limits(Some(4096), Some(8192)), Duration::ZERO);
    let bad = r#"{"op":"script.poll","args":{"reset":true}}"#;
    let r = p.invoke(1, "raw_loop", &raw_loop(bad, 0, 300_000));
    assert!(matches!(r, Err(Fault::Deadline)), "{r:?}");
    assert!(seen.borrow().is_empty(), "never relayed");
}

/// Calls that wait on the agent start a new segment: a loop of
/// `process.run` that waits far longer than the deadline in total finishes.
#[test]
fn a_loop_of_process_runs_finishes() {
    let (mut p, seen, _e) = load(
        scripts_limits(Some(4096), Some(8192)),
        Duration::from_millis(40),
    );
    let started = Instant::now();
    let r = p.invoke(1, "raw_loop", &raw_loop(RUN, 10, 10_000)).unwrap();
    assert_eq!(counts(&r), (10, 0));
    assert!(started.elapsed() >= Duration::from_millis(400));
    assert_eq!(seen.borrow().len(), 10);
}

/// A bridge refusal reaches the guest with its script detail (the host's
/// `for_op` keeps it on the bridge's own operations).
#[test]
fn a_bridge_refusal_reaches_the_guest_with_its_detail() {
    let (mut p, _seen, _e) = load(scripts_limits(Some(4096), Some(8192)), Duration::ZERO);
    let r = p
        .invoke(
            1,
            "raw_request",
            &Params::from([("json".into(), ParamValue::String(REMOVE.into()))]),
        )
        .unwrap();
    let ActionOutcome::Ok { data, .. } = r else {
        panic!("{r:?}")
    };
    let Some(ParamValue::String(reply)) = data.get("reply") else {
        panic!("{data:?}")
    };
    let reply: serde_json::Value = serde_json::from_str(reply).unwrap();
    assert_eq!(reply["error"]["code"], "not-permitted");
    assert_eq!(reply["error"]["script"]["code"], "needsConfirmation");
    assert_eq!(
        reply["error"]["script"]["confirmation"]["token"],
        "0123456789abcdef0123456789abcdef"
    );
}

/// Without the Scripts budget, a guest has the default 256 calls; with it,
/// 4,096, and the 4,097th traps.
#[test]
fn the_call_budget_comes_from_the_limits() {
    let (mut p, _, _e) = load(scripts_limits(Some(4096), Some(8192)), Duration::ZERO);
    let r = p.invoke(1, "raw_loop", &raw_loop(TIME, 4096, 0)).unwrap();
    assert_eq!(counts(&r), (4096, 0));

    let (mut p, _, _e) = load(scripts_limits(Some(4096), Some(8192)), Duration::ZERO);
    let r = p.invoke(1, "raw_loop", &raw_loop(TIME, 4097, 0));
    assert!(
        matches!(&r, Err(Fault::GuestAbi(m)) if m.contains("too many host calls")),
        "{r:?}"
    );

    let (mut p, _, _e) = load(scripts_limits(None, None), Duration::ZERO);
    assert_eq!(
        counts(&p.invoke(1, "raw_loop", &raw_loop(TIME, 256, 0)).unwrap()),
        (256, 0)
    );
    let (mut p, _, _e) = load(scripts_limits(None, None), Duration::ZERO);
    assert!(matches!(
        p.invoke(1, "raw_loop", &raw_loop(TIME, 257, 0)),
        Err(Fault::GuestAbi(_))
    ));
}

/// The budget is per invocation: the next one starts again from zero.
#[test]
fn the_budgets_are_per_invocation() {
    let (mut p, _, _e) = load(scripts_limits(Some(100), Some(100)), Duration::ZERO);
    for i in 1..=3 {
        let r = p.invoke(i, "raw_loop", &raw_loop(TIME, 100, 0)).unwrap();
        assert_eq!(counts(&r), (100, 0));
        let r = p.invoke(i, "raw_loop", &raw_loop(POLL, 100, 0)).unwrap();
        assert_eq!(counts(&r), (100, 0));
    }
}

/// Polls have their own budget (D2) when the limits give one, and count
/// as ordinary calls otherwise.
#[test]
fn polls_are_counted_apart() {
    let (mut p, _, _e) = load(scripts_limits(Some(10), Some(50)), Duration::ZERO);
    let r = p.invoke(1, "raw_loop", &raw_loop(POLL, 50, 0)).unwrap();
    assert_eq!(counts(&r), (50, 0));

    let (mut p, _, _e) = load(scripts_limits(Some(10), Some(50)), Duration::ZERO);
    let r = p.invoke(1, "raw_loop", &raw_loop(POLL, 51, 0));
    assert!(
        matches!(&r, Err(Fault::GuestAbi(m)) if m.contains("script.poll")),
        "{r:?}"
    );

    let (mut p, _, _e) = load(scripts_limits(Some(10), None), Duration::ZERO);
    assert!(matches!(
        p.invoke(1, "raw_loop", &raw_loop(POLL, 11, 0)),
        Err(Fault::GuestAbi(_))
    ));
}

#[test]
fn the_budgets_are_clamped_to_the_hard_maxima() {
    let l = EffectiveLimits::clamp(&GuestLimits {
        host_calls: Some(1_000_000),
        polls: Some(1_000_000),
        deadline_ms: 1_000_000,
        ..GuestLimits::default()
    });
    assert_eq!(l.host_calls, 4096);
    assert_eq!(l.polls, Some(8192));
    assert_eq!(l.deadline_ticks, 1000, "10 s at most");
    let l = EffectiveLimits::clamp(&GuestLimits {
        host_calls: Some(0),
        ..GuestLimits::default()
    });
    assert_eq!(l.host_calls, 1);
    let l = EffectiveLimits::clamp(&GuestLimits::default());
    assert_eq!((l.host_calls, l.polls), (256, None));
}

/// Every other guest is unchanged: an ordinary call still resets the
/// deadline, and its errors never carry the script detail.
#[test]
fn ordinary_calls_still_reset_the_deadline() {
    let (mut p, _, _e) = load(scripts_limits(Some(4096), Some(8192)), Duration::ZERO);
    // 2,000 calls with 100k iterations between them take longer than the
    // 200 ms deadline in total, and finish.
    let r = p
        .invoke(1, "raw_loop", &raw_loop(TIME, 2000, 100_000))
        .unwrap();
    assert_eq!(counts(&r), (2000, 0));
}
