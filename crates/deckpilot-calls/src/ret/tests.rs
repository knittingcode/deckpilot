//! The return controller on a virtual clock: the timer presets and custom
//! times (T-R1), one session per device with stale ids ignored and no
//! duplicate timers (T-R8), and the run outcomes of each setting.

use super::*;

type Controller = ReturnController<&'static str>;

fn at(secs: u64) -> Instant {
    Instant::from_secs(secs)
}

fn timer(secs: u64) -> ReturnSettings {
    ReturnSettings {
        mode: ReturnMode::timer(secs).unwrap(),
        ..ReturnSettings::default()
    }
}

fn completion() -> ReturnSettings {
    ReturnSettings {
        mode: ReturnMode::Completion,
        ..ReturnSettings::default()
    }
}

// ------------------------------------------------------------------ T-R1

#[test]
fn timer_presets_and_custom_times() {
    for secs in TIMER_PRESETS.into_iter().chain([5, 45, 600]) {
        let mut c = Controller::default();
        let (id, replaced) = c.suspend("deck", timer(secs), at(100));
        assert!(replaced.is_none());
        assert_eq!(c.next_wake(), Some(at(100 + secs)));
        assert!(c.tick(at(100 + secs - 1)).is_empty(), "{secs}: early");
        assert!(c.is_suspended(&"deck"));
        assert_eq!(
            c.tick(at(100 + secs)),
            [ReturnEnded {
                device: "deck",
                id,
                reason: ReturnReason::TimerElapsed
            }]
        );
        assert!(!c.is_suspended(&"deck"));
        assert_eq!(c.next_wake(), None, "{secs}: nothing left scheduled");
    }
}

#[test]
fn custom_times_are_bounded() {
    for bad in [0, 4, 601, 3600] {
        assert!(ReturnMode::timer(bad).is_err(), "{bad}");
    }
    for bad in [9, 1801] {
        assert!(ReturnMode::completion_or_timeout(bad).is_err(), "{bad}");
    }
    assert_eq!(
        ReturnMode::completion_or_timeout(10).unwrap(),
        ReturnMode::CompletionOrTimeout {
            max: Duration::from_secs(10)
        }
    );
}

#[test]
fn the_default_is_completion_or_sixty_seconds() {
    let settings = ReturnSettings::default();
    assert_eq!(
        settings.mode,
        ReturnMode::CompletionOrTimeout {
            max: Duration::from_secs(60)
        }
    );
    assert_eq!(settings.qualify, Qualify::default());
    assert!(!settings.qualify.navigation);
    assert_eq!(settings.on_failure, AfterRun::Resume);
    assert_eq!(settings.on_cancel, AfterRun::Stay);
    assert!(!settings.all_devices);
    let mut c = Controller::default();
    c.suspend("deck", settings, at(0));
    assert_eq!(
        c.tick(at(60))[0].reason,
        ReturnReason::MaxTimeout,
        "the maximum bounds a run that never ends"
    );
}

// -------------------------------------------------------- run outcomes

#[test]
fn a_qualifying_run_resumes_when_it_really_ends() {
    let mut c = Controller::default();
    let (id, _) = c.suspend("deck", ReturnSettings::default(), at(0));
    assert!(c.hold(&"deck", id, RunToken(1), RunKind::Workflow));
    // Pressing the key is not the end: nothing until the run reports it.
    assert_eq!(c.tick(at(10)).len(), 0);
    let ended = c
        .run_finished(&"deck", RunToken(1), RunEnd::Succeeded)
        .unwrap();
    assert_eq!(ended.reason, ReturnReason::RunSucceeded);
    assert!(ended.resumes());
    assert_eq!(c.next_wake(), None);
}

#[test]
fn failure_and_cancellation_follow_the_settings() {
    for (end, after, resumes) in [
        (
            RunEnd::Failed,
            AfterRun::Resume,
            Some(ReturnReason::RunFailed),
        ),
        (RunEnd::Failed, AfterRun::Stay, None),
        (
            RunEnd::Cancelled,
            AfterRun::Resume,
            Some(ReturnReason::RunCancelled),
        ),
        (RunEnd::Cancelled, AfterRun::Stay, None),
    ] {
        let mut c = Controller::default();
        let settings = ReturnSettings {
            on_failure: after,
            on_cancel: after,
            ..ReturnSettings::default()
        };
        let (id, _) = c.suspend("deck", settings, at(0));
        c.hold(&"deck", id, RunToken(1), RunKind::DetachedAction);
        let got = c.run_finished(&"deck", RunToken(1), end).map(|e| e.reason);
        assert_eq!(got, resumes, "{end:?} {after:?}");
        if resumes.is_none() {
            // Staying: still suspended, still bounded by the maximum.
            assert!(c.is_suspended(&"deck"));
            assert_eq!(c.tick(at(60))[0].reason, ReturnReason::MaxTimeout);
        }
    }
}

#[test]
fn an_interrupted_run_counts_for_nothing() {
    let mut c = Controller::default();
    let (id, _) = c.suspend("deck", completion(), at(0));
    c.hold(&"deck", id, RunToken(1), RunKind::Action);
    assert!(
        c.run_finished(&"deck", RunToken(1), RunEnd::Interrupted)
            .is_none()
    );
    assert!(c.is_suspended(&"deck"));
    // Mode B without a qualifying end: suspended until the call ends.
    assert_eq!(c.next_wake(), None);
    assert_eq!(c.call_ended()[0].reason, ReturnReason::CallEnded);
}

#[test]
fn navigation_and_empty_keys_do_not_count_unless_configured() {
    let mut c = Controller::default();
    let (id, _) = c.suspend("deck", ReturnSettings::default(), at(0));
    assert!(!c.hold(&"deck", id, RunToken(1), RunKind::Navigation));
    assert!(!c.hold(&"deck", id, RunToken(2), RunKind::Nothing));
    assert!(
        c.run_finished(&"deck", RunToken(1), RunEnd::Succeeded)
            .is_none()
    );
    let settings = ReturnSettings {
        qualify: Qualify {
            navigation: true,
            ..Qualify::default()
        },
        ..ReturnSettings::default()
    };
    let (id, _) = c.suspend("deck", settings, at(1));
    assert!(c.hold(&"deck", id, RunToken(3), RunKind::Navigation));
    assert!(!c.hold(&"deck", id, RunToken(4), RunKind::Nothing));
    assert_eq!(
        c.run_finished(&"deck", RunToken(3), RunEnd::Succeeded)
            .unwrap()
            .reason,
        ReturnReason::RunSucceeded
    );
}

#[test]
fn a_timer_ignores_runs() {
    let mut c = Controller::default();
    let (id, _) = c.suspend("deck", timer(20), at(0));
    assert!(!c.hold(&"deck", id, RunToken(1), RunKind::Action));
    assert!(
        c.run_finished(&"deck", RunToken(1), RunEnd::Succeeded)
            .is_none()
    );
    assert_eq!(c.tick(at(20))[0].reason, ReturnReason::TimerElapsed);
}

#[test]
fn the_first_qualifying_end_wins() {
    let mut c = Controller::default();
    let (id, _) = c.suspend("deck", ReturnSettings::default(), at(0));
    c.hold(&"deck", id, RunToken(1), RunKind::Action);
    c.hold(&"deck", id, RunToken(2), RunKind::Workflow);
    assert!(
        c.run_finished(&"deck", RunToken(2), RunEnd::Succeeded)
            .is_some()
    );
    // The other run's end finds no session.
    assert!(
        c.run_finished(&"deck", RunToken(1), RunEnd::Succeeded)
            .is_none()
    );
}

// ------------------------------------------------------------------ T-R8

#[test]
fn one_session_per_device_and_stale_ids_are_ignored() {
    let mut c = Controller::default();
    let (first, _) = c.suspend("deck", ReturnSettings::default(), at(0));
    assert!(c.hold(&"deck", first, RunToken(1), RunKind::Action));
    // A second suspension replaces the first: a new id, one deadline.
    let (second, replaced) = c.suspend("deck", timer(30), at(5));
    assert_ne!(first, second);
    assert_eq!(replaced.unwrap().reason, ReturnReason::Dropped);
    assert_eq!(c.scheduled(), 1);
    assert_eq!(c.next_wake(), Some(at(35)));
    // The first session's run and hold are stale.
    assert!(!c.hold(&"deck", first, RunToken(2), RunKind::Action));
    assert!(
        c.run_finished(&"deck", RunToken(1), RunEnd::Succeeded)
            .is_none()
    );
    // The first session's deadline (60 s) never fires.
    assert_eq!(c.tick(at(35))[0].id, second);
    assert_eq!(c.tick(at(60)).len(), 0);
    assert_eq!(c.scheduled(), 0);
}

#[test]
fn repeated_presses_never_stack_timers() {
    let mut c = Controller::default();
    let mut ids = Vec::new();
    for press in 0..10 {
        let (id, _) = c.suspend("deck", timer(20), at(press));
        ids.push(id);
        assert_eq!(c.scheduled(), 1);
    }
    ids.dedup();
    assert_eq!(ids.len(), 10, "every suspension has its own id");
    let ended = c.tick(at(100));
    assert_eq!(ended.len(), 1);
    assert_eq!(ended[0].id, *ids.last().unwrap());
}

#[test]
fn devices_have_their_own_sessions() {
    let mut c = Controller::default();
    let (a, _) = c.suspend("mk2", timer(20), at(0));
    let (b, _) = c.suspend("plus", timer(60), at(0));
    assert_eq!(c.scheduled(), 2);
    let ended = c.tick(at(20));
    assert_eq!(
        ended,
        [ReturnEnded {
            device: "mk2",
            id: a,
            reason: ReturnReason::TimerElapsed
        }]
    );
    assert_eq!(c.current(&"plus"), Some(b));
    assert!(c.user_resumed(&"plus").unwrap().resumes());
    assert!(c.user_resumed(&"plus").is_none());
}

#[test]
fn the_call_ending_ends_every_session_without_resuming() {
    let mut c = Controller::default();
    c.suspend("mk2", ReturnSettings::default(), at(0));
    c.suspend("xl", completion(), at(0));
    let ended = c.call_ended();
    assert_eq!(ended.len(), 2);
    assert!(ended.iter().all(|e| !e.resumes()));
    assert_eq!(c.next_wake(), None);
    assert!(c.tick(at(1000)).is_empty(), "no stale restore");
}

#[test]
fn an_incoming_call_preempts_and_resumes() {
    let mut c = Controller::default();
    c.suspend("mk2", ReturnSettings::default(), at(0));
    let ended = c.preempt();
    assert_eq!(ended[0].reason, ReturnReason::Preempted);
    assert!(ended[0].resumes());
    assert_eq!(c.preempt().len(), 0);
}

#[test]
fn a_restart_has_no_session() {
    let c = Controller::default();
    assert_eq!(c.next_wake(), None);
    assert!(!c.is_suspended(&"deck"));
}

/// A mode built from its public fields, outside the ranges, is clamped
/// where it is used: never an instant or an unbounded wait.
#[test]
fn a_mode_built_by_hand_is_clamped() {
    for (mode, wake) in [
        (
            ReturnMode::Timer {
                after: Duration::ZERO,
            },
            100 + TIMER_RANGE.0,
        ),
        (
            ReturnMode::Timer {
                after: Duration::from_secs(u64::MAX / 4),
            },
            100 + TIMER_RANGE.1,
        ),
        (
            ReturnMode::CompletionOrTimeout {
                max: Duration::from_secs(1),
            },
            100 + MAX_TIMEOUT_RANGE.0,
        ),
        (
            ReturnMode::CompletionOrTimeout {
                max: Duration::from_secs(1_000_000),
            },
            100 + MAX_TIMEOUT_RANGE.1,
        ),
    ] {
        let mut c = Controller::default();
        let settings = ReturnSettings {
            mode,
            ..ReturnSettings::default()
        };
        c.suspend("deck", settings, at(100));
        assert_eq!(c.next_wake(), Some(at(wake)), "{mode:?}");
    }
}
