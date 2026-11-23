//! `deckpilot.workflows` end to end through the real agent and host
//! (feature `plugins`; see `support/first_party.rs`): a manifest-only
//! plugin, `default` class as `bundled.json` releases it.
//!
//! * `a_fresh_install_gets_the_workflows_screen`: seeded and enabled at
//!   the first start with its one low-tier permission, no actions, and the
//!   `workflows` view and its navigation entry listed, translated;
//! * `workflows_run_without_the_plugin`: the engine stays in the core:
//!   with the plugin disabled its view is no longer listed (host views
//!   only while enabled), and a workflow still runs from the app;
//! * `keys_keep_graph_workflows_without_running_them_while_disabled`:
//!   disabled plugins preserve bindings without executing them. Keys
//!   bound to a graph workflow are kept, do not run and say why (code
//!   `workflowPluginOff`); a Multi Action (a step workflow, the core's)
//!   keeps running; enabling the plugin again restores the runs.
#![cfg(feature = "plugins")]

#[path = "support/first_party.rs"]
mod first_party;
mod support;

use deckpilot_core::{
    BindingTarget, ControlBinding, DeviceLayout, ExecutionState, ExecutionTrigger, OnFailure,
    Profile, Trigger, Workflow, WorkflowId,
};
use deckpilot_device::{ControlId, ControlInput, DeviceId};
use deckpilot_ipc::plugins::{PermissionStatus, PermissionTier, PluginState};
use deckpilot_plugin_bundled::BundleClass;
use first_party::{Env, FirstParty};

const PLUGIN: FirstParty = FirstParty {
    id: "deckpilot.workflows",
    released: "default",
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_fresh_install_gets_the_workflows_screen() {
    let env = Env::new(
        "plugins-workflows",
        PLUGIN,
        BundleClass::Default,
        None,
        None,
    );
    let agent = env.start().await;
    let summary = agent.wait_state(PluginState::Enabled).await;
    assert!(summary.actions.is_empty(), "{summary:?}");
    assert_eq!(summary.name, "Workflows");
    let views: Vec<(String, String, String)> = summary
        .navigation
        .iter()
        .map(|e| (e.id.clone(), e.view.kind.clone(), e.label.clone()))
        .collect();
    assert_eq!(
        views,
        [(
            "workflows".to_owned(),
            "workflows".to_owned(),
            "Workflows".to_owned()
        )]
    );
    let details = agent.details().await;
    let permissions: Vec<(String, PermissionTier, PermissionStatus)> = details
        .permissions
        .iter()
        .map(|p| (p.key.clone(), p.tier, p.status))
        .collect();
    assert_eq!(
        permissions,
        [(
            "hostViews".to_owned(),
            PermissionTier::Low,
            PermissionStatus::Granted
        )]
    );
    let justification = details.permissions[0].justification.as_deref().unwrap();
    assert!(!justification.starts_with("t:"), "{justification}");
    // Nothing to invoke.
    let refused = agent
        .request(deckpilot_ipc::Request::InvokePluginAction {
            plugin: PLUGIN.id.into(),
            action: "run".into(),
            params: std::collections::BTreeMap::default(),
        })
        .await;
    assert!(
        !matches!(
            refused,
            Ok(deckpilot_ipc::Response::PluginActionResult {
                outcome: deckpilot_ipc::plugins::PluginActionOutcome::Ok { .. }
            })
        ),
        "{refused:?}"
    );
    agent.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn workflows_run_without_the_plugin() {
    let mut profile = Profile::new("Work");
    let workflow = support::waiting_workflow("Short", 10);
    let id = workflow.id;
    profile.workflows.push(workflow);
    let env = Env::new(
        "plugins-workflows-off",
        PLUGIN,
        BundleClass::Default,
        Some((&profile, None)),
        None,
    );
    let agent = env.start().await;
    agent.wait_state(PluginState::Enabled).await;
    agent.disable().await;
    let summary = agent.summary().await.unwrap();
    assert_eq!(summary.state, PluginState::Disabled);
    // The app's own Workflows screen is listed only while the plugin is
    // enabled: disabled, it has no entry (plugin API 1.9, `hostViews`).
    assert_eq!(summary.navigation, Vec::new(), "{summary:?}");
    let record = agent.start_workflow(id).await;
    assert_eq!(
        record.summary.state,
        ExecutionState::Succeeded,
        "{record:#?}"
    );
    // A restart keeps the user's decision.
    agent.stop().await;
    let agent = env.start().await;
    let summary = agent.seeded().await;
    assert_eq!(summary.state, PluginState::Disabled, "{}", agent.log());
    agent.enable().await;
    let summary = agent.wait_state(PluginState::Enabled).await;
    assert_eq!(summary.navigation.len(), 1, "{summary:?}");
    agent.stop().await;
}

/// The runs of `workflow` once `count` exist, all finished.
async fn runs(
    agent: &first_party::Agent,
    workflow: WorkflowId,
    count: usize,
) -> Vec<ExecutionTrigger> {
    let deadline = std::time::Instant::now() + support::TIMEOUT;
    loop {
        let Ok(deckpilot_ipc::Response::Executions { executions }) = agent
            .request(deckpilot_ipc::Request::ListExecutions {
                workflow: Some(workflow),
                limit: None,
            })
            .await
        else {
            panic!("executions")
        };
        if executions.len() == count
            && executions
                .iter()
                .all(|r| r.state != ExecutionState::Running)
        {
            return executions.iter().map(|r| r.trigger).collect();
        }
        assert!(
            executions.len() <= count && std::time::Instant::now() < deadline,
            "{executions:#?}\n{}",
            agent.log()
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

async fn tap(agent: &first_party::Agent, device: &DeviceId, index: u16) {
    for input in [ControlInput::Press, ControlInput::Release] {
        agent
            .ok(deckpilot_ipc::Request::SimulateInput {
                device: device.clone(),
                control: ControlId::Key { index },
                input,
            })
            .await;
    }
}

/// The next status bar notification with a code.
async fn told(events: &mut deckpilot_ipc::AgentClient) -> deckpilot_core::notify::Notification {
    loop {
        let event = tokio::time::timeout(support::TIMEOUT, events.next_event())
            .await
            .expect("a notification in time")
            .unwrap();
        if let deckpilot_ipc::AgentEvent::Notification { notification } = event
            && notification.code.is_some()
        {
            return notification;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn keys_keep_graph_workflows_without_running_them_while_disabled() {
    let mut profile = Profile::new("Work");
    let graph = support::waiting_workflow("Graph", 10);
    let log = support::registry()
        .create_instance(
            "Logged",
            deckpilot_modules::general::log_id(),
            &deckpilot_core::ActionParameters::new().with(
                "message",
                deckpilot_core::ParameterValue::Text("multi".into()),
            ),
        )
        .unwrap();
    let multi = Workflow::new("Multi").step(log.id, OnFailure::Stop);
    let (graph_id, multi_id) = (graph.id, multi.id);
    profile.actions.push(log);
    profile.workflows.extend([graph, multi]);
    let mut layout = DeviceLayout::new("Main", deckpilot_device::models::stream_deck_mk2().id);
    for (index, workflow) in [(0, graph_id), (1, multi_id)] {
        layout.pages[0].bindings.push(ControlBinding::new(
            ControlId::Key { index },
            Trigger::Press,
            BindingTarget::Workflow { workflow },
        ));
    }
    profile.layouts.push(layout);
    let mut env = Env::new(
        "plugins-workflows-keys",
        PLUGIN,
        BundleClass::Default,
        Some((&profile, None)),
        None,
    );
    env.extra_args = vec!["--mock-devices", "mk2"];
    let agent = env.start().await;
    agent.wait_state(PluginState::Enabled).await;
    let device = loop {
        let deckpilot_ipc::Response::Status { status } =
            agent.ok(deckpilot_ipc::Request::GetStatus).await
        else {
            panic!("status")
        };
        if let Some(device) = status.devices.iter().find(|d| d.page.is_some()) {
            break device.id.clone();
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };
    let mut events = agent.app_events().await;

    // Enabled: both keys run.
    tap(&agent, &device, 0).await;
    assert_eq!(runs(&agent, graph_id, 1).await, [ExecutionTrigger::Device]);
    tap(&agent, &device, 1).await;
    runs(&agent, multi_id, 1).await;

    // Disabled: the graph workflow's key is kept, does not run, and says
    // why; the Multi Action is the core's and runs.
    agent.disable().await;
    tap(&agent, &device, 0).await;
    let notification = told(&mut events).await;
    assert_eq!(
        notification.code.as_deref(),
        Some("workflowPluginOff"),
        "{notification:?}"
    );
    assert!(
        notification.body.contains("Workflows plugin"),
        "{notification:?}"
    );
    tap(&agent, &device, 1).await;
    runs(&agent, multi_id, 2).await;
    assert_eq!(runs(&agent, graph_id, 1).await.len(), 1);
    // Nothing was removed from the profile.
    let store = support::open_store(&env.paths, &env.key);
    let stored = deckpilot_storage::ConfigStore::load_profile(&store, profile.id)
        .unwrap()
        .unwrap();
    assert_eq!(stored.layouts[0].pages[0].bindings.len(), 2);
    assert!(stored.workflow(graph_id).is_some());

    // Enabled again: the key runs again.
    agent.enable().await;
    tap(&agent, &device, 0).await;
    runs(&agent, graph_id, 2).await;
    agent.stop().await;
}
