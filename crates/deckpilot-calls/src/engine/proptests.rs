//! T-S3: no state without an exit. Random event orders (a fixed-seed
//! generator, so every run is the same) from two providers, with commands
//! and clock steps in between; after each step the invariants hold, and
//! once events stop every call times out and the engine is idle with
//! nothing scheduled.

use std::time::Duration;

use super::tests::{all_supported, consent, key, pid};
use super::*;

/// xorshift64*: deterministic, no dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[usize::try_from(self.below(items.len() as u64)).unwrap()]
    }
}

const PHASES: [ObservedPhase; 9] = [
    ObservedPhase::Ringing,
    ObservedPhase::Dialing,
    ObservedPhase::Connecting,
    ObservedPhase::Connected,
    ObservedPhase::Ending,
    ObservedPhase::Ended {
        cause: ObservedEnd::Left,
    },
    ObservedPhase::Ended {
        cause: ObservedEnd::Rejected,
    },
    ObservedPhase::Ended {
        cause: ObservedEnd::RemoteHangUp,
    },
    ObservedPhase::Unknown,
];

const ACTIONS: [CallAction; 6] = [
    CallAction::Answer,
    CallAction::Reject,
    CallAction::End,
    CallAction::MicToggle,
    CallAction::MicSet { muted: true },
    CallAction::CameraToggle,
];

fn check_invariants(engine: &Engine, step: usize) {
    assert!(engine.tracks().len() <= MAX_TRACKS, "step {step}");
    let live = engine.tracks().iter().any(|(_, s)| s.is_live());
    let any = !engine.tracks().is_empty();
    // Anything tracked has a way out on the clock.
    if any {
        assert!(
            engine.next_wake().is_some(),
            "step {step}: no wake for {:?}",
            engine.tracks()
        );
    }
    // The overlay is shown exactly while the focused call is live.
    let focused_live = engine.snapshot().state.is_live();
    match engine.overlay() {
        OverlayIntent::Shown { .. } => assert!(focused_live, "step {step}"),
        OverlayIntent::Hidden => assert!(!focused_live, "step {step}"),
    }
    if !live {
        assert!(!matches!(engine.overlay(), OverlayIntent::Shown { .. }));
    }
}

#[allow(clippy::too_many_lines)]
fn run(seed: u64) {
    let mut rng = Rng(seed);
    let mut engine = Engine::default();
    let providers = [pid("alpha"), pid(ProviderId::OS_SIGNALS)];
    let mut sessions = [0_u64; 2];
    for p in &providers {
        engine.add_provider(p.clone(), consent(), all_supported());
    }
    let mut now = Instant::ZERO;
    let mut tokens: Vec<(usize, CommandToken)> = Vec::new();
    for step in 0..400 {
        let which = usize::try_from(rng.below(2)).unwrap();
        let provider = &providers[which];
        match rng.below(10) {
            0 => {
                sessions[which] += 1;
                engine.on_event(
                    provider,
                    ProviderEvent::Connected {
                        session: ProviderSession(sessions[which]),
                    },
                    now,
                );
            }
            1 => engine.on_event(
                provider,
                ProviderEvent::Disconnected {
                    reason: rng.pick(&[
                        DisconnectReason::AppExited,
                        DisconnectReason::Network,
                        DisconnectReason::Protocol,
                    ]),
                },
                now,
            ),
            2 => engine.on_event(
                provider,
                ProviderEvent::CallGone {
                    key: key(rng.pick(&["a", "b", "c"])),
                },
                now,
            ),
            3 => {
                if let Ok(Request::Send { token, .. }) = engine.request(rng.pick(&ACTIONS), now) {
                    tokens.push((which, token));
                }
            }
            4 => {
                if let Some((owner, token)) = tokens.pop() {
                    let outcome = match rng.below(3) {
                        0 => CommandOutcome::Confirmed,
                        1 => CommandOutcome::Unknown,
                        _ => CommandOutcome::Rejected { code: "no".into() },
                    };
                    engine.on_event(
                        &providers[owner],
                        ProviderEvent::CommandOutcome { token, outcome },
                        now,
                    );
                }
            }
            5 | 6 => {
                now = now.after(Duration::from_millis(rng.below(20_000)));
                engine.tick(now);
            }
            _ => {
                let observation = CallObservation {
                    key: key(rng.pick(&["a", "b", "c", "d", "e"])),
                    kind: CallKind::Unknown,
                    phase: rng.pick(&PHASES),
                    direction: rng.pick(&[
                        None,
                        Some(Direction::Incoming),
                        Some(Direction::Outgoing),
                    ]),
                    mic_muted: rng.pick(&[Tri::Unknown, Tri::Known(true), Tri::Known(false)]),
                    camera_on: rng.pick(&[Tri::Unknown, Tri::Known(true)]),
                    title: None,
                    source: if which == 1 {
                        Evidence::OsSignal
                    } else {
                        Evidence::AppApi
                    },
                    observed_at: now,
                };
                engine.on_event(provider, ProviderEvent::Call(observation), now);
            }
        }
        check_invariants(&engine, step);
    }
    // No more events: every call times out on the clock alone.
    let t = engine.timeouts();
    let bound = now
        .after(t.active_max)
        .after(t.inferred_max)
        .after(t.active_max)
        .after(Duration::from_secs(3600));
    let mut wakes = 0;
    while let Some(wake) = engine.next_wake() {
        assert!(wake <= bound, "seed {seed}: a wake beyond every timeout");
        now = wake.max(now);
        engine.tick(now);
        wakes += 1;
        assert!(wakes < 10_000, "seed {seed}: the clock never settles");
    }
    assert!(
        engine.tracks().is_empty(),
        "seed {seed}: {:?}",
        engine.tracks()
    );
    assert_eq!(engine.snapshot().state, CallState::Idle);
    assert_eq!(engine.overlay(), OverlayIntent::Hidden);
}

#[test]
fn no_state_without_an_exit_over_random_event_orders() {
    for seed in 1..=64_u64 {
        run(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15));
    }
}
