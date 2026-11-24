//! Keys whose action's plugin is missing (`docs/key-text.md`, "Keys whose
//! plugin is missing"): the compact missing-plugin face with the key's
//! title and look, never stored; the binding comes back as it was once
//! the plugin serves the action again; a key with another gesture that
//! runs is not marked unavailable.

use deckpilot_device::models;

use super::*;
use crate::testing::Echo;
use crate::{ActionParameters, ActionTypeId, KeyAppearance, ParameterValue};

const PLUGIN_TYPE: &str = "acme.tools.greet";

fn key(index: u16) -> ControlId {
    ControlId::Key { index }
}

struct Setup {
    profile: Profile,
    layout: DeviceLayout,
    greet: ActionInstanceId,
    echo: ActionInstanceId,
}

fn look() -> KeyAppearance {
    KeyAppearance {
        background: Some(crate::Color([20, 40, 90]).into()),
        icon: Some(crate::IconRef {
            id: crate::AssetRef::parse("deckpilot.house").unwrap(),
            fit: crate::IconFit::Fit,
        }),
        ..KeyAppearance::default()
    }
}

/// Key 0 runs the plugin's `greet` (with a parameter and the user's own
/// title and look) on its press; key 1 the core's `test.echo`.
fn setup() -> Setup {
    let mut profile = Profile::new("Work");
    let instance = |name: &str, action_type: &str| crate::ActionInstance {
        id: ActionInstanceId::new(),
        name: name.into(),
        action_type: ActionTypeId::new(action_type).unwrap(),
        parameters: ActionParameters::new().with("message", ParameterValue::Text("hi".into())),
        description: String::new(),
        icon: Some("rocket".into()),
        label: None,
    };
    let greet = instance("Greet", PLUGIN_TYPE);
    let echo = instance("Echo", "test.echo");
    let (greet_id, echo_id) = (greet.id, echo.id);
    profile.actions.extend([greet, echo]);
    let mut layout = DeviceLayout::new("Main", DeviceModelId::new(models::STREAM_DECK_MK2_ID));
    let home = layout.home_page;
    layout.page_mut(home).unwrap().bindings.extend([
        ControlBinding {
            label: Some("Repo SHOP".into()),
            appearance: Some(look()),
            ..ControlBinding::new(key(0), Trigger::Press, BindingTarget::action(greet_id))
        },
        ControlBinding::new(key(1), Trigger::Press, BindingTarget::action(echo_id)),
    ]);
    Setup {
        profile,
        layout,
        greet: greet_id,
        echo: echo_id,
    }
}

/// The core's handlers, and the plugin's `greet` when `installed`; the
/// plugin's type is marked unavailable otherwise, as the module registry
/// does for a plugin that is disabled or not installed.
fn registry(installed: bool) -> ActionRegistry {
    let mut registry = crate::testing::registry();
    if installed {
        registry
            .register(Box::new(Echo::with_id(PLUGIN_TYPE)))
            .unwrap();
    } else {
        registry.mark_unavailable(
            [ActionTypeId::new(PLUGIN_TYPE).unwrap()],
            "Acme Tools is not installed",
        );
    }
    registry
}

fn faces(registry: &ActionRegistry, s: &Setup) -> BTreeMap<ControlId, KeyFace> {
    let model = models::stream_deck_mk2();
    page_faces(
        registry,
        &s.layout,
        s.layout.home_page,
        &s.profile,
        None,
        &model.capabilities.controls(),
        Some(&model.capabilities),
    )
    .into_iter()
    .collect()
}

#[test]
fn a_missing_plugin_shows_the_plug_with_the_title_and_look_of_the_key() {
    let s = setup();
    let faces = faces(&registry(false), &s);
    let face = &faces[&key(0)];
    assert!(face.unavailable);
    assert_eq!(face.symbol.as_deref(), Some(symbols::PLUGIN_MISSING));
    assert_eq!(face.title, "Repo SHOP", "the user's title stays");
    assert_eq!(face.detail, None, "no warning text on the key");
    assert_eq!(face.appearance, Some(look()), "the key's look is kept");
    // The other keys are not touched.
    assert!(!faces[&key(1)].unavailable);
    assert_eq!(faces[&key(1)].style, FaceStyle::Action);
    // Untitled, it shows the action's name.
    let mut untitled = setup();
    untitled.layout.pages[0].bindings[0].label = None;
    let face = &self::faces(&registry(false), &untitled)[&key(0)];
    assert!(face.unavailable);
    assert_eq!(face.title, "Greet");
}

#[test]
fn the_key_is_back_as_it_was_once_the_plugin_serves_it_again() {
    let s = setup();
    let stored = (s.profile.clone(), s.layout.clone());
    let missing = faces(&registry(false), &s);
    // Nothing of the fallback is stored: profile and layout are unchanged.
    assert_eq!((s.profile.clone(), s.layout.clone()), stored);
    let restored = faces(&registry(true), &s);
    let face = &restored[&key(0)];
    assert!(!face.unavailable);
    assert_eq!(face.style, FaceStyle::Action);
    assert_eq!(face.title, "Repo SHOP");
    assert_eq!(
        face.symbol.as_deref(),
        Some("rocket"),
        "the action's own icon"
    );
    assert_eq!(face.appearance, Some(look()));
    assert_ne!(missing[&key(0)], *face);
    // The same as a key that never lost its plugin.
    assert_eq!(restored, faces(&registry(true), &setup_like(&s)));
}

/// `s` again (a copy, as a fresh read of the same stored profile).
fn setup_like(s: &Setup) -> Setup {
    Setup {
        profile: s.profile.clone(),
        layout: s.layout.clone(),
        greet: s.greet,
        echo: s.echo,
    }
}

#[test]
fn another_gesture_that_runs_keeps_the_key_from_looking_unavailable() {
    let mut s = setup();
    // Key 0: click runs the missing plugin's action, long press the echo.
    let page = &mut s.layout.pages[0];
    page.bindings[0].trigger = Trigger::Click;
    page.bindings.push(ControlBinding::new(
        key(0),
        Trigger::LongPress,
        BindingTarget::action(s.echo),
    ));
    let face = &faces(&registry(false), &s)[&key(0)];
    assert!(!face.unavailable, "the long press still runs");
    assert_eq!(face.style, FaceStyle::Action);
    assert_eq!(face.title, "Repo SHOP", "the shown gesture's title");
    assert_eq!(face.detail, None);
    assert_eq!(face.appearance, Some(look()));

    // The shown gesture runs, another one does not: the key as it is.
    let mut s = setup();
    let page = &mut s.layout.pages[0];
    page.bindings[1] = ControlBinding::new(key(1), Trigger::Click, BindingTarget::action(s.echo));
    page.bindings.push(ControlBinding::new(
        key(1),
        Trigger::DoubleClick,
        BindingTarget::action(s.greet),
    ));
    let face = &faces(&registry(false), &s)[&key(1)];
    assert!(!face.unavailable);
    assert_eq!(face.title, "Echo");

    // Every gesture needs the missing plugin: the key is unavailable.
    let mut s = setup();
    let page = &mut s.layout.pages[0];
    page.bindings[0].trigger = Trigger::Click;
    page.bindings.push(ControlBinding::new(
        key(0),
        Trigger::LongPress,
        BindingTarget::action(s.greet),
    ));
    assert!(faces(&registry(false), &s)[&key(0)].unavailable);
}

/// The device draws the faces the editor previews: the same pipeline
/// (`page_faces`) for the simulated and the physical device.
#[test]
fn the_device_shows_the_same_face_as_the_preview() {
    let s = setup();
    let preview = faces(&registry(false), &s);
    let mut profile = s.profile.clone();
    profile.layouts.push(s.layout.clone());
    let mut runtime = Runtime::new(
        crate::Engine::new(registry(false)),
        deckpilot_device::ModelCatalog::builtin(),
        crate::MachineConfig::new(crate::Os::current()),
    );
    runtime.set_profile(Some(profile), None);
    let device = DeviceId {
        model: DeviceModelId::new(models::STREAM_DECK_MK2_ID),
        serial: "SIM".into(),
    };
    runtime
        .handle_event(&DeviceEvent::Connected {
            device: device.clone(),
        })
        .unwrap();
    let shown: BTreeMap<ControlId, KeyFace> = runtime.faces(&device).into_iter().collect();
    assert_eq!(shown[&key(0)], preview[&key(0)]);
    assert!(shown[&key(0)].unavailable);
    // Pressing it runs nothing and keeps the binding.
    let pressed = runtime.handle_event(&DeviceEvent::Input {
        device: device.clone(),
        control: key(0),
        input: deckpilot_device::ControlInput::Press,
    });
    assert!(pressed.is_ok());
    assert_eq!(
        runtime
            .profile()
            .map(|p| p.layouts[0].pages[0].bindings.len()),
        Some(2)
    );
}
