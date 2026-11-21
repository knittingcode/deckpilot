//! The built-in gesture preset *Brightness Control* end to end in the core:
//! registered through the module registry like a plugin's presets, applied
//! to a simulated Stream Deck MK.2 key as ordinary bindings that validate,
//! and timed by the core's gesture engine with synthetic instants (no
//! sleeping, no clock).

use std::time::{Duration, Instant};

use deckpilot_core::gesture::engine::{GestureEngine, GestureOutput, KeyContext};
use deckpilot_core::gesture::{Gesture, GestureTimings};
use deckpilot_core::{
    ActionHandler, ActionInstanceId, ActionParameters, BindingTarget, ControlBinding, DeviceLayout,
    FeatureModule, GesturePreset, GesturePresetId, ModuleError, ModuleManifest, ModuleRegistry, Os,
    PresetGesture, PresetIssue, Profile, Trigger,
};
use deckpilot_device::{ControlId, ControlInput, DeviceId, ModelCatalog, models};
use deckpilot_module_deck as deck;

fn key(index: u16) -> ControlId {
    ControlId::Key { index }
}

fn registry() -> ModuleRegistry {
    deckpilot_modules::builtin(Os::current())
}

fn brightness_control(registry: &ModuleRegistry) -> GesturePreset {
    let id = GesturePresetId::new(deck::BRIGHTNESS_CONTROL).unwrap();
    let (preset, enabled) = registry.gesture_preset(&id).expect("registered");
    assert!(enabled);
    preset.clone()
}

#[test]
fn the_deck_module_contributes_brightness_control() {
    let registry = registry();
    let preset = brightness_control(&registry);
    assert_eq!(preset.name, "Brightness Control");
    assert_eq!(preset.category, "Device");
    // Listed with its module, as `list_modules` sends it to the app.
    let info = registry
        .modules()
        .into_iter()
        .find(|m| m.manifest.id.as_str() == deck::MODULE_ID)
        .unwrap();
    assert_eq!(info.gesture_presets, vec![preset]);
    let json = serde_json::to_value(&info).unwrap();
    assert_eq!(json["gesturePresets"][0]["id"], deck::BRIGHTNESS_CONTROL);
    assert_eq!(json["gesturePresets"][0]["gestures"][0]["gesture"], "click");
    assert_eq!(
        json["gesturePresets"][0]["gestures"][0]["parameters"]["change"]["value"],
        "dimmer"
    );
    // Modules without presets send none.
    let general = registry
        .modules()
        .into_iter()
        .find(|m| m.manifest.id.as_str() == "general")
        .unwrap();
    assert!(
        serde_json::to_value(&general)
            .unwrap()
            .get("gesturePresets")
            .is_none()
    );
}

/// The deck module with a preset of its own instead of the built-in one.
struct DeckWith(deck::DeckModule, GesturePreset);

impl FeatureModule for DeckWith {
    fn manifest(&self) -> &ModuleManifest {
        self.0.manifest()
    }

    fn action_handlers(&self, config: &ActionParameters) -> Vec<Box<dyn ActionHandler>> {
        self.0.action_handlers(config)
    }

    fn gesture_presets(&self) -> Vec<GesturePreset> {
        vec![self.1.clone(), self.1.clone()]
    }
}

#[test]
fn the_registry_refuses_a_module_with_a_broken_preset() {
    let mut broken = deck::brightness_control();
    broken.gestures[0].action = deckpilot_core::ActionTypeId::new("general.log").unwrap();
    let mut registry = ModuleRegistry::new(Os::current());
    let Err(ModuleError::GesturePreset(issues)) =
        registry.register(Box::new(DeckWith(deck::DeckModule::new(), broken)))
    else {
        panic!("a broken preset is refused");
    };
    assert!(issues.contains(&PresetIssue::Duplicate(deck::BRIGHTNESS_CONTROL.into())));
    assert!(issues.iter().any(
        |i| matches!(i, PresetIssue::UnknownAction { action, .. } if action == "general.log")
    ));
    // Nothing of the module was registered.
    assert_eq!(registry.modules(), Vec::new());
}

/// A profile with one single-action key from before presets, and the
/// preset applied to an empty key of a fresh MK.2 layout.
fn applied() -> (
    Profile,
    DeviceLayout,
    Vec<(PresetGesture, ActionInstanceId)>,
) {
    let registry = registry();
    let actions = registry.action_registry();
    let preset = brightness_control(&registry);
    let types: Vec<_> = registry
        .modules()
        .into_iter()
        .flat_map(|m| m.action_types)
        .collect();
    let mk2 = models::stream_deck_mk2();
    let mut profile = Profile::new("Presets");
    let next = actions
        .create_instance("Next page", deck::next_page_id(), &ActionParameters::new())
        .unwrap();
    let legacy = ControlBinding::new(key(0), Trigger::Press, BindingTarget::action(next.id));
    profile.actions.push(next);
    let mut made = Vec::new();
    let (bindings, skipped) = preset
        .bindings(
            key(7),
            &mk2.capabilities.control_capabilities(key(7)).unwrap(),
            &types,
            |assignment| {
                // The editor creates (or reuses) an ordinary action per gesture.
                let instance = actions
                    .create_instance(
                        &assignment.label,
                        assignment.action.clone(),
                        &assignment.parameters,
                    )
                    .unwrap();
                made.push((assignment.gesture, instance.id));
                let id = instance.id;
                profile.actions.push(instance);
                id
            },
        )
        .unwrap();
    assert_eq!(skipped, Vec::new());
    let mut layout = DeviceLayout::new("Main", mk2.id.clone());
    layout.pages[0].bindings.push(legacy);
    layout.pages[0].bindings.extend(bindings);
    (profile, layout, made)
}

#[test]
fn applied_to_an_mk2_key_it_is_ordinary_bindings() {
    let (profile, layout, made) = applied();
    let catalog = ModelCatalog::builtin();
    assert_eq!(layout.validate(&catalog, &profile), Vec::new());
    let page = &layout.pages[0];
    // The single-action key is untouched and stays raw.
    assert_eq!(page.bindings[0].trigger, Trigger::Press);
    assert!(!page.bound_gestures(key(0)).any());
    // Three gesture bindings of three Brightness actions, release free.
    let triggers: Vec<_> = page
        .bindings
        .iter()
        .filter(|b| b.control == key(7))
        .map(|b| b.trigger)
        .collect();
    assert_eq!(
        triggers,
        vec![Trigger::Click, Trigger::DoubleClick, Trigger::LongPress]
    );
    assert_eq!(made.len(), 3);
    let names: Vec<_> = made
        .iter()
        .map(|(_, id)| profile.action(*id).unwrap().name.clone())
        .collect();
    assert_eq!(
        names,
        [
            "Decrease brightness by 10%",
            "Set brightness to 100%",
            "Set brightness to 50%"
        ]
    );
    for (_, id) in &made {
        let action = profile.action(*id).unwrap();
        assert_eq!(action.action_type, deck::brightness_id());
    }
    // Stored as plain bindings: nothing of the preset in the document.
    let json = serde_json::to_string(&layout).unwrap();
    assert!(!json.contains("brightness-control"));
}

struct Bench {
    engine: GestureEngine,
    base: Instant,
    context: KeyContext,
    device: DeviceId,
}

impl Bench {
    fn new(layout: &DeviceLayout) -> Self {
        let page = &layout.pages[0];
        Self {
            engine: GestureEngine::new(),
            base: Instant::now(),
            context: KeyContext {
                bound: page.bound_gestures(key(7)),
                timings: page.gesture_timings(key(7), GestureTimings::default()),
                page: Some(page.id),
                generation: 1,
            },
            device: DeviceId {
                model: layout.model.clone(),
                serial: "MK2-SIM".into(),
            },
        }
    }

    fn input(&mut self, ms: u64, input: ControlInput) -> Vec<Gesture> {
        let context = self.context;
        let out = self.engine.on_input(
            self.base + Duration::from_millis(ms),
            &self.device,
            1,
            key(7),
            input,
            || context,
        );
        gestures(out)
    }

    fn advance(&mut self, ms: u64) -> Vec<Gesture> {
        gestures(self.engine.advance(self.base + Duration::from_millis(ms)))
    }
}

fn gestures(out: Vec<GestureOutput>) -> Vec<Gesture> {
    out.into_iter()
        .map(|o| match o {
            GestureOutput::Fired(fired) => fired.gesture,
            GestureOutput::Edge { .. } => panic!("a gesture key passes no raw edges"),
        })
        .collect()
}

/// The binding each recognized gesture runs, by its action's name.
fn runs(profile: &Profile, layout: &DeviceLayout, gesture: Gesture) -> String {
    let binding = layout.pages[0].gesture_binding(key(7), gesture).unwrap();
    let BindingTarget::Action { action, .. } = &binding.target else {
        panic!("an action");
    };
    profile.action(*action).unwrap().name.clone()
}

#[test]
fn the_core_engine_times_the_applied_gestures() {
    let (profile, layout, _) = applied();
    let mut bench = Bench::new(&layout);
    // Defaults: double press within 300 ms of the release, long press 700 ms.
    assert_eq!(bench.context.timings, GestureTimings::default());

    // A single press waits out the double-press window, then dims.
    assert_eq!(bench.input(0, ControlInput::Press), Vec::new());
    assert_eq!(bench.input(80, ControlInput::Release), Vec::new());
    assert_eq!(
        bench.advance(380),
        Vec::new(),
        "the window includes its end"
    );
    assert_eq!(bench.advance(381), vec![Gesture::Click]);
    assert_eq!(
        runs(&profile, &layout, Gesture::Click),
        "Decrease brightness by 10%"
    );

    // A second press inside the window is a double press, at once.
    assert_eq!(bench.input(1_000, ControlInput::Press), Vec::new());
    assert_eq!(bench.input(1_060, ControlInput::Release), Vec::new());
    assert_eq!(
        bench.input(1_200, ControlInput::Press),
        vec![Gesture::DoubleClick]
    );
    // Its release is swallowed: no click, no release action.
    assert_eq!(bench.input(1_250, ControlInput::Release), Vec::new());
    assert_eq!(bench.advance(5_000), Vec::new());
    assert_eq!(
        runs(&profile, &layout, Gesture::DoubleClick),
        "Set brightness to 100%"
    );

    // Held to the threshold: a long press, once; the release does nothing.
    assert_eq!(bench.input(10_000, ControlInput::Press), Vec::new());
    assert_eq!(bench.advance(10_699), Vec::new());
    assert_eq!(bench.advance(10_700), vec![Gesture::LongPress]);
    assert_eq!(bench.advance(20_000), Vec::new());
    assert_eq!(bench.input(20_001, ControlInput::Release), Vec::new());
    assert_eq!(
        runs(&profile, &layout, Gesture::LongPress),
        "Set brightness to 50%"
    );
    assert_eq!(bench.engine.pending(), 0);
}
