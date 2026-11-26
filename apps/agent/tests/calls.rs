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
    let deck = start_bare(f).await;
    deck.agent
        .handle
        .install_calls_mock(MockProvider::new(), back)
        .await;
    deck
}

/// An unlocked agent with the MK.2 and Calls not installed.
async fn start_bare(f: &Fixture) -> Deck {
    let dir = TempDir::new("deckpilot-calls");
    let (paths, db) = support::data_dir(&dir, &f.profile, &[]);
    let mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), "CALLS-1");
    let clock = FakeInstant::default();
    let agent = TestAgent::start_with_folder_clock(&paths, &db, &mock, &clock);
    agent.unlock().await;
    Deck {
        agent,
        mock,
        device,
        clock,
        _dir: dir,
    }
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
        Request::SetCallsBackToWork {
            model: deck.device.model.clone(),
            back: deckpilot_ipc::calls::BackToWork::default(),
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

/// T-R7 at agent level, with two MK.2s in the meeting: one going back to
/// work leaves the other on the call; with `returnAllDevices` both go.
#[tokio::test(flavor = "multi_thread")]
async fn other_devices_stay_on_the_call_unless_all_devices_is_set() {
    for all in [false, true] {
        let f = fixture();
        let back = ActionParameters::new().with("returnAllDevices", ParameterValue::Boolean(all));
        let deck = start_with(&f, back).await;
        let second = deck.mock.plug(&models::stream_deck_mk2(), "CALLS-2");
        deck.join().await;
        let shows = async |device: &DeviceId| {
            let view = deck.agent.handle.calls_view().await.unwrap();
            view.devices
                .iter()
                .any(|(d, shown, _)| d == device && *shown)
        };
        let deadline = tokio::time::Instant::now() + TIMEOUT;
        while !shows(&second).await {
            assert!(
                tokio::time::Instant::now() <= deadline,
                "the second device shows the call"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        deck.back_to_work().await;
        assert!(!shows(&deck.device).await);
        assert_eq!(
            shows(&second).await,
            !all,
            "returnAllDevices {all}: the other device"
        );
        deck.agent.shutdown().await;
    }
}

/// Calls installed while the configuration is locked, with a meeting
/// already going on: the managed layout can be seeded only once unlocked,
/// and the overlay is shown then, not at the call's next change.
#[tokio::test(flavor = "multi_thread")]
async fn a_call_going_on_before_the_layout_is_seeded_shows_the_overlay_once_it_is() {
    let f = fixture();
    let dir = TempDir::new("deckpilot-calls-locked");
    let (paths, db) = support::data_dir(&dir, &f.profile, &[]);
    let mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), "CALLS-1");
    let clock = FakeInstant::default();
    let agent = TestAgent::start_with_folder_clock(&paths, &db, &mock, &clock);
    let mut provider = MockProvider::new();
    provider.join(deckpilot_calls::clock::Instant::ZERO, "meeting");
    agent
        .handle
        .install_calls_mock(provider, ActionParameters::new())
        .await;
    let deck = Deck {
        agent,
        mock,
        device,
        clock,
        _dir: dir,
    };
    deck.advance(1).await;
    assert_eq!(
        deck.agent.handle.calls_view().await.unwrap().state,
        CallState::Active
    );
    assert!(!deck.shows_overlay().await, "locked: no layout yet");
    deck.agent.unlock().await;
    deck.wait_overlay(true).await;
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
    assert_eq!(layout.back, Some(BackToWork::default()));
    assert_eq!(
        layout.back.map(|b| b.mode),
        Some(BackToWorkMode::CompletionOrMax { max_seconds: 60 })
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
    assert_eq!(set.layouts[0].back, Some(timer));
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
    match &refused {
        Err(deckpilot_ipc::ClientError::Agent(error)) => {
            assert_eq!(
                error.code,
                deckpilot_ipc::ErrorCode::InvalidRequest,
                "{error:?}"
            );
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(status().await.layouts[0].back, Some(timer));
    // A model without a Calls layout.
    let unknown = deck
        .agent
        .request(Request::SetCallsBackToWork {
            model: models::stream_deck_xl().id,
            back: timer,
        })
        .await;
    match &unknown {
        Err(deckpilot_ipc::ClientError::Agent(error)) => {
            assert_eq!(error.code, deckpilot_ipc::ErrorCode::NotFound);
            assert_eq!(
                error.reason.as_deref(),
                Some(deckpilot_ipc::calls::reason::NO_LAYOUT)
            );
        }
        other => panic!("{other:?}"),
    }
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

/// With only the consent to observe calls, no control of the pages is
/// supported (`consent-not-given`), whatever the provider can, and the
/// status says nothing can be controlled.
#[tokio::test(flavor = "multi_thread")]
async fn observing_only_lists_every_control_as_not_allowed() {
    use deckpilot_calls::provider::ConsentState;
    use deckpilot_ipc::{Request, Response};
    let f = fixture();
    let dir = TempDir::new("deckpilot-calls-observe");
    let (paths, db) = support::data_dir(&dir, &f.profile, &[]);
    let mock = MockBackend::new();
    let _device = mock.plug(&models::stream_deck_mk2(), "CALLS-O");
    let clock = FakeInstant::default();
    let agent = TestAgent::start_with_folder_clock(&paths, &db, &mock, &clock);
    agent.unlock().await;
    let observe = ConsentState {
        observe: true,
        control: false,
        titles: false,
    };
    agent
        .handle
        .install_calls_mock_with(MockProvider::new(), ActionParameters::new(), observe)
        .await;
    let Response::CallsStatus { status } = agent.request(Request::GetCallsStatus).await.unwrap()
    else {
        panic!("calls status")
    };
    assert!(!status.controls, "observe only: nothing controls");
    for control in status.layouts[0]
        .pages
        .iter()
        .flat_map(|p| p.controls.iter())
    {
        if control.action == "return-to-work" {
            assert!(control.supported);
        } else {
            assert!(!control.supported, "{control:?}");
            assert_eq!(control.reason.as_deref(), Some("consent-not-given"));
        }
    }
    agent.shutdown().await;
}

/// Two "Back to work" keys on the managed layout: setting Back to work
/// writes both, in one go, and each keeps a parameter the settings do not
/// name.
#[tokio::test(flavor = "multi_thread")]
async fn back_to_work_is_written_to_every_key_keeping_other_parameters() {
    use deckpilot_ipc::calls::{BackToWork, BackToWorkMode};
    use deckpilot_ipc::{Request, Response};
    let f = fixture();
    let deck = start(&f).await;
    let store = deck.agent.store();
    let mk2 = models::stream_deck_mk2().id;
    let managed = store
        .managed_layout("deckpilot.calls", &mk2)
        .unwrap()
        .expect("seeded");
    // A second "Back to work" key, and a parameter of the user's on each.
    let mut layout = managed.layout.clone();
    let mut actions = managed.actions.clone();
    let first = actions
        .iter_mut()
        .find(|a| a.action_type.as_str() == "deckpilot.calls.return-to-work")
        .unwrap();
    first.parameters = first
        .parameters
        .clone()
        .with("note", ParameterValue::Text("mine".into()));
    let mut second = first.clone();
    second.id = deckpilot_core::ActionInstanceId::new();
    second.parameters = second
        .parameters
        .clone()
        .with("note", ParameterValue::Text("second".into()));
    let home = layout.home_page;
    let used: Vec<ControlId> = layout
        .page(home)
        .unwrap()
        .bindings
        .iter()
        .map(|b| b.control)
        .collect();
    let free = (0..15u16)
        .map(key)
        .find(|k| !used.contains(k))
        .expect("a free key");
    layout
        .page_mut(home)
        .unwrap()
        .bindings
        .push(ControlBinding::new(
            free,
            Trigger::Press,
            BindingTarget::action(second.id),
        ));
    let ids = [first.id, second.id];
    actions.push(second);
    store
        .seed_managed_layout(
            "deckpilot.calls",
            &layout,
            &actions,
            &managed.template_version,
        )
        .unwrap();
    // Set a 20 s timer.
    let timer = BackToWork {
        mode: BackToWorkMode::Timer { seconds: 20 },
        ..BackToWork::default()
    };
    let Response::CallsStatus { .. } = deck
        .agent
        .request(Request::SetCallsBackToWork {
            model: mk2.clone(),
            back: timer,
        })
        .await
        .unwrap()
    else {
        panic!("calls status")
    };
    let stored = store
        .managed_layout("deckpilot.calls", &mk2)
        .unwrap()
        .unwrap();
    for (id, note) in ids.into_iter().zip(["mine", "second"]) {
        let action = stored.actions.iter().find(|a| a.id == id).unwrap();
        assert_eq!(action.parameters.text("mode"), Some("timer"), "{id}");
        assert_eq!(
            action.parameters.get("seconds"),
            Some(&ParameterValue::Integer(20)),
            "{id}"
        );
        assert_eq!(action.parameters.text("note"), Some(note), "kept: {id}");
    }
    deck.agent.shutdown().await;
}

// ------------------------------------------------- C6b: the editor's preview

/// Waits until the test view of Calls satisfies `check`.
async fn wait_view(
    deck: &Deck,
    what: &str,
    check: impl Fn(&deckpilot_agent::service::calls::CallsView) -> bool,
) {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        let view = deck.agent.handle.calls_view().await.unwrap();
        if check(&view) {
            return;
        }
        assert!(tokio::time::Instant::now() <= deadline, "{what}: {view:?}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn preview(deck: &Deck, page: deckpilot_ipc::calls::CallsPageKind) -> deckpilot_ipc::Request {
    deckpilot_ipc::Request::PreviewCallsPage {
        device: deck.device.clone(),
        page,
    }
}

fn refused_with(
    result: Result<deckpilot_ipc::Response, deckpilot_ipc::ClientError>,
) -> deckpilot_ipc::IpcError {
    match result {
        Err(deckpilot_ipc::ClientError::Agent(error)) => error,
        other => panic!("expected the agent's refusal, got {other:?}"),
    }
}

/// Locked: no preview starts, and none is left.
#[tokio::test(flavor = "multi_thread")]
async fn a_preview_is_refused_while_locked() {
    use deckpilot_ipc::calls::CallsPageKind;
    use deckpilot_ipc::{ErrorCode, Request};
    let f = fixture();
    let deck = start(&f).await;
    deck.agent.request(Request::Lock).await.unwrap();
    let mut editor = deck.agent.client().await;
    let error = refused_with(editor.request(preview(&deck, CallsPageKind::Active)).await);
    assert_eq!(error.code, ErrorCode::Locked);
    let view = deck.agent.handle.calls_view().await.unwrap();
    assert_eq!((view.previews, view.frames), (0, 0), "{view:?}");
    deck.agent.shutdown().await;
}

/// A real call takes over a preview at once, a preview never starts during
/// a call, and the call's end leaves no frame.
#[tokio::test(flavor = "multi_thread")]
async fn a_real_call_takes_over_a_preview() {
    use deckpilot_ipc::calls::CallsPageKind;
    let f = fixture();
    let deck = start(&f).await;
    let mut editor = deck.agent.client().await;
    editor
        .request(preview(&deck, CallsPageKind::Incoming))
        .await
        .unwrap();
    wait_view(&deck, "the preview", |v| v.previews == 1 && v.frames == 1).await;
    assert!(deck.shows_overlay().await);
    deck.join().await;
    wait_view(&deck, "the call's frame only", |v| {
        v.previews == 0 && v.frames == 1 && v.state == CallState::Active
    })
    .await;
    let error = refused_with(editor.request(preview(&deck, CallsPageKind::Active)).await);
    assert_eq!(
        error.reason.as_deref(),
        Some(deckpilot_ipc::calls::reason::PREVIEW_IN_CALL)
    );
    deck.hang_up().await;
    wait_view(&deck, "no frame after the call", |v| v.frames == 0).await;
    deck.agent.shutdown().await;
}

/// The preview belongs to the editor's connection: closing it ends it.
#[tokio::test(flavor = "multi_thread")]
async fn closing_the_connection_ends_the_preview() {
    use deckpilot_ipc::calls::CallsPageKind;
    let f = fixture();
    let deck = start(&f).await;
    let mut editor = deck.agent.client().await;
    editor
        .request(preview(&deck, CallsPageKind::Active))
        .await
        .unwrap();
    wait_view(&deck, "the preview", |v| v.previews == 1 && v.frames == 1).await;
    drop(editor);
    wait_view(&deck, "the preview ended with the connection", |v| {
        v.previews == 0 && v.frames == 0
    })
    .await;
    deck.agent.shutdown().await;
}

/// Only a connection declaring the desktop (the configuration app)
/// previews: any other client kind (a plugin host, the command line) is
/// refused, and a protocol 18 one never sends it. The kind is declared,
/// not authenticated: a policy guard behind the token.
#[tokio::test(flavor = "multi_thread")]
async fn a_connection_not_declaring_the_desktop_cannot_preview() {
    use deckpilot_ipc::calls::CallsPageKind;
    use deckpilot_ipc::{AgentClient, ClientError, ClientKind, ErrorCode};
    let f = fixture();
    let deck = start(&f).await;
    let mut other = AgentClient::connect(&deck.agent.paths, ClientKind::Other)
        .await
        .unwrap();
    let error = refused_with(other.request(preview(&deck, CallsPageKind::Active)).await);
    assert_eq!(error.code, ErrorCode::Unauthorized);
    assert_eq!(
        error.reason.as_deref(),
        Some(deckpilot_ipc::calls::reason::PREVIEW_DESKTOP_ONLY)
    );
    let mut old = AgentClient::connect_with_protocol(&deck.agent.paths, ClientKind::Desktop, 18)
        .await
        .unwrap();
    let refused = old.request(preview(&deck, CallsPageKind::Active)).await;
    assert!(
        matches!(refused, Err(ClientError::Unsupported { needs: 19, .. })),
        "{refused:?}"
    );
    let view = deck.agent.handle.calls_view().await.unwrap();
    assert_eq!((view.previews, view.frames), (0, 0), "{view:?}");
    deck.agent.shutdown().await;
}

/// No frame leaks: a new preview replaces the old (one per device), the
/// end request, the device's disconnect and a lock each leave no frame.
#[tokio::test(flavor = "multi_thread")]
async fn a_preview_never_leaves_a_frame_behind() {
    use deckpilot_ipc::Request;
    use deckpilot_ipc::calls::CallsPageKind;
    let f = fixture();
    let deck = start(&f).await;
    let mut editor = deck.agent.client().await;
    editor
        .request(preview(&deck, CallsPageKind::Incoming))
        .await
        .unwrap();
    editor
        .request(preview(&deck, CallsPageKind::Outgoing))
        .await
        .unwrap();
    wait_view(&deck, "one frame, replaced", |v| {
        v.previews == 1 && v.frames == 1
    })
    .await;
    editor
        .request(Request::EndCallsPreview {
            device: deck.device.clone(),
        })
        .await
        .unwrap();
    wait_view(&deck, "ended", |v| v.previews == 0 && v.frames == 0).await;
    // Unplugged while previewing, then plugged in again: nothing comes back.
    editor
        .request(preview(&deck, CallsPageKind::Active))
        .await
        .unwrap();
    wait_view(&deck, "the preview", |v| v.frames == 1).await;
    deck.mock.unplug(&deck.device);
    wait_view(&deck, "ended by the disconnect", |v| {
        v.previews == 0 && v.frames == 0
    })
    .await;
    deck.mock
        .plug(&models::stream_deck_mk2(), &deck.device.serial);
    deck.advance(1).await;
    wait_view(&deck, "nothing back after the reconnect", |v| {
        v.previews == 0 && v.frames == 0
    })
    .await;
    // Locked while previewing (once the device is back).
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    while editor
        .request(preview(&deck, CallsPageKind::Active))
        .await
        .is_err()
    {
        assert!(
            tokio::time::Instant::now() <= deadline,
            "the device came back"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    wait_view(&deck, "the preview", |v| v.frames == 1).await;
    deck.agent.request(Request::Lock).await.unwrap();
    wait_view(&deck, "ended by the lock", |v| {
        v.previews == 0 && v.frames == 0
    })
    .await;
    deck.agent.shutdown().await;
}

/// T-E2E for every class with key displays, not only the MK.2: on an XL
/// (`grid-large`) and a Plus (`keys+dials`) the overlay comes with the
/// call, the device draws its "Back to work" key where the class's
/// template puts it, and the overlay goes with the call.
#[tokio::test(flavor = "multi_thread")]
async fn the_overlay_is_drawn_on_every_class_of_device() {
    for model in [models::stream_deck_xl(), models::stream_deck_plus()] {
        let f = fixture();
        let dir = TempDir::new("deckpilot-calls-class");
        let (paths, db) = support::data_dir(&dir, &f.profile, &[]);
        let mock = MockBackend::new();
        let device = mock.plug(&model, "CALLS-C");
        let clock = FakeInstant::default();
        let agent = TestAgent::start_with_folder_clock(&paths, &db, &mock, &clock);
        agent.unlock().await;
        agent
            .handle
            .install_calls_mock(MockProvider::new(), ActionParameters::new())
            .await;
        let deck = Deck {
            agent,
            mock,
            device: device.clone(),
            clock,
            _dir: dir,
        };
        deck.join().await;
        // Where the class's template puts "Back to work".
        let shape = Shape::of(&model.capabilities);
        let template = builtin_templates()
            .into_iter()
            .find(|t| t.shape == shape)
            .unwrap();
        let generated = generate(&template, &model.id, &model.capabilities).unwrap();
        let active = generated.pages[&deckpilot_calls::layouts::Page::Active];
        let back = generated
            .layout
            .page(active)
            .unwrap()
            .bindings
            .iter()
            .find(|b| generated.action_of(b.target.action_id().unwrap()) == Some("return-to-work"))
            .map(|b| b.control)
            .unwrap();
        let spec = model.capabilities.keys.and_then(|keys| keys.image).unwrap();
        let face = KeyFace::new(FaceStyle::Action, "Back to work");
        let image = deckpilot_agent::render::render(&face, spec.width, spec.height);
        assert!(
            deck.mock
                .wait_for(&device, TIMEOUT, |frames, _| frames.get(&back)
                    == Some(&image)),
            "{}: the device shows the overlay's Back to work key",
            model.id
        );
        deck.hang_up().await;
        deck.wait_overlay(false).await;
        deck.agent.shutdown().await;
    }
}

/// Waits for a `callsChanged` in which `device` previews `page` (`None`:
/// none) and `also` holds.
async fn wait_preview(
    events: &mut deckpilot_ipc::AgentClient,
    device: &deckpilot_device::DeviceId,
    page: Option<deckpilot_ipc::calls::CallsPageKind>,
    also: impl Fn(&deckpilot_ipc::calls::CallsStatus) -> bool,
    what: &str,
) {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        let event = tokio::time::timeout(left, events.next_event())
            .await
            .unwrap_or_else(|_| panic!("no callsChanged: {what}"))
            .unwrap();
        if let deckpilot_ipc::AgentEvent::CallsChanged { status } = event {
            let shown = status
                .devices
                .iter()
                .find(|d| &d.device == device)
                .and_then(|d| d.preview);
            if shown == page && also(&status) {
                return;
            }
        }
    }
}

/// The app follows the agent's previews: `callsChanged` names the page a
/// device previews, and drops it on every way a preview ends: the end
/// request, the editor's connection closing, a lock, the dismiss key, the
/// device's disconnect and a real call.
#[tokio::test(flavor = "multi_thread")]
async fn calls_changed_follows_the_preview() {
    use deckpilot_ipc::Request;
    use deckpilot_ipc::calls::CallsPageKind;
    let any = |_: &deckpilot_ipc::calls::CallsStatus| true;
    let f = fixture();
    let deck = start(&f).await;
    let mut events = deck.agent.events().await;
    let mut editor = deck.agent.client().await;
    let device = deck.device.clone();
    let shown = Some(CallsPageKind::Outgoing);

    // The end request.
    editor
        .request(preview(&deck, CallsPageKind::Outgoing))
        .await
        .unwrap();
    wait_preview(&mut events, &device, shown, any, "shown").await;
    editor
        .request(Request::EndCallsPreview {
            device: device.clone(),
        })
        .await
        .unwrap();
    wait_preview(&mut events, &device, None, any, "the end request").await;

    // The editor's connection closes.
    editor
        .request(preview(&deck, CallsPageKind::Outgoing))
        .await
        .unwrap();
    wait_preview(&mut events, &device, shown, any, "shown").await;
    drop(editor);
    wait_preview(&mut events, &device, None, any, "the connection closed").await;
    let mut editor = deck.agent.client().await;

    // The dismiss key (the model's last key).
    editor
        .request(preview(&deck, CallsPageKind::Outgoing))
        .await
        .unwrap();
    wait_preview(&mut events, &device, shown, any, "shown").await;
    let keys = models::stream_deck_mk2().capabilities.keys.unwrap();
    deck.tap(keys.key_count() - 1);
    wait_preview(&mut events, &device, None, any, "the dismiss key").await;

    // A lock.
    editor
        .request(preview(&deck, CallsPageKind::Outgoing))
        .await
        .unwrap();
    wait_preview(&mut events, &device, shown, any, "shown").await;
    deck.agent.request(Request::Lock).await.unwrap();
    wait_preview(&mut events, &device, None, any, "the lock").await;
    deck.agent.unlock().await;

    // The device's disconnect: it is gone from the list (no preview named).
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    while editor
        .request(preview(&deck, CallsPageKind::Outgoing))
        .await
        .is_err()
    {
        assert!(tokio::time::Instant::now() <= deadline, "unlocked again");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    wait_preview(&mut events, &device, shown, any, "shown").await;
    deck.mock.unplug(&deck.device);
    wait_preview(
        &mut events,
        &device,
        None,
        |s| s.devices.iter().all(|d| d.device != device),
        "the disconnect",
    )
    .await;
    deck.mock
        .plug(&models::stream_deck_mk2(), &deck.device.serial);
    deck.advance(1).await;

    // A real call.
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    while editor
        .request(preview(&deck, CallsPageKind::Outgoing))
        .await
        .is_err()
    {
        assert!(
            tokio::time::Instant::now() <= deadline,
            "the device came back"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    wait_preview(&mut events, &device, shown, any, "shown").await;
    deck.join().await;
    wait_preview(
        &mut events,
        &device,
        None,
        |s| s.state == "active",
        "the real call",
    )
    .await;
    deck.agent.shutdown().await;
}

/// The status's revision: the same content keeps its number in every
/// answer and event, a change counts it up, and `callsChanged` carries
/// the number a later `getCallsStatus` repeats.
#[tokio::test(flavor = "multi_thread")]
async fn the_calls_status_counts_its_revisions() {
    use deckpilot_ipc::{AgentEvent, Request, Response};
    let f = fixture();
    let deck = start(&f).await;
    let mut events = deck.agent.events().await;
    let status = || async {
        let Response::CallsStatus { status } =
            deck.agent.request(Request::GetCallsStatus).await.unwrap()
        else {
            panic!("calls status")
        };
        status
    };
    let first = status().await;
    assert!(first.revision >= 1);
    assert_eq!(status().await.revision, first.revision, "nothing changed");
    deck.join().await;
    let changed = loop {
        if let AgentEvent::CallsChanged { status } =
            tokio::time::timeout(TIMEOUT, events.next_event())
                .await
                .unwrap()
                .unwrap()
            && status.state == "active"
        {
            break status;
        }
    };
    assert!(changed.revision > first.revision, "a change counts up");
    let again = status().await;
    assert_eq!(again.state, "active");
    assert!(again.revision >= changed.revision, "never behind the event");
    deck.agent.shutdown().await;
}

/// A locked device never shows call content: the overlay on screen goes
/// when the configuration is locked, and the device itself draws "Locked"
/// (its frames, not only the runtime's view). Nothing comes back while
/// locked even as a second call rings, which the engine still sees. After
/// unlocking, the device has exactly one Calls frame, on the page of the
/// call that has the focus then (the ringing one).
#[tokio::test(flavor = "multi_thread")]
async fn a_locked_device_never_shows_the_call() {
    use deckpilot_ipc::Request;
    let f = fixture();
    let deck = start(&f).await;
    deck.join().await;
    let spec = models::stream_deck_mk2()
        .capabilities
        .keys
        .and_then(|keys| keys.image)
        .unwrap();
    let render = |face: KeyFace| deckpilot_agent::render::render(&face, spec.width, spec.height);
    let locked = render(KeyFace::new(FaceStyle::Notice, "Locked"));
    let blank = render(KeyFace::empty());
    let back = key(Deck::back_to_work_key());
    let back_face = render(KeyFace::new(FaceStyle::Action, "Back to work"));
    assert!(
        deck.mock
            .wait_for(&deck.device, TIMEOUT, |frames, _| frames.get(&back)
                == Some(&back_face)),
        "the overlay is on the device"
    );
    deck.agent.request(Request::Lock).await.unwrap();
    // The device draws "Locked" on its first key and nothing of the call.
    let shows_locked = |frames: &std::collections::BTreeMap<ControlId, _>| {
        frames.get(&key(0)) == Some(&locked)
            && frames
                .iter()
                .filter(|(control, _)| **control != key(0))
                .all(|(_, frame)| *frame == blank)
    };
    assert!(
        deck.mock
            .wait_for(&deck.device, TIMEOUT, |frames, _| shows_locked(frames)),
        "the locked device draws Locked and nothing else"
    );
    // A second call rings while locked: the engine sees it, the device
    // still shows only "Locked".
    deck.agent
        .handle
        .calls_mock(|mock, now| mock.ring(now, "second"))
        .await;
    for _ in 0..3 {
        deck.advance(20).await;
        assert_eq!(
            deck.agent.handle.calls_view().await.unwrap().state,
            CallState::Incoming,
            "the ringing call is seen while locked"
        );
        assert!(
            shows_locked(&deck.mock.frames(&deck.device)),
            "call content on a locked device"
        );
    }
    // Unlocked: exactly one Calls frame, on the ringing call's page.
    deck.agent.unlock().await;
    deck.wait_overlay(true).await;
    let view = deck.agent.handle.calls_view().await.unwrap();
    assert_eq!(view.frames, 1, "exactly one Calls frame: {view:?}");
    assert_eq!(view.devices.len(), 1, "{view:?}");
    assert_eq!(view.devices[0].0, deck.device);
    assert_eq!(view.devices[0].2.as_deref(), Some("Incoming call"));
    assert!(
        deck.mock
            .wait_for(&deck.device, TIMEOUT, |frames, _| !shows_locked(frames)),
        "the device no longer shows Locked"
    );
    deck.agent.shutdown().await;
}

/// The per-device overlay setting survives an agent restart: turned off,
/// it stays off after the restart, and a call then shows no overlay on that
/// device. It is kept machine-local (the `settings` table), never in a
/// profile.
#[tokio::test(flavor = "multi_thread")]
async fn the_overlay_setting_survives_a_restart() {
    use deckpilot_ipc::{Request, Response};
    let f = fixture();
    let dir = TempDir::new("deckpilot-calls-setting");
    let (paths, db) = support::data_dir(&dir, &f.profile, &[]);
    let mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), "CALLS-S");
    let clock = FakeInstant::default();
    let agent = TestAgent::start_with_folder_clock(&paths, &db, &mock, &clock);
    agent.unlock().await;
    agent
        .handle
        .install_calls_mock(MockProvider::new(), ActionParameters::new())
        .await;
    agent.status().await;
    let Response::CallsStatus { status } = agent
        .request(Request::SetCallsOverlay {
            device: device.clone(),
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
            .any(|d| d.device == device && !d.overlay)
    );
    let saved = deckpilot_storage::ConfigStore::setting(&agent.store(), "calls.overlay.devices")
        .unwrap()
        .expect("saved");
    assert!(saved.contains("CALLS-S"), "{saved}");
    agent.shutdown().await;

    let agent = TestAgent::start_with_folder_clock(&paths, &db, &mock, &clock);
    agent.unlock().await;
    agent
        .handle
        .install_calls_mock(MockProvider::new(), ActionParameters::new())
        .await;
    agent.status().await;
    let Response::CallsStatus { status } = agent.request(Request::GetCallsStatus).await.unwrap()
    else {
        panic!("calls status")
    };
    assert!(
        status
            .devices
            .iter()
            .any(|d| d.device == device && !d.overlay),
        "still off after the restart: {:?}",
        status.devices
    );
    // A call: no overlay on this device.
    agent
        .handle
        .calls_mock(|mock, now| mock.join(now, "meeting"))
        .await;
    let deck = Deck {
        agent,
        mock,
        device,
        clock,
        _dir: dir,
    };
    deck.advance(5).await;
    let view = deck.agent.handle.calls_view().await.unwrap();
    assert_eq!(view.state, CallState::Active);
    assert_eq!(view.frames, 0, "{view:?}");
    deck.agent.shutdown().await;
}

/// Plugin API 1.16's `call.state` and `call.control` as the Calls plugin's
/// executor asks the agent (after the broker allowed them; the broker's
/// and the executor's refusals are tested in their crates): the user's
/// consents, detection-only Teams, the confirmed outcome, the plugin id
/// and the lock (T-P1).
#[cfg(feature = "plugins")]
mod plugin_ops {
    use deckpilot_calls::os_signals::{
        Capture, OsSignalsProvider, Platform, Profile as SignalsProfile, SignalSource, Signals,
        SourceError,
    };
    use deckpilot_calls::provider::{ConsentState, Tri};
    use deckpilot_ipc::Request;
    use deckpilot_plugin_abi::calls::{CallAction, CallStateReply};
    use serde_json::Value;

    use super::*;

    const CALLS: &str = "deckpilot.calls";

    async fn state(deck: &Deck) -> Result<CallStateReply, (String, String)> {
        deck.agent
            .handle
            .plugin_call(CALLS, None)
            .await
            .map(|v| serde_json::from_value(v).unwrap())
    }

    /// A control, with the folder clock moved in steps of 100 ms until it
    /// is answered (the mock confirms 200 ms after the command).
    async fn control(deck: &Deck, action: CallAction) -> Result<Value, (String, String)> {
        let call = deck.agent.handle.plugin_call(CALLS, Some(action));
        tokio::pin!(call);
        let deadline = tokio::time::Instant::now() + TIMEOUT;
        loop {
            tokio::select! {
                answer = &mut call => return answer,
                () = tokio::time::sleep(Duration::from_millis(20)) => {
                    assert!(tokio::time::Instant::now() <= deadline, "answered");
                    deck.clock.advance(Duration::from_millis(100));
                    deck.agent.status().await;
                }
            }
        }
    }

    /// How many commands the mock received.
    async fn sent_commands(deck: &Deck) -> Option<usize> {
        let (tx, rx) = std::sync::mpsc::channel();
        deck.agent
            .handle
            .calls_mock(move |mock, _| {
                let _ = tx.send(mock.sent().len());
            })
            .await;
        rx.try_recv().ok()
    }

    fn code(answer: Result<Value, (String, String)>) -> (String, String) {
        answer.expect_err("refused")
    }

    /// The mock's full path (test hooks only): the state, a confirmed
    /// toggle, a set of the state the call has, and a confirmed end.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_plugin_reads_and_controls_a_call_of_the_mock() {
        let f = fixture();
        let deck = start(&f).await;
        let idle = state(&deck).await.unwrap();
        assert_eq!(idle.state, "idle");
        deck.join().await;
        let active = state(&deck).await.unwrap();
        assert_eq!(active.state, "active");
        assert_eq!(active.provider.as_deref(), Some("mock"));
        assert!(active.connected);
        assert_eq!(active.title, None, "no title without the opt-in");
        for id in ["end", "mic-toggle", "mic-mute", "camera-toggle"] {
            assert!(active.controls[id].supported, "{id}");
        }
        let muted_before = active.mic_muted;
        assert_eq!(
            control(&deck, CallAction::MicToggle).await.unwrap(),
            "confirmed"
        );
        let after = state(&deck).await.unwrap();
        assert_eq!(after.mic_muted, muted_before.map(|m| !m));
        let now = after.mic_muted.unwrap();
        assert_eq!(
            control(&deck, CallAction::MicSet { muted: now })
                .await
                .unwrap(),
            "alreadyThere"
        );
        assert_eq!(control(&deck, CallAction::End).await.unwrap(), "confirmed");
        let sent = sent_commands(&deck).await;
        assert_eq!(sent, Some(2), "the toggle and the end; nothing for the set");
        // Not in a call any more: nothing to answer.
        deck.advance(30).await;
        let (refusal, _) = code(control(&deck, CallAction::Answer).await);
        assert_eq!(refusal, "calls-not-in-call");
        deck.agent.shutdown().await;
    }

    /// Observing without the consent to control: the state, every control
    /// shown disabled for the consent, and every control refused before
    /// anything reaches the call app.
    #[tokio::test(flavor = "multi_thread")]
    async fn without_the_control_consent_nothing_is_controlled() {
        let f = fixture();
        let deck = start_observing(&f).await;
        deck.join().await;
        let active = state(&deck).await.unwrap();
        assert_eq!(active.state, "active");
        for (id, support) in &active.controls {
            assert!(!support.supported, "{id}");
            assert_eq!(support.reason.as_deref(), Some("consent-not-given"), "{id}");
        }
        for action in [
            CallAction::End,
            CallAction::MicToggle,
            CallAction::CameraSet { on: true },
        ] {
            let (refusal, _) = code(control(&deck, action).await);
            assert_eq!(refusal, "calls-consent", "{action:?}");
        }
        let sent = sent_commands(&deck).await;
        assert_eq!(sent, Some(0));
        deck.agent.shutdown().await;
    }

    async fn start_observing(f: &Fixture) -> Deck {
        let deck = start_bare(f).await;
        deck.agent
            .handle
            .install_calls_mock_with(
                MockProvider::new(),
                ActionParameters::new(),
                ConsentState {
                    observe: true,
                    control: false,
                    titles: false,
                },
            )
            .await;
        deck
    }

    /// Without the consent to observe calls, Calls does not run: both
    /// operations are refused and say why.
    #[tokio::test(flavor = "multi_thread")]
    async fn without_the_observe_consent_both_are_refused() {
        let f = fixture();
        let deck = start_bare(&f).await;
        deck.agent
            .handle
            .install_calls_mock_with(
                MockProvider::new(),
                ActionParameters::new(),
                ConsentState::default(),
            )
            .await;
        let (refusal, _) = state(&deck).await.unwrap_err();
        assert_eq!(refusal, "calls-provider-offline");
        let (refusal, _) = code(control(&deck, CallAction::End).await);
        assert_eq!(refusal, "calls-provider-offline");
        deck.agent.shutdown().await;
    }

    /// The agent checks the plugin a third time, and a locked DeckPilot
    /// answers nothing about calls.
    #[tokio::test(flavor = "multi_thread")]
    async fn another_plugin_and_a_lock_are_refused() {
        let f = fixture();
        let deck = start(&f).await;
        deck.join().await;
        for plugin in ["deckpilot.workspace", "acme.calls"] {
            let refusal = deck
                .agent
                .handle
                .plugin_call(plugin, None)
                .await
                .unwrap_err();
            assert_eq!(refusal.0, "calls-consent", "{plugin}");
            let refusal = deck
                .agent
                .handle
                .plugin_call(plugin, Some(CallAction::End))
                .await
                .unwrap_err();
            assert_eq!(refusal.0, "calls-consent", "{plugin}");
        }
        deck.agent.request(Request::Lock).await.unwrap();
        let (refusal, _) = state(&deck).await.unwrap_err();
        assert_eq!(refusal, "calls-locked");
        let (refusal, _) = code(control(&deck, CallAction::End).await);
        assert_eq!(refusal, "calls-locked");
        deck.agent.shutdown().await;
    }

    /// A control that waits when Calls is started again is answered at
    /// once, and never by a command of the new engine.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_waiting_control_is_answered_when_calls_starts_again() {
        use deckpilot_calls::testing::Response;
        let f = fixture();
        let deck = start(&f).await;
        deck.join().await;
        deck.agent
            .handle
            .calls_mock(|mock, _| {
                mock.respond(
                    deckpilot_calls::provider::CallAction::MicToggle,
                    Response::Never,
                );
            })
            .await;
        let handle = deck.agent.handle.clone();
        let waiting = handle.plugin_call(CALLS, Some(CallAction::MicToggle));
        tokio::pin!(waiting);
        // It waits for a confirmation that never comes.
        tokio::select! {
            answer = &mut waiting => panic!("answered early: {answer:?}"),
            () = tokio::time::sleep(Duration::from_millis(300)) => {}
        }
        deck.agent
            .handle
            .install_calls_mock(MockProvider::new(), ActionParameters::new())
            .await;
        let started = std::time::Instant::now();
        let (refusal, _) = waiting.await.unwrap_err();
        assert_eq!(refusal, "calls-provider-offline");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "answered at once"
        );
        deck.agent.shutdown().await;
    }

    /// A control the plugin's lane already gave up on (cancelled, or told
    /// it failed) when the agent reaches it is refused, and nothing is sent
    /// to the call app.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_control_the_lane_gave_up_on_is_never_sent() {
        let f = fixture();
        let deck = start(&f).await;
        deck.join().await;
        let refusal = deck
            .agent
            .handle
            .plugin_call_given_up(CALLS, CallAction::MicToggle)
            .await
            .expect("an answer");
        assert_eq!(refusal.0, "calls-unconfirmed");
        assert!(refusal.1.contains("too late"), "{}", refusal.1);
        assert_eq!(sent_commands(&deck).await, Some(0));
        deck.agent.shutdown().await;
    }

    /// Installing Calls again without the consent to observe removes the
    /// earlier install: nothing answers from its old consents, and its
    /// frames go.
    #[tokio::test(flavor = "multi_thread")]
    async fn withdrawing_the_observe_consent_removes_calls() {
        let f = fixture();
        let deck = start(&f).await;
        deck.join().await;
        assert_eq!(state(&deck).await.unwrap().state, "active");
        deck.agent
            .handle
            .install_calls_mock_with(
                MockProvider::new(),
                ActionParameters::new(),
                ConsentState::default(),
            )
            .await;
        let (refusal, _) = state(&deck).await.unwrap_err();
        assert_eq!(refusal, "calls-provider-offline");
        let (refusal, _) = code(control(&deck, CallAction::End).await);
        assert_eq!(refusal, "calls-provider-offline");
        deck.wait_overlay(false).await;
        deck.agent.shutdown().await;
    }

    /// Teams holding the microphone, as a Windows signals fixture shows it.
    struct TeamsInACall;

    impl SignalSource for TeamsInACall {
        fn platform(&self) -> Platform {
            Platform::Windows
        }

        fn sample(
            &mut self,
            _now: deckpilot_calls::clock::Instant,
        ) -> Result<Signals, SourceError> {
            Ok(Signals {
                captures: vec![Capture {
                    executable: "ms-teams.exe".into(),
                    active: true,
                }],
                cameras: Vec::new(),
                camera_in_use: Tri::Unknown,
            })
        }
    }

    /// Detection-only Teams, even with every consent: the call is seen,
    /// the microphone's mute state stays unknown, and every control is
    /// refused with its reason (`provider-has-no-api`). No control is
    /// claimed anywhere.
    #[tokio::test(flavor = "multi_thread")]
    async fn teams_is_detection_only_and_every_control_says_why() {
        let f = fixture();
        let deck = start_bare(&f).await;
        deck.agent
            .handle
            .install_calls_provider(
                OsSignalsProvider::new(SignalsProfile::teams(), Box::new(TeamsInACall)),
                ActionParameters::new(),
                ConsentState {
                    observe: true,
                    control: true,
                    titles: true,
                },
            )
            .await;
        // Operating-system evidence counts after 2 s.
        let deadline = tokio::time::Instant::now() + TIMEOUT;
        let active = loop {
            deck.advance(1).await;
            let now = state(&deck).await.unwrap();
            if now.state == "active" {
                break now;
            }
            assert!(tokio::time::Instant::now() <= deadline, "{}", now.state);
        };
        assert_eq!(active.provider.as_deref(), Some("os-signals"));
        assert_eq!(
            active.mic_muted, None,
            "holding the microphone is no mute state"
        );
        assert_eq!(active.title, None);
        assert!(!active.controls.is_empty());
        for (id, support) in &active.controls {
            assert!(!support.supported, "{id}");
            assert_eq!(
                support.reason.as_deref(),
                Some("provider-has-no-api"),
                "{id}"
            );
        }
        for action in [
            CallAction::Answer,
            CallAction::Reject,
            CallAction::CancelOutgoing,
            CallAction::End,
            CallAction::MicToggle,
            CallAction::MicSet { muted: true },
            CallAction::MicSet { muted: false },
            CallAction::CameraToggle,
            CallAction::CameraSet { on: true },
            CallAction::CameraSet { on: false },
        ] {
            let (refusal, reason) = code(control(&deck, action).await);
            match action {
                // Only in an incoming or outgoing call, which Teams never
                // shows.
                CallAction::Answer | CallAction::Reject | CallAction::CancelOutgoing => {
                    assert_eq!(refusal, "calls-not-in-call", "{action:?}");
                }
                _ => {
                    assert_eq!(refusal, "calls-unsupported", "{action:?}");
                    assert_eq!(reason, "provider-has-no-api", "{action:?}");
                }
            }
        }
        deck.agent.shutdown().await;
    }
}
