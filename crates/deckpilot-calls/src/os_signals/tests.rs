//! T-X1 (parsers on fixtures), T-T1 (attribution to Teams, hysteresis, a
//! source that fails) and T-T2 (every control unsupported, no text claims
//! control). No live operating-system query is made: only fixtures.

use std::collections::VecDeque;

use super::*;
use crate::engine::{Engine, OverlayIntent};
use crate::provider::ConsentState;
use crate::snapshot::CallState;

const PW_DUMP: &str = include_str!("fixtures/pw-dump-browser-call.json");
const WASAPI_TEAMS: &str = include_str!("fixtures/wasapi-teams-in-call.json");
const WASAPI_OTHER: &str = include_str!("fixtures/wasapi-other-app.json");
const CORE_AUDIO_TEAMS: &str = include_str!("fixtures/core-audio-teams-in-call.json");
const CORE_AUDIO_IDLE: &str = include_str!("fixtures/core-audio-idle.json");

// ----------------------------------------------------------------- T-X1

#[test]
fn pw_dump_gives_the_capture_and_camera_streams_by_binary() {
    let signals = parse_pw_dump(PW_DUMP).unwrap();
    assert_eq!(
        signals.captures,
        [
            Capture {
                executable: "chromium".into(),
                active: true
            },
            Capture {
                executable: "obs".into(),
                active: false
            }
        ]
    );
    assert_eq!(signals.camera_in_use, Tri::Known(true));
    assert_eq!(signals.cameras.len(), 1);
    assert!(parse_pw_dump("not json").is_err());
}

#[test]
fn wasapi_sessions_give_the_image_names_only() {
    let signals = parse_wasapi_sessions(WASAPI_TEAMS).unwrap();
    assert_eq!(signals.captures[0].executable, "ms-teams.exe", "no path");
    assert!(signals.captures[0].active);
    assert!(!signals.captures[1].active);
    assert_eq!(
        signals.camera_in_use,
        Tri::Unknown,
        "no camera signal on Windows"
    );
    assert!(parse_wasapi_sessions("{}").is_err());
}

#[test]
fn core_audio_records_give_the_running_input_processes() {
    let signals = parse_core_audio(CORE_AUDIO_TEAMS).unwrap();
    assert!(
        signals
            .captures
            .iter()
            .any(|c| c.executable == "MSTeams" && c.active)
    );
    assert_eq!(signals.camera_in_use, Tri::Known(true));
    let idle = parse_core_audio(CORE_AUDIO_IDLE).unwrap();
    assert_eq!(idle.camera_in_use, Tri::Known(false));
}

// ----------------------------------------------------------------- T-T1

#[test]
fn teams_is_recognized_by_its_executables_only() {
    let teams = Profile::teams();
    let windows = parse_wasapi_sessions(WASAPI_TEAMS).unwrap();
    assert!(attribute(&teams, Platform::Windows, &windows).holds_microphone);
    // Another app holding the microphone, Teams' session expired: no call.
    let other = parse_wasapi_sessions(WASAPI_OTHER).unwrap();
    assert!(!attribute(&teams, Platform::Windows, &other).holds_microphone);
    let mac = parse_core_audio(CORE_AUDIO_TEAMS).unwrap();
    let on_mac = attribute(&teams, Platform::MacOs, &mac);
    assert!(on_mac.holds_microphone);
    // The camera is in use, but not attributed to Teams on macOS.
    assert_eq!(on_mac.camera, Tri::Unknown);
    assert!(
        !attribute(
            &teams,
            Platform::MacOs,
            &parse_core_audio(CORE_AUDIO_IDLE).unwrap()
        )
        .holds_microphone
    );
    // Linux: a browser holds the microphone; Teams is never claimed.
    let linux = parse_pw_dump(PW_DUMP).unwrap();
    let on_linux = attribute(&teams, Platform::Linux, &linux);
    assert!(!on_linux.holds_microphone);
    assert_eq!(on_linux.camera, Tri::Unknown);
    assert!(!teams.matches(Platform::MacOs, "MSTeamsHelper"));
    assert!(teams.matches(Platform::Windows, "MS-TEAMS.EXE"));
}

/// A source that replays fixtures, one per sample.
struct Replay {
    platform: Platform,
    samples: VecDeque<Result<Signals, SourceError>>,
    last: Signals,
}

impl Replay {
    fn new(platform: Platform, samples: Vec<Result<Signals, SourceError>>) -> Self {
        Self {
            platform,
            samples: samples.into(),
            last: Signals::default(),
        }
    }
}

impl SignalSource for Replay {
    fn platform(&self) -> Platform {
        self.platform
    }

    fn sample(&mut self, _now: Instant) -> Result<Signals, SourceError> {
        match self.samples.pop_front() {
            Some(Ok(signals)) => {
                self.last = signals.clone();
                Ok(signals)
            }
            Some(Err(error)) => Err(error),
            None => Ok(self.last.clone()),
        }
    }
}

fn consent() -> ConsentState {
    ConsentState {
        observe: true,
        control: true,
        titles: false,
    }
}

/// The provider and the engine, sampled once a second.
fn run(provider: &mut OsSignalsProvider, engine: &mut Engine, seconds: std::ops::Range<u64>) {
    let id = provider.id();
    for second in seconds {
        let now = Instant::from_secs(second);
        for event in provider.poll(now) {
            engine.on_event(&id, event, now);
        }
        engine.tick(now);
    }
}

fn setup(
    platform: Platform,
    samples: Vec<Result<Signals, SourceError>>,
) -> (OsSignalsProvider, Engine) {
    let mut provider =
        OsSignalsProvider::new(Profile::teams(), Box::new(Replay::new(platform, samples)));
    let discovery = provider.discover(Instant::ZERO);
    let mut engine = Engine::default();
    engine.add_provider(provider.id(), consent(), discovery.capabilities);
    provider
        .connect(&ProviderCx {
            consent: consent(),
            session: ProviderSession(1),
        })
        .unwrap();
    engine.on_event(
        &provider.id(),
        ProviderEvent::Connected {
            session: ProviderSession(1),
        },
        Instant::ZERO,
    );
    (provider, engine)
}

#[test]
fn a_teams_call_is_inferred_with_hysteresis_and_ends_when_the_microphone_stays_free() {
    let teams = parse_wasapi_sessions(WASAPI_TEAMS).unwrap();
    let free = parse_wasapi_sessions(WASAPI_OTHER).unwrap();
    let mut samples = vec![Ok(teams.clone()); 5];
    // A 3 s flap (switching headsets) does not end the call.
    samples.extend(vec![Ok(free.clone()); 3]);
    samples.extend(vec![Ok(teams.clone()); 5]);
    samples.extend(vec![Ok(free.clone()); 12]);
    let (mut provider, mut engine) = setup(Platform::Windows, samples);
    run(&mut provider, &mut engine, 0..2);
    assert_eq!(engine.snapshot().state, CallState::Idle, "not before 2 s");
    run(&mut provider, &mut engine, 2..5);
    let snapshot = engine.snapshot();
    assert_eq!(snapshot.state, CallState::Active);
    assert_eq!(
        snapshot.mic_muted,
        Tri::Unknown,
        "holding the mic is not 'not muted'"
    );
    assert!(matches!(engine.overlay(), OverlayIntent::Shown { .. }));
    run(&mut provider, &mut engine, 5..13);
    assert_eq!(
        engine.snapshot().state,
        CallState::Active,
        "the flap is bridged"
    );
    run(&mut provider, &mut engine, 13..30);
    assert!(matches!(
        engine.snapshot().state,
        CallState::Ended { .. } | CallState::Idle
    ));
    assert_eq!(engine.overlay(), OverlayIntent::Hidden);
}

#[test]
fn a_failing_source_makes_the_call_unknown_and_then_lost() {
    let teams = parse_core_audio(CORE_AUDIO_TEAMS).unwrap();
    let mut samples = vec![Ok(teams); 4];
    samples.extend((0..30).map(|_| Err(SourceError(FailureCode::new("source-gone")))));
    let (mut provider, mut engine) = setup(Platform::MacOs, samples);
    run(&mut provider, &mut engine, 0..4);
    assert_eq!(engine.snapshot().state, CallState::Active);
    run(&mut provider, &mut engine, 4..6);
    assert_eq!(engine.snapshot().state, CallState::Unknown);
    run(&mut provider, &mut engine, 6..25);
    assert!(!engine.snapshot().state.is_live());
}

#[test]
fn on_linux_nothing_is_claimed_for_teams() {
    let browser = parse_pw_dump(PW_DUMP).unwrap();
    let (mut provider, mut engine) = setup(Platform::Linux, vec![Ok(browser); 10]);
    assert!(matches!(
        provider.discover(Instant::ZERO).availability,
        Availability::Unsupported {
            reason: UnsupportedReason::PlatformNotSupported
        }
    ));
    run(&mut provider, &mut engine, 0..10);
    assert_eq!(engine.snapshot().state, CallState::Idle);
}

// ----------------------------------------------------------------- T-T2

#[test]
fn every_teams_control_is_unsupported_and_only_detection_is_inferred() {
    for platform in [Platform::MacOs, Platform::Windows] {
        let set = capabilities(&Profile::teams(), platform);
        assert!(!set.controls_anything(), "{platform:?}");
        for capability in Capability::ALL {
            let support = set.get(capability);
            match capability {
                Capability::DetectMeeting | Capability::MicReadState => {
                    assert_eq!(support, Support::Inferred, "{capability:?}");
                }
                _ => assert!(
                    matches!(support, Support::Unsupported { .. }),
                    "{capability:?}"
                ),
            }
        }
        // Control capabilities name the missing API as the reason.
        assert_eq!(
            set.get(Capability::MicToggle),
            Support::Unsupported {
                reason: UnsupportedReason::ProviderHasNoApi
            }
        );
    }
}

#[test]
fn commands_are_refused_before_anything_is_sent() {
    let (mut provider, _) = setup(Platform::Windows, Vec::new());
    for action in [
        crate::provider::CallAction::End,
        crate::provider::CallAction::MicToggle,
        crate::provider::CallAction::CameraSet { on: false },
        crate::provider::CallAction::Answer,
    ] {
        assert_eq!(
            provider.command(
                CallCommand {
                    key: CallKey::new("os-signals:teams").unwrap(),
                    action,
                },
                CommandToken {
                    session: ProviderSession(1),
                    serial: 1,
                },
            ),
            Err(CommandRefusal::Unsupported(
                UnsupportedReason::ProviderHasNoApi
            ))
        );
    }
}

/// The faces of a Teams call show the reason on every control, and no
/// face says a control worked or what Teams' mute state is.
#[test]
fn faces_never_claim_control_of_teams() {
    use crate::driver::{Locale, face};
    let teams = parse_wasapi_sessions(WASAPI_TEAMS).unwrap();
    let (mut provider, mut engine) = setup(Platform::Windows, vec![Ok(teams); 5]);
    run(&mut provider, &mut engine, 0..5);
    let snapshot = engine.snapshot();
    assert_eq!(snapshot.state, CallState::Active);
    for id in [
        "mic-toggle",
        "mic-mute",
        "mic-unmute",
        "camera-toggle",
        "camera-on",
        "camera-off",
        "end",
    ] {
        for locale in [Locale::En, Locale::Pl] {
            let face = face(id, &snapshot, locale);
            assert_eq!(
                face.style,
                deckpilot_core::FaceStyle::Notice,
                "{id} {locale:?}"
            );
            let detail = face.detail.clone().unwrap_or_default();
            for claim in ["Muted", "Wyciszony", "Włączony", "Waiting", "Czekam"] {
                assert!(!detail.contains(claim), "{id}: {detail}");
            }
        }
    }
}
