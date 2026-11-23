//! Contexts: the compatibility matrix, serialization, declaration rules,
//! the save reports (reported, never refused; stored or new), the alias
//! rule and the warn-only execution guard.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use deckpilot_device::{ControlId, ControlKind, DeviceModelId, InputCapability, models};

use super::*;
use crate::graph::{NodeKind, NodeSpec, from_steps};
use crate::resolution::Invocation;
use crate::{
    ActionControls, ActionHandler, ActionParameters, ActionRegistry, ActionResult, ActionTypeId,
    ActiveProjectContext, BindingTarget, ControlBinding, DeviceLayout, Engine, ExecutionContext,
    MachineConfig, OnFailure, Os, Profile, RunOrigin, Workflow,
};

use ActionContext::{Display, Gesture, Key, Sequence, Workflow as Node};

fn definition(id: &str, contexts: Option<&[ActionContext]>) -> ActionType {
    ActionType {
        contexts: contexts.map(|c| ActionContexts::of(c.iter().copied())),
        editor_navigation: None,
        title: None,
        id: ActionTypeId::new(id).unwrap(),
        name: id.into(),
        description: String::new(),
        category: "Test".into(),
        parameters: Vec::new(),
        requires_project: false,
        node: NodeSpec::default(),
        deprecated: false,
        controls: Some(ActionControls::keys()),
    }
}

/// Every subset of the five contexts.
fn subsets() -> Vec<Vec<ActionContext>> {
    (0u8..32)
        .map(|bits| {
            ActionContext::ALL
                .iter()
                .enumerate()
                .filter(|(i, _)| bits & (1 << i) != 0)
                .map(|(_, c)| *c)
                .collect()
        })
        .collect()
}

#[test]
fn the_matrix_of_declared_and_legacy_contexts() {
    for declared in subsets() {
        let action = definition("test.typed", Some(&declared));
        for context in ActionContext::ALL {
            let expected = declared.contains(&context)
                // A key's press takes a display-only type: it does nothing.
                || (context == Key && declared.contains(&Display));
            let verdict = action.context_compatibility(context);
            assert_eq!(
                verdict.is_compatible(),
                expected,
                "{declared:?} in {context}"
            );
            if !expected {
                assert_eq!(
                    verdict.reason(),
                    Some(&IncompatibilityReason::ContextUnsupported {
                        context,
                        supported: declared.clone(),
                    })
                );
            }
        }
    }
    // Without a declaration: all five, as before contexts existed.
    let legacy = definition("test.legacy", None);
    assert_eq!(*legacy.effective_contexts(), ActionContexts::legacy());
    for context in ActionContext::ALL {
        assert!(legacy.context_compatibility(context).is_compatible());
    }
}

#[test]
fn a_bindings_context_comes_from_its_trigger() {
    use crate::Trigger;
    use deckpilot_device::TouchGesture;
    for trigger in [
        Trigger::Press,
        Trigger::Release,
        Trigger::Click,
        Trigger::RotateClockwise,
        Trigger::RotateCounterClockwise,
        Trigger::Touch {
            gesture: TouchGesture::Tap,
        },
    ] {
        assert_eq!(ActionContext::of_trigger(trigger), Key, "{trigger:?}");
    }
    assert_eq!(ActionContext::of_trigger(Trigger::DoubleClick), Gesture);
    assert_eq!(ActionContext::of_trigger(Trigger::LongPress), Gesture);
}

#[test]
fn contexts_serialize_as_the_apps_contract() {
    // Identifiers, in the app's order.
    let all = serde_json::to_value(ActionContexts::legacy()).unwrap();
    assert_eq!(
        all,
        serde_json::json!(["key", "gesture", "display", "sequence", "workflow"])
    );
    let parsed: ActionContexts =
        serde_json::from_value(serde_json::json!(["workflow", "key"])).unwrap();
    assert_eq!(parsed, ActionContexts::of([Key, Node]));
    assert!(serde_json::from_value::<ActionContexts>(serde_json::json!(["press"])).is_err());

    // Not sent without a declaration: older apps and agents see the same
    // type as before.
    let legacy = serde_json::to_value(definition("test.legacy", None)).unwrap();
    assert!(legacy.get("contexts").is_none());
    let declared = definition("test.typed", Some(&[Sequence, Node]));
    let json = serde_json::to_value(&declared).unwrap();
    assert_eq!(
        json["contexts"],
        serde_json::json!(["sequence", "workflow"])
    );
    assert_eq!(
        serde_json::from_value::<ActionType>(json).unwrap(),
        declared
    );
    // A type from an agent before contexts reads as undeclared.
    let mut old = serde_json::to_value(&declared).unwrap();
    old.as_object_mut().unwrap().remove("contexts");
    assert_eq!(
        serde_json::from_value::<ActionType>(old).unwrap().contexts,
        None
    );

    // The reason is tagged like the controls' reasons.
    let reason = declared
        .context_compatibility(Key)
        .reason()
        .cloned()
        .unwrap();
    let json = serde_json::to_value(&reason).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "type": "contextUnsupported",
            "context": "key",
            "supported": ["sequence", "workflow"],
        })
    );
    assert_eq!(
        serde_json::from_value::<IncompatibilityReason>(json).unwrap(),
        reason
    );
    assert_eq!(
        reason.to_string(),
        "the action is not meant to run on a key's press; it runs as a Multi Action step and as \
         a workflow node"
    );
}

#[test]
fn declarations_are_checked_against_the_controls() {
    assert_eq!(
        definition("test.legacy", None).context_issues(),
        Vec::<String>::new()
    );
    assert_eq!(
        definition("test.empty", Some(&[])).context_issues(),
        vec!["no context is declared".to_owned()]
    );
    assert_eq!(
        definition("test.all", Some(&ActionContext::ALL)).context_issues(),
        Vec::<String>::new()
    );
    // A gesture needs a double click or long press, a key a primary input.
    let mut press_only = definition("test.press", Some(&[Key, Gesture]));
    press_only.controls =
        Some(ActionControls::new().with(ControlKind::Key, [InputCapability::Press]));
    assert_eq!(
        press_only.context_issues(),
        vec!["gesture: the controls declare no double click or long press".to_owned()]
    );
    let mut long_only = definition("test.long", Some(&[Key, Gesture]));
    long_only.controls =
        Some(ActionControls::new().with(ControlKind::Key, [InputCapability::LongPress]));
    assert_eq!(
        long_only.context_issues(),
        vec!["key: the controls declare no press, release, click, rotation or touch".to_owned()]
    );
}

/// Counts its runs.
struct Typed {
    definition: ActionType,
    runs: Arc<AtomicUsize>,
}

impl ActionHandler for Typed {
    fn definition(&self) -> &ActionType {
        &self.definition
    }

    fn execute(&self, _: &ActionParameters, _: &ExecutionContext<'_>) -> ActionResult {
        self.runs.fetch_add(1, Ordering::SeqCst);
        ActionResult::success("ran")
    }
}

#[derive(Default)]
struct Recorder(Mutex<Vec<ContextWarning>>);

impl ContextWarnings for Recorder {
    fn warn(&self, warning: &ContextWarning) {
        self.0.lock().unwrap().push(warning.clone());
    }
}

impl Recorder {
    fn taken(&self) -> Vec<ContextWarning> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

const STEP_ONLY: &str = "test.step-only";
const KEY_ONLY: &str = "test.key-only";
const DISPLAY_ONLY: &str = "test.display-only";
const LEGACY: &str = "test.legacy";

struct Fixture {
    registry: ActionRegistry,
    profile: Profile,
    runs: std::collections::BTreeMap<&'static str, Arc<AtomicUsize>>,
    ids: std::collections::BTreeMap<&'static str, crate::ActionInstanceId>,
}

fn fixture() -> Fixture {
    let mut registry = ActionRegistry::default();
    let mut profile = Profile::new("Work");
    let mut runs = std::collections::BTreeMap::new();
    let mut ids = std::collections::BTreeMap::new();
    for (id, contexts) in [
        (STEP_ONLY, Some(&[Sequence, Node][..])),
        (KEY_ONLY, Some(&[Key, Gesture][..])),
        (DISPLAY_ONLY, Some(&[Display][..])),
        (LEGACY, None),
    ] {
        let count = Arc::new(AtomicUsize::new(0));
        registry
            .register(Box::new(Typed {
                definition: definition(id, contexts),
                runs: count.clone(),
            }))
            .unwrap();
        let instance = registry
            .create_instance(id, ActionTypeId::new(id).unwrap(), &ActionParameters::new())
            .unwrap();
        ids.insert(id, instance.id);
        runs.insert(id, count);
        profile.actions.push(instance);
    }
    Fixture {
        registry,
        profile,
        runs,
        ids,
    }
}

const KEY_0: ControlId = ControlId::Key { index: 0 };
const KEY_1: ControlId = ControlId::Key { index: 1 };
const KEY_2: ControlId = ControlId::Key { index: 2 };

fn layout(f: &Fixture) -> DeviceLayout {
    let run = |id: &str| BindingTarget::action(f.ids[id]);
    let mut layout = DeviceLayout::new("Keys", DeviceModelId::new(models::STREAM_DECK_MK2_ID));
    layout.pages[0].bindings = vec![
        // Not meant for a key: reported.
        ControlBinding::new(KEY_0, crate::Trigger::Press, run(STEP_ONLY)),
        // A display-only type on a key's click: fine.
        ControlBinding::new(KEY_1, crate::Trigger::Click, run(DISPLAY_ONLY)),
        // Gestures: a key action may, a display-only one may not.
        ControlBinding::new(KEY_1, crate::Trigger::LongPress, run(KEY_ONLY)),
        ControlBinding::new(KEY_2, crate::Trigger::DoubleClick, run(DISPLAY_ONLY)),
        // No declaration: everywhere.
        ControlBinding::new(KEY_2, crate::Trigger::Click, run(LEGACY)),
    ];
    layout
}

#[test]
fn saving_a_layout_reports_context_issues_as_stored_or_new_and_refuses_none() {
    let f = fixture();
    let catalog = deckpilot_device::ModelCatalog::builtin();
    let stored = layout(&f);
    let mut edited = stored.clone();
    // A new binding not meant for a key.
    edited.pages[0].bindings.push(ControlBinding::new(
        ControlId::Key { index: 3 },
        crate::Trigger::Click,
        BindingTarget::action(f.ids[STEP_ONLY]),
    ));
    let issues = edited.context_issues_for_save(Some(&stored), &f.registry, &f.profile);
    let summary: Vec<_> = issues
        .iter()
        .map(|issue| match &issue.site {
            ContextSite::Binding {
                control, trigger, ..
            } => (*control, *trigger, issue.stored),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        summary,
        [
            (KEY_0, crate::Trigger::Press, true),
            (KEY_2, crate::Trigger::DoubleClick, true),
            (ControlId::Key { index: 3 }, crate::Trigger::Click, false),
        ]
    );
    assert_eq!(
        issues[0].reason,
        IncompatibilityReason::ContextUnsupported {
            context: Key,
            supported: vec![Sequence, Node],
        }
    );
    assert_eq!(
        issues[1].reason,
        IncompatibilityReason::ContextUnsupported {
            context: Gesture,
            supported: vec![Display],
        }
    );
    // A new layout: nothing is stored.
    assert!(
        edited
            .context_issues_for_save(None, &f.registry, &f.profile)
            .iter()
            .all(|issue| !issue.stored)
    );
    // Nothing of this is a refusal: validation and the control checks the
    // save path refuses with know nothing of contexts.
    assert_eq!(edited.validate(&catalog, &f.profile), Vec::new());
    let (refused, kept) =
        edited.compatibility_for_save(Some(&stored), &catalog, &f.registry, &f.profile);
    assert!(
        refused.is_empty() && kept.is_empty(),
        "{refused:?} {kept:?}"
    );
    // Reported in words with the place.
    assert_eq!(
        issues[0].to_string(),
        "page `Home`: key 0 (Press): the action is not meant to run on a key's press; it runs \
         as a Multi Action step and as a workflow node"
    );
    let json = serde_json::to_value(&issues[0]).unwrap();
    assert_eq!(json["site"]["kind"], "binding");
    assert_eq!(json["site"]["pageName"], "Home");
    assert_eq!(json["reason"]["type"], "contextUnsupported");
    assert_eq!(json["stored"], true);
}

#[test]
fn a_missing_action_or_an_unavailable_type_gets_no_context_verdict() {
    let mut f = fixture();
    let mut layout = layout(&f);
    // An instance of a type no module provides (a missing or disabled
    // plugin), and a binding to an action that is gone.
    let orphan = crate::ActionInstance {
        action_type: ActionTypeId::new("missing.plugin").unwrap(),
        ..f.profile.actions[0].clone()
    };
    let orphan = crate::ActionInstance {
        id: crate::ActionInstanceId::new(),
        ..orphan
    };
    layout.pages[0].bindings = vec![
        ControlBinding::new(
            KEY_0,
            crate::Trigger::Press,
            BindingTarget::action(orphan.id),
        ),
        ControlBinding::new(
            KEY_1,
            crate::Trigger::Press,
            BindingTarget::action(crate::ActionInstanceId::new()),
        ),
    ];
    let mut workflow = Workflow::new("Steps").step(orphan.id, OnFailure::Stop);
    f.profile.actions.push(orphan);
    assert_eq!(
        layout.context_issues_for_save(None, &f.registry, &f.profile),
        Vec::new()
    );
    assert_eq!(
        workflow.context_issues(None, &f.profile, &f.registry),
        Vec::new()
    );
    workflow.graph = Some(from_steps(&workflow, &f.profile).graph);
    assert_eq!(
        workflow.context_issues(None, &f.profile, &f.registry),
        Vec::new()
    );
}

#[test]
fn saving_a_multi_action_classifies_steps_as_a_multiset() {
    let f = fixture();
    let key = f.ids[KEY_ONLY];
    let step = f.ids[STEP_ONLY];
    let stored = Workflow::new("Steps")
        .step(key, OnFailure::Stop)
        .step(step, OnFailure::Stop)
        .step(key, OnFailure::Continue);
    // Saved before: both deck-like steps are stored, wherever they moved.
    let mut edited = stored.clone();
    edited.steps.swap(0, 1);
    let issues = edited.context_issues(Some(&stored), &f.profile, &f.registry);
    assert_eq!(
        issues
            .iter()
            .map(|i| (i.site.clone(), i.action, i.stored))
            .collect::<Vec<_>>(),
        [
            (ContextSite::Step { index: 1 }, key, true),
            (ContextSite::Step { index: 2 }, key, true),
        ]
    );
    // A third use of the same action is new; the two stored ones stay
    // stored (the one at its index first).
    let edited = stored.clone().step(key, OnFailure::Stop);
    let issues = edited.context_issues(Some(&stored), &f.profile, &f.registry);
    assert_eq!(
        issues.iter().map(|i| i.stored).collect::<Vec<_>>(),
        [true, true, false]
    );
    assert_eq!(
        issues[0].reason,
        IncompatibilityReason::ContextUnsupported {
            context: Sequence,
            supported: vec![Key, Gesture],
        }
    );
    // A new workflow: all new. The validation that refuses a save knows
    // nothing of contexts.
    let issues = stored.context_issues(None, &f.profile, &f.registry);
    assert!(issues.iter().all(|i| !i.stored));
    assert_eq!(
        stored.validate(&f.profile, &f.registry),
        Vec::<String>::new()
    );
}

#[test]
fn saving_a_graph_classifies_nodes_by_id_and_action() {
    let f = fixture();
    let steps = Workflow::new("Flow")
        .step(f.ids[KEY_ONLY], OnFailure::Stop)
        .step(f.ids[STEP_ONLY], OnFailure::Stop)
        .step(f.ids[DISPLAY_ONLY], OnFailure::Stop);
    let mut stored = steps.clone();
    stored.graph = Some(from_steps(&steps, &f.profile).graph);
    let issues = stored.context_issues(Some(&stored), &f.profile, &f.registry);
    let nodes: Vec<_> = issues
        .iter()
        .map(|i| (i.site.to_string(), i.stored))
        .collect();
    assert_eq!(
        nodes,
        [("step_1".to_owned(), true), ("step_3".to_owned(), true)]
    );
    assert!(matches!(
        issues[0].reason,
        IncompatibilityReason::ContextUnsupported { context: Node, .. }
    ));
    // The same node id with another action is new.
    let mut edited = stored.clone();
    for node in &mut edited.graph.as_mut().unwrap().nodes {
        if let NodeKind::Action { action, .. } = &mut node.kind
            && *action == f.ids[STEP_ONLY]
        {
            *action = f.ids[KEY_ONLY];
        }
    }
    let issues = edited.context_issues(Some(&stored), &f.profile, &f.registry);
    assert_eq!(
        issues
            .iter()
            .map(|i| (i.site.to_string(), i.stored))
            .collect::<Vec<_>>(),
        [
            ("step_1".to_owned(), true),
            ("step_2".to_owned(), false),
            ("step_3".to_owned(), true),
        ]
    );
    // A graph's steps are for reference only: not checked as steps.
    assert!(
        issues
            .iter()
            .all(|i| matches!(i.site, ContextSite::Node { .. }))
    );
}

#[test]
fn the_alias_rule_reports_contexts_the_plugin_dropped() {
    let builtin = definition("legacy.fetch", Some(&[Key, Gesture, Sequence, Node]));
    // A plugin without a declaration supports everything.
    assert_eq!(
        crate::alias::context_incompatibilities(&builtin, &definition("p.fetch", None)),
        Vec::<String>::new()
    );
    let narrower = definition("p.fetch", Some(&[Sequence, Node, Display]));
    assert_eq!(
        crate::alias::context_incompatibilities(&builtin, &narrower),
        [
            "context `key` is no longer supported",
            "context `gesture` is no longer supported",
        ]
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn the_guard_warns_once_per_use_and_always_runs() {
    let Fixture {
        registry,
        mut profile,
        runs,
        ids,
    } = fixture();
    let recorder = Arc::new(Recorder::default());
    let engine = Engine::new(registry).with_context_warnings(recorder.clone());
    let machine = MachineConfig::new(Os::current());
    let mut projects = ActiveProjectContext::restore(&profile, None);
    let run = |engine: &Engine,
               profile: &Profile,
               projects: &mut ActiveProjectContext,
               id: &str,
               invocation: &Invocation| {
        engine
            .run_action_from(profile, &machine, projects, ids[id], invocation)
            .unwrap()
    };
    let key = Invocation::default()
        .with_origin(RunOrigin::Key)
        .with_context(Key);

    // A step-only action on a key: runs, warned about once.
    for _ in 0..3 {
        let result = run(&engine, &profile, &mut projects, STEP_ONLY, &key);
        assert!(result.is_success(), "{result:?}");
    }
    assert_eq!(runs[STEP_ONLY].load(Ordering::SeqCst), 3);
    let warnings = recorder.taken();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert_eq!(warnings[0].action, ids[STEP_ONLY]);
    assert_eq!(
        warnings[0].reason,
        IncompatibilityReason::ContextUnsupported {
            context: Key,
            supported: vec![Sequence, Node],
        }
    );

    // A display-only one on a key, a key one on a gesture, a legacy one
    // anywhere: nothing to say.
    run(&engine, &profile, &mut projects, DISPLAY_ONLY, &key);
    let gesture = Invocation::default()
        .with_origin(RunOrigin::Gesture)
        .with_context(Gesture);
    run(&engine, &profile, &mut projects, KEY_ONLY, &gesture);
    for context in ActionContext::ALL {
        run(
            &engine,
            &profile,
            &mut projects,
            LEGACY,
            &Invocation::default().with_context(context),
        );
    }
    assert_eq!(recorder.taken(), Vec::new());

    // The same action in another context is another use: warned once.
    run(&engine, &profile, &mut projects, STEP_ONLY, &gesture);
    run(&engine, &profile, &mut projects, STEP_ONLY, &gesture);
    assert_eq!(recorder.taken().len(), 1);

    // Runs from the app and Test runs have no context: never checked.
    for origin in [RunOrigin::App, RunOrigin::Test] {
        let app = Invocation::default().with_origin(origin);
        for id in [STEP_ONLY, KEY_ONLY, DISPLAY_ONLY] {
            assert!(run(&engine, &profile, &mut projects, id, &app).is_success());
        }
    }
    assert_eq!(recorder.taken(), Vec::new());

    // Steps run as steps, nodes as nodes: a key action in a Multi Action
    // and in a graph is warned about once each, and runs.
    let steps = Workflow::new("Steps")
        .step(ids[KEY_ONLY], OnFailure::Continue)
        .step(ids[STEP_ONLY], OnFailure::Continue);
    let steps_id = steps.id;
    let mut graph = Workflow::new("Graph")
        .step(ids[KEY_ONLY], OnFailure::Continue)
        .step(ids[STEP_ONLY], OnFailure::Continue);
    graph.graph = Some(from_steps(&graph, &profile).graph);
    let graph_id = graph.id;
    profile.workflows.extend([steps, graph]);
    let before = runs[KEY_ONLY].load(Ordering::SeqCst);
    for _ in 0..2 {
        let report = engine
            .run_workflow(&profile, &machine, &mut projects, steps_id)
            .unwrap();
        assert_eq!(report.status, crate::ActionStatus::Succeeded, "{report:?}");
        let report = engine
            .execute_workflow(
                &profile,
                &machine,
                &mut projects,
                graph_id,
                &crate::CancellationToken::new(),
                &|_| {},
            )
            .unwrap();
        assert_eq!(report.status, crate::graph::executor::RunStatus::Succeeded);
    }
    assert_eq!(runs[KEY_ONLY].load(Ordering::SeqCst), before + 4);
    let warnings = recorder.taken();
    assert_eq!(
        warnings
            .iter()
            .map(|w| (w.workflow, w.reason.clone()))
            .collect::<Vec<_>>(),
        [
            (
                Some(steps_id),
                IncompatibilityReason::ContextUnsupported {
                    context: Sequence,
                    supported: vec![Key, Gesture],
                }
            ),
            (
                Some(graph_id),
                IncompatibilityReason::ContextUnsupported {
                    context: Node,
                    supported: vec![Key, Gesture],
                }
            ),
        ]
    );
}
