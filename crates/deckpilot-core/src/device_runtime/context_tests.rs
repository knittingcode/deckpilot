//! Contexts in the device runtime (decision D1: warn, do not refuse): a
//! binding outside its action's contexts runs, and the first run of each
//! is reported; a gesture binding runs in `gesture`, any other in `key`;
//! a Random Action's pick and a detached action carry their context; runs
//! from the app are never checked.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use deckpilot_device::{ControlInput, models};

use super::*;
use crate::graph::NodeSpec;
use crate::{
    ActionContexts, ActionControls, ActionHandler, ActionParameters, ActionType, ActionTypeId,
    ContextWarning, ContextWarnings, ExecutionContext, IncompatibilityReason, Os,
};

struct Counting {
    definition: ActionType,
    runs: Arc<AtomicUsize>,
    detached: bool,
}

impl ActionHandler for Counting {
    fn definition(&self) -> &ActionType {
        &self.definition
    }

    fn execute(&self, _: &ActionParameters, _: &ExecutionContext<'_>) -> ActionResult {
        self.runs.fetch_add(1, Ordering::SeqCst);
        ActionResult::success("ran")
    }

    fn runs_detached(&self) -> bool {
        self.detached
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
    fn contexts(&self) -> Vec<(ActionInstanceId, ActionContext)> {
        std::mem::take(&mut *self.0.lock().unwrap())
            .into_iter()
            .map(|w| match w.reason {
                IncompatibilityReason::ContextUnsupported { context, .. } => (w.action, context),
                other => panic!("{other:?}"),
            })
            .collect()
    }
}

const STEPS: &str = "test.steps";
const PICKED: &str = "test.picked";
const KEYS: &str = "test.keys";
const LATER: &str = "test.later";

fn device() -> DeviceId {
    DeviceId {
        model: DeviceModelId::new(models::STREAM_DECK_MK2_ID),
        serial: "MK2".into(),
    }
}

fn key(index: u16) -> ControlId {
    ControlId::Key { index }
}

struct Setup {
    runtime: Runtime,
    recorder: Arc<Recorder>,
    runs: BTreeMap<&'static str, Arc<AtomicUsize>>,
    ids: BTreeMap<&'static str, ActionInstanceId>,
}

fn setup() -> Setup {
    let mut registry = ActionRegistry::default();
    let mut runs = BTreeMap::new();
    let mut ids = BTreeMap::new();
    let mut profile = Profile::new("Work");
    let steps = [ActionContext::Sequence, ActionContext::Workflow];
    for (id, contexts, detached) in [
        (STEPS, &steps[..], false),
        (PICKED, &steps[..], false),
        (KEYS, &[ActionContext::Key][..], false),
        (LATER, &steps[..], true),
    ] {
        let count = Arc::new(AtomicUsize::new(0));
        registry
            .register(Box::new(Counting {
                definition: ActionType {
                    contexts: Some(ActionContexts::of(contexts.iter().copied())),
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
                },
                runs: count.clone(),
                detached,
            }))
            .unwrap();
        let instance = registry
            .create_instance(id, ActionTypeId::new(id).unwrap(), &ActionParameters::new())
            .unwrap();
        ids.insert(id, instance.id);
        runs.insert(id, count);
        profile.actions.push(instance);
    }
    let run = |id: &str| BindingTarget::action(ids[id]);
    let mut layout = DeviceLayout::new("Keys", device().model);
    layout.pages[0].bindings = vec![
        ControlBinding::new(key(0), Trigger::Press, run(STEPS)),
        ControlBinding::new(key(1), Trigger::LongPress, run(STEPS)),
        ControlBinding::new(key(2), Trigger::Click, run(KEYS)),
        ControlBinding::new(key(3), Trigger::DoubleClick, run(KEYS)),
        ControlBinding::new(key(4), Trigger::Press, run(LATER)),
    ];
    profile.layouts.push(layout);
    let recorder = Arc::new(Recorder::default());
    let mut runtime = Runtime::new(
        Engine::new(registry).with_context_warnings(recorder.clone()),
        ModelCatalog::builtin(),
        MachineConfig::new(Os::current()),
    );
    runtime.set_profile(Some(profile), None);
    runtime
        .handle_event(&DeviceEvent::Connected { device: device() })
        .unwrap();
    Setup {
        runtime,
        recorder,
        runs,
        ids,
    }
}

fn press(index: u16) -> DeviceEvent {
    DeviceEvent::Input {
        device: device(),
        control: key(index),
        input: ControlInput::Press,
    }
}

#[test]
fn a_binding_outside_its_contexts_runs_and_is_reported_once() {
    let mut s = setup();
    for _ in 0..3 {
        let outcome = s.runtime.handle_event(&press(0)).unwrap();
        assert!(
            matches!(outcome, RuntimeOutcome::Action { ref result, .. } if result.is_success()),
            "{outcome:?}"
        );
    }
    assert_eq!(s.runs[STEPS].load(Ordering::SeqCst), 3);
    assert_eq!(s.recorder.contexts(), [(s.ids[STEPS], ActionContext::Key)]);

    // A long press is a gesture: the same action there is reported once.
    for _ in 0..2 {
        let outcome = s
            .runtime
            .dispatch_gesture(&device(), key(1), Gesture::LongPress)
            .unwrap();
        assert!(
            matches!(outcome, RuntimeOutcome::Action { .. }),
            "{outcome:?}"
        );
    }
    assert_eq!(s.runs[STEPS].load(Ordering::SeqCst), 5);
    assert_eq!(
        s.recorder.contexts(),
        [(s.ids[STEPS], ActionContext::Gesture)]
    );

    // A click is a key's input; a double click is a gesture.
    s.runtime
        .dispatch_gesture(&device(), key(2), Gesture::Click)
        .unwrap();
    assert_eq!(s.recorder.contexts(), Vec::new());
    s.runtime
        .dispatch_gesture(&device(), key(3), Gesture::DoubleClick)
        .unwrap();
    assert_eq!(s.runs[KEYS].load(Ordering::SeqCst), 2);
    assert_eq!(
        s.recorder.contexts(),
        [(s.ids[KEYS], ActionContext::Gesture)]
    );

    // Run from the app: never checked.
    for id in [STEPS, KEYS] {
        assert!(s.runtime.run_action(s.ids[id]).unwrap().is_success());
    }
    assert_eq!(s.recorder.contexts(), Vec::new());
}

#[test]
fn a_random_pick_runs_as_a_key_and_a_detached_action_keeps_its_context() {
    let mut s = setup();
    let outcome = s
        .runtime
        .run_picked(
            &device(),
            BindingTarget::action(s.ids[PICKED]),
            crate::RunOrigin::Gesture,
        )
        .unwrap();
    assert!(
        matches!(outcome, RuntimeOutcome::Action { .. }),
        "{outcome:?}"
    );
    assert_eq!(s.runs[PICKED].load(Ordering::SeqCst), 1);
    assert_eq!(s.recorder.contexts(), [(s.ids[PICKED], ActionContext::Key)]);

    let RuntimeOutcome::ActionRequested {
        action,
        overrides,
        device: from,
        context,
    } = s.runtime.handle_event(&press(4)).unwrap()
    else {
        panic!("expected a detached request");
    };
    assert_eq!(context, Some(ActionContext::Key));
    let run = s
        .runtime
        .detached_action(
            action,
            Some(overrides),
            Some(from),
            crate::RunOrigin::Key,
            context,
        )
        .unwrap();
    assert!(run.run().unwrap().is_success());
    assert_eq!(s.runs[LATER].load(Ordering::SeqCst), 1);
    assert_eq!(s.recorder.contexts(), [(s.ids[LATER], ActionContext::Key)]);
    // From the app (`runAction`): no context.
    let run = s
        .runtime
        .detached_action(action, None, None, crate::RunOrigin::App, None)
        .unwrap();
    assert!(run.run().unwrap().is_success());
    assert_eq!(s.recorder.contexts(), Vec::new());
}

/// A request serialized before `context` existed reads as one without a
/// context, and one without a context serializes as before.
#[test]
fn an_action_request_without_a_context_reads_and_writes_as_before() {
    let action = ActionInstanceId::new();
    let without = RuntimeOutcome::ActionRequested {
        action,
        overrides: ActionParameters::new(),
        device: device(),
        context: None,
    };
    let json = serde_json::to_value(&without).unwrap();
    // Internally tagged: the request's fields sit beside `type`.
    assert_eq!(json["type"], "actionRequested");
    assert!(json.get("context").is_none(), "{json}");
    assert_eq!(
        serde_json::from_value::<RuntimeOutcome>(json.clone()).unwrap(),
        without
    );

    // The same with a context: only the field is added, and it reads back.
    let with = RuntimeOutcome::ActionRequested {
        action,
        overrides: ActionParameters::new(),
        device: device(),
        context: Some(ActionContext::Gesture),
    };
    let written = serde_json::to_string(&with).unwrap();
    assert!(written.contains(r#""context":"gesture""#), "{written}");
    assert_eq!(
        serde_json::from_str::<RuntimeOutcome>(&written).unwrap(),
        with
    );
    let older = written.replace(r#","context":"gesture""#, "");
    assert_eq!(serde_json::to_string(&without).unwrap(), older);
    assert_eq!(
        serde_json::from_str::<RuntimeOutcome>(&older).unwrap(),
        without
    );
}
