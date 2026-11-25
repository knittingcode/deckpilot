//! Calls against a real agent with a simulated MK.2 and the mock call
//! provider (installed through the test hooks: no build ships a provider
//! yet). The clock is the folder clock the test moves; nothing waits for a
//! timeout. No real call app is involved, and a Stream Deck plugged into
//! the machine is never opened.
//!
//! - The overlay is drawn on the device (T-E2E) and goes with the call.
//! - "Wróć do pracy" through the agent's real runs: a workflow's
//!   `Execution::Finished` brings the call back (T-R3), a run slower than
//!   the 60 s maximum does not hold it (T-R4), navigation does not count
//!   (T-R9), an action run in place does (T-R2), the meeting ending
//!   mid-return pops it (T-R5), and a device unplugged and plugged in
//!   again shows what the session says then (T-R6).
//! - Nothing runs during the switch (T-R10): the release of "Wróć do pracy"
//!   over the user's own key runs nothing.

mod support;

use std::time::Duration;

use deckpilot_calls::layouts::{Shape, builtin_templates, generate};
use deckpilot_calls::snapshot::CallState;
use deckpilot_calls::testing::MockProvider;
use deckpilot_core::{
    ActionParameters, BindingTarget, ControlBinding, DeviceLayout, ExecutionState, FaceStyle,
    KeyFace, LayoutPage, OnFailure, ParameterValue, Profile, Trigger, Workflow, WorkflowId,
    WorkflowStep,
};
use deckpilot_device::mock::MockBackend;
use deckpilot_device::{ControlId, DeviceId, models};
use deckpilot_modules::general;
use support::{FakeInstant, TIMEOUT, TempDir, TestAgent};

fn key(index: u16) -> ControlId {
    ControlId::Key { index }
}

fn press(index: u16, target: BindingTarget) -> ControlBinding {
    ControlBinding::new(key(index), Trigger::Press, target)
}

struct Fixture {
    profile: Profile,
    /// A Multi Action of one log step.
    quick: WorkflowId,
    long: WorkflowId,
    /// A graph workflow that waits a minute (cancellable).
    wait: WorkflowId,
}

/// Home: key 1 opens Other (navigation only), key 2 a quick workflow, key
/// 3 one with a minute's delay, key 10 (under the overlay's "Wróć do
/// pracy") a log action run in place.
fn fixture() -> Fixture {
    let registry = support::registry();
    let log = |name: &str| {
        registry
            .create_instance(
                name,
                general::log_id(),
                &ActionParameters::new().with("message", ParameterValue::Text(name.into())),
            )
            .unwrap()
    };
    let first = log("first");
    let under = log("under");
    // Fails: no project is active.
    let broken = log("{project.key}");
    let delay = registry
        .create_instance(
            "Delay",
            general::delay_id(),
            &ActionParameters::new().with("milliseconds", ParameterValue::Integer(500)),
        )
        .unwrap();
    let quick = Workflow::new("Quick").step(first.id, OnFailure::Stop);
    let mut wait = WorkflowStep::new(delay.id, OnFailure::Stop);
    wait.overrides = ActionParameters::new().with("milliseconds", ParameterValue::Integer(60_000));
    let mut slow = Workflow::new("Long").step(first.id, OnFailure::Stop);
    slow.steps.push(wait);
    let mut profile = Profile::new("Work");
    let (quick_id, long_id) = (quick.id, slow.id);
    let under_id = under.id;
    let broken_id = broken.id;
    profile.actions.extend([first, under, broken, delay]);
    let wait = support::waiting_workflow("Wait", 60_000);
    let wait_id = wait.id;
    profile.workflows.extend([quick, slow, wait]);
    let mut layout = DeviceLayout::new("Main", models::stream_deck_mk2().id);
    let home = layout.home_page;
    let other = LayoutPage::new("Other");
    let other_id = other.id;
    layout.pages.push(other);
    layout.page_mut(home).unwrap().bindings.extend([
        press(1, BindingTarget::OpenPage { page: other_id }),
        press(2, BindingTarget::Workflow { workflow: quick_id }),
        press(3, BindingTarget::Workflow { workflow: long_id }),
        press(10, BindingTarget::action(under_id)),
        press(5, BindingTarget::action(broken_id)),
        press(6, BindingTarget::Workflow { workflow: wait_id }),
        // A long press runs the quick workflow (a gesture key).
        ControlBinding::new(
            key(4),
            Trigger::LongPress,
            BindingTarget::Workflow { workflow: quick_id },
        ),
    ]);
    profile.layouts.push(layout);
    Fixture {
        profile,
        quick: quick_id,
        long: long_id,
        wait: wait_id,
    }
}

struct Deck {
    agent: TestAgent,
    mock: MockBackend,
    device: DeviceId,
    clock: FakeInstant,
    _dir: TempDir,
}

async fn start(f: &Fixture) -> Deck {
    start_with(f, ActionParameters::new()).await
}

/// With the "Back to work" parameters `back`.
async fn start_with(f: &Fixture, back: ActionParameters) -> Deck {
    let dir = TempDir::new("deckpilot-calls");
    let (paths, db) = support::data_dir(&dir, &f.profile, &[]);
    let mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), "CALLS-1");
    let clock = FakeInstant::default();
    let agent = TestAgent::start_with_folder_clock(&paths, &db, &mock, &clock);
    agent.unlock().await;
    let deck = Deck {
        agent,
        mock,
        device,
        clock,
        _dir: dir,
    };
    deck.agent
        .handle
        .install_calls_mock(MockProvider::new(), back)
        .await;
    deck
}

impl Deck {
    /// Joins a meeting now and waits until the device shows the overlay.
    async fn join(&self) {
        self.agent
            .handle
            .calls_mock(|mock, now| mock.join(now, "meeting"))
            .await;
        self.wait_overlay(true).await;
    }

    async fn hang_up(&self) {
        self.agent
            .handle
            .calls_mock(|mock, now| mock.hang_up(now, "meeting"))
            .await;
    }

    async fn shows_overlay(&self) -> bool {
        let view = self.agent.handle.calls_view().await.unwrap();
        view.devices
            .iter()
            .any(|(device, shown, _)| *device == self.device && *shown)
    }

    async fn wait_overlay(&self, shown: bool) {
        let deadline = tokio::time::Instant::now() + TIMEOUT;
        loop {
            if self.shows_overlay().await == shown {
                return;
            }
            assert!(
                tokio::time::Instant::now() <= deadline,
                "the overlay is shown: {}",
                !shown
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Moves the clock; a request lets the loop look at Calls.
    async fn advance(&self, seconds: u64) {
        self.clock.advance(Duration::from_secs(seconds));
        self.agent.status().await;
        self.agent.status().await;
    }

    fn tap(&self, index: u16) {
        self.mock.tap(&self.device, key(index));
    }

    /// The overlay's "Wróć do pracy" key on an MK.2.
    fn back_to_work_key() -> u16 {
        let mk2 = models::stream_deck_mk2();
        let shape = Shape::of(&mk2.capabilities);
        let template = builtin_templates()
            .into_iter()
            .find(|t| t.shape == shape)
            .unwrap();
        let generated = generate(&template, &mk2.id, &mk2.capabilities).unwrap();
        let (_, page) = generated
            .pages
            .iter()
            .find(|(p, _)| **p == deckpilot_calls::layouts::Page::Active)
            .unwrap();
        let binding = generated
            .layout
            .page(*page)
            .unwrap()
            .bindings
            .iter()
            .find(|b| generated.action_of(b.target.action_id().unwrap()) == Some("return-to-work"))
            .unwrap();
        match binding.control {
            ControlId::Key { index } => index,
            other => panic!("{other}"),
        }
    }

    async fn back_to_work(&self) {
        self.tap(Self::back_to_work_key());
        self.wait_overlay(false).await;
    }

    async fn wait_run(&self, workflow: WorkflowId, state: ExecutionState) {
        self.agent
            .wait_for_runs(workflow, |runs| {
                runs.first().is_some_and(|r| r.state == state)
            })
            .await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_overlay_is_drawn_on_the_device_and_goes_with_the_call() {
    let f = fixture();
    let deck = start(&f).await;
    assert!(
        !deck.shows_overlay().await,
        "no call: the device's own layout"
    );
    deck.join().await;
    let view = deck.agent.handle.calls_view().await.unwrap();
    assert_eq!(view.state, CallState::Active);
    assert!(
        view.devices
            .iter()
            .any(|(_, _, page)| page.as_deref() == Some("In a call"))
    );
    // T-E2E: the device draws the overlay's face on its key.
    let spec = models::stream_deck_mk2()
        .capabilities
        .keys
        .and_then(|keys| keys.image)
        .unwrap();
    let face = KeyFace::new(FaceStyle::Action, "Back to work");
    let image = deckpilot_agent::render::render(&face, spec.width, spec.height);
    let back = key(Deck::back_to_work_key());
    assert!(
        deck.mock
            .wait_for(&deck.device, TIMEOUT, |frames, _| frames.get(&back)
                == Some(&image)),
        "the device shows the overlay's Back to work key"
    );
    deck.hang_up().await;
    deck.wait_overlay(false).await;
    deck.agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_workflows_real_end_brings_the_call_back() {
    let f = fixture();
    let deck = start(&f).await;
    deck.join().await;
    deck.back_to_work().await;
    deck.tap(2);
    deck.wait_run(f.quick, ExecutionState::Succeeded).await;
    deck.wait_overlay(true).await;
    deck.agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_slow_run_does_not_hold_the_call_past_sixty_seconds() {
    let f = fixture();
    let deck = start(&f).await;
    deck.join().await;
    deck.back_to_work().await;
    deck.tap(3);
    deck.agent
        .wait_for_runs(f.long, |runs| !runs.is_empty())
        .await;
    deck.advance(30).await;
    assert!(!deck.shows_overlay().await);
    deck.advance(31).await;
    deck.wait_overlay(true).await;
    let view = deck.agent.handle.calls_view().await.unwrap();
    assert_eq!(view.scheduled_returns, 0, "no timer left");
    deck.agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_does_not_count_and_an_action_run_in_place_does() {
    let f = fixture();
    let deck = start(&f).await;
    deck.join().await;
    deck.back_to_work().await;
    deck.tap(1);
    deck.agent.status().await;
    assert!(
        !deck.shows_overlay().await,
        "opening a page is not a completion"
    );
    // (Other has nothing bound; the next case starts on Home again.)
    deck.agent.shutdown().await;

    let deck = start(&f).await;
    deck.join().await;
    deck.back_to_work().await;
    deck.tap(10);
    deck.wait_overlay(true).await;
    deck.agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_meeting_ending_mid_return_pops_the_overlay_for_good() {
    let f = fixture();
    let deck = start(&f).await;
    deck.join().await;
    deck.back_to_work().await;
    deck.hang_up().await;
    deck.advance(1).await;
    let view = deck.agent.handle.calls_view().await.unwrap();
    assert!(view.devices.is_empty(), "popped: {view:?}");
    assert_eq!(view.scheduled_returns, 0);
    deck.advance(120).await;
    assert!(!deck.shows_overlay().await, "never back to an ended call");
    deck.agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_device_plugged_in_again_shows_what_its_session_says() {
    let f = fixture();
    let deck = start(&f).await;
    deck.join().await;
    deck.back_to_work().await;
    deck.mock.unplug(&deck.device);
    deck.agent.status().await;
    // Away past the 60 s maximum: back on the call when it returns.
    deck.advance(61).await;
    let device = deck.mock.plug(&models::stream_deck_mk2(), "CALLS-1");
    assert_eq!(device, deck.device);
    deck.agent.status().await;
    deck.wait_overlay(true).await;
    deck.agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_release_of_back_to_work_runs_nothing_underneath() {
    let f = fixture();
    let deck = start(&f).await;
    deck.join().await;
    // "Wróć do pracy" is over the user's key 10, which logs "under".
    assert_eq!(Deck::back_to_work_key(), 10);
    deck.back_to_work().await;
    deck.agent.status().await;
    let view = deck.agent.handle.calls_view().await.unwrap();
    assert_eq!(view.state, CallState::Active);
    assert!(!deck.shows_overlay().await);
    // The suspension's only run would be key 10's; none happened, so the
    // session still waits (mode C, 60 s).
    assert_eq!(view.scheduled_returns, 1);
    deck.agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_incoming_call_takes_a_device_that_went_back_to_work() {
    let f = fixture();
    let deck = start(&f).await;
    deck.join().await;
    deck.back_to_work().await;
    deck.agent
        .handle
        .calls_mock(|mock, now| mock.ring(now, "second"))
        .await;
    deck.wait_overlay(true).await;
    let view = deck.agent.handle.calls_view().await.unwrap();
    assert_eq!(view.state, CallState::Incoming);
    assert!(
        view.devices
            .iter()
            .any(|(_, _, page)| page.as_deref() == Some("Incoming call"))
    );
    assert_eq!(view.scheduled_returns, 0, "the return session ended");
    deck.agent.shutdown().await;
}

fn choice(key: &str, value: &str) -> ActionParameters {
    ActionParameters::new().with(key, ParameterValue::Choice(value.into()))
}

/// T-R2: a failed action resumes by default and stays with
/// `onFailure: stay`.
#[tokio::test(flavor = "multi_thread")]
async fn a_failed_action_resumes_or_stays_as_set() {
    let f = fixture();
    for (back, resumes) in [
        (ActionParameters::new(), true),
        (choice("onFailure", "stay"), false),
    ] {
        let deck = start_with(&f, back).await;
        deck.join().await;
        deck.back_to_work().await;
        deck.tap(5);
        deck.agent.status().await;
        deck.agent.status().await;
        assert_eq!(deck.shows_overlay().await, resumes, "resumes: {resumes}");
        deck.agent.shutdown().await;
    }
}

/// T-R2: a cancelled workflow stays by default and resumes with
/// `onCancel: resume`.
#[tokio::test(flavor = "multi_thread")]
async fn a_cancelled_workflow_stays_or_resumes_as_set() {
    let f = fixture();
    for (back, resumes) in [
        (ActionParameters::new(), false),
        (choice("onCancel", "resume"), true),
    ] {
        let deck = start_with(&f, back).await;
        deck.join().await;
        deck.back_to_work().await;
        deck.tap(6);
        let run = deck
            .agent
            .wait_for_runs(f.wait, |runs| !runs.is_empty())
            .await[0]
            .id;
        deck.agent
            .request(deckpilot_ipc::Request::CancelExecution { execution: run })
            .await
            .unwrap();
        deck.wait_run(f.wait, ExecutionState::Cancelled).await;
        deck.agent.status().await;
        assert_eq!(deck.shows_overlay().await, resumes, "resumes: {resumes}");
        deck.agent.shutdown().await;
    }
}

/// T-R3 (Multi Action): the quick workflow is a Multi Action; in mode B the
/// call comes back when its last step is over.
#[tokio::test(flavor = "multi_thread")]
async fn a_multi_action_brings_the_call_back_when_its_last_step_is_over() {
    let f = fixture();
    let deck = start_with(&f, choice("mode", "completion")).await;
    deck.join().await;
    deck.back_to_work().await;
    deck.tap(2);
    deck.wait_run(f.quick, ExecutionState::Succeeded).await;
    deck.wait_overlay(true).await;
    deck.agent.shutdown().await;
}

/// T-R10 for gesture keys: a long press that started before the overlay
/// came and went does not run when it fires; without a switch it does.
#[tokio::test(flavor = "multi_thread")]
async fn a_long_press_across_a_switch_runs_nothing() {
    use deckpilot_device::ControlInput;
    let f = fixture();
    let deck = start(&f).await;
    deck.mock
        .send_input(&deck.device, key(4), ControlInput::Press);
    deck.agent.status().await;
    deck.join().await;
    deck.hang_up().await;
    deck.wait_overlay(false).await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    deck.mock
        .send_input(&deck.device, key(4), ControlInput::Release);
    deck.agent.status().await;
    assert!(
        deck.agent.runs(f.quick).await.is_empty(),
        "the long press ran"
    );
    // The same long press without a switch runs.
    deck.mock
        .send_input(&deck.device, key(4), ControlInput::Press);
    tokio::time::sleep(Duration::from_millis(1200)).await;
    deck.mock
        .send_input(&deck.device, key(4), ControlInput::Release);
    deck.wait_run(f.quick, ExecutionState::Succeeded).await;
    deck.agent.shutdown().await;
}

/// The managed layout is seeded into the configuration for the model that
/// connects, and the user's edit of it is what the overlay shows; an agent
/// restart (T-R6) shows the call again without a stale suspension.
#[tokio::test(flavor = "multi_thread")]
async fn the_stored_managed_layout_is_seeded_and_survives_a_restart() {
    let f = fixture();
    let dir = TempDir::new("deckpilot-calls-restart");
    let (paths, db) = support::data_dir(&dir, &f.profile, &[]);
    let mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), "CALLS-R");
    let clock = FakeInstant::default();
    let agent = TestAgent::start_with_folder_clock(&paths, &db, &mock, &clock);
    agent.unlock().await;
    agent
        .handle
        .install_calls_mock(MockProvider::new(), ActionParameters::new())
        .await;
    agent.status().await;
    let mk2 = models::stream_deck_mk2().id;
    let seeded = agent
        .store()
        .managed_layout("deckpilot.calls", &mk2)
        .unwrap()
        .expect("seeded for the connected model");
    assert!(!seeded.edited);
    let deck = Deck {
        agent,
        mock: mock.clone(),
        device: device.clone(),
        clock: clock.clone(),
        _dir: TempDir::new("deckpilot-calls-unused"),
    };
    deck.join().await;
    deck.back_to_work().await;
    deck.agent.shutdown().await;

    // The user renames the layout (an edit); the agent restarts.
    let store = support::open_store(&paths, &db);
    store
        .rename_shared_layout(seeded.layout.id, "Moje Rozmowy")
        .unwrap();
    drop(store);
    let agent = TestAgent::start_with_folder_clock(&paths, &db, &mock, &clock);
    agent.unlock().await;
    agent
        .handle
        .install_calls_mock(MockProvider::new(), ActionParameters::new())
        .await;
    let deck = Deck {
        agent,
        mock,
        device,
        clock,
        _dir: dir,
    };
    deck.join().await;
    let view = deck.agent.handle.calls_view().await.unwrap();
    assert_eq!(
        view.scheduled_returns, 0,
        "no suspension survives a restart"
    );
    assert_eq!(view.layouts, ["Moje Rozmowy"], "the user's edit is shown");
    deck.agent.shutdown().await;
}

/// Protocol 19: the app reads Calls, gets `callsChanged` when the call
/// changes, turns a device's overlay off and restores a managed layout,
/// keeping the user's version. No title or process ever travels.
#[tokio::test(flavor = "multi_thread")]
async fn the_app_reads_and_changes_calls_over_ipc() {
    use deckpilot_ipc::{AgentEvent, Request, Response};
    let f = fixture();
    let deck = start(&f).await;
    let mut events = deck.agent.events().await;
    let Response::CallsStatus { status } =
        deck.agent.request(Request::GetCallsStatus).await.unwrap()
    else {
        panic!("calls status")
    };
    assert!(status.installed);
    assert_eq!(status.state, "idle");
    assert!(status.controls, "the mock controls calls");
    assert!(
        status
            .devices
            .iter()
            .any(|d| d.device == deck.device && d.overlay && !d.shown)
    );
    let layout = status
        .layouts
        .first()
        .expect("the MK.2's managed layout")
        .clone();
    assert!(!layout.edited);
    let json = serde_json::to_string(&status).unwrap();
    assert!(
        !json.contains("title") && !json.contains("process"),
        "{json}"
    );

    deck.join().await;
    let changed = loop {
        match tokio::time::timeout(TIMEOUT, events.next_event())
            .await
            .unwrap()
            .unwrap()
        {
            AgentEvent::CallsChanged { status } if status.state == "active" => break status,
            _ => {}
        }
    };
    assert!(
        changed
            .devices
            .iter()
            .any(|d| d.device == deck.device && d.shown)
    );

    // The device's overlay off: it goes at once, the call goes on.
    let Response::CallsStatus { status } = deck
        .agent
        .request(Request::SetCallsOverlay {
            device: deck.device.clone(),
            on: false,
        })
        .await
        .unwrap()
    else {
        panic!("calls status")
    };
    assert!(
        status
            .devices
            .iter()
            .any(|d| d.device == deck.device && !d.overlay && !d.shown)
    );
    assert_eq!(status.state, "active");
    assert!(!deck.shows_overlay().await);

    // Restore defaults keeps an edited layout as the user's own.
    deck.agent
        .store()
        .rename_shared_layout(layout.layout.parse().unwrap(), "Edited")
        .unwrap();
    let Response::CallsLayoutRestored { copy } = deck
        .agent
        .request(Request::RestoreCallsLayout {
            model: layout.model.clone(),
            copy_name: "Rozmowy (moja wersja)".into(),
        })
        .await
        .unwrap()
    else {
        panic!("restored")
    };
    let copy = copy.expect("the user's version is kept");
    let kept = deck
        .agent
        .store()
        .shared_layout(copy.parse().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(kept.layout.name, "Rozmowy (moja wersja)");
    let Response::CallsStatus { status } =
        deck.agent.request(Request::GetCallsStatus).await.unwrap()
    else {
        panic!("calls status")
    };
    assert!(status.layouts.iter().all(|l| !l.edited), "regenerated");
    deck.agent.shutdown().await;
}

/// A connection of protocol 18 is never asked, and never told, about
/// Calls: the client refuses the requests before they leave, the agent
/// refuses them from such a connection when they are sent anyway, and its
/// subscription gets no `callsChanged` while a protocol 19 one does.
#[tokio::test(flavor = "multi_thread")]
async fn a_protocol_18_connection_cannot_ask_about_calls() {
    use deckpilot_ipc::framing::{read_message, write_message};
    use deckpilot_ipc::{AgentClient, ClientError, ClientKind, Request};
    use deckpilot_ipc::{AgentEvent, ErrorCode, ServerMessage, SharedStream};
    use serde_json::json;
    use tokio::io::BufReader;
    let f = fixture();
    let deck = start(&f).await;
    let mut old = AgentClient::connect_with_protocol(&deck.agent.paths, ClientKind::Desktop, 18)
        .await
        .unwrap();
    for request in [
        Request::GetCallsStatus,
        Request::SetCallsOverlay {
            device: deck.device.clone(),
            on: false,
        },
        Request::RestoreCallsLayout {
            model: deck.device.model.clone(),
            copy_name: "Mine".into(),
        },
    ] {
        let refused = old.request(request).await;
        assert!(
            matches!(refused, Err(ClientError::Unsupported { needs: 19, .. })),
            "{refused:?}"
        );
    }
    assert!(!deck.shows_overlay().await);

    // A raw protocol 18 desktop connection, subscribed.
    let token = std::fs::read_to_string(&deck.agent.paths.token_file).unwrap();
    let stream = SharedStream::new(deck.agent.paths.endpoint().connect().await.unwrap());
    let mut reader = BufReader::new(stream.clone());
    let mut writer = stream;
    let receive = async move |reader: &mut BufReader<SharedStream>| {
        tokio::time::timeout(
            Duration::from_millis(300),
            read_message::<_, ServerMessage>(reader),
        )
        .await
        .ok()
        .and_then(|m| m.ok().flatten())
    };
    write_message(
        &mut writer,
        &json!({"type":"hello","protocol":18,"token":token.trim(),"client":"desktop"}),
    )
    .await
    .unwrap();
    assert!(matches!(
        receive(&mut reader).await,
        Some(ServerMessage::Welcome { .. })
    ));
    write_message(
        &mut writer,
        &json!({"type":"request","id":1,"request":{"type":"subscribe"}}),
    )
    .await
    .unwrap();
    let mut current = deck.agent.events().await;
    deck.join().await;
    // The protocol 19 subscription is told.
    loop {
        if let AgentEvent::CallsChanged { .. } = tokio::time::timeout(TIMEOUT, current.next_event())
            .await
            .unwrap()
            .unwrap()
        {
            break;
        }
    }
    // Sent anyway at protocol 18: refused by the agent.
    write_message(
        &mut writer,
        &json!({"type":"request","id":2,"request":{"type":"getCallsStatus"}}),
    )
    .await
    .unwrap();
    let mut refused = None;
    while let Some(message) = receive(&mut reader).await {
        match message {
            ServerMessage::Event {
                event: AgentEvent::CallsChanged { .. },
            } => panic!("a protocol 18 connection was told about Calls"),
            ServerMessage::Error { id: Some(2), error } => refused = Some(error.code),
            _ => {}
        }
    }
    assert_eq!(refused, Some(ErrorCode::Unsupported));
    deck.agent.shutdown().await;
}

/// The managed layouts are read from the store once, not on every tick of
/// a call; a configuration reload reads them again and tells the app that
/// the user edited one; a device that is not connected is refused.
#[tokio::test(flavor = "multi_thread")]
async fn calls_status_reads_layouts_once_and_follows_edits() {
    use deckpilot_ipc::{AgentEvent, ClientError, ErrorCode, Request, Response};
    let f = fixture();
    let deck = start(&f).await;
    let mut events = deck.agent.events().await;
    let Response::CallsStatus { status } =
        deck.agent.request(Request::GetCallsStatus).await.unwrap()
    else {
        panic!("calls status")
    };
    let layout = status.layouts.first().expect("the MK.2's layout").clone();
    deck.join().await;
    let reads = deck.agent.handle.calls_view().await.unwrap().layouts_reads;
    assert!(reads >= 1);
    for _ in 0..20 {
        deck.advance(1).await;
    }
    deck.agent.request(Request::GetCallsStatus).await.unwrap();
    assert_eq!(
        deck.agent.handle.calls_view().await.unwrap().layouts_reads,
        reads,
        "not read again on the tick"
    );

    // Edited in the app, then the configuration is reloaded.
    deck.agent
        .store()
        .rename_shared_layout(layout.layout.parse().unwrap(), "Edited")
        .unwrap();
    deck.agent
        .request(Request::ReloadConfiguration)
        .await
        .unwrap();
    loop {
        match tokio::time::timeout(TIMEOUT, events.next_event())
            .await
            .unwrap()
            .unwrap()
        {
            AgentEvent::CallsChanged { status } if status.layouts.iter().any(|l| l.edited) => {
                break;
            }
            _ => {}
        }
    }

    // A device that is not connected.
    let refused = deck
        .agent
        .request(Request::SetCallsOverlay {
            device: deckpilot_device::DeviceId {
                model: deck.device.model.clone(),
                serial: "NOT-HERE".into(),
            },
            on: false,
        })
        .await;
    match refused {
        Err(ClientError::Agent(error)) => {
            assert_eq!(error.code, ErrorCode::NotFound);
            assert_eq!(
                error.reason.as_deref(),
                Some(deckpilot_ipc::calls::reason::UNKNOWN_DEVICE)
            );
        }
        other => panic!("{other:?}"),
    }
    deck.agent.shutdown().await;
}

/// A seed that fails while the database is busy is tried again, after a
/// wait that doubles (1 s, then 2 s), never before: nothing is stored
/// meanwhile, the first seed that goes through is the only write, and the
/// overlay uses it.
#[tokio::test(flavor = "multi_thread")]
async fn a_busy_seed_is_tried_again_with_a_growing_wait_and_stored_once() {
    let f = fixture();
    let dir = TempDir::new("deckpilot-calls-busy");
    let (paths, db) = support::data_dir(&dir, &f.profile, &[]);
    let mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), "CALLS-B");
    let clock = FakeInstant::default();
    let agent = TestAgent::start_with_folder_clock(&paths, &db, &mock, &clock);
    agent.unlock().await;
    agent.handle.calls_fail_seeds(2).await;
    agent
        .handle
        .install_calls_mock(MockProvider::new(), ActionParameters::new())
        .await;
    let deck = Deck {
        agent,
        mock,
        device,
        clock,
        _dir: dir,
    };
    let mk2 = models::stream_deck_mk2().id;
    let look = async |millis: u64| {
        deck.clock.advance(Duration::from_millis(millis));
        deck.agent.status().await;
        deck.agent.status().await;
        let view = deck.agent.handle.calls_view().await.unwrap();
        let stored = deck
            .agent
            .store()
            .managed_layout("deckpilot.calls", &mk2)
            .unwrap();
        (view.seed_attempts, stored)
    };
    // At install: the first try fails, nothing is stored.
    let (attempts, stored) = look(0).await;
    assert_eq!(attempts, 1);
    assert!(stored.is_none());
    // Not before the first wait (1 s) is over.
    assert_eq!(look(500).await.0, 1);
    // At 1 s: tried again, busy again; the next wait is 2 s.
    let (attempts, stored) = look(500).await;
    assert_eq!(attempts, 2);
    assert!(stored.is_none());
    assert_eq!(look(1000).await.0, 2, "not before 3 s");
    // At 3 s: stored.
    let (attempts, stored) = look(1000).await;
    assert_eq!(attempts, 3);
    let first = stored.expect("stored once the database is free");
    // Nothing is written again afterwards.
    for _ in 0..5 {
        let (attempts, stored) = look(5000).await;
        assert_eq!(attempts, 3, "no further write");
        assert_eq!(stored.as_ref(), Some(&first));
    }
    // The overlay shows the stored layout.
    deck.join().await;
    let view = deck.agent.handle.calls_view().await.unwrap();
    assert_eq!(view.layouts, std::slice::from_ref(&first.layout.name));
    deck.agent.shutdown().await;
}

/// C7 over IPC: each managed layout's call pages with their controls and
/// whether the provider supports them; *Back to work* read (the default:
/// completion or 60 s) and set (a 20 s timer the device then follows, the
/// layout counting as edited); out-of-range times refused.
#[tokio::test(flavor = "multi_thread")]
async fn the_call_pages_and_back_to_work_over_ipc() {
    use deckpilot_ipc::calls::{BackToWork, BackToWorkMode, CallsPageKind};
    use deckpilot_ipc::{Request, Response};
    let f = fixture();
    let deck = start(&f).await;
    let status = async || match deck.agent.request(Request::GetCallsStatus).await.unwrap() {
        Response::CallsStatus { status } => status,
        other => panic!("{other:?}"),
    };
    let layout = status().await.layouts[0].clone();
    let kinds: Vec<CallsPageKind> = layout.pages.iter().map(|p| p.page).collect();
    assert_eq!(
        kinds,
        [
            CallsPageKind::Incoming,
            CallsPageKind::Outgoing,
            CallsPageKind::Active
        ]
    );
    let active = &layout.pages[2];
    let control = |action: &str| {
        active
            .controls
            .iter()
            .find(|c| c.action == action)
            .unwrap_or_else(|| panic!("{action} on {active:?}"))
            .clone()
    };
    assert!(control("mic-toggle").supported, "the mock controls calls");
    assert!(control("return-to-work").supported);
    assert_eq!(layout.back, BackToWork::default());
    assert_eq!(
        layout.back.mode,
        BackToWorkMode::CompletionOrMax { max_seconds: 60 }
    );

    // A 20 s timer.
    let timer = BackToWork {
        mode: BackToWorkMode::Timer { seconds: 20 },
        ..BackToWork::default()
    };
    let Response::CallsStatus { status: set } = deck
        .agent
        .request(Request::SetCallsBackToWork {
            model: layout.model.clone(),
            back: timer,
        })
        .await
        .unwrap()
    else {
        panic!("calls status")
    };
    assert_eq!(set.layouts[0].back, timer);
    assert!(set.layouts[0].edited, "a Back to work change is an edit");
    // The device follows it: back after 20 s, not before.
    deck.join().await;
    deck.back_to_work().await;
    deck.advance(19).await;
    assert!(!deck.shows_overlay().await, "not before 20 s");
    deck.advance(1).await;
    deck.wait_overlay(true).await;

    // Out of range: refused, nothing changes.
    let refused = deck
        .agent
        .request(Request::SetCallsBackToWork {
            model: layout.model.clone(),
            back: BackToWork {
                mode: BackToWorkMode::Timer { seconds: 3 },
                ..BackToWork::default()
            },
        })
        .await;
    assert!(refused.is_err(), "{refused:?}");
    assert_eq!(status().await.layouts[0].back, timer);
    deck.agent.shutdown().await;
}

/// With a detection-only provider (like Teams) every call control of the
/// pages is unsupported, with the reason; "Back to work" still works.
#[tokio::test(flavor = "multi_thread")]
async fn a_detection_only_provider_lists_its_controls_as_unsupported() {
    use deckpilot_ipc::{Request, Response};
    let f = fixture();
    let dir = TempDir::new("deckpilot-calls-detect");
    let (paths, db) = support::data_dir(&dir, &f.profile, &[]);
    let mock = MockBackend::new();
    let _device = mock.plug(&models::stream_deck_mk2(), "CALLS-D");
    let clock = FakeInstant::default();
    let agent = TestAgent::start_with_folder_clock(&paths, &db, &mock, &clock);
    agent.unlock().await;
    agent
        .handle
        .install_calls_mock(MockProvider::detection_only(), ActionParameters::new())
        .await;
    let Response::CallsStatus { status } = agent.request(Request::GetCallsStatus).await.unwrap()
    else {
        panic!("calls status")
    };
    assert!(!status.controls);
    let controls: Vec<_> = status.layouts[0]
        .pages
        .iter()
        .flat_map(|p| p.controls.iter())
        .collect();
    assert_ne!(controls.len(), 0);
    for control in controls {
        if control.action == "return-to-work" {
            assert!(control.supported);
        } else {
            assert!(!control.supported, "{control:?}");
            assert_eq!(control.reason.as_deref(), Some("provider-has-no-api"));
        }
    }
    agent.shutdown().await;
}
