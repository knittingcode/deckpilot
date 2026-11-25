//! The engine on a virtual clock: every transition of the table and none
//! other (T-S1), the end causes (T-S2), provider disconnects (T-S4),
//! focus with simultaneous calls (T-S6), no optimistic success (T-S7) and
//! sets on toggle-only providers (T-S8).

use super::*;
use crate::provider::{Capability, Support};

pub(crate) const MOCK: &str = ProviderId::MOCK;

pub(crate) fn pid(id: &str) -> ProviderId {
    ProviderId::new(id).unwrap()
}

pub(crate) fn key(text: &str) -> CallKey {
    CallKey::new(text).unwrap()
}

pub(crate) fn all_supported() -> CapabilitySet {
    CapabilitySet::from_reported(Capability::ALL.into_iter().map(|c| (c, Support::Supported))).0
}

pub(crate) fn consent() -> ConsentState {
    ConsentState {
        observe: true,
        control: true,
        titles: false,
    }
}

/// An engine with one connected provider and a virtual clock.
pub(crate) struct Harness {
    pub engine: Engine,
    pub now: Instant,
    pub provider: ProviderId,
    pub session: u64,
}

impl Harness {
    pub fn new() -> Self {
        Self::with(MOCK, all_supported())
    }

    pub fn with(provider: &str, capabilities: CapabilitySet) -> Self {
        let mut engine = Engine::default();
        let provider = pid(provider);
        engine.add_provider(provider.clone(), consent(), capabilities);
        let mut h = Self {
            engine,
            now: Instant::ZERO,
            provider,
            session: 0,
        };
        h.connect();
        h
    }

    pub fn connect(&mut self) {
        self.session += 1;
        let event = ProviderEvent::Connected {
            session: ProviderSession(self.session),
        };
        self.event(event);
    }

    pub fn event(&mut self, event: ProviderEvent) {
        let provider = self.provider.clone();
        self.engine.on_event(&provider, event, self.now);
    }

    pub fn see(&mut self, call: &str, phase: ObservedPhase, direction: Option<Direction>) {
        self.see_full(
            call,
            phase,
            direction,
            Tri::Unknown,
            Tri::Unknown,
            Evidence::AppApi,
        );
    }

    pub fn see_full(
        &mut self,
        call: &str,
        phase: ObservedPhase,
        direction: Option<Direction>,
        mic: Tri,
        camera: Tri,
        source: Evidence,
    ) {
        let observation = CallObservation {
            key: key(call),
            kind: CallKind::OneToOne,
            phase,
            direction,
            mic_muted: mic,
            camera_on: camera,
            title: None,
            source,
            observed_at: self.now,
        };
        self.event(ProviderEvent::Call(observation));
    }

    pub fn gone(&mut self, call: &str) {
        self.event(ProviderEvent::CallGone { key: key(call) });
    }

    /// Moves the clock to `secs` (seconds from the start), running every
    /// timeout due on the way at its own instant.
    pub fn at(&mut self, secs: u64) {
        self.at_millis(secs * 1000);
    }

    pub fn at_millis(&mut self, millis: u64) {
        let target = Instant::from_millis(millis);
        while let Some(wake) = self.engine.next_wake() {
            if wake > target {
                break;
            }
            self.now = wake.max(self.now);
            self.engine.tick(self.now);
        }
        self.now = target.max(self.now);
        self.engine.tick(self.now);
    }

    pub fn state(&self) -> CallState {
        self.engine.snapshot().state
    }

    pub fn ended(&self) -> Option<EndCause> {
        match self.state() {
            CallState::Ended { cause } => Some(cause),
            _ => None,
        }
    }

    pub fn done(&mut self) -> Vec<(CommandToken, CommandEnd)> {
        self.engine
            .drain_events()
            .into_iter()
            .filter_map(|e| match e {
                EngineEvent::CommandDone { token, end } => Some((token, end)),
                _ => None,
            })
            .collect()
    }

    pub fn send(&mut self, action: CallAction) -> CommandToken {
        match self.engine.request(action, self.now) {
            Ok(Request::Send { token, .. }) => token,
            other => panic!("{action:?}: {other:?}"),
        }
    }
}

use Direction::{Incoming as In, Outgoing as Out};
use ObservedPhase as P;

// ------------------------------------------------------------------ T-S1

#[test]
fn idle_into_each_way_a_call_starts() {
    let mut h = Harness::new();
    assert_eq!(h.state(), CallState::Idle);
    h.see("a", P::Ringing, Some(In));
    assert_eq!(h.state(), CallState::Incoming);

    let mut h = Harness::new();
    h.see("a", P::Dialing, Some(Out));
    assert_eq!(h.state(), CallState::Outgoing);

    // Operating-system signals and presence report only "in a call".
    let mut h = Harness::new();
    h.see("a", P::Connected, None);
    assert_eq!(h.state(), CallState::Active);
}

#[test]
fn phases_that_do_not_start_a_call_track_nothing() {
    for phase in [
        P::Connecting,
        P::Ending,
        P::Ended {
            cause: ObservedEnd::Left,
        },
        P::Unknown,
    ] {
        let mut h = Harness::new();
        h.see("a", phase, None);
        assert_eq!(h.state(), CallState::Idle, "{phase:?}");
        assert_eq!(h.engine.tracks().len(), 0);
    }
}

#[test]
fn incoming_transitions() {
    let mut h = Harness::new();
    h.see("a", P::Ringing, Some(In));
    h.see("a", P::Connecting, Some(In));
    assert_eq!(h.state(), CallState::Connecting);
    h.see("a", P::Connected, Some(In));
    assert_eq!(h.state(), CallState::Active);

    let mut h = Harness::new();
    h.see("a", P::Ringing, Some(In));
    h.see("a", P::Connected, Some(In));
    assert_eq!(h.state(), CallState::Active);
}

#[test]
fn outgoing_transitions() {
    let mut h = Harness::new();
    h.see("a", P::Dialing, Some(Out));
    h.see("a", P::Connecting, Some(Out));
    assert_eq!(h.state(), CallState::Connecting);
    h.see("a", P::Connected, Some(Out));
    assert_eq!(h.state(), CallState::Active);

    let mut h = Harness::new();
    h.see("a", P::Dialing, Some(Out));
    h.see("a", P::Connected, Some(Out));
    assert_eq!(h.state(), CallState::Active);
}

#[test]
fn active_ending_ended_idle() {
    let mut h = Harness::new();
    h.see("a", P::Connected, None);
    h.see("a", P::Ending, None);
    assert_eq!(h.state(), CallState::Ending);
    h.see(
        "a",
        P::Ended {
            cause: ObservedEnd::Left,
        },
        None,
    );
    assert_eq!(h.ended(), Some(EndCause::Left));
    // Ended lingers for `ended_show`, then the device is idle.
    h.at(1);
    assert_eq!(h.ended(), Some(EndCause::Left));
    h.at(2);
    assert_eq!(h.state(), CallState::Idle);
    assert_eq!(h.engine.tracks().len(), 0);
}

#[test]
fn no_other_transition_is_accepted() {
    // Active does not ring again, connect again or go back to dialing.
    let mut h = Harness::new();
    h.see("a", P::Connected, None);
    for phase in [P::Ringing, P::Dialing, P::Connecting] {
        h.see("a", phase, None);
        assert_eq!(h.state(), CallState::Active, "{phase:?}");
    }
    // Ending does not become active again.
    h.see("a", P::Ending, None);
    h.see("a", P::Connected, None);
    assert_eq!(h.state(), CallState::Ending);
    // Incoming does not jump to ending; outgoing does not ring as incoming.
    let mut h = Harness::new();
    h.see("a", P::Ringing, Some(In));
    h.see("a", P::Ending, None);
    assert_eq!(h.state(), CallState::Incoming);
    h.see("a", P::Dialing, None);
    assert_eq!(h.state(), CallState::Incoming);
    // An ended call seen again after its end is a new call, not the old
    // one revived.
    let mut h = Harness::new();
    h.see("a", P::Connected, None);
    h.see(
        "a",
        P::Ended {
            cause: ObservedEnd::RemoteHangUp,
        },
        None,
    );
    let ended = h.engine.tracks()[0].0;
    h.at_millis(500);
    h.see("a", P::Connected, None);
    assert_eq!(
        h.engine.track_state(ended),
        Some(&CallState::Ended {
            cause: EndCause::RemoteHangUp
        })
    );
    assert_eq!(h.state(), CallState::Active);
    assert_ne!(h.engine.focus(), Some(ended));
}

#[test]
fn every_live_state_times_out() {
    let t = Timeouts::default();
    // Incoming rings at most `ring_max`, then it is missed.
    let mut h = Harness::new();
    h.see("a", P::Ringing, Some(In));
    h.at(t.ring_max.as_secs() - 1);
    assert_eq!(h.state(), CallState::Incoming);
    h.at(t.ring_max.as_secs());
    assert_eq!(h.ended(), Some(EndCause::Missed));

    // Outgoing dials at most `dial_max`, then it is unknown, then lost.
    let mut h = Harness::new();
    h.see("a", P::Dialing, Some(Out));
    h.at(t.dial_max.as_secs());
    assert_eq!(h.state(), CallState::Unknown);
    h.at(t.dial_max.as_secs() + t.stale_max.as_secs());
    assert_eq!(h.ended(), Some(EndCause::Lost));

    // Connecting at most `connect_max`.
    let mut h = Harness::new();
    h.see("a", P::Ringing, Some(In));
    h.see("a", P::Connecting, Some(In));
    h.at(t.connect_max.as_secs());
    assert_eq!(h.state(), CallState::Unknown);

    // Ending at most `ending_max`, then ended (left).
    let mut h = Harness::new();
    h.see("a", P::Connected, None);
    h.see("a", P::Ending, None);
    h.at(t.ending_max.as_secs());
    assert_eq!(h.ended(), Some(EndCause::Left));

    // Active without any news for `active_max`: unknown.
    let mut h = Harness::new();
    h.see("a", P::Connected, None);
    h.at(t.active_max.as_secs() - 1);
    h.see("a", P::Connected, None);
    h.at(t.active_max.as_secs() + 10);
    assert_eq!(h.state(), CallState::Active, "snapshots refresh it");
    h.at(2 * t.active_max.as_secs());
    assert_eq!(h.state(), CallState::Unknown);
}

// ------------------------------------------------------------------ T-S2

#[test]
fn incoming_ends_rejected_answered_elsewhere_or_missed() {
    for (observed, cause) in [
        (ObservedEnd::Rejected, EndCause::Rejected),
        (ObservedEnd::AnsweredElsewhere, EndCause::AnsweredElsewhere),
        (ObservedEnd::Missed, EndCause::Missed),
        (ObservedEnd::Unknown, EndCause::Missed),
    ] {
        let mut h = Harness::new();
        h.see("a", P::Ringing, Some(In));
        h.see("a", P::Ended { cause: observed }, Some(In));
        assert_eq!(h.ended(), Some(cause), "{observed:?}");
    }
    // It stopped ringing without being answered.
    let mut h = Harness::new();
    h.see("a", P::Ringing, Some(In));
    h.gone("a");
    assert_eq!(h.ended(), Some(EndCause::Missed));
}

#[test]
fn outgoing_ends_cancelled_declined_or_unanswered() {
    for (observed, cause) in [
        (ObservedEnd::Cancelled, EndCause::Cancelled),
        (ObservedEnd::Declined, EndCause::Declined),
        (ObservedEnd::NoAnswer, EndCause::NoAnswer),
    ] {
        let mut h = Harness::new();
        h.see("a", P::Dialing, Some(Out));
        h.see("a", P::Ended { cause: observed }, Some(Out));
        assert_eq!(h.ended(), Some(cause));
    }
    let mut h = Harness::new();
    h.see("a", P::Dialing, Some(Out));
    h.gone("a");
    assert_eq!(h.ended(), Some(EndCause::NoAnswer));
}

#[test]
fn active_ends_by_remote_hang_up_leaving_or_unknown() {
    for (observed, cause) in [
        (ObservedEnd::RemoteHangUp, EndCause::RemoteHangUp),
        (ObservedEnd::Left, EndCause::Left),
        (ObservedEnd::Missed, EndCause::Unknown),
    ] {
        let mut h = Harness::new();
        h.see("a", P::Connected, None);
        h.see("a", P::Ended { cause: observed }, None);
        assert_eq!(h.ended(), Some(cause));
    }
    let mut h = Harness::new();
    h.see("a", P::Connected, None);
    h.gone("a");
    assert_eq!(h.ended(), Some(EndCause::Unknown));
}

#[test]
fn failed_shows_briefly_then_idle() {
    let mut h = Harness::new();
    h.see("a", P::Connected, None);
    h.engine.provider_failed(&pid(MOCK), "Weird Text!", h.now);
    assert_eq!(
        h.state(),
        CallState::Failed {
            code: FailureCode::new(FailureCode::PROTOCOL)
        }
    );
    h.at(3);
    assert_eq!(h.state(), CallState::Idle);
    // A protocol disconnect fails the provider's calls too.
    let mut h = Harness::new();
    h.see("a", P::Connected, None);
    h.event(ProviderEvent::Disconnected {
        reason: DisconnectReason::Protocol,
    });
    assert!(matches!(h.state(), CallState::Failed { .. }));
}

// ------------------------------------------------------------------ T-S4

#[test]
fn a_disconnect_makes_calls_unknown_and_a_reconnect_reconciles() {
    let mut h = Harness::new();
    h.see("a", P::Connected, None);
    h.event(ProviderEvent::Disconnected {
        reason: DisconnectReason::AppExited,
    });
    assert_eq!(h.state(), CallState::Unknown);
    assert!(!h.engine.snapshot().connected);
    // The overlay stays on the active page while the call may come back.
    assert!(matches!(
        h.engine.overlay(),
        OverlayIntent::Shown {
            page: OverlayPage::Active,
            ..
        }
    ));
    h.at(10);
    h.connect();
    h.see("a", P::Connected, None);
    assert_eq!(h.state(), CallState::Active);
    assert!(h.engine.snapshot().connected);
}

#[test]
fn an_unknown_call_without_evidence_is_lost() {
    let mut h = Harness::new();
    h.see("a", P::Connected, None);
    let OverlayIntent::Shown { session, .. } = h.engine.overlay() else {
        panic!("shown")
    };
    h.event(ProviderEvent::Disconnected {
        reason: DisconnectReason::Network,
    });
    h.at(14);
    assert_eq!(h.state(), CallState::Unknown);
    h.at(15);
    assert_eq!(h.ended(), Some(EndCause::Lost));
    // The overlay goes when the call is lost.
    assert_eq!(h.engine.overlay(), OverlayIntent::Hidden);
    // A reconnect after that starts a new call and a new overlay session
    // (an observation at the instant it was lost would be late).
    h.at_millis(15_500);
    h.connect();
    h.see("a", P::Connected, None);
    let OverlayIntent::Shown { session: next, .. } = h.engine.overlay() else {
        panic!("shown")
    };
    assert_ne!(next, session);
}

#[test]
fn removing_a_provider_ends_its_calls() {
    let mut h = Harness::new();
    h.see("a", P::Connected, None);
    h.engine.remove_provider(&pid(MOCK), h.now);
    assert_eq!(h.ended(), Some(EndCause::Lost));
    assert_eq!(h.engine.overlay(), OverlayIntent::Hidden);
    // Its events are dropped from now on.
    h.see("b", P::Connected, None);
    assert_ne!(h.state(), CallState::Active);
}

#[test]
fn an_agent_restart_starts_empty() {
    // Nothing is persisted: a new engine knows no call until a provider
    // reports one.
    let engine = Engine::default();
    assert_eq!(engine.snapshot().state, CallState::Idle);
    assert_eq!(engine.overlay(), OverlayIntent::Hidden);
    assert_eq!(engine.next_wake(), None);
}

// ------------------------------------------------------- inferred signals

#[test]
fn operating_system_signals_need_a_moment_on_and_off() {
    let mut h = Harness::with(ProviderId::OS_SIGNALS, all_supported());
    let see = |h: &mut Harness| {
        h.see_full(
            "teams",
            P::Connected,
            None,
            Tri::Unknown,
            Tri::Unknown,
            Evidence::OsSignal,
        );
    };
    see(&mut h);
    assert_eq!(h.state(), CallState::Idle, "not before infer_on");
    // A flap shorter than `infer_on` shows nothing.
    h.gone("teams");
    h.at(3);
    assert_eq!(h.state(), CallState::Idle);
    see(&mut h);
    h.at(5);
    assert_eq!(h.state(), CallState::Active);
    // Gone for less than `infer_off`: still active.
    h.gone("teams");
    h.at(12);
    assert_eq!(h.state(), CallState::Active);
    see(&mut h);
    h.at(30);
    assert_eq!(h.state(), CallState::Active);
    // Gone for `infer_off`: ended.
    h.gone("teams");
    h.at(38);
    assert_eq!(h.ended(), Some(EndCause::Unknown));
}

#[test]
fn the_call_app_restarting_is_not_a_hang_up() {
    let mut h = Harness::with(ProviderId::OS_SIGNALS, all_supported());
    let see = |h: &mut Harness| {
        h.see_full(
            "teams",
            P::Connected,
            None,
            Tri::Unknown,
            Tri::Unknown,
            Evidence::OsSignal,
        );
    };
    see(&mut h);
    h.at(2);
    see(&mut h);
    assert_eq!(h.state(), CallState::Active);
    h.event(ProviderEvent::Disconnected {
        reason: DisconnectReason::AppExited,
    });
    assert_eq!(h.state(), CallState::Unknown);
    h.at(10);
    h.connect();
    see(&mut h);
    assert_eq!(h.state(), CallState::Active, "back within stale_max");
}

#[test]
fn inferred_calls_may_be_kept_off_the_overlay() {
    let mut h = Harness::with(ProviderId::OS_SIGNALS, all_supported());
    h.engine.set_overlay_on_inferred(false);
    h.see_full(
        "t",
        P::Connected,
        None,
        Tri::Unknown,
        Tri::Unknown,
        Evidence::OsSignal,
    );
    h.at(2);
    assert_eq!(h.state(), CallState::Active);
    assert_eq!(h.engine.overlay(), OverlayIntent::Hidden);
}

// ------------------------------------------------------------------ T-S6

#[test]
fn focus_order_with_simultaneous_calls() {
    let mut h = Harness::new();
    h.see("meeting", P::Connected, None);
    h.at(1);
    h.see("call", P::Ringing, Some(In));
    // Incoming outranks active: the overlay shows the incoming page.
    assert_eq!(h.state(), CallState::Incoming);
    let OverlayIntent::Shown { session, page, .. } = h.engine.overlay() else {
        panic!("shown")
    };
    assert_eq!(page, OverlayPage::Incoming);
    // A newer incoming call outranks the older one.
    h.at(2);
    h.see("second", P::Ringing, Some(In));
    let second = h.engine.focus();
    h.gone("second");
    assert_ne!(h.engine.focus(), second);
    assert_eq!(h.state(), CallState::Incoming);
    // Missed: back to the meeting, in the same overlay session.
    h.gone("call");
    h.at(5);
    assert_eq!(h.state(), CallState::Active);
    let OverlayIntent::Shown {
        session: still,
        page,
        ..
    } = h.engine.overlay()
    else {
        panic!("shown")
    };
    assert_eq!((still, page), (session, OverlayPage::Active));
}

#[test]
fn outgoing_outranks_active_and_unknown_comes_last() {
    let mut h = Harness::new();
    h.see("x", P::Connected, None);
    h.see("y", P::Dialing, Some(Out));
    assert_eq!(h.state(), CallState::Outgoing);
    // Two providers: an unknown call never takes the focus from a live one.
    let mut engine = Engine::default();
    let (a, b) = (pid("alpha"), pid("beta"));
    for p in [&a, &b] {
        engine.add_provider(p.clone(), consent(), all_supported());
        engine.on_event(
            p,
            ProviderEvent::Connected {
                session: ProviderSession(1),
            },
            Instant::ZERO,
        );
    }
    let obs = |k: &str| CallObservation {
        key: key(k),
        kind: CallKind::Meeting,
        phase: P::Connected,
        direction: None,
        mic_muted: Tri::Unknown,
        camera_on: Tri::Unknown,
        title: None,
        source: Evidence::AppApi,
        observed_at: Instant::ZERO,
    };
    engine.on_event(&a, ProviderEvent::Call(obs("m1")), Instant::from_secs(1));
    engine.on_event(&b, ProviderEvent::Call(obs("m2")), Instant::from_secs(2));
    // Most recently connected first.
    assert_eq!(engine.snapshot().provider, Some(b.clone()));
    engine.on_event(
        &b,
        ProviderEvent::Disconnected {
            reason: DisconnectReason::Network,
        },
        Instant::from_secs(3),
    );
    assert_eq!(engine.snapshot().provider, Some(a));
    assert_eq!(engine.snapshot().state, CallState::Active);
}

#[test]
fn meeting_to_meeting_keeps_the_overlay_session() {
    let mut h = Harness::new();
    h.see("one", P::Connected, None);
    let OverlayIntent::Shown {
        session,
        generation,
        ..
    } = h.engine.overlay()
    else {
        panic!("shown")
    };
    h.at(60);
    h.see("two", P::Connected, None);
    let states = h.engine.tracks();
    assert_eq!(
        states.iter().map(|(_, s)| s.clone()).collect::<Vec<_>>(),
        [
            CallState::Ended {
                cause: EndCause::Left
            },
            CallState::Active
        ]
    );
    let OverlayIntent::Shown {
        session: still,
        generation: newer,
        page,
    } = h.engine.overlay()
    else {
        panic!("shown")
    };
    assert_eq!((still, page), (session, OverlayPage::Active));
    assert!(newer > generation, "faces redraw for the new meeting");
}

#[test]
fn at_most_four_calls_are_tracked() {
    let mut h = Harness::new();
    for (i, k) in ["a", "b", "c", "d", "e"].iter().enumerate() {
        h.at(i as u64);
        h.see(k, P::Ringing, Some(In));
    }
    assert_eq!(h.engine.tracks().len(), MAX_TRACKS);
    assert!(
        h.engine
            .drain_events()
            .iter()
            .any(|e| matches!(e, EngineEvent::Overflow { .. }))
    );
}

// ------------------------------------------------------------------ T-S7

#[test]
fn a_command_without_confirmation_changes_nothing() {
    let mut h = Harness::new();
    h.see_full(
        "a",
        P::Connected,
        None,
        Tri::Known(false),
        Tri::Unknown,
        Evidence::AppApi,
    );
    let token = h.send(CallAction::MicToggle);
    // Pending, and the state is what was observed.
    let snap = h.engine.snapshot();
    assert_eq!(snap.pending, Some(CallAction::MicToggle));
    assert_eq!(snap.mic_muted, Tri::Known(false));
    // The provider says it did it, but no observation shows it: not yet.
    h.event(ProviderEvent::CommandOutcome {
        token,
        outcome: CommandOutcome::Confirmed,
    });
    assert_eq!(h.engine.snapshot().mic_muted, Tri::Known(false));
    assert_eq!(h.done().len(), 0);
    h.at(5);
    assert_eq!(h.done(), [(token, CommandEnd::Unconfirmed)]);
    let snap = h.engine.snapshot();
    assert!(snap.unconfirmed && snap.pending.is_none());
    assert_eq!(snap.mic_muted, Tri::Known(false));
    h.at(7);
    assert!(!h.engine.snapshot().unconfirmed);
}

#[test]
fn an_observation_confirms_a_command() {
    let mut h = Harness::new();
    h.see_full(
        "a",
        P::Connected,
        None,
        Tri::Known(false),
        Tri::Unknown,
        Evidence::AppApi,
    );
    let token = h.send(CallAction::MicSet { muted: true });
    h.at_millis(500);
    h.see_full(
        "a",
        P::Connected,
        None,
        Tri::Known(true),
        Tri::Unknown,
        Evidence::AppApi,
    );
    assert_eq!(h.done(), [(token, CommandEnd::Confirmed)]);
    assert_eq!(h.engine.snapshot().mic_muted, Tri::Known(true));
    // A set of the state it has sends nothing.
    assert_eq!(
        h.engine.request(CallAction::MicSet { muted: true }, h.now),
        Ok(Request::AlreadyThere)
    );
}

#[test]
fn answer_and_end_are_confirmed_by_the_call() {
    let mut h = Harness::new();
    h.see("a", P::Ringing, Some(In));
    let answer = h.send(CallAction::Answer);
    assert_eq!(h.state(), CallState::Incoming, "never optimistic");
    h.see("a", P::Connected, Some(In));
    assert_eq!(h.done(), [(answer, CommandEnd::Confirmed)]);
    let end = h.send(CallAction::End);
    assert_eq!(h.state(), CallState::Active);
    // `end` is confirmed by the provider: active → ending → ended (left).
    h.event(ProviderEvent::CommandOutcome {
        token: end,
        outcome: CommandOutcome::Confirmed,
    });
    assert_eq!(h.state(), CallState::Ending);
    assert_eq!(h.done(), [(end, CommandEnd::Confirmed)]);
    h.gone("a");
    assert_eq!(h.ended(), Some(EndCause::Left));
}

#[test]
fn reject_and_cancel() {
    let mut h = Harness::new();
    h.see("a", P::Ringing, Some(In));
    let reject = h.send(CallAction::Reject);
    h.gone("a");
    assert_eq!(h.ended(), Some(EndCause::Rejected));
    assert_eq!(h.done(), [(reject, CommandEnd::Confirmed)]);

    let mut h = Harness::new();
    h.see("a", P::Dialing, Some(Out));
    let cancel = h.send(CallAction::CancelOutgoing);
    h.gone("a");
    assert_eq!(h.ended(), Some(EndCause::Cancelled));
    assert_eq!(h.done(), [(cancel, CommandEnd::Confirmed)]);
}

#[test]
fn stale_and_forged_outcomes_are_dropped() {
    let mut h = Harness::new();
    h.see("a", P::Ringing, Some(In));
    let token = h.send(CallAction::Answer);
    // Another serial, or the right serial of another session: dropped.
    for forged in [
        CommandToken {
            session: token.session,
            serial: token.serial + 1,
        },
        CommandToken {
            session: ProviderSession(token.session.0 + 7),
            serial: token.serial,
        },
    ] {
        h.event(ProviderEvent::CommandOutcome {
            token: forged,
            outcome: CommandOutcome::Rejected {
                code: crate::FailureCode::new("no"),
            },
        });
    }
    assert_eq!(h.done().len(), 0);
    assert_eq!(h.engine.snapshot().pending, Some(CallAction::Answer));
    // A reconnect makes the old command unconfirmable.
    h.connect();
    assert_eq!(h.done(), [(token, CommandEnd::Unconfirmed)]);
}

#[test]
fn one_command_at_a_time_and_refusals_are_reported() {
    let mut h = Harness::new();
    h.see_full(
        "a",
        P::Connected,
        None,
        Tri::Known(false),
        Tri::Unknown,
        Evidence::AppApi,
    );
    let token = h.send(CallAction::MicToggle);
    assert_eq!(
        h.engine.request(CallAction::MicToggle, h.now),
        Err(GateError::Unconfirmed)
    );
    h.engine.command_refused(token, &CommandRefusal::Busy);
    assert_eq!(
        h.done(),
        [(
            token,
            CommandEnd::Rejected {
                code: "busy".into()
            }
        )]
    );
    h.send(CallAction::MicToggle);
    h.event(ProviderEvent::CommandOutcome {
        token: CommandToken {
            session: ProviderSession(h.session),
            serial: token.serial + 1,
        },
        outcome: CommandOutcome::Unknown,
    });
    assert!(h.engine.snapshot().unconfirmed);
}

#[test]
fn commands_need_a_call_consent_and_a_connection() {
    let mut h = Harness::new();
    assert_eq!(
        h.engine.request(CallAction::End, h.now),
        Err(GateError::NotInCall)
    );
    h.see("a", P::Connected, None);
    h.engine.add_provider(
        pid(MOCK),
        ConsentState {
            control: false,
            ..consent()
        },
        all_supported(),
    );
    assert_eq!(
        h.engine.request(CallAction::End, h.now),
        Err(GateError::Consent)
    );
    // Without the control consent the snapshot shows no control.
    assert!(!h.engine.snapshot().capabilities.controls_anything());
    h.engine.add_provider(pid(MOCK), consent(), all_supported());
    h.event(ProviderEvent::Disconnected {
        reason: DisconnectReason::Network,
    });
    assert_eq!(
        h.engine.request(CallAction::End, h.now),
        Err(GateError::ProviderOffline)
    );
}

// ------------------------------------------------------------------ T-S8

#[test]
fn a_set_on_a_toggle_only_provider() {
    let caps = CapabilitySet::from_reported([
        (Capability::MicToggle, Support::Supported),
        (Capability::MicReadState, Support::Supported),
    ])
    .0;
    let mut h = Harness::with(MOCK, caps);
    h.see("a", P::Connected, None);
    assert_eq!(
        h.engine.request(CallAction::MicSet { muted: true }, h.now),
        Err(GateError::Unsupported(UnsupportedReason::ToggleOnly))
    );
    h.see_full(
        "a",
        P::Connected,
        None,
        Tri::Known(false),
        Tri::Unknown,
        Evidence::AppApi,
    );
    let Ok(Request::Send { command, .. }) =
        h.engine.request(CallAction::MicSet { muted: true }, h.now)
    else {
        panic!("sent")
    };
    assert_eq!(command.action, CallAction::MicToggle);
    // Confirmed by the observed state the set asked for.
    h.see_full(
        "a",
        P::Connected,
        None,
        Tri::Known(true),
        Tri::Unknown,
        Evidence::AppApi,
    );
    assert!(matches!(h.done()[..], [(_, CommandEnd::Confirmed)]));
}

#[test]
fn titles_only_with_the_opt_in() {
    let mut h = Harness::new();
    let observation = |title: &str| CallObservation {
        key: key("a"),
        kind: CallKind::Meeting,
        phase: P::Connected,
        direction: None,
        mic_muted: Tri::Unknown,
        camera_on: Tri::Unknown,
        title: Some(Private::new(title.to_owned())),
        source: Evidence::AppApi,
        observed_at: Instant::ZERO,
    };
    h.event(ProviderEvent::Call(observation("Standup")));
    assert_eq!(h.engine.snapshot().title, None);
    h.engine.add_provider(
        pid(MOCK),
        ConsentState {
            titles: true,
            ..consent()
        },
        all_supported(),
    );
    h.event(ProviderEvent::Call(observation("Standup")));
    assert_eq!(h.engine.snapshot().title.as_deref(), Some("Standup"));
}

#[test]
fn missing_capabilities_are_reported_once() {
    let mut h = Harness::new();
    h.engine.report_missing(&pid(MOCK), 3);
    h.engine.report_missing(&pid(MOCK), 3);
    let reports = h
        .engine
        .drain_events()
        .into_iter()
        .filter(|e| matches!(e, EngineEvent::MissingCapabilities { .. }))
        .count();
    assert_eq!(reports, 1);
}

#[test]
fn since_stays_while_the_state_does() {
    let mut h = Harness::new();
    h.at(1);
    h.see("a", P::Connected, None);
    let since = h.engine.snapshot().since;
    h.at(30);
    h.see_full(
        "a",
        P::Connected,
        None,
        Tri::Known(true),
        Tri::Unknown,
        Evidence::AppApi,
    );
    assert_eq!(h.engine.snapshot().since, since);
    assert_eq!(since, 1000);
}

// ------------------------------------------------------- review (#452)

fn observation_at(call: &str, phase: ObservedPhase, at: Instant) -> CallObservation {
    CallObservation {
        key: key(call),
        kind: CallKind::OneToOne,
        phase,
        direction: None,
        mic_muted: Tri::Unknown,
        camera_on: Tri::Unknown,
        title: None,
        source: Evidence::AppApi,
        observed_at: at,
    }
}

/// A late or out-of-order observation neither revives an ended call
/// (while it lingers and after it went) nor overwrites newer state; an
/// observation made after the end is a new call.
#[test]
fn late_observations_do_not_revive_or_overwrite() {
    let mut h = Harness::new();
    h.at(1);
    h.event(ProviderEvent::Call(CallObservation {
        mic_muted: Tri::Known(true),
        ..observation_at("a", P::Connected, Instant::from_secs(1))
    }));
    h.at(3);
    // Older than what the track shows: dropped.
    h.event(ProviderEvent::Call(CallObservation {
        mic_muted: Tri::Known(false),
        ..observation_at("a", P::Connected, Instant::ZERO)
    }));
    assert_eq!(h.engine.snapshot().mic_muted, Tri::Known(true));
    h.event(ProviderEvent::Call(observation_at(
        "a",
        P::Ended {
            cause: ObservedEnd::Left,
        },
        Instant::from_secs(3),
    )));
    assert_eq!(h.ended(), Some(EndCause::Left));
    // A connected observation made before the end, arriving late.
    h.event(ProviderEvent::Call(observation_at(
        "a",
        P::Connected,
        Instant::from_secs(2),
    )));
    assert_eq!(h.ended(), Some(EndCause::Left), "lingering: not revived");
    h.at(10);
    assert_eq!(h.state(), CallState::Idle);
    h.event(ProviderEvent::Call(observation_at(
        "a",
        P::Connected,
        Instant::from_secs(2),
    )));
    assert_eq!(h.state(), CallState::Idle, "gone: not revived");
    // Made after the end: a new call.
    h.event(ProviderEvent::Call(observation_at(
        "a",
        P::Connected,
        Instant::from_secs(10),
    )));
    assert_eq!(h.state(), CallState::Active);
}

/// A call that went without an observation (gone) is not revived by an
/// observation made before it went.
#[test]
fn an_observation_from_before_a_gone_call_is_late() {
    let mut h = Harness::new();
    h.see("a", P::Ringing, Some(In));
    h.at(4);
    h.gone("a");
    assert_eq!(h.ended(), Some(EndCause::Missed));
    h.event(ProviderEvent::Call(CallObservation {
        direction: Some(In),
        ..observation_at("a", P::Ringing, Instant::from_secs(2))
    }));
    assert_eq!(h.ended(), Some(EndCause::Missed));
}

fn detection_only() -> CapabilitySet {
    CapabilitySet::from_reported(Capability::ALL.into_iter().map(|c| {
        let support = match c {
            Capability::DetectMeeting => Support::Inferred,
            _ => Support::Unsupported {
                reason: UnsupportedReason::ProviderHasNoApi,
            },
        };
        (c, support)
    }))
    .0
}

/// A mic or camera command is never confirmed from an inferred state or
/// operating-system evidence: only the app's own report confirms it.
#[test]
fn inferred_state_never_confirms_a_toggle() {
    // The state can be read only by inference: nothing can confirm.
    let caps = CapabilitySet::from_reported(Capability::ALL.into_iter().map(|c| {
        let support = match c {
            Capability::MicReadState | Capability::CameraReadState => Support::Inferred,
            _ => Support::Supported,
        };
        (c, support)
    }))
    .0;
    let mut h = Harness::with(MOCK, caps);
    h.see_full(
        "a",
        P::Connected,
        None,
        Tri::Known(false),
        Tri::Known(false),
        Evidence::AppApi,
    );
    let mic = h.send(CallAction::MicToggle);
    h.at(1);
    h.see_full(
        "a",
        P::Connected,
        None,
        Tri::Known(true),
        Tri::Known(false),
        Evidence::AppApi,
    );
    h.at(6);
    assert_eq!(h.done(), [(mic, CommandEnd::Unconfirmed)]);

    // Readable state, but the confirming observation is an OS signal.
    let mut h = Harness::new();
    h.see_full(
        "a",
        P::Connected,
        None,
        Tri::Known(false),
        Tri::Known(false),
        Evidence::AppApi,
    );
    let camera = h.send(CallAction::CameraToggle);
    h.at(1);
    h.see_full(
        "a",
        P::Connected,
        None,
        Tri::Known(false),
        Tri::Known(true),
        Evidence::OsSignal,
    );
    assert_eq!(h.done().len(), 0, "not confirmed by a signal");
    h.at(6);
    assert_eq!(h.done(), [(camera, CommandEnd::Unconfirmed)]);
}

/// Operating-system observations waiting to be believed are bounded.
#[test]
fn candidates_are_bounded() {
    let mut h = Harness::with(ProviderId::OS_SIGNALS, all_supported());
    for n in 0..(MAX_CANDIDATES * 4) {
        h.see_full(
            &format!("c{n}"),
            P::Connected,
            None,
            Tri::Unknown,
            Tri::Unknown,
            Evidence::OsSignal,
        );
    }
    assert_eq!(h.engine.candidates.len(), MAX_CANDIDATES);
    assert!(
        h.engine
            .drain_events()
            .iter()
            .any(|e| matches!(e, EngineEvent::Overflow { .. }))
    );
}

/// A detection-only provider (like Teams) cannot widen itself at runtime:
/// its `Capabilities` event is clamped to what the host registered, and
/// every action is refused.
#[test]
fn a_detection_only_provider_cannot_make_itself_controllable() {
    let mut h = Harness::with(ProviderId::OS_SIGNALS, detection_only());
    h.event(ProviderEvent::Capabilities(all_supported()));
    h.see_full(
        "teams",
        P::Connected,
        None,
        Tri::Known(false),
        Tri::Known(false),
        Evidence::OsSignal,
    );
    h.at(3);
    assert_eq!(h.state(), CallState::Active);
    assert!(!h.engine.snapshot().capabilities.controls_anything());
    for action in [
        CallAction::Answer,
        CallAction::Reject,
        CallAction::CancelOutgoing,
        CallAction::End,
        CallAction::MicToggle,
        CallAction::MicSet { muted: true },
        CallAction::CameraToggle,
        CallAction::CameraSet { on: false },
    ] {
        let result = h.engine.request(action, h.now);
        assert!(result.is_err(), "{action:?}: {result:?}");
    }
    // Narrowing still works.
    h.event(ProviderEvent::Capabilities(CapabilitySet::none(
        UnsupportedReason::ProviderHasNoApi,
    )));
    assert_eq!(
        h.engine
            .snapshot()
            .capabilities
            .get(Capability::DetectMeeting),
        Support::Unsupported {
            reason: UnsupportedReason::ProviderHasNoApi
        }
    );
}

/// Nothing is observed without the observe consent, and withdrawing it
/// ends the provider's calls and drops what it observed; withdrawing the
/// title consent scrubs titles.
#[test]
fn observations_need_the_observe_consent() {
    let mut engine = Engine::default();
    let mock = pid(MOCK);
    let without = ConsentState {
        observe: false,
        control: false,
        titles: false,
    };
    engine.add_provider(mock.clone(), without, all_supported());
    engine.on_event(
        &mock,
        ProviderEvent::Connected {
            session: ProviderSession(1),
        },
        Instant::ZERO,
    );
    engine.on_event(
        &mock,
        ProviderEvent::Call(observation_at("a", P::Connected, Instant::ZERO)),
        Instant::ZERO,
    );
    assert_eq!(engine.tracks().len(), 0, "not observed without consent");

    let mut h = Harness::new();
    h.engine.add_provider(
        pid(MOCK),
        ConsentState {
            titles: true,
            ..consent()
        },
        all_supported(),
    );
    h.event(ProviderEvent::Call(CallObservation {
        title: Some(Private::new("Standup".to_owned())),
        ..observation_at("a", P::Connected, Instant::ZERO)
    }));
    let token = h.send(CallAction::End);
    // Titles withdrawn: scrubbed from the track.
    h.engine.add_provider(pid(MOCK), consent(), all_supported());
    assert!(h.engine.tracks.iter().all(|t| t.title.is_none()));
    // Observe withdrawn: the call ends as lost, its command unconfirmed.
    h.engine.add_provider(pid(MOCK), without, all_supported());
    assert_eq!(h.ended(), Some(EndCause::Lost));
    assert_eq!(h.done(), [(token, CommandEnd::Unconfirmed)]);
    assert!(!matches!(h.engine.overlay(), OverlayIntent::Shown { .. }));
    h.at(1);
    h.see("b", P::Connected, None);
    assert!(h.engine.tracks().iter().all(|(_, s)| !s.is_live()));
}

/// A command waiting on an inferred call that goes by itself ends as
/// unconfirmed (it is not left without an end).
#[test]
fn a_command_on_an_inferred_call_that_goes_ends_unconfirmed() {
    let mut h = Harness::with(ProviderId::OS_SIGNALS, all_supported());
    h.see_full(
        "teams",
        P::Connected,
        None,
        Tri::Unknown,
        Tri::Unknown,
        Evidence::OsSignal,
    );
    h.at(3);
    assert_eq!(h.state(), CallState::Active);
    let token = h.send(CallAction::End);
    h.engine.drain_events();
    h.gone("teams");
    // The command's own deadline is later than infer_off here.
    h.engine.timeouts.command_max = Duration::from_secs(60);
    h.engine.tracks[0].pending.as_mut().unwrap().deadline = h.now.after(Duration::from_secs(60));
    h.at(12);
    assert!(matches!(h.ended(), Some(EndCause::Unknown)));
    assert_eq!(h.done(), [(token, CommandEnd::Unconfirmed)]);
}

/// With no call, the snapshot shows no control without the control
/// consent.
#[test]
fn the_idle_snapshot_shows_no_control_without_consent() {
    let mut engine = Engine::default();
    engine.add_provider(
        pid(MOCK),
        ConsentState {
            observe: true,
            control: false,
            titles: false,
        },
        all_supported(),
    );
    assert_eq!(engine.snapshot().state, CallState::Idle);
    assert!(!engine.snapshot().capabilities.controls_anything());
}

/// One provider's signals do not keep another provider's unknown call
/// alive past the stale timeout.
#[test]
fn inferred_evidence_is_per_provider() {
    let mut engine = Engine::default();
    let os = pid(ProviderId::OS_SIGNALS);
    let mock = pid(MOCK);
    for id in [&os, &mock] {
        engine.add_provider(id.clone(), consent(), all_supported());
        engine.on_event(
            id,
            ProviderEvent::Connected {
                session: ProviderSession(1),
            },
            Instant::ZERO,
        );
    }
    engine.on_event(
        &os,
        ProviderEvent::Call(CallObservation {
            source: Evidence::OsSignal,
            ..observation_at("teams", P::Connected, Instant::ZERO)
        }),
        Instant::ZERO,
    );
    engine.tick(Instant::from_secs(3));
    engine.on_event(
        &mock,
        ProviderEvent::Call(observation_at("a", P::Connected, Instant::from_secs(3))),
        Instant::from_secs(3),
    );
    engine.on_event(
        &mock,
        ProviderEvent::Call(observation_at("a", P::Unknown, Instant::from_secs(4))),
        Instant::from_secs(4),
    );
    let stale = engine.timeouts().stale_max.as_secs();
    engine.tick(Instant::from_secs(4 + stale));
    let states: Vec<CallState> = engine.tracks().into_iter().map(|(_, s)| s).collect();
    assert!(
        states.contains(&CallState::Ended {
            cause: EndCause::Lost
        }),
        "{states:?}"
    );
}

/// The event list is bounded between drains.
#[test]
fn events_are_bounded() {
    let mut h = Harness::new();
    for n in 0..(MAX_EVENTS as u64) {
        h.at(n * 10);
        h.see(&format!("a{}", n % 3), P::Connected, None);
        h.see(
            &format!("a{}", n % 3),
            P::Ended {
                cause: ObservedEnd::Left,
            },
            None,
        );
    }
    assert!(h.engine.drain_events().len() <= MAX_EVENTS);
}

/// An observation stamped exactly at the end instant is late: a stale,
/// reordered "connected" does not bring the overlay back. One a moment
/// later is a new call.
#[test]
fn an_observation_at_the_end_instant_is_late() {
    let mut h = Harness::new();
    h.at(1);
    h.see("a", P::Connected, None);
    h.at(2);
    h.see(
        "a",
        P::Ended {
            cause: ObservedEnd::Left,
        },
        None,
    );
    // Same instant as the end, while it lingers and after it went.
    h.see("a", P::Connected, None);
    assert_eq!(h.ended(), Some(EndCause::Left));
    assert!(!matches!(h.engine.overlay(), OverlayIntent::Shown { .. }));
    h.at(10);
    h.event(ProviderEvent::Call(observation_at(
        "a",
        P::Connected,
        Instant::from_secs(2),
    )));
    assert_eq!(h.state(), CallState::Idle);
    assert!(!matches!(h.engine.overlay(), OverlayIntent::Shown { .. }));
    h.see("a", P::Connected, None);
    assert_eq!(h.state(), CallState::Active, "after the end: a new call");
}
