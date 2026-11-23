use std::collections::BTreeMap;

use deckpilot_device::{DeviceModelId, models};

use super::*;
use crate::graph::{
    ErrorPolicy, GRAPH_VERSION, Node, NodeId, Port, Position, ValueKind, WorkflowGraph,
    WorkflowInput,
};
use crate::testing::{echo_type, registry};
use crate::{
    ActionInstance, ActionInstanceId, ActionParameters, ControlBinding, DeviceLayout, LayoutId,
    ParameterValue,
};

fn n(id: &str) -> NodeId {
    NodeId::new(id).unwrap()
}

fn node(id: &str, kind: NodeKind) -> Node {
    Node {
        id: n(id),
        kind,
        label: String::new(),
        position: Position::default(),
    }
}

/// A graph workflow: Trigger, then `middle` (if any), then Stop.
fn graph_workflow(name: &str, middle: Option<NodeKind>) -> Workflow {
    let mut nodes = vec![node("start", NodeKind::Trigger)];
    let mut edges = Vec::new();
    let mut last = "start";
    if let Some(kind) = middle {
        nodes.push(node("middle", kind));
        edges.push(crate::graph::Edge {
            from: n(last),
            port: Port::Out,
            to: n("middle"),
        });
        last = "middle";
    }
    nodes.push(node("end", NodeKind::Stop));
    edges.push(crate::graph::Edge {
        from: n(last),
        port: Port::Out,
        to: n("end"),
    });
    let mut workflow = Workflow::new(name);
    workflow.graph = Some(WorkflowGraph {
        version: GRAPH_VERSION,
        nodes,
        edges,
    });
    workflow
}

fn action(name: &str, parameters: ActionParameters) -> ActionInstance {
    ActionInstance {
        id: ActionInstanceId::new(),
        name: name.into(),
        action_type: echo_type(),
        parameters,
        description: String::new(),
        icon: None,
        label: None,
    }
}

fn key(index: u16) -> ControlId {
    ControlId::Key { index }
}

fn layout() -> DeviceLayout {
    DeviceLayout::new("Main", DeviceModelId::new(models::STREAM_DECK_MK2_ID))
}

fn bind(layout: &mut DeviceLayout, control: ControlId, trigger: Trigger, workflow: WorkflowId) {
    layout.pages[0].bindings.push(ControlBinding::new(
        control,
        trigger,
        BindingTarget::Workflow { workflow },
    ));
}

/// A profile whose workflow "Deploy" is used everywhere a workflow can
/// be: two keys, a long press, a Call and a Loop node, a Random Action's
/// candidates, a Switch's states and a key's overrides of a Random Action.
fn referenced() -> (Profile, WorkflowId) {
    let mut profile = Profile::new("Work");
    let deploy = graph_workflow("Deploy", None);
    let id = deploy.id;
    profile.workflows.push(deploy);
    let caller = graph_workflow(
        "Release",
        Some(NodeKind::Call {
            workflow: id,
            inputs: BTreeMap::new(),
            policy: ErrorPolicy::default(),
        }),
    );
    profile.workflows.push(caller);
    let looper = graph_workflow(
        "Nightly",
        Some(NodeKind::Loop {
            workflow: id,
            times: 2,
        }),
    );
    profile.workflows.push(looper);
    let candidates = format!("workflow:{id} 2\naction:{}", ActionInstanceId::new());
    profile.actions.push(action(
        "Random",
        ActionParameters::new().with("message", ParameterValue::Text(candidates)),
    ));
    profile.actions.push(action(
        "Switch",
        ActionParameters::new().with("message", ParameterValue::Text(format!("workflow:{id}"))),
    ));
    let plain = action("Plain", ActionParameters::new());
    let plain_id = plain.id;
    profile.actions.push(plain);
    let mut layout = layout();
    bind(&mut layout, key(0), Trigger::Press, id);
    bind(&mut layout, key(1), Trigger::Press, id);
    bind(&mut layout, key(2), Trigger::LongPress, id);
    layout.pages[0].bindings.push(ControlBinding::new(
        key(3),
        Trigger::Press,
        BindingTarget::Action {
            action: plain_id,
            overrides: ActionParameters::new()
                .with("message", ParameterValue::Text(format!("workflow:{id}"))),
        },
    ));
    profile.layouts.push(layout);
    (profile, id)
}

fn kinds(usages: &[Usage]) -> Vec<&'static str> {
    usages
        .iter()
        .map(|usage| match usage {
            Usage::WorkflowStep { .. } => "step",
            Usage::LayoutBinding { .. } => "binding",
            Usage::WorkflowNode { .. } => "node",
            Usage::ActionParameter { .. } => "parameter",
            Usage::WorkSessionSetting { .. } => "workSession",
        })
        .collect()
}

#[test]
fn every_kind_of_reference_is_counted() {
    let (profile, id) = referenced();
    let usages = profile.workflow_references(id);
    assert_eq!(
        kinds(&usages),
        [
            "binding",
            "binding",
            "binding",
            "parameter",
            "node",
            "node",
            "parameter",
            "parameter"
        ],
        "{usages:#?}"
    );
    // Gestures are told apart from presses.
    assert!(usages.iter().any(|u| matches!(
        u,
        Usage::LayoutBinding { control, trigger: Some(Trigger::LongPress), .. } if *control == key(2)
    )));
    assert!(usages.iter().any(|u| matches!(
        u,
        Usage::WorkflowNode { workflow_name, node, .. } if workflow_name == "Release" && node.as_str() == "middle"
    )));
    assert!(usages.iter().any(|u| matches!(
        u,
        Usage::ActionParameter { action_name, .. } if action_name == "Switch"
    )));
    // The old list counts the keys only; it is kept as it was.
    assert_eq!(profile.workflow_usages(id).len(), 3);
}

#[test]
fn a_workflow_used_only_by_nodes_or_actions_is_not_deleted_silently() {
    // Before, only key bindings counted: these deleted silently.
    let (mut profile, id) = referenced();
    for layout in &mut profile.layouts {
        layout.pages[0].bindings.clear();
    }
    assert_eq!(profile.delete_workflow(id), Err(ProfileIssue::InUse(4)));
    // Only a Call node.
    profile.actions.clear();
    profile.workflows.retain(|w| w.name != "Nightly");
    assert_eq!(profile.delete_workflow(id), Err(ProfileIssue::InUse(1)));
    profile.workflows.retain(|w| w.name != "Release");
    assert_eq!(profile.delete_workflow(id).unwrap().id, id);
}

#[test]
fn a_workflow_calling_itself_is_not_its_own_reference() {
    let mut profile = Profile::new("Work");
    let mut workflow = graph_workflow("Again", None);
    let id = workflow.id;
    workflow.graph.as_mut().unwrap().nodes.push(node(
        "self",
        NodeKind::Loop {
            workflow: id,
            times: 1,
        },
    ));
    profile.workflows.push(workflow);
    assert_eq!(profile.workflow_references(id), Vec::new());
}

#[test]
fn the_fingerprint_names_the_places_not_their_names_or_order() {
    let (mut profile, id) = referenced();
    let before = profile.workflow_fingerprint(id);
    assert_eq!(before, profile.workflow_fingerprint(id));
    // Renaming a layout or reordering keeps it.
    profile.layouts[0].name = "Renamed".into();
    profile.layouts[0].pages[0].bindings.reverse();
    assert_eq!(profile.workflow_fingerprint(id), before);
    // Another key changes it.
    bind(&mut profile.layouts[0], key(5), Trigger::Press, id);
    assert_ne!(profile.workflow_fingerprint(id), before);
    // As does the same key on another trigger.
    let (mut other, other_id) = referenced();
    let original = other.workflow_fingerprint(other_id);
    other.layouts[0].pages[0].bindings[0].trigger = Trigger::Click;
    assert_ne!(other.workflow_fingerprint(other_id), original);
    // Nothing refers to it: a fingerprint of its own.
    let empty = ReferenceFingerprint::of(&[]);
    assert_eq!(empty, ReferenceFingerprint::of(&[]));
    assert_ne!(empty, before);
}

#[test]
fn a_confirmed_delete_keeps_the_keys_unless_asked_and_never_edits_graphs_or_parameters() {
    let (mut profile, id) = referenced();
    let fingerprint = profile.workflow_fingerprint(id);
    let graphs = profile.workflows[1..].to_vec();
    let actions = profile.actions.clone();

    // Kept: the keys stay and refer to the missing workflow.
    let mut kept = profile.clone();
    let deleted = kept
        .delete_workflow_confirmed(
            id,
            &fingerprint,
            DeleteOptions::default(),
            &mut WorkSessionWorkflows::default(),
        )
        .unwrap();
    assert_eq!(deleted.workflow.id, id);
    assert_eq!(deleted.cleared, Vec::new());
    assert!(kept.workflow(id).is_none());
    assert_eq!(kept.layouts, profile.layouts);
    assert_eq!(kept.workflows, graphs);
    assert_eq!(kept.actions, actions);

    // Cleared: only the bindings that start it go.
    let deleted = profile
        .delete_workflow_confirmed(
            id,
            &fingerprint,
            DeleteOptions {
                clear_bindings: true,
                ..DeleteOptions::default()
            },
            &mut WorkSessionWorkflows::default(),
        )
        .unwrap();
    assert_eq!(kinds(&deleted.cleared), ["binding"; 3]);
    let left: Vec<ControlId> = profile.layouts[0].pages[0]
        .bindings
        .iter()
        .map(|b| b.control)
        .collect();
    assert_eq!(left, [key(3)]);
    assert_eq!(profile.workflows, graphs);
    assert_eq!(profile.actions, actions);
}

#[test]
fn a_confirmed_delete_fails_when_the_references_changed() {
    let (mut profile, id) = referenced();
    let fingerprint = profile.workflow_fingerprint(id);
    // Another key was bound meanwhile.
    bind(&mut profile.layouts[0], key(7), Trigger::Press, id);
    let before = profile.clone();
    assert_eq!(
        profile.delete_workflow_confirmed(
            id,
            &fingerprint,
            DeleteOptions {
                clear_bindings: true,
                ..DeleteOptions::default()
            },
            &mut WorkSessionWorkflows::default()
        ),
        Err(ProfileIssue::ReferencesChanged)
    );
    assert_eq!(profile, before);
    // A key went away meanwhile.
    let (mut profile, id) = referenced();
    let fingerprint = profile.workflow_fingerprint(id);
    profile.layouts[0].pages[0].bindings.remove(0);
    assert_eq!(
        profile.delete_workflow_confirmed(
            id,
            &fingerprint,
            DeleteOptions::default(),
            &mut WorkSessionWorkflows::default()
        ),
        Err(ProfileIssue::ReferencesChanged)
    );
    // A stale fingerprint of nothing.
    assert_eq!(
        profile.delete_workflow_confirmed(
            id,
            &ReferenceFingerprint::of(&[]),
            DeleteOptions::default(),
            &mut WorkSessionWorkflows::default()
        ),
        Err(ProfileIssue::ReferencesChanged)
    );
    assert_eq!(
        profile.delete_workflow_confirmed(
            WorkflowId::new(),
            &fingerprint,
            DeleteOptions::default(),
            &mut WorkSessionWorkflows::default()
        ),
        Err(ProfileIssue::NotFound("workflow"))
    );
}

#[test]
fn an_unreferenced_workflow_is_deleted_with_the_empty_fingerprint() {
    let mut profile = Profile::new("Work");
    let workflow = graph_workflow("Alone", None);
    let id = workflow.id;
    profile.workflows.push(workflow);
    let deleted = profile
        .delete_workflow_confirmed(
            id,
            &ReferenceFingerprint::of(&[]),
            DeleteOptions {
                clear_bindings: true,
                ..DeleteOptions::default()
            },
            &mut WorkSessionWorkflows::default(),
        )
        .unwrap();
    assert_eq!(deleted.workflow.id, id);
}

#[test]
fn usages_serialize_with_their_new_kinds_and_trigger() {
    let (profile, id) = referenced();
    let json = serde_json::to_value(profile.workflow_references(id)).unwrap();
    assert_eq!(json[0]["type"], "layoutBinding");
    assert_eq!(json[0]["trigger"], serde_json::json!({ "type": "press" }));
    assert_eq!(json[4]["type"], "workflowNode");
    assert_eq!(json[4]["workflowName"], "Release");
    assert_eq!(json[4]["node"], "middle");
    assert_eq!(json[6]["type"], "actionParameter");
    assert_eq!(json[6]["actionName"], "Random");
    // An older usage without a trigger still reads.
    let old: Usage = serde_json::from_value(serde_json::json!({
        "type": "layoutBinding", "layout": profile.layouts[0].id, "layoutName": "Main",
        "page": profile.layouts[0].home_page, "pageName": "Home",
        "control": serde_json::to_value(key(0)).unwrap(),
    }))
    .unwrap();
    assert!(matches!(old, Usage::LayoutBinding { trigger: None, .. }));
}

#[test]
fn a_workflow_with_a_required_input_is_blocked_and_an_invalid_graph_too() {
    let registry = registry();
    let mut profile = Profile::new("Work");
    let ready = graph_workflow("Ready", None);
    let mut inputs = graph_workflow("Needs", None);
    inputs.inputs.push(WorkflowInput {
        required: true,
        ..WorkflowInput::new("target", ValueKind::Text)
    });
    let mut optional = graph_workflow("Optional", None);
    optional.inputs.push(WorkflowInput {
        default: Some("x".into()),
        ..WorkflowInput::new("target", ValueKind::Text)
    });
    let mut invalid = graph_workflow("Broken", None);
    invalid.graph.as_mut().unwrap().edges.clear();
    let steps = Workflow::new("Steps");
    profile
        .workflows
        .extend([ready, inputs, optional, invalid, steps]);
    let blocks: Vec<Option<WorkflowBlockCode>> = profile
        .workflows
        .iter()
        .map(|w| workflow_block(w, &profile, &registry).map(|b| b.code))
        .collect();
    assert_eq!(
        blocks,
        [
            None,
            Some(WorkflowBlockCode::NeedsInputs),
            None,
            Some(WorkflowBlockCode::InvalidGraph),
            None
        ]
    );
    // The same verdicts as the engine's check before a run.
    let engine = crate::Engine::new(registry);
    for workflow in &profile.workflows {
        assert_eq!(
            engine.check_runnable(&profile, workflow.id).is_ok(),
            workflow_block(workflow, &profile, engine.registry()).is_none(),
            "{}",
            workflow.name
        );
    }
    let block = workflow_block(&profile.workflows[1], &profile, engine.registry()).unwrap();
    assert!(block.message.contains("Sub-workflow"), "{}", block.message);
    assert_eq!(
        serde_json::to_value(&block).unwrap()["code"],
        serde_json::json!("needsInputs")
    );
}

#[test]
fn the_scope_tells_a_deleted_workflow_from_another_profiles() {
    let profile = Profile::new("Work");
    let foreign = WorkflowId::new();
    let deleted = WorkflowId::new();
    let (shared, own) = (LayoutId::new(), LayoutId::new());
    let unknown = WorkflowScope::default();
    assert_eq!(
        unknown.absent_text(Some(shared), deleted),
        SystemText::NotInThisProfile
    );
    let scope = WorkflowScope {
        foreign: Some(BTreeMap::from([
            (shared, BTreeSet::from([foreign])),
            (own, BTreeSet::new()),
        ])),
        graphs_off: false,
    };
    assert_eq!(
        scope.absent_text(Some(shared), foreign),
        SystemText::NotInThisProfile
    );
    assert_eq!(
        scope.absent_text(Some(shared), deleted),
        SystemText::WorkflowMissing
    );
    // Decided by the key's own layout: in a layout no other profile
    // shares, the other profile's workflow is missing here.
    assert_eq!(
        scope.absent_text(Some(own), foreign),
        SystemText::WorkflowMissing
    );
    // From the app (no layout), by any layout.
    assert_eq!(
        scope.absent_text(None, foreign),
        SystemText::NotInThisProfile
    );
    assert_eq!(
        scope.refusal(&profile, Some(shared), deleted),
        Some(WorkflowRefusal::Missing)
    );
    assert_eq!(
        scope.refusal(&profile, Some(shared), foreign),
        Some(WorkflowRefusal::NotInThisProfile)
    );
    assert_eq!(
        scope.refusal(&profile, Some(own), foreign),
        Some(WorkflowRefusal::Missing)
    );
}

#[test]
fn with_the_plugin_off_graph_workflows_are_refused_and_multi_actions_are_not() {
    let mut profile = Profile::new("Work");
    let graph = graph_workflow("Graph", None);
    let steps = Workflow::new("Steps");
    let (graph_id, steps_id) = (graph.id, steps.id);
    profile.workflows.extend([graph, steps]);
    let off = WorkflowScope {
        foreign: None,
        graphs_off: true,
    };
    assert_eq!(
        off.refusal(&profile, None, graph_id),
        Some(WorkflowRefusal::PluginOff)
    );
    assert_eq!(off.refusal(&profile, None, steps_id), None);
    let on = WorkflowScope::default();
    assert_eq!(on.refusal(&profile, None, graph_id), None);
}

#[test]
fn refusal_codes_are_stable() {
    let all = [
        (WorkflowRefusal::Missing, "workflowMissing"),
        (WorkflowRefusal::NotInThisProfile, "workflowNotInProfile"),
        (WorkflowRefusal::PluginOff, "workflowPluginOff"),
        (WorkflowRefusal::Invalid, "workflowInvalid"),
        (WorkflowRefusal::NeedsInputs, "workflowNeedsInputs"),
        (WorkflowRefusal::AlreadyRunning, "workflowAlreadyRunning"),
        (WorkflowRefusal::Limit, "workflowLimit"),
    ];
    for (refusal, code) in all {
        assert_eq!(refusal.code(), code);
        assert_eq!(WorkflowRefusal::from_code(code), Some(refusal));
        assert_eq!(
            serde_json::to_value(refusal).unwrap(),
            serde_json::json!(code)
        );
    }
}

#[test]
fn saving_refuses_new_bad_workflow_bindings_and_keeps_stored_ones() {
    let registry = registry();
    let mut profile = Profile::new("Work");
    let graph = graph_workflow("Graph", None);
    let steps = Workflow::new("Steps");
    let mut needs = graph_workflow("Needs", None);
    needs.inputs.push(WorkflowInput {
        required: true,
        ..WorkflowInput::new("target", ValueKind::Text)
    });
    let other_graph = graph_workflow("Other graph", None);
    let (graph_id, steps_id, needs_id, other_id) = (graph.id, steps.id, needs.id, other_graph.id);
    profile.workflows.extend([graph, steps, needs, other_graph]);
    let foreign = WorkflowId::new();

    let mut stored = layout();
    bind(&mut stored, key(0), Trigger::Press, foreign);
    bind(&mut stored, key(1), Trigger::Press, graph_id);
    let mut edited = stored.clone();
    // Targets the stored layout does not start yet.
    bind(&mut edited, key(2), Trigger::Press, WorkflowId::new());
    bind(&mut edited, key(3), Trigger::Press, other_id);
    bind(&mut edited, key(4), Trigger::Press, steps_id);
    bind(&mut edited, key(5), Trigger::DoubleClick, needs_id);

    // Plugin on: another profile's workflow and one that needs inputs.
    let on = edited.workflow_binding_issues(Some(&stored), &profile, &registry, false);
    let summary: Vec<(ControlId, WorkflowBindingReason, bool)> =
        on.iter().map(|i| (i.control, i.reason, i.stored)).collect();
    assert_eq!(
        summary,
        [
            (key(0), WorkflowBindingReason::WorkflowNotInProfile, true),
            (key(2), WorkflowBindingReason::WorkflowNotInProfile, false),
            (key(5), WorkflowBindingReason::WorkflowNeedsInputs, false),
        ]
    );
    // Plugin off: graph workflows too, Multi Actions never.
    let off = edited.workflow_binding_issues(Some(&stored), &profile, &registry, true);
    let summary: Vec<(ControlId, WorkflowBindingReason, bool)> = off
        .iter()
        .map(|i| (i.control, i.reason, i.stored))
        .collect();
    assert_eq!(
        summary,
        [
            (key(0), WorkflowBindingReason::WorkflowNotInProfile, true),
            (key(1), WorkflowBindingReason::PluginFeatureOff, true),
            (key(2), WorkflowBindingReason::WorkflowNotInProfile, false),
            (key(3), WorkflowBindingReason::PluginFeatureOff, false),
            (key(5), WorkflowBindingReason::PluginFeatureOff, false),
        ]
    );
    // Moving a stored key to another trigger, another key or another page,
    // or duplicating its page, is no new reference: kept while the plugin
    // is off.
    let mut moved = stored.clone();
    moved.pages[0].bindings[0].trigger = Trigger::LongPress;
    moved.pages[0].bindings[1].control = key(9);
    let mut copy = moved.pages[0].clone();
    copy.id = PageId::new();
    copy.name = "Copy".into();
    moved.pages.push(copy);
    let issues = moved.workflow_binding_issues(Some(&stored), &profile, &registry, true);
    assert_eq!(issues.len(), 4);
    assert!(issues.iter().all(|i| i.stored), "{issues:#?}");
    // A workflow the layout did not start yet is new anywhere.
    bind(&mut moved, key(10), Trigger::Press, WorkflowId::new());
    let issues = moved.workflow_binding_issues(Some(&stored), &profile, &registry, true);
    assert_eq!(issues.iter().filter(|i| !i.stored).count(), 1);
    // A new layout has no stored bindings.
    let issues = stored.workflow_binding_issues(None, &profile, &registry, false);
    assert!(issues.iter().all(|i| !i.stored));
    assert_eq!(
        serde_json::to_value(&issues[0]).unwrap()["reason"],
        "workflowNotInProfile"
    );
}

#[test]
fn faces_tell_missing_foreign_and_plugin_off_apart_and_follow_renames() {
    let registry = registry();
    let mut profile = Profile::new("Work");
    let graph = graph_workflow("Deploy", None);
    let steps = Workflow::new("Steps");
    let (graph_id, steps_id) = (graph.id, steps.id);
    profile.workflows.extend([graph, steps]);
    let foreign = WorkflowId::new();
    let deleted = WorkflowId::new();
    let mut layout = layout();
    bind(&mut layout, key(0), Trigger::Press, graph_id);
    bind(&mut layout, key(1), Trigger::Press, graph_id);
    layout.pages[0].bindings[1].label = Some("Ship".into());
    bind(&mut layout, key(2), Trigger::Press, steps_id);
    bind(&mut layout, key(3), Trigger::Press, foreign);
    bind(&mut layout, key(4), Trigger::Press, deleted);
    let controls: Vec<ControlId> = (0..5).map(key).collect();
    let faces = |profile: &Profile, scope: &WorkflowScope| -> Vec<(String, Option<String>)> {
        crate::page_faces_in(
            &registry,
            &layout,
            layout.home_page,
            profile,
            None,
            &controls,
            None,
            scope,
        )
        .into_iter()
        .map(|(_, face)| (face.title, face.detail))
        .collect()
    };
    let scope = WorkflowScope {
        foreign: Some(BTreeMap::from([(layout.id, BTreeSet::from([foreign]))])),
        graphs_off: false,
    };
    let shown = faces(&profile, &scope);
    let text = |t: SystemText| Some(t.text(crate::i18n::Language::En).to_owned());
    assert_eq!(shown[0], ("Deploy".into(), None));
    assert_eq!(shown[1], ("Ship".into(), None));
    assert_eq!(shown[2], ("Steps".into(), None));
    assert_eq!(shown[3].1, text(SystemText::NotInThisProfile));
    assert_eq!(shown[4].1, text(SystemText::WorkflowMissing));

    // A rename reaches every key without a title of its own.
    let mut renamed = profile.clone();
    renamed.workflows[0].name = "Deploy all".into();
    let shown = faces(&renamed, &scope);
    assert_eq!(shown[0].0, "Deploy all");
    assert_eq!(shown[1].0, "Ship");

    // Plugin off: graph keys show it, Multi Actions do not.
    let off = WorkflowScope {
        graphs_off: true,
        ..scope
    };
    let shown = faces(&profile, &off);
    assert_eq!(shown[0], ("Deploy".into(), text(SystemText::WorkflowsOff)));
    assert_eq!(shown[1], ("Ship".into(), text(SystemText::WorkflowsOff)));
    assert_eq!(shown[2], ("Steps".into(), None));

    // Without a scope, as before.
    let shown = crate::page_faces(
        &registry,
        &layout,
        layout.home_page,
        &profile,
        None,
        &controls,
        None,
    );
    assert_eq!(
        shown[4].1.detail.as_deref(),
        Some(SystemText::NotInThisProfile.text(crate::i18n::Language::En))
    );
}

#[test]
fn an_edit_keeps_the_id_so_the_keys_follow_it() {
    let (mut profile, id) = referenced();
    let before = profile.layouts.clone();
    let mut edited = profile.workflow(id).unwrap().clone();
    edited.name = "Deploy v2".into();
    edited.description = "now with tests".into();
    edited.graph = graph_workflow("x", Some(NodeKind::Delay { ms: 5 })).graph;
    let slot = profile.workflows.iter_mut().find(|w| w.id == id).unwrap();
    *slot = edited;
    assert_eq!(profile.layouts, before);
    assert_eq!(profile.workflow_references(id).len(), 8);
}

#[test]
fn work_session_settings_are_references_and_are_cleared_only_when_asked() {
    let (mut profile, id) = referenced();
    let mut session = WorkSessionWorkflows {
        start: Some(id),
        verification: Some(id),
    };
    let usages = profile.workflow_references_with(id, &session);
    let settings: Vec<&Usage> = usages
        .iter()
        .filter(|u| matches!(u, Usage::WorkSessionSetting { .. }))
        .collect();
    assert_eq!(settings.len(), 2);
    assert_eq!(
        serde_json::to_value(settings[1]).unwrap(),
        serde_json::json!({ "type": "workSessionSetting", "setting": "verificationWorkflow" })
    );
    let fingerprint = ReferenceFingerprint::of(&usages);
    // Part of the fingerprint: the settings changing since the preview is
    // a change.
    assert_ne!(fingerprint, profile.workflow_fingerprint(id));
    let mut changed = WorkSessionWorkflows {
        start: Some(id),
        verification: None,
    };
    assert_eq!(
        profile.delete_workflow_confirmed(id, &fingerprint, DeleteOptions::default(), &mut changed),
        Err(ProfileIssue::ReferencesChanged)
    );
    // Never cleared implicitly: clearing the keys is not enough.
    let before = (profile.clone(), session);
    let keys_only = DeleteOptions {
        clear_bindings: true,
        clear_work_session: false,
    };
    assert_eq!(
        profile.delete_workflow_confirmed(id, &fingerprint, keys_only, &mut session),
        Err(ProfileIssue::UsedByWorkSession)
    );
    assert_eq!((profile.clone(), session), before);
    // Asked: the settings are cleared, the keys stay unless asked too.
    let deleted = profile
        .delete_workflow_confirmed(
            id,
            &fingerprint,
            DeleteOptions {
                clear_bindings: false,
                clear_work_session: true,
            },
            &mut session,
        )
        .unwrap();
    assert_eq!(
        deleted.cleared_work_session,
        [
            WorkSessionSetting::StartWorkflow,
            WorkSessionSetting::VerificationWorkflow
        ]
    );
    assert_eq!(session, WorkSessionWorkflows::default());
    assert_eq!(profile.layouts, before.0.layouts);
    // Another workflow's settings are left alone.
    let other = WorkflowId::new();
    let mut theirs = WorkSessionWorkflows {
        start: Some(other),
        verification: None,
    };
    assert_eq!(theirs.clear(id), Vec::new());
    assert_eq!(theirs.start, Some(other));
}

#[test]
fn a_node_mapping_that_names_a_workflow_is_a_reference() {
    let mut profile = Profile::new("Work");
    let target = graph_workflow("Target", None);
    let id = target.id;
    profile.workflows.push(target);
    let random = action("Random", ActionParameters::new());
    let random_id = random.id;
    profile.actions.push(random);
    let mut mappings = BTreeMap::new();
    mappings.insert(
        "message".to_owned(),
        crate::graph::Mapping::Template {
            template: format!("workflow:{id} 3"),
        },
    );
    profile.workflows.push(graph_workflow(
        "Picker",
        Some(NodeKind::Action {
            action: random_id,
            overrides: ActionParameters::new(),
            mappings,
            policy: ErrorPolicy::default(),
        }),
    ));
    let usages = profile.workflow_references(id);
    assert!(
        matches!(
            &usages[..],
            [Usage::WorkflowNode { workflow_name, .. }] if workflow_name == "Picker"
        ),
        "{usages:#?}"
    );
    assert_eq!(profile.delete_workflow(id), Err(ProfileIssue::InUse(1)));
}

/// Drafts (`crate::completeness`): a graph saved with a node that needs
/// configuration is listed as an invalid graph (a key may start it and is
/// refused until it is filled in, as the engine refuses it), its Call
/// node still counts as a reference, and a key to it is not refused when
/// the layout is saved.
#[test]
fn a_draft_graph_still_refers_and_is_blocked_like_the_engine_blocks_it() {
    let registry = registry();
    let mut profile = Profile::new("Work");
    let target = graph_workflow("Target", None);
    let id = target.id;
    profile.workflows.push(target);
    // `test.echo` needs `message`: a draft action without it.
    let draft = action("Draft", ActionParameters::new());
    let draft_id = draft.id;
    profile.actions.push(draft);
    let mut caller = graph_workflow(
        "Caller",
        Some(NodeKind::Call {
            workflow: id,
            inputs: BTreeMap::new(),
            policy: ErrorPolicy::default(),
        }),
    );
    // start → call → draft → end
    let graph = caller.graph.as_mut().unwrap();
    graph.nodes.push(node(
        "draft",
        NodeKind::Action {
            action: draft_id,
            overrides: ActionParameters::new(),
            mappings: BTreeMap::new(),
            policy: ErrorPolicy::default(),
        },
    ));
    graph.edges.retain(|e| e.from != n("middle"));
    for (from, to) in [("middle", "draft"), ("draft", "end")] {
        graph.edges.push(crate::graph::Edge {
            from: n(from),
            port: Port::Out,
            to: n(to),
        });
    }
    let caller_id = caller.id;
    profile.workflows.push(caller);
    // Saving the draft graph is allowed (#416).
    let saved = profile.workflow(caller_id).unwrap();
    assert!(
        saved
            .validate(&profile, &registry)
            .iter()
            .all(|issue| !issue.contains("draft")),
        "{:?}",
        saved.validate(&profile, &registry)
    );
    // Not runnable until filled in, the same verdict as the engine's.
    let block = workflow_block(saved, &profile, &registry).unwrap();
    assert_eq!(block.code, WorkflowBlockCode::InvalidGraph);
    let engine = crate::Engine::new(registry);
    assert!(engine.check_runnable(&profile, caller_id).is_err());
    // Its Call node still refers to the target, so the target is not
    // deleted silently.
    assert!(matches!(
        &profile.workflow_references(id)[..],
        [Usage::WorkflowNode { workflow_name, .. }] if workflow_name == "Caller"
    ));
    assert_eq!(profile.delete_workflow(id), Err(ProfileIssue::InUse(1)));
    // A key to the draft graph is kept and runs (and is refused) as any
    // invalid graph: not a save refusal.
    let mut layout = layout();
    bind(&mut layout, key(0), Trigger::Press, caller_id);
    assert_eq!(
        layout.workflow_binding_issues(None, &profile, engine.registry(), false),
        Vec::new()
    );
}
