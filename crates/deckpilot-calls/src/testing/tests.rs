//! The owner's test list (`design-calls.md` §11), on the mock provider, the
//! driver and a real core runtime, all on the engine's virtual clock. No
//! real call app is involved, and nothing here claims any Teams control.
//!
//! Full control: answer, reject, cancel, end, microphone and camera,
//! confirmed, rejected, never answered or unknown; detection only refuses.
//!
//! "Wróć do pracy": 20, 30, 60 s and custom; after success, failure,
//! cancellation and a workflow; the timeout; the meeting ending
//! mid-return; an agent restart; disconnect and reconnect; simultaneous
//! calls; several devices; repeated presses; folder and page kept; no
//! duplicate timers; no stale restore; nothing run during the switch.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use deckpilot_core::{
    ActionHandler, ActionInstance, ActionInstanceId, ActionParameters, ActionRegistry,
    ActionResult, ActionType, ActionTypeId, AutoExit, BindingTarget, ControlBinding, DeviceLayout,
    Engine as CoreEngine, ExecutionContext, LayoutPage, MachineConfig, Os, PageId, ParameterValue,
    Profile, Runtime, RuntimeOutcome, Trigger,
};
use deckpilot_device::{
    ControlId, ControlInput, DeviceEvent, DeviceId, DeviceModelId, ModelCatalog, models,
};

use super::*;
use crate::driver::{CallsDriver, KeyOutcome};
use crate::engine::{Engine, Request};
use crate::layouts::{Page, Shape, builtin_templates, generate};
use crate::provider::{ConsentState, ProviderCx};
use crate::ret::{RunEnd, RunKind, RunToken};
use crate::snapshot::{CallState, EndCause};

// ------------------------------------------------------------------ world

/// `test.echo`: succeeds, or fails for the message `fail`; counts its runs.
struct Echo(ActionType, Arc<AtomicUsize>);

impl ActionHandler for Echo {
    fn definition(&self) -> &ActionType {
        &self.0
    }

    fn execute(&self, parameters: &ActionParameters, _: &ExecutionContext<'_>) -> ActionResult {
        self.1.fetch_add(1, Ordering::SeqCst);
        if parameters.text("message") == Some("fail") {
            ActionResult::failure("failed on purpose")
        } else {
            ActionResult::success("ok")
        }
    }
}

fn echo_type() -> ActionTypeId {
    ActionTypeId::new("test.echo").unwrap()
}

#[allow(clippy::default_trait_access)]
fn echo(runs: Arc<AtomicUsize>) -> Echo {
    Echo(
        ActionType {
            id: echo_type(),
            name: "Echo".into(),
            description: String::new(),
            category: "Test".into(),
            parameters: vec![deckpilot_core::ParameterSpec {
                key: "message".into(),
                label: "message".into(),
                description: None,
                kind: deckpilot_core::ParameterKind::Text {
                    multiline: false,
                    max_length: Some(100),
                },
                required: true,
                default: None,
            }],
            requires_project: false,
            node: Default::default(),
            deprecated: false,
            controls: None,
            contexts: None,
            editor_navigation: None,
            title: None,
        },
        runs,
    )
}

fn key(index: u16) -> ControlId {
    ControlId::Key { index }
}

fn instance(message: &str) -> ActionInstance {
    ActionInstance {
        id: ActionInstanceId::new(),
        name: message.into(),
        action_type: echo_type(),
        parameters: ActionParameters::new().with("message", ParameterValue::Text(message.into())),
        description: String::new(),
        icon: None,
        label: None,
    }
}

/// The user's MK.2 layout: key 10 (where the overlay's "Back to work" is)
/// runs an action, key 1 opens the folder Tools (Auto Exit 30 s), key 2
/// runs an action, key 3 opens the page Other, key 4 runs one that fails.
struct Own {
    profile: Profile,
    home: PageId,
    tools: PageId,
}

fn own() -> Own {
    let mut profile = Profile::new("Work");
    let ok = instance("ok");
    let fail = instance("fail");
    let (ok_id, fail_id) = (ok.id, fail.id);
    profile.actions.extend([ok, fail]);
    let mut layout = DeviceLayout::new("Main", DeviceModelId::new(models::STREAM_DECK_MK2_ID));
    let home = layout.home_page;
    let mut tools = LayoutPage::folder("Tools", home);
    tools.auto_exit = Some(AutoExit {
        enabled: true,
        seconds: 30,
    });
    tools.bindings.push(ControlBinding::new(
        key(2),
        Trigger::Press,
        BindingTarget::action(ok_id),
    ));
    let other = LayoutPage::new("Other");
    let (tools_id, other_id) = (tools.id, other.id);
    layout.pages.extend([tools, other]);
    layout.explicit_tree = true;
    layout.page_mut(home).unwrap().bindings.extend([
        ControlBinding::new(key(10), Trigger::Press, BindingTarget::action(ok_id)),
        ControlBinding::new(
            key(1),
            Trigger::Press,
            BindingTarget::OpenPage { page: tools_id },
        ),
        ControlBinding::new(key(2), Trigger::Press, BindingTarget::action(ok_id)),
        ControlBinding::new(
            key(3),
            Trigger::Press,
            BindingTarget::OpenPage { page: other_id },
        ),
        ControlBinding::new(key(4), Trigger::Press, BindingTarget::action(fail_id)),
    ]);
    profile.layouts.push(layout);
    profile.layouts.push(DeviceLayout::new(
        "Main",
        DeviceModelId::new(models::STREAM_DECK_PLUS_ID),
    ));
    Own {
        profile,
        home,
        tools: tools_id,
    }
}

struct H {
    runtime: Runtime,
    driver: CallsDriver,
    mock: MockProvider,
    now: Instant,
    mk2: DeviceId,
    plus: DeviceId,
    own: Own,
    runs: Arc<AtomicUsize>,
    next_run: u64,
}

fn device(model: &str, serial: &str) -> DeviceId {
    DeviceId {
        model: DeviceModelId::new(model),
        serial: serial.into(),
    }
}

fn consent() -> ConsentState {
    ConsentState {
        observe: true,
        control: true,
        titles: false,
    }
}

impl H {
    fn new() -> Self {
        Self::with(MockProvider::new(), &ActionParameters::new())
    }

    /// With the "Back to work" parameters `back`.
    #[allow(clippy::needless_pass_by_value)]
    fn back(back: ActionParameters) -> Self {
        Self::with(MockProvider::new(), &back)
    }

    fn with(mut mock: MockProvider, back: &ActionParameters) -> Self {
        let runs = Arc::new(AtomicUsize::new(0));
        let mut registry = ActionRegistry::default();
        registry.register(Box::new(echo(runs.clone()))).unwrap();
        let catalog = ModelCatalog::builtin();
        let mut runtime = Runtime::new(
            CoreEngine::new(registry),
            catalog.clone(),
            MachineConfig::new(Os::current()),
        );
        let own = own();
        runtime.set_profile(Some(own.profile.clone()), None);
        let mut engine = Engine::default();
        engine.add_provider(mock.id(), consent(), mock.capabilities().clone());
        let mut driver = CallsDriver::new(engine);
        for model in catalog.iter() {
            let shape = Shape::of(&model.capabilities);
            let mut template = builtin_templates()
                .into_iter()
                .find(|t| t.shape == shape)
                .unwrap();
            for (_, controls) in &mut template.pages {
                for control in controls.iter_mut().filter(|c| c.action == "return-to-work") {
                    control.parameters = back.clone();
                }
            }
            driver.set_layout(
                model.id.clone(),
                generate(&template, &model.id, &model.capabilities).unwrap(),
            );
        }
        mock.connect(&ProviderCx {
            consent: consent(),
            session: ProviderSession(1),
        })
        .unwrap();
        let mut h = Self {
            runtime,
            driver,
            mock,
            now: Instant::ZERO,
            mk2: device(models::STREAM_DECK_MK2_ID, "A"),
            plus: device(models::STREAM_DECK_PLUS_ID, "B"),
            own,
            runs,
            next_run: 1,
        };
        let (mk2, plus) = (h.mk2.clone(), h.plus.clone());
        h.connect(&mk2);
        h.connect(&plus);
        h.pump();
        h
    }

    /// Delivers what the mock has due now.
    fn pump(&mut self) {
        let id = self.mock.id();
        for event in self.mock.poll(self.now) {
            self.driver
                .on_provider_event(&mut self.runtime, &id, event, self.now);
        }
    }

    /// Moves the clock to `secs`, at every instant something is due.
    fn at(&mut self, secs: u64) {
        self.at_millis(secs * 1000);
    }

    fn at_millis(&mut self, millis: u64) {
        let target = Instant::from_millis(millis);
        loop {
            let next = [self.driver.next_wake(), self.mock.next_due()]
                .into_iter()
                .flatten()
                .min();
            match next {
                Some(due) if due <= target => {
                    self.now = due.max(self.now);
                    self.pump();
                    self.driver.tick(&mut self.runtime, self.now);
                }
                _ => break,
            }
        }
        self.now = target.max(self.now);
        self.pump();
        self.driver.tick(&mut self.runtime, self.now);
    }

    fn connect(&mut self, device: &DeviceId) {
        self.runtime
            .handle_event(&DeviceEvent::Connected {
                device: device.clone(),
            })
            .unwrap();
        self.driver.device_connected(&mut self.runtime, self.now);
    }

    fn disconnect(&mut self, device: &DeviceId) {
        self.runtime
            .handle_event(&DeviceEvent::Disconnected {
                device: device.clone(),
            })
            .unwrap();
    }

    fn input(
        &mut self,
        device: &DeviceId,
        control: ControlId,
        input: ControlInput,
    ) -> RuntimeOutcome {
        self.runtime
            .handle_event(&DeviceEvent::Input {
                device: device.clone(),
                control,
                input,
            })
            .unwrap()
    }

    /// A press as the agent handles it: overlay keys go to the driver
    /// (commands to the mock), keys of a suspended device's own
    /// presentation are held for its return session.
    fn press(
        &mut self,
        device: &DeviceId,
        control: ControlId,
    ) -> (RuntimeOutcome, Option<KeyOutcome>) {
        let outcome = self.input(device, control, ControlInput::Press);
        let handled = match &outcome {
            RuntimeOutcome::OverlayAction {
                session,
                action_type,
                parameters,
                ..
            } => {
                let handled = self.driver.overlay_key(
                    &mut self.runtime,
                    device,
                    *session,
                    action_type.as_str(),
                    parameters,
                    self.now,
                );
                if let KeyOutcome::Send(Request::Send { command, token, .. }) = &handled
                    && let Err(refusal) = self.mock.command(command.clone(), *token)
                {
                    self.driver.engine_mut().command_refused(*token, &refusal);
                }
                Some(handled)
            }
            RuntimeOutcome::OverlayDismissed { session, .. } => {
                self.driver.dismissed(device, *session);
                None
            }
            RuntimeOutcome::Action { result, .. } => {
                let run = self.run(device, RunKind::Action);
                let end = if result.is_success() {
                    RunEnd::Succeeded
                } else {
                    RunEnd::Failed
                };
                self.finish(device, run, end);
                None
            }
            RuntimeOutcome::Navigated { .. } => {
                let run = self.run(device, RunKind::Navigation);
                self.finish(device, run, RunEnd::Succeeded);
                None
            }
            _ => None,
        };
        (outcome, handled)
    }

    fn tap(
        &mut self,
        device: &DeviceId,
        control: ControlId,
    ) -> (RuntimeOutcome, Option<KeyOutcome>) {
        let pressed = self.press(device, control);
        self.input(device, control, ControlInput::Release);
        pressed
    }

    /// A run of `kind` a key of `device` started (a workflow, a detached
    /// action): it ends with [`H::finish`].
    fn run(&mut self, device: &DeviceId, kind: RunKind) -> RunToken {
        let run = RunToken(self.next_run);
        self.next_run += 1;
        self.driver.run_started(device, run, kind);
        run
    }

    fn finish(&mut self, device: &DeviceId, run: RunToken, end: RunEnd) {
        self.driver
            .run_finished(&mut self.runtime, device, run, end, self.now);
    }

    fn overlay_control(&self, device: &DeviceId, id: &str) -> ControlId {
        let (layout, page) = self.runtime.overlay_page(device).expect("an overlay");
        let generated_action = |action: ActionInstanceId| {
            self.driver_layout(device)
                .action_of(action)
                .map(str::to_owned)
        };
        layout
            .page(page)
            .unwrap()
            .bindings
            .iter()
            .find(|b| generated_action(b.target.action_id().unwrap()).as_deref() == Some(id))
            .unwrap()
            .control
    }

    #[allow(clippy::unused_self)]
    fn driver_layout(&self, device: &DeviceId) -> crate::layouts::Generated {
        let catalog = ModelCatalog::builtin();
        let model = catalog.get(&device.model).unwrap();
        let shape = Shape::of(&model.capabilities);
        let template = builtin_templates()
            .into_iter()
            .find(|t| t.shape == shape)
            .unwrap();
        generate(&template, &model.id, &model.capabilities).unwrap()
    }

    fn page(&self, device: &DeviceId) -> Option<Page> {
        let (_, page) = self.runtime.overlay_page(device)?;
        self.driver_layout(device)
            .pages
            .into_iter()
            .find(|(_, id)| *id == page)
            .map(|(p, _)| p)
    }

    fn shows(&self, device: &DeviceId) -> bool {
        self.runtime.shows_overlay(device)
    }

    fn state(&self) -> CallState {
        self.driver.engine().snapshot().state
    }

    /// "Back to work" on `device`.
    fn back_to_work(&mut self, device: &DeviceId) {
        let control = self.overlay_control(device, "return-to-work");
        let (_, handled) = self.tap(device, control);
        assert_eq!(handled, Some(KeyOutcome::Presentation));
        assert!(!self.shows(device), "the device's own presentation");
    }
}

fn timer(seconds: i64) -> ActionParameters {
    ActionParameters::new()
        .with("mode", ParameterValue::Choice("timer".into()))
        .with("seconds", ParameterValue::Integer(seconds))
}

fn completion() -> ActionParameters {
    ActionParameters::new().with("mode", ParameterValue::Choice("completion".into()))
}

// ----------------------------------------------------------- full control

#[test]
fn an_incoming_call_is_answered_through_the_mock() {
    let mut h = H::new();
    let mk2 = h.mk2.clone();
    h.mock.ring(Instant::from_secs(1), "c");
    h.at(1);
    assert_eq!(h.state(), CallState::Incoming);
    assert_eq!(h.page(&mk2), Some(Page::Incoming));
    let answer = h.overlay_control(&mk2, "answer");
    h.tap(&mk2, answer);
    assert_eq!(
        h.state(),
        CallState::Incoming,
        "not before the app confirms"
    );
    h.at_millis(1200);
    assert_eq!(h.state(), CallState::Active);
    assert_eq!(h.page(&mk2), Some(Page::Active));
    assert_eq!(h.mock.sent().len(), 1);
}

/// Whether a track of the engine ended with `cause`.
fn ended_with(h: &H, cause: EndCause) -> bool {
    h.driver
        .engine()
        .tracks()
        .into_iter()
        .any(|(_, state)| state == CallState::Ended { cause })
}

#[test]
fn reject_cancel_and_end_are_confirmed_by_the_mock() {
    let mut h = H::new();
    let mk2 = h.mk2.clone();
    h.mock.ring(Instant::from_secs(1), "in");
    h.at(1);
    let reject = h.overlay_control(&mk2, "reject");
    h.tap(&mk2, reject);
    h.at(2);
    assert!(!h.shows(&mk2), "rejected: the overlay goes");
    assert!(ended_with(&h, EndCause::Rejected));
    h.at(5);
    h.mock.dial(Instant::from_secs(5), "out");
    h.at(5);
    assert_eq!(h.page(&mk2), Some(Page::Outgoing));
    let cancel = h.overlay_control(&mk2, "cancel-outgoing");
    h.tap(&mk2, cancel);
    h.at(6);
    assert!(!h.shows(&mk2));
    assert!(ended_with(&h, EndCause::Cancelled));
    h.at(10);
    h.mock.join(Instant::from_secs(10), "m");
    h.at(10);
    let end = h.overlay_control(&mk2, "end");
    h.tap(&mk2, end);
    h.at(11);
    assert!(!h.shows(&mk2));
    assert!(ended_with(&h, EndCause::Left));
    let causes: Vec<_> = h.mock.sent().iter().map(|c| c.action).collect();
    assert_eq!(
        causes,
        [
            CallAction::Reject,
            CallAction::CancelOutgoing,
            CallAction::End
        ]
    );
}

#[test]
fn mute_and_camera_show_the_confirmed_state() {
    let mut h = H::new();
    let mk2 = h.mk2.clone();
    h.mock.join(Instant::from_secs(1), "m");
    h.at(1);
    let mic = h.overlay_control(&mk2, "mic-toggle");
    let camera = h.overlay_control(&mk2, "camera-toggle");
    h.tap(&mk2, mic);
    assert_eq!(h.driver.engine().snapshot().mic_muted, Tri::Known(false));
    h.at(2);
    assert_eq!(h.driver.engine().snapshot().mic_muted, Tri::Known(true));
    h.tap(&mk2, camera);
    h.at(3);
    assert_eq!(h.driver.engine().snapshot().camera_on, Tri::Known(true));
}

#[test]
fn rejected_never_answered_and_unknown_commands_change_nothing() {
    let mut h = H::new();
    let mk2 = h.mk2.clone();
    h.mock.join(Instant::from_secs(1), "m");
    h.at(1);
    let mic = h.overlay_control(&mk2, "mic-toggle");
    for response in [
        Response::Reject("host-disallows".into()),
        Response::Never,
        Response::Unknown,
    ] {
        h.mock.respond(CallAction::MicToggle, response.clone());
        let before = h.mock.sent().len();
        h.tap(&mk2, mic);
        assert_eq!(h.mock.sent().len(), before + 1, "{response:?}: sent");
        let later = h.now.as_millis() / 1000 + 8;
        h.at(later);
        assert_eq!(
            h.driver.engine().snapshot().mic_muted,
            Tri::Known(false),
            "{response:?}"
        );
        assert_eq!(h.driver.engine().snapshot().pending, None);
    }
}

#[test]
fn a_detection_only_provider_sends_nothing() {
    let mut h = H::with(MockProvider::detection_only(), &ActionParameters::new());
    let mk2 = h.mk2.clone();
    h.mock.join(Instant::from_secs(1), "m");
    h.at(1);
    for id in ["mic-toggle", "camera-toggle", "end"] {
        let control = h.overlay_control(&mk2, id);
        let (_, handled) = h.tap(&mk2, control);
        assert!(matches!(handled, Some(KeyOutcome::Refused(_))), "{id}");
    }
    assert_eq!(h.mock.sent().len(), 0);
}

#[test]
fn the_call_app_restarting_keeps_the_overlay_and_reconnects() {
    let mut h = H::new();
    let mk2 = h.mk2.clone();
    h.mock.join(Instant::from_secs(1), "m");
    h.mock
        .restart_app(Instant::from_secs(10), Instant::from_secs(14));
    h.mock.join(Instant::from_secs(14), "m");
    h.at(11);
    assert_eq!(h.state(), CallState::Unknown);
    assert!(h.shows(&mk2), "the mute key does not vanish mid-sentence");
    h.at(15);
    assert_eq!(h.state(), CallState::Active);
    // Commands work in the new session.
    let mic = h.overlay_control(&mk2, "mic-toggle");
    h.tap(&mk2, mic);
    h.at(16);
    assert_eq!(h.driver.engine().snapshot().mic_muted, Tri::Known(true));
}

#[test]
fn a_lost_provider_ends_the_call_as_lost() {
    let mut h = H::new();
    let mk2 = h.mk2.clone();
    h.mock.join(Instant::from_secs(1), "m");
    h.mock.script(
        Instant::from_secs(5),
        ProviderEvent::Disconnected {
            reason: DisconnectReason::Network,
        },
    );
    h.at(19);
    assert!(h.shows(&mk2));
    h.at(20);
    assert!(!h.shows(&mk2));
    assert_eq!(
        h.state(),
        CallState::Ended {
            cause: EndCause::Lost
        }
    );
}

// ------------------------------------------------------- Back to work: A

#[test]
fn back_to_work_timers_of_20_30_60_seconds_and_custom() {
    for seconds in [20, 30, 60, 45] {
        let mut h = H::back(timer(seconds));
        let mk2 = h.mk2.clone();
        h.mock.join(Instant::from_secs(1), "m");
        h.at(1);
        h.back_to_work(&mk2);
        let back = 1 + u64::try_from(seconds).unwrap();
        h.at(back - 1);
        assert!(!h.shows(&mk2), "{seconds}: not early");
        h.at(back);
        assert!(h.shows(&mk2), "{seconds}: back on time");
        assert_eq!(h.page(&mk2), Some(Page::Active));
        assert_eq!(h.driver.returns().scheduled(), 0);
    }
}

// ------------------------------------------------------- Back to work: B

#[test]
fn back_to_work_after_a_successful_action() {
    let mut h = H::back(completion());
    let mk2 = h.mk2.clone();
    h.mock.join(Instant::from_secs(1), "m");
    h.at(1);
    h.back_to_work(&mk2);
    h.at(5);
    let (outcome, _) = h.tap(&mk2, key(2));
    assert!(matches!(outcome, RuntimeOutcome::Action { .. }));
    assert!(h.shows(&mk2), "the action ran to its end: back to the call");
}

#[test]
fn back_to_work_after_a_failure_resumes_or_stays_as_set() {
    let mut h = H::back(completion());
    let mk2 = h.mk2.clone();
    h.mock.join(Instant::from_secs(1), "m");
    h.at(1);
    h.back_to_work(&mk2);
    h.tap(&mk2, key(4));
    assert!(h.shows(&mk2), "on failure: resume (default)");

    let mut stay = completion();
    stay = stay.with("onFailure", ParameterValue::Choice("stay".into()));
    let mut h = H::back(stay);
    let mk2 = h.mk2.clone();
    h.mock.join(Instant::from_secs(1), "m");
    h.at(1);
    h.back_to_work(&mk2);
    h.tap(&mk2, key(4));
    assert!(!h.shows(&mk2), "on failure: stay");
    h.tap(&mk2, key(2));
    assert!(h.shows(&mk2), "the next success brings it back");
}

#[test]
fn back_to_work_after_a_cancellation_stays_or_resumes_as_set() {
    let mut h = H::back(completion());
    let mk2 = h.mk2.clone();
    h.mock.join(Instant::from_secs(1), "m");
    h.at(1);
    h.back_to_work(&mk2);
    let run = h.run(&mk2, RunKind::DetachedAction);
    h.at(3);
    h.finish(&mk2, run, RunEnd::Cancelled);
    assert!(!h.shows(&mk2), "cancelled: stay (default)");

    let resume = completion().with("onCancel", ParameterValue::Choice("resume".into()));
    let mut h = H::back(resume);
    let mk2 = h.mk2.clone();
    h.mock.join(Instant::from_secs(1), "m");
    h.at(1);
    h.back_to_work(&mk2);
    let run = h.run(&mk2, RunKind::DetachedAction);
    h.finish(&mk2, run, RunEnd::Cancelled);
    assert!(h.shows(&mk2), "cancelled: resume as set");
}

#[test]
fn back_to_work_after_a_workflow_waits_for_its_real_end() {
    let mut h = H::back(completion());
    let mk2 = h.mk2.clone();
    h.mock.join(Instant::from_secs(1), "m");
    h.at(1);
    h.back_to_work(&mk2);
    let run = h.run(&mk2, RunKind::Workflow);
    h.at(40);
    assert!(!h.shows(&mk2), "the press is not the end");
    h.finish(&mk2, run, RunEnd::Succeeded);
    assert!(h.shows(&mk2));
}

#[test]
fn navigation_only_keys_do_not_bring_the_call_back_unless_set() {
    let mut h = H::back(completion());
    let mk2 = h.mk2.clone();
    h.mock.join(Instant::from_secs(1), "m");
    h.at(1);
    h.back_to_work(&mk2);
    h.tap(&mk2, key(3));
    assert!(!h.shows(&mk2), "opening a page is not a completion");

    let navigation = completion().with("countNavigation", ParameterValue::Boolean(true));
    let mut h = H::back(navigation);
    let mk2 = h.mk2.clone();
    h.mock.join(Instant::from_secs(1), "m");
    h.at(1);
    h.back_to_work(&mk2);
    h.tap(&mk2, key(3));
    assert!(h.shows(&mk2), "counted as set");
}

// ------------------------------------------------------- Back to work: C

#[test]
fn the_default_comes_back_at_the_end_or_after_60_seconds() {
    let mut h = H::new();
    let mk2 = h.mk2.clone();
    h.mock.join(Instant::from_secs(1), "m");
    h.at(1);
    h.back_to_work(&mk2);
    let slow = h.run(&mk2, RunKind::Workflow);
    h.at(60);
    assert!(!h.shows(&mk2));
    h.at(61);
    assert!(h.shows(&mk2), "the maximum before the slow run");
    // Its late end changes nothing.
    h.at(90);
    h.finish(&mk2, slow, RunEnd::Succeeded);
    assert!(h.shows(&mk2));
    assert_eq!(h.driver.returns().scheduled(), 0);

    // A run that ends first wins.
    let mut h = H::new();
    let mk2 = h.mk2.clone();
    h.mock.join(Instant::from_secs(1), "m");
    h.at(1);
    h.back_to_work(&mk2);
    h.at(10);
    h.tap(&mk2, key(2));
    assert!(h.shows(&mk2));
    assert_eq!(h.driver.returns().scheduled(), 0, "the timer went with it");
}

// ------------------------------------------------ Back to work: lifecycle

#[test]
fn the_meeting_ending_mid_return_pops_and_never_resumes() {
    let mut h = H::back(timer(30));
    let mk2 = h.mk2.clone();
    h.mock.join(Instant::from_secs(1), "m");
    h.mock.hang_up(Instant::from_secs(10), "m");
    h.at(1);
    h.back_to_work(&mk2);
    let run = h.run(&mk2, RunKind::Workflow);
    h.at(10);
    assert!(h.runtime.overlays(&mk2).is_empty(), "popped");
    assert_eq!(h.driver.returns().scheduled(), 0, "the timer is cancelled");
    h.finish(&mk2, run, RunEnd::Succeeded);
    h.at(60);
    assert!(!h.shows(&mk2), "never back to an ended call");
}

#[test]
fn an_agent_restart_shows_the_call_again_without_a_stale_return() {
    let mut h = H::back(timer(30));
    let mk2 = h.mk2.clone();
    h.mock.join(Instant::from_secs(1), "m");
    h.at(1);
    h.back_to_work(&mk2);
    // The agent restarts: a new runtime and driver; the call goes on and
    // the provider reports it again.
    let mut restarted = H::back(timer(30));
    restarted.mock.join(Instant::from_secs(1), "m");
    restarted.at(1);
    assert!(restarted.shows(&mk2), "the call, not a suspended frame");
    assert_eq!(restarted.driver.returns().scheduled(), 0);
    restarted.at(31);
    assert!(restarted.shows(&mk2));
}

#[test]
fn a_device_disconnected_while_suspended() {
    let mut h = H::back(timer(20));
    let mk2 = h.mk2.clone();
    h.mock.join(Instant::from_secs(1), "m");
    h.at(1);
    h.back_to_work(&mk2);
    h.disconnect(&mk2);
    // Back before the deadline: still its own presentation.
    h.at(10);
    h.connect(&mk2);
    assert!(!h.shows(&mk2));
    // Away past the deadline: it comes back to the call.
    h.disconnect(&mk2);
    h.at(30);
    h.connect(&mk2);
    assert!(h.shows(&mk2));
}

#[test]
fn an_incoming_call_preempts_back_to_work() {
    let mut h = H::back(timer(60));
    let mk2 = h.mk2.clone();
    h.mock.join(Instant::from_secs(1), "m");
    h.mock.ring(Instant::from_secs(10), "second");
    h.at(1);
    h.back_to_work(&mk2);
    let run = h.run(&mk2, RunKind::Workflow);
    h.at(10);
    assert_eq!(h.page(&mk2), Some(Page::Incoming));
    assert_eq!(h.driver.returns().scheduled(), 0);
    // The run keeps going; its end is ignored.
    h.finish(&mk2, run, RunEnd::Succeeded);
    assert_eq!(h.page(&mk2), Some(Page::Incoming));
}

#[test]
fn other_devices_are_unaffected_unless_all_devices_is_set() {
    let mut h = H::back(timer(30));
    let (mk2, plus) = (h.mk2.clone(), h.plus.clone());
    h.mock.join(Instant::from_secs(1), "m");
    h.at(1);
    h.back_to_work(&mk2);
    assert!(h.shows(&plus));

    let all = timer(30).with("returnAllDevices", ParameterValue::Boolean(true));
    let mut h = H::back(all);
    let (mk2, plus) = (h.mk2.clone(), h.plus.clone());
    h.mock.join(Instant::from_secs(1), "m");
    h.at(1);
    h.back_to_work(&mk2);
    assert!(!h.shows(&plus));
    assert_eq!(h.driver.returns().scheduled(), 2, "one session each");
    h.at(31);
    assert!(h.shows(&mk2) && h.shows(&plus));
}

#[test]
fn repeated_presses_have_one_session_and_no_duplicate_timer() {
    let mut h = H::back(timer(20));
    let mk2 = h.mk2.clone();
    h.mock.join(Instant::from_secs(1), "m");
    h.at(1);
    let mut ids = Vec::new();
    for round in 0..5 {
        h.back_to_work(&mk2);
        ids.push(h.driver.returns().current(&mk2).unwrap());
        assert_eq!(h.driver.returns().scheduled(), 1, "round {round}");
        // *Show calls* brings it back at once and ends the session.
        assert!(h.driver.show_calls(&mut h.runtime, &mk2, h.now));
        assert!(h.shows(&mk2));
        assert_eq!(h.driver.returns().scheduled(), 0);
        h.at(2 + round);
    }
    ids.dedup();
    assert_eq!(ids.len(), 5, "a new ReturnId each time");
}

#[test]
fn no_stale_restore_after_the_session_ended() {
    let mut h = H::back(timer(20));
    let mk2 = h.mk2.clone();
    h.mock.join(Instant::from_secs(1), "m");
    h.at(1);
    h.back_to_work(&mk2);
    h.at(5);
    h.driver.show_calls(&mut h.runtime, &mk2, h.now);
    // A second "Back to work" at 10: its own 20 s, not the first's.
    h.at(10);
    h.back_to_work(&mk2);
    h.at(21);
    assert!(!h.shows(&mk2), "the first deadline (21 s) restores nothing");
    h.at(30);
    assert!(h.shows(&mk2));
}

#[test]
fn the_folder_and_page_are_kept_through_the_call() {
    let mut h = H::back(timer(20));
    let mk2 = h.mk2.clone();
    h.tap(&mk2, key(1));
    assert_eq!(h.runtime.current_page(&mk2), Some(h.own.tools));
    h.mock.join(Instant::from_secs(1), "m");
    h.at(1);
    h.back_to_work(&mk2);
    assert_eq!(
        h.runtime.current_page(&mk2),
        Some(h.own.tools),
        "the folder, as left"
    );
    h.at(21);
    assert!(h.shows(&mk2));
    h.mock.hang_up(Instant::from_secs(100), "m");
    h.at(100);
    assert!(!h.shows(&mk2));
    assert_eq!(
        h.runtime.current_page(&mk2),
        Some(h.own.tools),
        "after the call"
    );
    let _ = h.own.home;
}

#[test]
fn nothing_runs_during_the_switch() {
    let mut h = H::back(timer(20));
    let mk2 = h.mk2.clone();
    h.mock.join(Instant::from_secs(1), "m");
    h.at(1);
    let back = h.overlay_control(&mk2, "return-to-work");
    assert_eq!(back, key(10), "over the user's own key 10");
    let before = h.runs.load(Ordering::SeqCst);
    // Pressed on the overlay; the device switches while it is held.
    h.press(&mk2, back);
    let release = h.input(&mk2, back, ControlInput::Release);
    assert!(matches!(release, RuntimeOutcome::Ignored { .. }));
    assert_eq!(
        h.runs.load(Ordering::SeqCst),
        before,
        "key 10 underneath did not run"
    );
    // A real press afterwards does.
    h.tap(&mk2, key(10));
    assert_eq!(h.runs.load(Ordering::SeqCst), before + 1);
}
