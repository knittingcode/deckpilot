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
    quick: WorkflowId,
    long: WorkflowId,
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
    profile.actions.extend([first, under, delay]);
    profile.workflows.extend([quick, slow]);
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
    ]);
    profile.layouts.push(layout);
    Fixture {
        profile,
        quick: quick_id,
        long: long_id,
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
        .install_calls_mock(MockProvider::new())
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
