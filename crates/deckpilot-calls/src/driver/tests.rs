//! The driver against a real core runtime, on the engine's virtual clock:
//! frames follow the call on every device with the overlay on (T-O3),
//! meeting to meeting keeps them (T-S5), dismiss and the watchdog (T-O5),
//! faces never claim more than was confirmed (T-C1), and the first
//! "Back to work" cases (the full set is in `testing`'s mock provider
//! tests).

use deckpilot_core::{
    DeviceLayout, Engine as CoreEngine, MachineConfig, Os, Profile, RuntimeOutcome,
};
use deckpilot_device::{ControlInput, DeviceEvent, ModelCatalog, models};

use super::*;
use crate::engine::tests::{all_supported, consent, key};
use crate::layouts::{Shape, builtin_templates, generate};
use crate::provider::{
    CallKind, CallObservation, CapabilitySet, Evidence, ObservedEnd, ObservedPhase,
    ProviderSession, UnsupportedReason,
};

pub(crate) fn device(model: &str, serial: &str) -> DeviceId {
    DeviceId {
        model: DeviceModelId::new(model),
        serial: serial.into(),
    }
}

pub(crate) struct World {
    pub runtime: Runtime,
    pub driver: CallsDriver,
    pub now: Instant,
    pub provider: ProviderId,
    pub mk2: DeviceId,
    pub plus: DeviceId,
    pub profile: Profile,
}

impl World {
    pub fn new() -> Self {
        Self::with(all_supported())
    }

    pub fn with(capabilities: CapabilitySet) -> Self {
        let catalog = ModelCatalog::builtin();
        let mut runtime = Runtime::new(
            CoreEngine::new(deckpilot_core::ActionRegistry::default()),
            catalog.clone(),
            MachineConfig::new(Os::current()),
        );
        let mut profile = Profile::new("Work");
        for model in [models::STREAM_DECK_MK2_ID, models::STREAM_DECK_PLUS_ID] {
            profile
                .layouts
                .push(DeviceLayout::new("Main", DeviceModelId::new(model)));
        }
        runtime.set_profile(Some(profile.clone()), None);
        let mut engine = crate::engine::Engine::default();
        let provider = ProviderId::new(ProviderId::MOCK).unwrap();
        engine.add_provider(provider.clone(), consent(), capabilities);
        let mut driver = CallsDriver::new(engine);
        for model in catalog.iter() {
            let shape = Shape::of(&model.capabilities);
            let template = builtin_templates()
                .into_iter()
                .find(|t| t.shape == shape)
                .unwrap();
            driver.set_layout(
                model.id.clone(),
                generate(&template, &model.id, &model.capabilities).unwrap(),
            );
        }
        let mut world = Self {
            runtime,
            driver,
            now: Instant::ZERO,
            provider,
            mk2: device(models::STREAM_DECK_MK2_ID, "A"),
            plus: device(models::STREAM_DECK_PLUS_ID, "B"),
            profile,
        };
        let (mk2, plus) = (world.mk2.clone(), world.plus.clone());
        world.connect(&mk2);
        world.connect(&plus);
        world.event(ProviderEvent::Connected {
            session: ProviderSession(1),
        });
        world
    }

    pub fn connect(&mut self, device: &DeviceId) {
        self.runtime
            .handle_event(&DeviceEvent::Connected {
                device: device.clone(),
            })
            .unwrap();
        self.driver.device_connected(&mut self.runtime, self.now);
    }

    pub fn disconnect(&mut self, device: &DeviceId) {
        self.runtime
            .handle_event(&DeviceEvent::Disconnected {
                device: device.clone(),
            })
            .unwrap();
    }

    pub fn event(&mut self, event: ProviderEvent) {
        let provider = self.provider.clone();
        self.driver
            .on_provider_event(&mut self.runtime, &provider, event, self.now);
    }

    pub fn see(&mut self, call: &str, phase: ObservedPhase, mic: Tri) {
        self.event(ProviderEvent::Call(CallObservation {
            key: key(call),
            kind: CallKind::Meeting,
            phase,
            direction: matches!(phase, ObservedPhase::Ringing)
                .then_some(crate::provider::Direction::Incoming),
            mic_muted: mic,
            camera_on: Tri::Known(false),
            title: None,
            source: Evidence::AppApi,
            observed_at: self.now,
        }));
    }

    pub fn active(&mut self, call: &str) {
        self.see(call, ObservedPhase::Connected, Tri::Known(false));
    }

    pub fn end(&mut self, call: &str) {
        self.see(
            call,
            ObservedPhase::Ended {
                cause: ObservedEnd::RemoteHangUp,
            },
            Tri::Unknown,
        );
    }

    /// Moves the clock to `secs`, waking the driver at every due instant.
    pub fn at(&mut self, secs: u64) {
        let target = Instant::from_secs(secs);
        while let Some(wake) = self.driver.next_wake() {
            if wake > target {
                break;
            }
            self.now = wake.max(self.now);
            self.driver.tick(&mut self.runtime, self.now);
        }
        self.now = target.max(self.now);
        self.driver.tick(&mut self.runtime, self.now);
    }

    pub fn shows(&self, device: &DeviceId) -> bool {
        self.runtime.shows_overlay(device)
    }

    pub fn page(&self, device: &DeviceId) -> Option<Page> {
        let (_, page) = self.runtime.overlay_page(device)?;
        let generated = &self.driver.layouts[&device.model];
        generated
            .pages
            .iter()
            .find(|(_, id)| **id == page)
            .map(|(p, _)| *p)
    }

    /// Presses and releases a key of `device`: what the runtime did, and
    /// what the driver did with an overlay key.
    pub fn tap(
        &mut self,
        device: &DeviceId,
        control: ControlId,
    ) -> (RuntimeOutcome, Option<KeyOutcome>) {
        let press = DeviceEvent::Input {
            device: device.clone(),
            control,
            input: ControlInput::Press,
        };
        let outcome = self.runtime.handle_event(&press).unwrap();
        let handled = match &outcome {
            RuntimeOutcome::OverlayAction {
                session,
                action_type,
                parameters,
                ..
            } => Some(self.driver.overlay_key(
                &mut self.runtime,
                device,
                *session,
                action_type.as_str(),
                parameters,
                self.now,
            )),
            RuntimeOutcome::OverlayDismissed { session, .. } => {
                self.driver.dismissed(device, *session);
                None
            }
            _ => None,
        };
        self.runtime
            .handle_event(&DeviceEvent::Input {
                device: device.clone(),
                control,
                input: ControlInput::Release,
            })
            .unwrap();
        (outcome, handled)
    }

    /// The control bound to Calls action `id` on `device`'s overlay page.
    pub fn control_of(&self, device: &DeviceId, id: &str) -> ControlId {
        let generated = &self.driver.layouts[&device.model];
        let (_, page) = self.runtime.overlay_page(device).unwrap();
        generated
            .layout
            .page(page)
            .unwrap()
            .bindings
            .iter()
            .find(|b| generated.action_of(b.target.action_id().unwrap()) == Some(id))
            .unwrap()
            .control
    }

    pub fn face(&self, device: &DeviceId, control: ControlId) -> KeyFace {
        self.runtime
            .faces(device)
            .into_iter()
            .find(|(c, _)| *c == control)
            .unwrap()
            .1
    }
}

#[test]
fn every_device_with_the_overlay_on_follows_the_call() {
    let mut w = World::new();
    let (mk2, plus) = (w.mk2.clone(), w.plus.clone());
    let own = w.runtime.navigation_state(&mk2);
    w.active("m");
    assert!(w.shows(&mk2) && w.shows(&plus));
    assert_eq!(w.page(&mk2), Some(Page::Active));
    w.end("m");
    assert!(
        !w.shows(&mk2) && !w.shows(&plus),
        "it goes on entering ended"
    );
    assert_eq!(w.runtime.navigation_state(&mk2), own);
    assert_eq!(w.runtime.all_overlays().len(), 0);
}

#[test]
fn the_page_follows_the_calls_state() {
    let mut w = World::new();
    let mk2 = w.mk2.clone();
    w.see("c", ObservedPhase::Ringing, Tri::Unknown);
    assert_eq!(w.page(&mk2), Some(Page::Incoming));
    let session = w.driver.frame(&mk2).unwrap();
    w.see("c", ObservedPhase::Connected, Tri::Known(false));
    assert_eq!(w.page(&mk2), Some(Page::Active));
    assert_eq!(
        w.driver.frame(&mk2),
        Some(session),
        "one frame for the call"
    );
}

#[test]
fn meeting_to_meeting_keeps_the_frame() {
    let mut w = World::new();
    let mk2 = w.mk2.clone();
    w.active("one");
    let session = w.driver.frame(&mk2).unwrap();
    w.at(60);
    w.active("two");
    assert_eq!(w.driver.frame(&mk2), Some(session), "no pop and push");
    assert!(w.shows(&mk2));
}

#[test]
fn the_per_device_setting_keeps_a_device_out() {
    let mut w = World::new();
    let (mk2, plus) = (w.mk2.clone(), w.plus.clone());
    w.driver
        .set_device_enabled(&mut w.runtime, &plus, false, Instant::ZERO);
    w.active("m");
    assert!(w.shows(&mk2));
    assert!(!w.shows(&plus));
    w.driver
        .set_device_enabled(&mut w.runtime, &mk2, false, Instant::ZERO);
    assert!(!w.shows(&mk2), "turned off during a call: it goes");
}

#[test]
fn a_device_that_connects_during_a_call_gets_the_overlay() {
    let mut w = World::new();
    let mk2 = w.mk2.clone();
    w.disconnect(&mk2);
    w.active("m");
    assert!(w.driver.frame(&mk2).is_none());
    w.connect(&mk2);
    assert!(w.shows(&mk2));
    // Disconnected and back during the call: the same frame.
    let session = w.driver.frame(&mk2).unwrap();
    w.disconnect(&mk2);
    w.connect(&mk2);
    assert_eq!(w.driver.frame(&mk2), Some(session));
    assert!(w.shows(&mk2));
}

#[test]
fn dismissed_for_the_call_and_shown_again_for_an_incoming_one() {
    let mut w = World::new();
    let (mk2, plus) = (w.mk2.clone(), w.plus.clone());
    w.active("m");
    let dismiss = w.driver.layouts[&mk2.model].dismiss.unwrap();
    let (outcome, _) = w.tap(&mk2, dismiss);
    assert!(matches!(outcome, RuntimeOutcome::OverlayDismissed { .. }));
    assert!(!w.shows(&mk2));
    assert!(w.shows(&plus), "other devices keep it");
    // Further news of the same call does not bring it back.
    w.see("m", ObservedPhase::Connected, Tri::Known(true));
    assert!(!w.shows(&mk2));
    // *Show calls* does.
    assert!(w.driver.show_calls(&mut w.runtime, &mk2, w.now));
    assert!(w.shows(&mk2));
    w.tap(&mk2, dismiss);
    // A new incoming call does too.
    w.see("x", ObservedPhase::Ringing, Tri::Unknown);
    assert_eq!(w.page(&mk2), Some(Page::Incoming));
}

#[test]
fn the_watchdog_pops_frames_it_does_not_know() {
    let mut w = World::new();
    let mk2 = w.mk2.clone();
    let generated = w.driver.layouts[&mk2.model].clone();
    let orphan = w
        .runtime
        .push_overlay(
            &mk2,
            OverlaySpec {
                owner: OverlayOwner::Calls,
                layout: generated.layout,
                actions: generated.actions,
                page: generated.pages[&Page::Active],
                dismiss: None,
            },
        )
        .unwrap();
    assert!(w.shows(&mk2));
    w.at(1);
    assert!(w.runtime.overlays(&mk2).iter().all(|v| v.session != orphan));
    assert_eq!(w.driver.watchdog_pops(), 1);
}

#[test]
fn faces_show_only_what_is_confirmed() {
    let mut w = World::new();
    let mk2 = w.mk2.clone();
    w.see("m", ObservedPhase::Connected, Tri::Known(true));
    let mic = w.control_of(&mk2, "mic-toggle");
    assert_eq!(w.face(&mk2, mic).detail.as_deref(), Some("Muted"));
    // Pressed: pending, still muted until the app says otherwise.
    let (_, handled) = w.tap(&mk2, mic);
    assert!(matches!(handled, Some(KeyOutcome::Send(_))));
    assert_eq!(w.face(&mk2, mic).detail.as_deref(), Some("Waiting…"));
    // No confirmation: "not confirmed", and the state stays.
    w.at(5);
    assert_eq!(w.face(&mk2, mic).detail.as_deref(), Some("Not confirmed"));
    w.at(8);
    assert_eq!(w.face(&mk2, mic).detail.as_deref(), Some("Muted"));
    w.see("m", ObservedPhase::Connected, Tri::Known(false));
    assert_eq!(w.face(&mk2, mic).detail.as_deref(), Some("On"));
}

#[test]
fn a_detection_only_provider_shows_disabled_controls_with_the_reason() {
    let (detection, _) = CapabilitySet::from_reported([
        (Capability::DetectMeeting, Support::Inferred),
        (Capability::MicReadState, Support::Inferred),
    ]);
    let mut w = World::with(detection);
    let mk2 = w.mk2.clone();
    w.active("m");
    for id in ["mic-toggle", "camera-toggle", "end"] {
        let control = w.control_of(&mk2, id);
        let face = w.face(&mk2, control);
        assert_eq!(face.style, FaceStyle::Notice, "{id}");
        assert_eq!(
            face.detail.as_deref(),
            Some("The call app offers no control")
        );
        // Pressed: refused with the reason, nothing sent.
        let (_, handled) = w.tap(&mk2, control);
        assert_eq!(
            handled,
            Some(KeyOutcome::Refused(GateError::Unsupported(
                UnsupportedReason::ProviderHasNoApi
            )))
        );
    }
    let _ = &w.profile;
}

#[test]
fn back_to_work_suspends_and_a_timer_brings_the_call_back() {
    let mut w = World::new();
    let (mk2, plus) = (w.mk2.clone(), w.plus.clone());
    w.active("m");
    let back = w.control_of(&mk2, "return-to-work");
    // The template's default: completion or 60 s.
    let (_, handled) = w.tap(&mk2, back);
    assert_eq!(handled, Some(KeyOutcome::Presentation));
    assert!(!w.shows(&mk2), "the device's own presentation");
    assert!(w.shows(&plus), "other devices keep the call");
    w.at(59);
    assert!(!w.shows(&mk2));
    w.at(60);
    assert!(w.shows(&mk2));
    assert_eq!(w.page(&mk2), Some(Page::Active));
}

#[test]
fn the_call_ending_while_suspended_pops_and_never_resumes() {
    let mut w = World::new();
    let mk2 = w.mk2.clone();
    w.active("m");
    let back = w.control_of(&mk2, "return-to-work");
    w.tap(&mk2, back);
    w.at(10);
    w.end("m");
    assert_eq!(w.runtime.overlays(&mk2).len(), 0);
    assert_eq!(w.driver.returns().scheduled(), 0);
    w.at(120);
    assert!(!w.shows(&mk2), "no stale restore");
}

#[test]
fn settings_are_read_from_the_parameters_with_defaults() {
    use deckpilot_core::ParameterValue;
    let defaults = settings_of(&ActionParameters::new());
    assert_eq!(defaults, ReturnSettings::default());
    let timer = settings_of(
        &ActionParameters::new()
            .with("mode", ParameterValue::Text("timer".into()))
            .with("seconds", ParameterValue::Integer(20)),
    );
    assert_eq!(timer.mode, ReturnMode::timer(20).unwrap());
    // Out of range: the default stays.
    let wild = settings_of(
        &ActionParameters::new()
            .with("mode", ParameterValue::Text("timer".into()))
            .with("seconds", ParameterValue::Integer(100_000)),
    );
    assert_eq!(wild.mode, ReturnMode::timer(30).unwrap());
    let completion = settings_of(
        &ActionParameters::new()
            .with("mode", ParameterValue::Text("completion".into()))
            .with("onCancel", ParameterValue::Text("resume".into()))
            .with("countNavigation", ParameterValue::Boolean(true)),
    );
    assert_eq!(completion.mode, ReturnMode::Completion);
    assert_eq!(completion.on_cancel, AfterRun::Resume);
    assert!(completion.qualify.navigation);
}

#[test]
fn dismissed_while_ringing_stays_gone_through_ticks() {
    let mut w = World::new();
    let mk2 = w.mk2.clone();
    w.see("c", ObservedPhase::Ringing, Tri::Unknown);
    let dismiss = w.driver.layouts[&mk2.model].dismiss.unwrap();
    w.tap(&mk2, dismiss);
    assert!(!w.shows(&mk2));
    w.at(1);
    w.at(5);
    w.see("c", ObservedPhase::Ringing, Tri::Unknown);
    assert!(!w.shows(&mk2), "the same ringing call: still dismissed");
    // Answered, still the same call: still dismissed.
    w.see("c", ObservedPhase::Connected, Tri::Known(false));
    assert!(!w.shows(&mk2));
    // Another incoming call shows it; when it goes, the dismissed call has
    // the focus again and the overlay goes with it.
    w.see("x", ObservedPhase::Ringing, Tri::Unknown);
    assert_eq!(w.page(&mk2), Some(Page::Incoming));
    w.see(
        "x",
        ObservedPhase::Ended {
            cause: ObservedEnd::Missed,
        },
        Tri::Unknown,
    );
    assert!(!w.shows(&mk2));
}

/// The *Back to work* parameters written by `parameters_of` read back as
/// the same settings, for every mode and outcome; out-of-range times are
/// clamped on the way.
#[test]
fn back_to_work_parameters_round_trip() {
    use crate::ret::{AfterRun, Qualify, ReturnMode, ReturnSettings};
    use std::time::Duration;
    let modes = [
        ReturnMode::timer(20).unwrap(),
        ReturnMode::timer(45).unwrap(),
        ReturnMode::Completion,
        ReturnMode::completion_or_timeout(60).unwrap(),
        ReturnMode::completion_or_timeout(600).unwrap(),
    ];
    for mode in modes {
        for (on_failure, on_cancel, all_devices) in [
            (AfterRun::Resume, AfterRun::Stay, false),
            (AfterRun::Stay, AfterRun::Resume, true),
        ] {
            let settings = ReturnSettings {
                mode,
                qualify: Qualify::default(),
                on_failure,
                on_cancel,
                all_devices,
            };
            assert_eq!(settings_of(&parameters_of(&settings)), settings, "{mode:?}");
        }
    }
    let wild = ReturnSettings {
        mode: ReturnMode::Timer {
            after: Duration::from_secs(1),
        },
        ..ReturnSettings::default()
    };
    assert_eq!(
        settings_of(&parameters_of(&wild)).mode,
        ReturnMode::timer(5).unwrap()
    );
}

/// The editor's preview of a call page: its own frame on the page asked
/// for, nothing of a call in its faces, its keys run nothing; a real call
/// takes over, and ending it leaves no frame.
#[test]
fn a_preview_shows_a_page_and_gives_way_to_a_real_call() {
    let mut w = World::new();
    let mk2 = w.mk2.clone();
    w.driver
        .preview(&mut w.runtime, &mk2, Page::Incoming)
        .unwrap();
    assert!(w.shows(&mk2));
    assert_eq!(w.page(&mk2), Some(Page::Incoming));
    // Replaced, never stacked.
    w.driver
        .preview(&mut w.runtime, &mk2, Page::Active)
        .unwrap();
    assert_eq!(w.page(&mk2), Some(Page::Active));
    assert_eq!(
        w.runtime
            .all_overlays()
            .iter()
            .map(|(_, v)| v.len())
            .sum::<usize>(),
        1
    );
    // Its keys run nothing.
    let mic = w.control_of(&mk2, "mic-toggle");
    let (_, handled) = w.tap(&mk2, mic);
    assert!(
        matches!(handled, None | Some(KeyOutcome::Ignored)),
        "{handled:?}"
    );
    // A real call takes over: the preview goes, the call's frame shows.
    w.active("m");
    assert_eq!(w.driver.previews().count(), 0);
    assert!(w.shows(&mk2) && w.shows(&w.plus.clone()));
    assert_eq!(
        w.runtime
            .all_overlays()
            .iter()
            .map(|(_, v)| v.len())
            .sum::<usize>(),
        2,
        "one frame per device: the call's"
    );
    assert_eq!(
        w.driver.preview(&mut w.runtime, &mk2, Page::Active),
        Err(PreviewError::InCall),
        "never during a call"
    );
    w.end("m");
    w.at(10);
    assert_eq!(w.runtime.all_overlays().len(), 0, "no frame left");
    // Ended by hand: no frame left.
    w.driver
        .preview(&mut w.runtime, &mk2, Page::Outgoing)
        .unwrap();
    w.driver.end_preview(&mut w.runtime, &mk2);
    assert_eq!(w.runtime.all_overlays().len(), 0);
    // Calls cleared (the plugin disabled): a preview goes too.
    w.driver
        .preview(&mut w.runtime, &mk2, Page::Active)
        .unwrap();
    w.driver.clear(&mut w.runtime);
    assert_eq!(w.driver.previews().count(), 0);
    assert_eq!(w.runtime.all_overlays().len(), 0);
}

/// A preview the device does not take keeps the one it replaces: the old
/// frame stays, known, on its page (pushed first, popped only after).
#[test]
fn a_refused_preview_keeps_the_previous_one() {
    let mut w = World::new();
    let mk2 = w.mk2.clone();
    w.driver
        .preview(&mut w.runtime, &mk2, Page::Incoming)
        .unwrap();
    w.driver.fail_next_preview = true;
    assert_eq!(
        w.driver.preview(&mut w.runtime, &mk2, Page::Active),
        Err(PreviewError::Refused)
    );
    assert_eq!(w.driver.preview_page(&mk2), Some(Page::Incoming));
    assert_eq!(w.page(&mk2), Some(Page::Incoming));
    assert_eq!(
        w.runtime
            .all_overlays()
            .iter()
            .map(|(_, v)| v.len())
            .sum::<usize>(),
        1
    );
    // Still the driver's: the watchdog leaves it alone.
    w.at(5);
    assert_eq!(w.driver.preview_page(&mk2), Some(Page::Incoming));
    assert!(w.shows(&mk2));
}
