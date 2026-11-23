//! Keys that start workflows (`docs/data-model.md`, "Workflows"), against
//! a real agent with a simulated MK.2: one workflow on several keys, each
//! press re-resolving it; a rename reaching every key; a deleted workflow
//! shown as "Workflow missing" and another profile's (a shared layout) as
//! "Not in this profile", neither run; refused starts told to the user
//! once, with a stable code, instead of only being logged; and a profile
//! switch never running another profile's workflow.

mod support;

use deckpilot_core::i18n::SystemText;
use deckpilot_core::notify::Notification;
use deckpilot_core::{
    BindingTarget, ControlBinding, DeviceLayout, ExecutionTrigger, FaceStyle, KeyFace, Profile,
    Trigger, WorkflowId,
};
use deckpilot_device::mock::MockBackend;
use deckpilot_device::{ConnectionState, ControlId, DeviceId, Frame, models};
use deckpilot_ipc::{AgentClient, AgentEvent, Request};
use deckpilot_storage::ConfigStore;
use support::{TIMEOUT, TempDir, TestAgent};

fn key(index: u16) -> ControlId {
    ControlId::Key { index }
}

fn starts(index: u16, workflow: WorkflowId) -> ControlBinding {
    ControlBinding::new(
        key(index),
        Trigger::Press,
        BindingTarget::Workflow { workflow },
    )
}

/// A short graph workflow: `start → wait 10 ms`.
fn short(name: &str) -> deckpilot_core::Workflow {
    support::waiting_workflow(name, 10)
}

fn frame(face: &KeyFace) -> Frame {
    let spec = models::stream_deck_mk2()
        .capabilities
        .keys
        .and_then(|keys| keys.image)
        .unwrap();
    deckpilot_agent::render::render_with(
        face,
        spec.width,
        spec.height,
        &deckpilot_agent::render::Icons::default(),
    )
}

/// The face of a key that starts the workflow `title`.
fn workflow_face(title: &str) -> Frame {
    frame(
        &KeyFace::new(FaceStyle::Workflow, title)
            .with_symbol(Some(deckpilot_core::device_runtime::symbols::WORKFLOW)),
    )
}

/// The face of a key whose workflow cannot run here: "Workflow", `detail`.
fn notice_face(detail: SystemText) -> Frame {
    frame(&KeyFace::system(FaceStyle::Notice, SystemText::Workflow).with_system_detail(detail))
}

fn shows(mock: &MockBackend, device: &DeviceId, index: u16, expected: &Frame) -> bool {
    mock.wait_for(device, TIMEOUT, |frames, _| {
        frames.get(&key(index)) == Some(expected)
    })
}

async fn connected(agent: &TestAgent, device: &DeviceId) {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        let status = agent.status().await;
        if status
            .devices
            .iter()
            .any(|d| d.id == *device && d.state == ConnectionState::Connected && d.page.is_some())
        {
            return;
        }
        assert!(tokio::time::Instant::now() < deadline, "{status:#?}");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

/// The next notification for the app's status bar.
async fn notification(events: &mut AgentClient) -> Notification {
    loop {
        let event = tokio::time::timeout(TIMEOUT, events.next_event())
            .await
            .expect("a notification in time")
            .unwrap();
        if let AgentEvent::Notification { notification } = event {
            return notification;
        }
    }
}

/// Waits until `workflow` has `count` runs, all over.
async fn finished_runs(agent: &TestAgent, workflow: WorkflowId, count: usize) {
    agent
        .wait_for_runs(workflow, |runs| {
            runs.len() == count
                && runs
                    .iter()
                    .all(|r| r.state != deckpilot_core::ExecutionState::Running)
        })
        .await;
}

/// No notification within a short while.
async fn quiet(events: &mut AgentClient) {
    let wait = std::time::Duration::from_millis(300);
    let deadline = tokio::time::Instant::now() + wait;
    while let Ok(event) = tokio::time::timeout_at(deadline, events.next_event()).await {
        if let AgentEvent::Notification { notification } = event.unwrap() {
            panic!("unexpected {notification:?}");
        }
    }
}

struct Setup {
    work: Profile,
    home: Profile,
    deploy: WorkflowId,
    needs: WorkflowId,
    slow: WorkflowId,
    homes: WorkflowId,
}

/// Work: Deploy on keys 0 and 1 (key 1 titled "Ship"), a deleted
/// workflow on key 2, Needs (a required input) on key 3, Slow on key 4,
/// Home's own workflow on key 5. Home shares Work's layout and has a
/// workflow of its own.
fn setup() -> Setup {
    let mut work = Profile::new("Work");
    let deploy = short("Deploy");
    let mut needs = short("Needs");
    needs.inputs.push(deckpilot_core::graph::WorkflowInput {
        required: true,
        ..deckpilot_core::graph::WorkflowInput::new(
            "target",
            deckpilot_core::graph::ValueKind::Text,
        )
    });
    let slow = support::waiting_workflow("Slow", 5_000);
    let (deploy_id, needs_id, slow_id) = (deploy.id, needs.id, slow.id);
    work.workflows.extend([deploy, needs, slow]);
    let mut home = Profile::new("Home");
    let own = short("Home only");
    let homes = own.id;
    home.workflows.push(own);
    let mut layout = DeviceLayout::new("Main", models::stream_deck_mk2().id);
    let mut titled = starts(1, deploy_id);
    titled.label = Some("Ship".into());
    layout.pages[0].bindings.extend([
        starts(0, deploy_id),
        titled,
        starts(2, WorkflowId::new()),
        starts(3, needs_id),
        starts(4, slow_id),
        starts(5, homes),
    ]);
    work.layouts.push(layout);
    Setup {
        work,
        home,
        deploy: deploy_id,
        needs: needs_id,
        slow: slow_id,
        homes,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // one scenario, step by step
async fn one_workflow_on_several_keys_and_refused_starts_are_told() {
    let dir = TempDir::new("deckpilot-workflow-keys");
    let Setup {
        work,
        home,
        deploy,
        needs,
        slow,
        homes,
    } = setup();
    let (paths, db_key) = support::data_dir(&dir, &work, &[]);
    {
        let store = support::open_store(&paths, &db_key);
        store.save_profile(&home).unwrap();
        store.assign_layout(home.id, work.layouts[0].id).unwrap();
    }
    let mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), "MK2-A");
    let agent = TestAgent::start_unlocked(&paths, &db_key, &mock, None).await;
    connected(&agent, &device).await;
    let mut events = agent.events().await;

    // The keys show the workflow by name, or by their own title.
    assert!(shows(&mock, &device, 0, &workflow_face("Deploy")), "key 0");
    assert!(shows(&mock, &device, 1, &workflow_face("Ship")), "key 1");
    // Deleted: "Workflow missing". Home's, in the shared layout: "Not in
    // this profile".
    assert!(
        shows(&mock, &device, 2, &notice_face(SystemText::WorkflowMissing)),
        "the deleted workflow"
    );
    assert!(
        shows(
            &mock,
            &device,
            5,
            &notice_face(SystemText::NotInThisProfile)
        ),
        "another profile's workflow"
    );

    // Both keys start the one workflow, as pressed on the device.
    mock.tap(&device, key(0));
    finished_runs(&agent, deploy, 1).await;
    mock.tap(&device, key(1));
    finished_runs(&agent, deploy, 2).await;
    let runs = agent.runs(deploy).await;
    assert!(runs.iter().all(|r| r.trigger == ExecutionTrigger::Device));

    // A deleted workflow does not run; the user is told once.
    mock.tap(&device, key(2));
    let told = notification(&mut events).await;
    assert_eq!(told.code.as_deref(), Some("workflowMissing"), "{told:?}");
    assert!(told.title.contains("did not start"), "{told:?}");
    assert!(!told.desktop && told.status_bar);
    mock.tap(&device, key(2));
    quiet(&mut events).await;

    // Another profile's workflow never runs here.
    mock.tap(&device, key(5));
    let told = notification(&mut events).await;
    assert_eq!(told.code.as_deref(), Some("workflowNotInProfile"));
    assert_eq!(agent.runs(homes).await.len(), 0);

    // A required input: it runs only from a Sub-workflow node.
    mock.tap(&device, key(3));
    let told = notification(&mut events).await;
    assert_eq!(
        told.code.as_deref(),
        Some("workflowNeedsInputs"),
        "{told:?}"
    );
    assert!(told.body.contains("Sub-workflow"), "{told:?}");
    assert_eq!(agent.runs(needs).await.len(), 0);

    // Running already.
    mock.tap(&device, key(4));
    agent
        .wait_for_runs(slow, |runs| {
            runs.len() == 1 && runs[0].state == deckpilot_core::ExecutionState::Running
        })
        .await;
    mock.tap(&device, key(4));
    let told = notification(&mut events).await;
    assert_eq!(told.code.as_deref(), Some("workflowAlreadyRunning"));
    let runs = agent.runs(slow).await;
    let _ = agent
        .request(Request::CancelExecution {
            execution: runs[0].id,
        })
        .await;

    // The app starting it is refused with the same reason, as an error.
    let refused = agent.start_workflow(WorkflowId::new()).await.unwrap_err();
    assert!(
        format!("{refused:?}").contains("workflowMissing"),
        "{refused:?}"
    );

    // A rename reaches every key without a title of its own, the id stays.
    agent
        .edit_profile(work.id, |profile| {
            profile.workflows[0].name = "Deploy all".into();
        })
        .await;
    assert!(
        shows(&mock, &device, 0, &workflow_face("Deploy all")),
        "renamed"
    );
    assert!(
        shows(&mock, &device, 1, &workflow_face("Ship")),
        "own title"
    );
    mock.tap(&device, key(0));
    finished_runs(&agent, deploy, 3).await;

    // In Home, the shared layout's keys never run Work's workflows, and
    // Home's own one runs.
    agent
        .request(Request::SetActiveProfile { profile: home.id })
        .await
        .unwrap();
    assert!(
        shows(
            &mock,
            &device,
            0,
            &notice_face(SystemText::NotInThisProfile)
        ),
        "Work's workflow in Home"
    );
    assert!(
        shows(&mock, &device, 5, &workflow_face("Home only")),
        "Home's own"
    );
    mock.tap(&device, key(0));
    let told = notification(&mut events).await;
    assert_eq!(told.code.as_deref(), Some("workflowNotInProfile"));
    mock.tap(&device, key(5));
    finished_runs(&agent, homes, 1).await;

    // Back in Work, its keys run again.
    agent
        .request(Request::SetActiveProfile { profile: work.id })
        .await
        .unwrap();
    assert!(
        shows(&mock, &device, 0, &workflow_face("Deploy all")),
        "Work again"
    );
    // Nothing of Work's ran while Home was active.
    assert_eq!(agent.runs(deploy).await.len(), 3);
    mock.tap(&device, key(0));
    finished_runs(&agent, deploy, 4).await;
    agent.shutdown().await;
}

/// The frame of the Multi Action Switch key "Switch" in the state `title`
/// at `position`.
fn switched(title: &str, position: &str) -> Frame {
    let state = deckpilot_core::ButtonState::new()
        .with_label(title)
        .with_detail(position)
        .with_glyph(deckpilot_core::Glyph::Dot);
    frame(&KeyFace::new(FaceStyle::Action, "Switch").with_state(&state))
}

/// With "advance on failure", a state whose run fails moves the switch
/// on, as before, and so does a workflow refused for a reason that lasts
/// until the user changes something (here: it needs input values); one
/// refused for a reason that passes by itself (here: it runs already)
/// keeps the state, so the next press tries it again. Both are told.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_switch_advances_past_permanent_refusals_and_holds_on_temporary_ones() {
    use deckpilot_core::{ActionParameters, ParameterValue};
    use deckpilot_modules::{deck, general};

    let dir = TempDir::new("deckpilot-workflow-keys-switch");
    let mut work = Profile::new("Work");
    let registry = support::registry();
    // A log message that needs a project fails without one.
    let fails = registry
        .create_instance(
            "Fails",
            general::log_id(),
            &ActionParameters::new().with("message", ParameterValue::Text("{project.key}".into())),
        )
        .unwrap();
    let mut needs = short("Needs");
    needs.inputs.push(deckpilot_core::graph::WorkflowInput {
        required: true,
        ..deckpilot_core::graph::WorkflowInput::new(
            "target",
            deckpilot_core::graph::ValueKind::Text,
        )
    });
    let slow = support::waiting_workflow("Slow", 5_000);
    let slow_id = slow.id;
    let states = format!(
        "action:{f} | One | dot\nworkflow:{n} | Two | dot\nworkflow:{s} | Three | dot\naction:{f} | Four | dot",
        f = fails.id,
        n = needs.id,
        s = slow.id
    );
    let switch = registry
        .create_instance(
            "Switch",
            deck::multi_switch_id(),
            &ActionParameters::new()
                .with(deck::switch::STATES, ParameterValue::Text(states))
                .with(
                    deck::switch::ADVANCE_ON_FAILURE,
                    ParameterValue::Boolean(true),
                ),
        )
        .unwrap();
    let mut layout = DeviceLayout::new("Main", models::stream_deck_mk2().id);
    layout.pages[0].bindings.push(ControlBinding::new(
        key(0),
        Trigger::Press,
        BindingTarget::action(switch.id),
    ));
    work.actions.extend([fails, switch]);
    work.workflows.extend([needs, slow]);
    work.layouts.push(layout);
    let (paths, db_key) = support::data_dir(&dir, &work, &[]);
    let mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), "MK2-A");
    let agent = TestAgent::start_unlocked(&paths, &db_key, &mock, None).await;
    connected(&agent, &device).await;
    let mut events = agent.events().await;

    assert!(shows(&mock, &device, 0, &switched("One", "1/4")), "first");
    // A run that fails: on to the next state.
    mock.tap(&device, key(0));
    assert!(
        shows(&mock, &device, 0, &switched("Two", "2/4")),
        "failed run"
    );
    // Permanent (needs inputs): told, and on to the next state.
    mock.tap(&device, key(0));
    let told = notification(&mut events).await;
    assert_eq!(
        told.code.as_deref(),
        Some("workflowNeedsInputs"),
        "{told:?}"
    );
    assert!(
        shows(&mock, &device, 0, &switched("Three", "3/4")),
        "permanent"
    );
    // Temporary (running already): told, and the state is kept.
    let running = agent.start_workflow(slow_id).await.unwrap();
    mock.tap(&device, key(0));
    let told = notification(&mut events).await;
    assert_eq!(
        told.code.as_deref(),
        Some("workflowAlreadyRunning"),
        "{told:?}"
    );
    quiet(&mut events).await;
    assert!(
        shows(&mock, &device, 0, &switched("Three", "3/4")),
        "temporary"
    );
    let _ = agent
        .request(Request::CancelExecution {
            execution: running.id,
        })
        .await;
    agent.shutdown().await;
}
