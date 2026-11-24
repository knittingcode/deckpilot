//! Folders that leave themselves, against a real agent with a simulated
//! MK.2 (`docs/data-model.md`, "Auto Exit" and "Exit after action
//! completes"): the countdown on the protected Back key, paused while a
//! workflow started from the folder runs (the whole run, not its first
//! step), resumed with what was left, and the folder left once the run is
//! over: succeeded, failed or cancelled; a run that cannot start leaves the
//! device in the folder. Countdowns are measured on a clock that moves only
//! when the test moves it: nothing here waits for a countdown. A Stream
//! Deck plugged into the machine is never opened.

mod support;

use std::time::Duration;

use deckpilot_core::device_runtime::folder_back_face;
use deckpilot_core::{
    ActionParameters, AutoExit, BindingTarget, ControlBinding, DeviceLayout, ExecutionState,
    FolderCountdown, LayoutPage, OnFailure, ParameterValue, Profile, Trigger, Workflow, WorkflowId,
    WorkflowStep,
};
use deckpilot_device::mock::MockBackend;
use deckpilot_device::{ConnectionState, ControlId, DeviceId, models};
use deckpilot_ipc::{AgentStatus, Request};
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
    /// A Multi Action: log, a delay of a minute, log.
    long: WorkflowId,
    /// Another one, to run beside it.
    other: WorkflowId,
    /// Log only: over at once.
    quick: WorkflowId,
    /// Fails in its first step (no project is active).
    failing: WorkflowId,
}

/// Home: key 5 opens Tools. Tools (Auto Exit after 10 s; Exit after action
/// completes as `exit_after`): key 3 the long run, key 4 the quick one,
/// key 6 the other long one, key 7 the failing one.
fn fixture(exit_after: bool, auto_exit: bool) -> Fixture {
    let registry = support::registry();
    let log_action = |name: &str, message: &str| {
        registry
            .create_instance(
                name,
                general::log_id(),
                &ActionParameters::new().with("message", ParameterValue::Text(message.into())),
            )
            .unwrap()
    };
    let first = log_action("First", "first");
    let last = log_action("Last", "last");
    let broken = log_action("Broken", "{project.key}");
    let delay = registry
        .create_instance(
            "Delay",
            general::delay_id(),
            &ActionParameters::new().with("milliseconds", ParameterValue::Integer(500)),
        )
        .unwrap();
    let long_run = |name: &str| {
        let mut wait = WorkflowStep::new(delay.id, OnFailure::Stop);
        wait.overrides =
            ActionParameters::new().with("milliseconds", ParameterValue::Integer(60_000));
        let mut workflow = Workflow::new(name)
            .step(first.id, OnFailure::Stop)
            .step(last.id, OnFailure::Stop);
        workflow.steps.insert(1, wait);
        workflow
    };
    let long = long_run("Long");
    let other = long_run("Other");
    let quick = Workflow::new("Quick").step(first.id, OnFailure::Stop);
    let failing = Workflow::new("Failing").step(broken.id, OnFailure::Stop);
    let mut profile = Profile::new("Work");
    profile.actions.extend([first, last, broken, delay]);
    let ids = (long.id, other.id, quick.id, failing.id);
    profile.workflows.extend([long, other, quick, failing]);
    let mut layout = DeviceLayout::new("Main", models::stream_deck_mk2().id);
    layout.explicit_tree = true;
    let home = layout.home_page;
    let mut tools = LayoutPage::folder("Tools", home);
    tools.auto_exit = Some(AutoExit {
        enabled: auto_exit,
        seconds: 10,
    });
    tools.auto_return = exit_after;
    let workflow = |workflow| BindingTarget::Workflow { workflow };
    tools.bindings.extend([
        press(3, workflow(ids.0)),
        press(4, workflow(ids.2)),
        press(6, workflow(ids.1)),
        press(7, workflow(ids.3)),
    ]);
    let tools_id = tools.id;
    layout
        .page_mut(home)
        .unwrap()
        .bindings
        .push(press(5, BindingTarget::OpenPage { page: tools_id }));
    layout.pages.push(tools);
    profile.layouts.push(layout);
    Fixture {
        profile,
        long: ids.0,
        other: ids.1,
        quick: ids.2,
        failing: ids.3,
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
    let dir = TempDir::new("deckpilot-folder-exit");
    let (paths, db) = support::data_dir(&dir, &f.profile, &[]);
    let mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), "FOLDER-1");
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
    deck.wait_page("Home").await;
    deck
}

impl Deck {
    async fn wait_status(&self, what: &str, predicate: impl Fn(&AgentStatus) -> bool) {
        let deadline = tokio::time::Instant::now() + TIMEOUT;
        loop {
            let status = self.agent.status().await;
            if predicate(&status) {
                return;
            }
            assert!(
                tokio::time::Instant::now() <= deadline,
                "{what}: {:?}",
                status.devices
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn wait_page(&self, name: &str) {
        self.wait_status(&format!("the device shows {name}"), |s| {
            s.devices.iter().any(|d| {
                d.id == self.device
                    && d.state == ConnectionState::Connected
                    && d.page.as_deref() == Some(name)
            })
        })
        .await;
    }

    async fn page(&self) -> Option<String> {
        let status = self.agent.status().await;
        status
            .devices
            .into_iter()
            .find(|d| d.id == self.device)
            .and_then(|d| d.page)
    }

    /// Moves the countdowns' clock; a request lets the agent's loop look
    /// at its folders on the moved clock.
    async fn advance(&self, seconds: u64) {
        self.clock.advance(Duration::from_secs(seconds));
        self.agent.status().await;
    }

    /// Waits until the protected Back key shows `countdown`.
    fn back_shows(&self, countdown: Option<FolderCountdown>) {
        let spec = models::stream_deck_mk2()
            .capabilities
            .keys
            .and_then(|keys| keys.image)
            .unwrap();
        let image =
            deckpilot_agent::render::render(&folder_back_face(countdown), spec.width, spec.height);
        assert!(
            self.mock
                .wait_for(&self.device, TIMEOUT, |frames, _| frames.get(&key(0))
                    == Some(&image)),
            "the Back key shows {countdown:?}"
        );
    }

    async fn run_state(&self, workflow: WorkflowId) -> Option<ExecutionState> {
        self.agent.runs(workflow).await.first().map(|r| r.state)
    }

    async fn wait_run(&self, workflow: WorkflowId, state: ExecutionState) {
        self.agent
            .wait_for_runs(workflow, |runs| {
                runs.first().is_some_and(|r| r.state == state)
            })
            .await;
    }

    /// Waits until the run of `workflow` is in its delay: its first step is
    /// over, the run is not.
    async fn in_delay(&self, workflow: WorkflowId) {
        let run = self
            .agent
            .wait_for_runs(workflow, |runs| !runs.is_empty())
            .await[0]
            .id;
        let deadline = tokio::time::Instant::now() + TIMEOUT;
        loop {
            let record = self.agent.execution(run).await;
            if record.summary.completed_nodes >= 1 {
                assert_eq!(record.summary.state, ExecutionState::Running);
                return;
            }
            assert!(tokio::time::Instant::now() <= deadline, "{record:#?}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn cancel(&self, workflow: WorkflowId) {
        let execution = self.agent.runs(workflow).await[0].id;
        self.agent
            .request(Request::CancelExecution { execution })
            .await
            .unwrap();
    }

    fn tap(&self, index: u16) {
        self.mock.tap(&self.device, key(index));
    }
}

#[allow(
    clippy::unnecessary_wraps,
    reason = "what the Back key shows: `None` is the plain key"
)]
fn counting(seconds: u32, paused: bool) -> Option<FolderCountdown> {
    Some(FolderCountdown { seconds, paused })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_back_key_counts_down_pauses_for_a_whole_run_and_resumes_with_what_was_left() {
    let f = fixture(false, true);
    let deck = start(&f).await;
    deck.tap(5);
    deck.wait_page("Tools").await;
    deck.back_shows(counting(10, false));
    // Once a second, on the moved clock.
    deck.advance(1).await;
    deck.back_shows(counting(9, false));
    deck.advance(2).await;
    deck.back_shows(counting(7, false));
    // A Multi Action starts at 3 s: paused with 7 s, for the whole run.
    let writes = deck.mock.control_writes(&deck.device);
    deck.tap(3);
    deck.back_shows(counting(7, true));
    deck.in_delay(f.long).await;
    deck.advance(20).await;
    assert_eq!(deck.page().await.as_deref(), Some("Tools"));
    deck.back_shows(counting(7, true));
    // Only the Back key (and the run's own key) were drawn again.
    let now = deck.mock.control_writes(&deck.device);
    let redrawn: Vec<_> = now
        .iter()
        .filter(|(control, count)| writes.get(control) != Some(count))
        .map(|(control, _)| *control)
        .collect();
    assert!(
        redrawn.iter().all(|c| *c == key(0) || *c == key(3)),
        "{redrawn:?}"
    );
    // Cancelled: over; the countdown goes on from 7 s.
    deck.cancel(f.long).await;
    deck.wait_run(f.long, ExecutionState::Cancelled).await;
    deck.back_shows(counting(7, false));
    assert_eq!(deck.page().await.as_deref(), Some("Tools"));
    deck.advance(6).await;
    deck.back_shows(counting(1, false));
    assert_eq!(deck.page().await.as_deref(), Some("Tools"));
    deck.advance(1).await;
    deck.wait_page("Home").await;
    deck.agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exit_after_completion_waits_for_the_last_step_and_any_end() {
    let f = fixture(true, false);
    let deck = start(&f).await;
    // No Auto Exit: the plain Back key.
    deck.tap(5);
    deck.wait_page("Tools").await;
    deck.back_shows(None);
    // Its first step done, the run goes on: the folder stays.
    deck.tap(3);
    deck.in_delay(f.long).await;
    deck.advance(600).await;
    assert_eq!(deck.page().await.as_deref(), Some("Tools"));
    deck.back_shows(None);
    // Cancelled is an end: the folder is left, the run is recorded.
    deck.cancel(f.long).await;
    deck.wait_page("Home").await;
    assert_eq!(
        deck.run_state(f.long).await,
        Some(ExecutionState::Cancelled)
    );
    // Succeeded.
    deck.tap(5);
    deck.wait_page("Tools").await;
    deck.tap(4);
    deck.wait_run(f.quick, ExecutionState::Succeeded).await;
    deck.wait_page("Home").await;
    // Failed, with the failure in the history.
    deck.tap(5);
    deck.wait_page("Tools").await;
    deck.tap(7);
    deck.wait_run(f.failing, ExecutionState::Failed).await;
    deck.wait_page("Home").await;
    deck.agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_runs_hold_the_folder_until_the_last_and_a_refused_one_stays() {
    let f = fixture(true, true);
    let deck = start(&f).await;
    deck.tap(5);
    deck.wait_page("Tools").await;
    deck.advance(2).await;
    deck.tap(3);
    deck.in_delay(f.long).await;
    deck.tap(6);
    deck.in_delay(f.other).await;
    deck.back_shows(counting(8, true));
    // One of two over: still paused, still inside.
    deck.cancel(f.long).await;
    deck.wait_run(f.long, ExecutionState::Cancelled).await;
    deck.advance(30).await;
    assert_eq!(deck.page().await.as_deref(), Some("Tools"));
    deck.back_shows(counting(8, true));
    // The last one: left right away, nothing resumes.
    deck.cancel(f.other).await;
    deck.wait_page("Home").await;
    // A run that cannot start (the workflow runs already, started from the
    // app): the device stays and the countdown is not paused by it.
    let running = deck.agent.start_workflow(f.long).await.unwrap();
    deck.tap(5);
    deck.wait_page("Tools").await;
    deck.back_shows(counting(10, false));
    deck.tap(3);
    deck.advance(1).await;
    deck.back_shows(counting(9, false));
    assert_eq!(deck.page().await.as_deref(), Some("Tools"));
    // Nor does the app's run, which no key of the folder started.
    deck.agent
        .request(Request::CancelExecution {
            execution: running.id,
        })
        .await
        .unwrap();
    deck.wait_run(f.long, ExecutionState::Cancelled).await;
    assert_eq!(deck.page().await.as_deref(), Some("Tools"));
    // The time alone still closes it.
    deck.advance(9).await;
    deck.wait_page("Home").await;
    deck.agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn leaving_by_hand_while_a_run_goes_on_leaves_it_unable_to_navigate() {
    let f = fixture(true, true);
    let deck = start(&f).await;
    deck.tap(5);
    deck.wait_page("Tools").await;
    deck.tap(3);
    deck.in_delay(f.long).await;
    // The protected Back, then into the folder again.
    deck.tap(0);
    deck.wait_page("Home").await;
    deck.tap(5);
    deck.wait_page("Tools").await;
    deck.back_shows(counting(10, false));
    // The old run's end does nothing to the new entry.
    deck.cancel(f.long).await;
    deck.wait_run(f.long, ExecutionState::Cancelled).await;
    assert_eq!(deck.page().await.as_deref(), Some("Tools"));
    deck.back_shows(counting(10, false));
    // A disconnect cancels the countdown; back, the device starts none.
    deck.tap(3);
    deck.in_delay(f.long).await;
    deck.mock.unplug(&deck.device);
    deck.wait_status("the device is gone", |s| {
        s.devices
            .iter()
            .all(|d| d.id != deck.device || d.state != ConnectionState::Connected)
    })
    .await;
    deck.cancel(f.long).await;
    deck.wait_run(f.long, ExecutionState::Cancelled).await;
    let device = deck.mock.plug(&models::stream_deck_mk2(), "FOLDER-1");
    assert_eq!(device, deck.device);
    deck.wait_page("Tools").await;
    deck.back_shows(None);
    deck.advance(60).await;
    assert_eq!(deck.page().await.as_deref(), Some("Tools"));
    deck.agent.shutdown().await;
}
