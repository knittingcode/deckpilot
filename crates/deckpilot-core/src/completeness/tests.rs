//! Draft actions: saved without their required values, refused when run.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use deckpilot_device::{ControlId, DeviceModelId, models};

use super::*;
use crate::graph::{Mapping, from_steps};
use crate::{
    ActionHandler, ActionInstance, ActionResult, ActionType, ActionTypeId, ActiveProjectContext,
    Capability, ControlBinding, Engine, EngineError, ExecutionContext, MachineConfig, ModuleId,
    OnFailure, Os, ParameterError, ParameterKind, ParameterSpec, ParameterValue, RegistryError,
};

/// `test.open`: a required application (no default), an optional count
/// and a required mode with a default. Counts its runs and needs the
/// process capability, like opening an application.
struct Open {
    definition: ActionType,
    runs: Arc<AtomicUsize>,
}

fn open_type() -> ActionTypeId {
    ActionTypeId::new("test.open").unwrap()
}

impl Open {
    fn new(runs: Arc<AtomicUsize>) -> Self {
        let spec = |key: &str, kind, required, default| ParameterSpec {
            key: key.into(),
            label: key.into(),
            description: None,
            kind,
            required,
            default,
        };
        Self {
            definition: ActionType {
                id: open_type(),
                name: "Open application".into(),
                description: String::new(),
                category: "Test".into(),
                parameters: vec![
                    spec(
                        "application",
                        ParameterKind::Text {
                            multiline: false,
                            max_length: Some(20),
                        },
                        true,
                        None,
                    ),
                    spec(
                        "count",
                        ParameterKind::Integer {
                            min: Some(1),
                            max: Some(10),
                        },
                        false,
                        None,
                    ),
                    spec(
                        "mode",
                        ParameterKind::Choice {
                            options: vec!["new".into(), "reuse".into()],
                        },
                        true,
                        Some(ParameterValue::Choice("new".into())),
                    ),
                ],
                requires_project: false,
                node: crate::graph::NodeSpec::default(),
                deprecated: false,
                controls: None,
                contexts: None,
                editor_navigation: None,
                title: None,
            },
            runs,
        }
    }
}

impl ActionHandler for Open {
    fn definition(&self) -> &ActionType {
        &self.definition
    }

    fn execute(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> ActionResult {
        self.runs.fetch_add(1, Ordering::SeqCst);
        if let Err(denied) = context.permit(Capability::ProcessExecution) {
            return denied.into();
        }
        ActionResult::success(format!(
            "opened {}",
            parameters.text("application").unwrap_or_default()
        ))
    }
}

struct World {
    engine: Engine,
    runs: Arc<AtomicUsize>,
    profile: Profile,
    draft: ActionInstanceId,
}

fn registry_with(runs: &Arc<AtomicUsize>, capabilities: Option<&[Capability]>) -> ActionRegistry {
    let mut registry = crate::testing::registry();
    let handler = Box::new(Open::new(runs.clone()));
    match capabilities {
        None => registry.register(handler).unwrap(),
        Some(granted) => registry
            .register_for_module(ModuleId::new("test").unwrap(), granted, handler)
            .unwrap(),
    }
    registry
}

fn world() -> World {
    let runs = Arc::new(AtomicUsize::new(0));
    let registry = registry_with(&runs, None);
    let mut profile = Profile::new("Work");
    // What dropping the type on a key saves: its declared defaults only.
    let draft = registry
        .create_instance("Open application", open_type(), &ActionParameters::new())
        .unwrap();
    let id = draft.id;
    profile.actions.push(draft);
    World {
        engine: Engine::new(registry),
        runs,
        profile,
        draft: id,
    }
}

fn app(name: &str) -> ActionParameters {
    ActionParameters::new().with("application", ParameterValue::Text(name.into()))
}

fn run(world: &World, overrides: Option<&ActionParameters>) -> ActionResult {
    let mut context = ActiveProjectContext::new(&world.profile);
    world
        .engine
        .run_action_with(
            &world.profile,
            &MachineConfig::new(Os::current()),
            &mut context,
            world.draft,
            overrides,
            &crate::resolution::Invocation::default(),
        )
        .unwrap()
}

fn assert_incomplete(result: &ActionResult, missing: &str) {
    assert!(!result.is_success(), "{result:?}");
    assert_eq!(result.code.as_deref(), Some(CONFIGURATION_INCOMPLETE));
    assert_eq!(
        result.data.get("missing").map(String::as_str),
        Some(missing)
    );
    assert!(
        result.message.contains(&format!(
            "parameter `{}` is required",
            missing.split(',').next().unwrap()
        )),
        "{}",
        result.message
    );
}

#[test]
fn a_draft_is_saved_without_its_required_value_and_round_trips() {
    let w = world();
    let draft = w.profile.action(w.draft).unwrap();
    // The declared default is filled in; nothing is invented for the
    // application.
    assert_eq!(
        draft.parameters,
        ActionParameters::new().with("mode", ParameterValue::Choice("new".into()))
    );
    assert_eq!(
        incomplete_actions(&w.profile, w.engine.registry()),
        BTreeMap::from([(w.draft, vec!["application".to_owned()])])
    );
    // Stored as any other action: no flag, no new field.
    let json = serde_json::to_value(draft).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "id": draft.id,
            "name": "Open application",
            "actionType": "test.open",
            "parameters": { "mode": { "type": "choice", "value": "new" } },
        })
    );
    let back: ActionInstance = serde_json::from_value(json).unwrap();
    assert_eq!(&back, draft);
    let profile: Profile =
        serde_json::from_str(&serde_json::to_string(&w.profile).unwrap()).unwrap();
    assert_eq!(profile, w.profile);
}

#[test]
fn a_blank_required_value_is_missing_too() {
    let w = world();
    let registry = w.engine.registry();
    for blank in ["", "   "] {
        let mut instance = w.profile.action(w.draft).unwrap().clone();
        instance.parameters = app(blank);
        assert_eq!(
            registry.validate_instance(&mut instance),
            Ok(vec!["application".to_owned()])
        );
        assert!(instance.parameters.get("application").is_none());
    }
}

#[test]
fn completing_the_fields_clears_the_incomplete_state() {
    let mut w = world();
    let mut instance = w.profile.action(w.draft).unwrap().clone();
    instance.parameters = instance.parameters.overlay(&app("Safari"));
    assert_eq!(
        w.engine.registry().validate_instance(&mut instance),
        Ok(Vec::new())
    );
    *w.profile.actions.first_mut().unwrap() = instance;
    assert!(incomplete_actions(&w.profile, w.engine.registry()).is_empty());
    let result = run(&w, None);
    assert!(result.is_success(), "{result:?}");
    assert_eq!(result.message, "opened Safari");
    assert_eq!(w.runs.load(Ordering::SeqCst), 1);
}

#[test]
fn every_other_problem_is_still_refused_when_saving() {
    let w = world();
    let registry = w.engine.registry();
    let refused = |parameters: ActionParameters| {
        let mut instance = w.profile.action(w.draft).unwrap().clone();
        instance.parameters = parameters;
        registry.validate_instance(&mut instance)
    };
    // A value of the wrong type for a parameter that is present, with the
    // application still missing.
    let wrong_type = ActionParameters::new().with("count", ParameterValue::Text("two".into()));
    assert!(matches!(
        refused(wrong_type),
        Err(RegistryError::InvalidParameters(errors))
            if errors == vec![ParameterError::WrongType { key: "count".into(), expected: "integer" }]
    ));
    let wrong_app = ActionParameters::new().with("application", ParameterValue::Integer(3));
    assert!(matches!(
        refused(wrong_app),
        Err(RegistryError::InvalidParameters(_))
    ));
    // Unknown parameters, sizes, ranges and choices.
    let unknown = ActionParameters::new().with("path", ParameterValue::Text("/".into()));
    assert!(matches!(
        refused(unknown),
        Err(RegistryError::InvalidParameters(_))
    ));
    assert!(matches!(
        refused(app(&"x".repeat(21))),
        Err(RegistryError::InvalidParameters(_))
    ));
    let range = ActionParameters::new().with("count", ParameterValue::Integer(11));
    assert!(matches!(
        refused(range),
        Err(RegistryError::InvalidParameters(_))
    ));
    let choice = ActionParameters::new().with("mode", ParameterValue::Choice("old".into()));
    assert!(matches!(
        refused(choice),
        Err(RegistryError::InvalidParameters(_))
    ));
}

#[test]
fn an_incomplete_run_is_refused_before_the_handler() {
    let w = world();
    let result = run(&w, None);
    assert_incomplete(&result, "application");
    assert_eq!(
        w.runs.load(Ordering::SeqCst),
        0,
        "the handler was not called"
    );
    // A blank override leaves it missing as well; a value completes it.
    assert_incomplete(&run(&w, Some(&app(" "))), "application");
    assert_eq!(w.runs.load(Ordering::SeqCst), 0);
    assert!(run(&w, Some(&app("Mail"))).is_success());
    assert_eq!(w.runs.load(Ordering::SeqCst), 1);
}

#[test]
fn permissions_are_still_checked_and_come_after_completeness() {
    let runs = Arc::new(AtomicUsize::new(0));
    // The module providing the type was granted nothing.
    let registry = registry_with(&runs, Some(&[]));
    let mut profile = Profile::new("Work");
    let complete = registry
        .create_instance("Open Mail", open_type(), &app("Mail"))
        .unwrap();
    let draft = registry
        .create_instance("Open", open_type(), &ActionParameters::new())
        .unwrap();
    let (complete_id, draft_id) = (complete.id, draft.id);
    profile.actions.extend([complete, draft]);
    let mut w = World {
        engine: Engine::new(registry),
        runs,
        profile,
        draft: complete_id,
    };
    let denied = run(&w, None);
    assert!(!denied.is_success());
    assert_eq!(denied.code.as_deref(), Some("not-permitted"), "{denied:?}");
    w.draft = draft_id;
    let before = w.runs.load(Ordering::SeqCst);
    assert_incomplete(&run(&w, None), "application");
    assert_eq!(w.runs.load(Ordering::SeqCst), before);
}

fn layout_with(bindings: Vec<ControlBinding>) -> DeviceLayout {
    let mut layout = DeviceLayout::new("Main", DeviceModelId::new(models::STREAM_DECK_PLUS_ID));
    layout.pages[0].bindings = bindings;
    layout
}

#[test]
fn bindings_of_every_trigger_report_their_missing_parameters() {
    let mut w = world();
    let key = ControlId::Key { index: 0 };
    let dial = ControlId::Encoder { index: 0 };
    let triggers = [
        (key, Trigger::Press),
        (key, Trigger::Release),
        (ControlId::Key { index: 1 }, Trigger::Click),
        (ControlId::Key { index: 1 }, Trigger::DoubleClick),
        (ControlId::Key { index: 1 }, Trigger::LongPress),
        (dial, Trigger::RotateClockwise),
        (dial, Trigger::RotateCounterClockwise),
        (
            ControlId::TouchZone { index: 0 },
            Trigger::Touch {
                gesture: deckpilot_device::TouchGesture::Tap,
            },
        ),
    ];
    let mut bindings: Vec<ControlBinding> = triggers
        .iter()
        .map(|(control, trigger)| {
            ControlBinding::new(*control, *trigger, BindingTarget::action(w.draft))
        })
        .collect();
    // A gesture's override that fills the value: complete, and kept.
    bindings.push(ControlBinding::new(
        ControlId::Key { index: 2 },
        Trigger::LongPress,
        BindingTarget::Action {
            action: w.draft,
            overrides: app("Mail"),
        },
    ));
    let layout = layout_with(bindings);
    let found = layout_incomplete(&layout, &w.profile, w.engine.registry());
    assert_eq!(found.len(), triggers.len());
    for (use_, (control, trigger)) in found.iter().zip(triggers) {
        assert_eq!(
            use_.site,
            IncompleteSite::Binding {
                layout: layout.id,
                page: layout.pages[0].id,
                control,
                trigger
            }
        );
        assert_eq!(use_.missing, vec!["application".to_owned()]);
    }
    // Saving: a draft binding and a completing override are no problem.
    assert_eq!(
        layout.check_overrides(w.engine.registry(), &w.profile),
        Vec::new()
    );
    w.profile.layouts.push(layout);
    assert_eq!(
        incomplete_uses(&w.profile, w.engine.registry()).len(),
        triggers.len()
    );
}

#[test]
fn an_override_is_still_checked_on_a_draft() {
    let w = world();
    let key = ControlId::Key { index: 0 };
    let with = |overrides: ActionParameters| {
        layout_with(vec![ControlBinding::new(
            key,
            Trigger::DoubleClick,
            BindingTarget::Action {
                action: w.draft,
                overrides,
            },
        )])
        .check_overrides(w.engine.registry(), &w.profile)
    };
    // Shape problems of an override are refused although the instance is
    // a draft (before drafts, they went unchecked then).
    assert_eq!(
        with(ActionParameters::new().with("count", ParameterValue::Integer(99))).len(),
        1
    );
    // A blank override of a required parameter is refused, as every
    // earlier agent refuses it when it loads the layout.
    assert_eq!(with(app("")).len(), 1);
    // A valid override of another parameter is fine.
    assert_eq!(
        with(ActionParameters::new().with("count", ParameterValue::Integer(2))),
        Vec::new()
    );
}

#[test]
fn multi_action_steps_with_a_draft_are_saved_and_the_step_is_refused() {
    let w = world();
    let echo = w
        .engine
        .registry()
        .create_instance(
            "Echo",
            crate::testing::echo_type(),
            &ActionParameters::new().with("message", ParameterValue::Text("after".into())),
        )
        .unwrap();
    let mut profile = w.profile.clone();
    let echo_id = echo.id;
    profile.actions.push(echo);
    let mut workflow = crate::Workflow::new("Morning")
        .step(w.draft, OnFailure::Continue)
        .step(w.draft, OnFailure::Continue)
        .step(echo_id, OnFailure::Continue);
    workflow.steps[1].overrides = app("Mail");
    assert_eq!(
        workflow.validate(&profile, w.engine.registry()),
        Vec::<String>::new()
    );
    assert_eq!(
        workflow_incomplete(&workflow, &profile, w.engine.registry()),
        vec![IncompleteUse {
            site: IncompleteSite::Step {
                workflow: workflow.id,
                index: 0
            },
            action: w.draft,
            missing: vec!["application".into()],
        }]
    );
    // A blank override of a required parameter is refused for a step too.
    let mut blank = workflow.clone();
    blank.steps[1].overrides = app("");
    assert_eq!(blank.validate(&profile, w.engine.registry()).len(), 1);

    let id = workflow.id;
    profile.workflows.push(workflow);
    let mut context = ActiveProjectContext::new(&profile);
    let report = w
        .engine
        .run_workflow(
            &profile,
            &MachineConfig::new(Os::current()),
            &mut context,
            id,
        )
        .unwrap();
    assert_incomplete(&report.steps[0].result, "application");
    assert!(report.steps[1].result.is_success());
    assert!(report.steps[2].result.is_success());
    // Only the completed step reached the handler.
    assert_eq!(w.runs.load(Ordering::SeqCst), 1);
}

#[test]
fn a_workflow_node_with_a_draft_is_saved_and_the_graph_does_not_start() {
    let w = world();
    let mut profile = w.profile.clone();
    let steps = crate::Workflow::new("Open")
        .step(w.draft, OnFailure::Stop)
        .step(w.draft, OnFailure::Stop);
    let mut workflow = steps.clone();
    let mut graph = from_steps(&steps, &profile).graph;
    // The second node gets its application from a mapping: not missing.
    let mapped = graph
        .nodes
        .iter_mut()
        .filter(|node| matches!(node.kind, NodeKind::Action { .. }))
        .nth(1)
        .unwrap();
    let mapped_id = mapped.id.clone();
    if let NodeKind::Action { mappings, .. } = &mut mapped.kind {
        mappings.insert(
            "application".into(),
            Mapping::Template {
                template: "Mail".into(),
            },
        );
    }
    workflow.graph = Some(graph);
    workflow.steps.clear();

    // Saving accepts it; the editor sees which node needs what.
    assert_eq!(
        workflow.validate(&profile, w.engine.registry()),
        Vec::<String>::new()
    );
    let issues = workflow.graph_issues(&profile, w.engine.registry());
    let incomplete: Vec<_> = issues
        .iter()
        .filter(|issue| issue.code.as_deref() == Some(CONFIGURATION_INCOMPLETE))
        .collect();
    assert_eq!(incomplete.len(), 1, "{issues:?}");
    assert_eq!(incomplete[0].parameter.as_deref(), Some("application"));
    assert_eq!(incomplete[0].severity, crate::graph::IssueSeverity::Error);
    assert_ne!(incomplete[0].node.as_ref(), Some(&mapped_id));
    let uses = workflow_incomplete(&workflow, &profile, w.engine.registry());
    assert_eq!(uses.len(), 1);
    assert!(matches!(&uses[0].site, IncompleteSite::Node { node, .. } if *node != mapped_id));

    // Running: refused before any node runs.
    let id = workflow.id;
    profile.workflows.push(workflow);
    assert!(matches!(
        w.engine.check_runnable(&profile, id),
        Err(EngineError::InvalidWorkflow(_))
    ));
    let mut context = ActiveProjectContext::new(&profile);
    assert!(
        w.engine
            .run_workflow(
                &profile,
                &MachineConfig::new(Os::current()),
                &mut context,
                id
            )
            .is_err()
    );
    assert_eq!(w.runs.load(Ordering::SeqCst), 0);
}

#[test]
fn unavailable_types_are_not_reported_as_incomplete() {
    let mut w = world();
    let mut registry = crate::testing::registry();
    registry.mark_unavailable([open_type()], "the Test plugin is disabled");
    // Creating one is refused as before; a saved one shows as unavailable.
    assert!(matches!(
        registry.create_instance("Open", open_type(), &ActionParameters::new()),
        Err(RegistryError::Unavailable(..))
    ));
    assert!(incomplete_actions(&w.profile, &registry).is_empty());
    w.profile.layouts.push(layout_with(vec![ControlBinding::new(
        ControlId::Key { index: 0 },
        Trigger::Press,
        BindingTarget::action(w.draft),
    )]));
    assert_eq!(incomplete_uses(&w.profile, &registry), Vec::new());
    // Its run says why it cannot run, not what it misses.
    let engine = Engine::new(registry);
    let mut context = ActiveProjectContext::new(&w.profile);
    let result = engine
        .run_action(
            &w.profile,
            &MachineConfig::new(Os::current()),
            &mut context,
            w.draft,
        )
        .unwrap();
    assert_ne!(result.code.as_deref(), Some(CONFIGURATION_INCOMPLETE));
    assert!(result.message.contains("unavailable"), "{}", result.message);
}

#[test]
fn completeness_serializes_as_the_app_reads_it() {
    // The JSON of `apps/desktop/src/lib/action-completeness.contract.test.ts`.
    let layout: LayoutId = "6f1e8c1e-0000-4000-8000-000000000001".parse().unwrap();
    let page: PageId = "6f1e8c1e-0000-4000-8000-000000000002".parse().unwrap();
    let workflow: WorkflowId = "6f1e8c1e-0000-4000-8000-000000000003".parse().unwrap();
    let action: ActionInstanceId = "6f1e8c1e-0000-4000-8000-000000000004".parse().unwrap();
    let uses = vec![
        IncompleteUse {
            site: IncompleteSite::Binding {
                layout,
                page,
                control: ControlId::Key { index: 0 },
                trigger: Trigger::LongPress,
            },
            action,
            missing: vec!["application".into()],
        },
        IncompleteUse {
            site: IncompleteSite::Step { workflow, index: 1 },
            action,
            missing: vec!["application".into()],
        },
        IncompleteUse {
            site: IncompleteSite::Node {
                workflow,
                node: crate::graph::NodeId::new("open").unwrap(),
            },
            action,
            missing: vec!["application".into()],
        },
    ];
    assert_eq!(
        serde_json::to_value(&uses).unwrap(),
        serde_json::json!([
            {
                "site": {
                    "kind": "binding",
                    "layout": layout,
                    "page": page,
                    "control": { "kind": "key", "index": 0 },
                    "trigger": { "type": "longPress" },
                },
                "action": action,
                "missing": ["application"],
            },
            {
                "site": { "kind": "step", "workflow": workflow, "index": 1 },
                "action": action,
                "missing": ["application"],
            },
            {
                "site": { "kind": "node", "workflow": workflow, "node": "open" },
                "action": action,
                "missing": ["application"],
            },
        ])
    );
    // A graph issue without a code reads and writes as before.
    let plain = serde_json::json!({ "severity": "error", "message": "x" });
    let issue: crate::graph::GraphIssue = serde_json::from_value(plain.clone()).unwrap();
    assert_eq!(serde_json::to_value(&issue).unwrap(), plain);
}

/// What a draft-aware app saves: a draft on a key's press, a click, a
/// long press completed by its override, in a Multi Action, and as a
/// graph node with and without a mapping. Written with fixed ids to
/// `tests/fixtures/draft-profile.json`, which an agent of 0.12.0 or 0.13.0
/// (whose core is the same) was run against: it loads the profile, keeps
/// the layout compatible and the workflows as they are, refuses the
/// draft's runs before its handler and runs the rest (see
/// `docs/action-architecture.md`, "Draft actions").
fn draft_profile() -> Profile {
    let registry = crate::testing::registry();
    let id = |n: u32| format!("00000000-0000-4000-8000-{n:012}");
    let mut profile = Profile::new("Drafts");
    profile.id = id(1).parse().unwrap();
    let mut draft = registry
        .create_instance(
            "Echo draft",
            crate::testing::echo_type(),
            &ActionParameters::new(),
        )
        .unwrap();
    draft.id = id(2).parse().unwrap();
    let mut complete = registry
        .create_instance(
            "Echo",
            crate::testing::echo_type(),
            &ActionParameters::new().with("message", ParameterValue::Text("done".into())),
        )
        .unwrap();
    complete.id = id(3).parse().unwrap();
    let (draft_id, complete_id) = (draft.id, complete.id);
    profile.actions.extend([draft, complete]);
    let message =
        |text: &str| ActionParameters::new().with("message", ParameterValue::Text(text.into()));

    let mut layout = DeviceLayout::new("Main", DeviceModelId::new(models::STREAM_DECK_MK2_ID));
    layout.id = id(4).parse().unwrap();
    layout.pages[0].id = id(5).parse().unwrap();
    layout.home_page = layout.pages[0].id;
    layout.pages[0].bindings = vec![
        ControlBinding::new(
            ControlId::Key { index: 0 },
            Trigger::Press,
            BindingTarget::action(draft_id),
        ),
        ControlBinding::new(
            ControlId::Key { index: 1 },
            Trigger::Click,
            BindingTarget::action(draft_id),
        ),
        ControlBinding::new(
            ControlId::Key { index: 1 },
            Trigger::LongPress,
            BindingTarget::Action {
                action: draft_id,
                overrides: message("long"),
            },
        ),
        ControlBinding::new(
            ControlId::Key { index: 2 },
            Trigger::Press,
            BindingTarget::action(complete_id),
        ),
    ];
    profile
        .active_layouts
        .insert(layout.model.clone(), layout.id);
    profile.layouts.push(layout);

    let mut steps = crate::Workflow::new("Multi")
        .step(draft_id, OnFailure::Continue)
        .step(complete_id, OnFailure::Continue)
        .step(draft_id, OnFailure::Continue);
    steps.id = id(6).parse().unwrap();
    steps.steps[2].overrides = message("filled");
    let mut graph = crate::Workflow::new("Graph");
    graph.id = id(7).parse().unwrap();
    graph.graph = Some(
        from_steps(
            &crate::Workflow::new("x").step(draft_id, OnFailure::Stop),
            &profile,
        )
        .graph,
    );
    let mut mapped = crate::Workflow::new("Mapped");
    mapped.id = id(8).parse().unwrap();
    let mut mapped_graph = from_steps(
        &crate::Workflow::new("x").step(draft_id, OnFailure::Stop),
        &profile,
    )
    .graph;
    for node in &mut mapped_graph.nodes {
        if let NodeKind::Action { mappings, .. } = &mut node.kind {
            mappings.insert(
                "message".into(),
                Mapping::Template {
                    template: "mapped".into(),
                },
            );
        }
    }
    mapped.graph = Some(mapped_graph);
    profile.workflows.extend([steps, graph, mapped]);
    profile
}

#[test]
fn the_saved_shape_of_drafts_is_the_fixture_older_agents_were_run_against() {
    let profile = draft_profile();
    let registry = crate::testing::registry();
    // What this version's save checks accept.
    assert_eq!(
        profile.layouts[0].check_overrides(&registry, &profile),
        Vec::new()
    );
    for workflow in &profile.workflows {
        assert_eq!(
            workflow.validate(&profile, &registry),
            Vec::<String>::new(),
            "{}",
            workflow.name
        );
    }
    // Press, click, the first step and the unmapped node.
    assert_eq!(incomplete_uses(&profile, &registry).len(), 4);
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/draft-profile.json"
    );
    let json = serde_json::to_string_pretty(&profile).unwrap() + "\n";
    if std::env::var_os("DECKPILOT_UPDATE_FIXTURES").is_some() {
        std::fs::write(path, &json).unwrap();
    }
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        json,
        "run with DECKPILOT_UPDATE_FIXTURES=1 to rewrite it, and run the older agents against it again"
    );
    let back: Profile = serde_json::from_str(&json).unwrap();
    assert_eq!(back, profile);
}
