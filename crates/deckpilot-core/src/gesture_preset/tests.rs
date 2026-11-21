//! Declaration checks and planning of gesture presets, with fixture action
//! types (the built-in *Brightness Control* is tested in
//! `deckpilot-modules/tests/gesture_presets.rs`).

use deckpilot_device::{ControlId, ControlKind, InputCapability, models};

use super::*;
use crate::graph::NodeSpec;
use crate::{ActionControls, ParameterKind, ParameterSpec, ParameterValue};

fn module() -> ModuleId {
    ModuleId::new("lights").unwrap()
}

/// `lights.dim`: an integer `step` (1-50, default 10); keys only, or keys and
/// stepping dials with `dials`.
fn dim(dials: bool) -> ActionType {
    ActionType {
        id: ActionTypeId::new("lights.dim").unwrap(),
        name: "Dim".into(),
        description: String::new(),
        category: "Lights".into(),
        parameters: vec![ParameterSpec {
            key: "step".into(),
            label: "Step".into(),
            description: None,
            kind: ParameterKind::Integer {
                min: Some(1),
                max: Some(50),
            },
            required: true,
            default: Some(ParameterValue::Integer(10)),
        }],
        requires_project: false,
        node: NodeSpec::default(),
        deprecated: false,
        controls: Some(if dials {
            ActionControls::keys_and_steps()
        } else {
            ActionControls::keys()
        }),
        editor_navigation: None,
        title: None,
    }
}

/// `lights.toggle`: no parameters, press and click only.
fn toggle() -> ActionType {
    ActionType {
        id: ActionTypeId::new("lights.toggle").unwrap(),
        controls: Some(ActionControls::new().with(
            ControlKind::Key,
            [InputCapability::Press, InputCapability::Click],
        )),
        parameters: Vec::new(),
        ..dim(false)
    }
}

fn types() -> Vec<ActionType> {
    vec![dim(true), toggle()]
}

fn assign(gesture: PresetGesture, action: &str, step: Option<i64>) -> GestureAssignment {
    let mut parameters = ActionParameters::new();
    if let Some(step) = step {
        parameters = parameters.with("step", ParameterValue::Integer(step));
    }
    GestureAssignment {
        gesture,
        action: ActionTypeId::new(action).unwrap(),
        parameters,
        label: format!("{gesture} label"),
    }
}

fn preset(gestures: Vec<GestureAssignment>) -> GesturePreset {
    GesturePreset {
        id: GesturePresetId::new("lights.dimmer").unwrap(),
        name: "Dimmer".into(),
        description: String::new(),
        category: "Lights".into(),
        icon: None,
        allow_partial: false,
        gestures,
    }
}

fn dimmer() -> GesturePreset {
    preset(vec![
        assign(PresetGesture::Click, "lights.dim", Some(5)),
        assign(PresetGesture::DoubleClick, "lights.dim", Some(20)),
        assign(PresetGesture::LongPress, "lights.dim", Some(50)),
    ])
}

#[test]
fn a_valid_declaration_passes() {
    assert_eq!(dimmer().check(&module(), &types()), Ok(()));
}

#[test]
fn ids_are_namespaced_and_serialize_in_camel_case() {
    let id = GesturePresetId::new("acme.lights.dimmer").unwrap();
    assert_eq!(id.owner(), Some(ModuleId::new("acme.lights").unwrap()));
    assert!(GesturePresetId::new("Bad Id").is_err());
    let json = serde_json::to_value(dimmer()).unwrap();
    assert_eq!(json["gestures"][1]["gesture"], "doubleClick");
    assert_eq!(json["allowPartial"], false);
    let back: GesturePreset = serde_json::from_value(json).unwrap();
    assert_eq!(back, dimmer());
}

#[test]
fn foreign_namespace_unknown_actions_and_bad_parameters_are_refused() {
    let mut foreign = dimmer();
    foreign.id = GesturePresetId::new("other.dimmer").unwrap();
    assert!(matches!(
        &foreign.check(&module(), &types()).unwrap_err()[..],
        [PresetIssue::Namespace { .. }]
    ));

    let unknown = preset(vec![
        assign(PresetGesture::Click, "lights.missing", None),
        // Another module's action, even one that exists, is not this one's.
        assign(PresetGesture::DoubleClick, "deck.brightness", None),
    ]);
    let issues = unknown.check(&module(), &types()).unwrap_err();
    assert_eq!(issues.len(), 2);
    assert!(
        issues
            .iter()
            .all(|i| matches!(i, PresetIssue::UnknownAction { .. }))
    );

    let invalid = preset(vec![assign(PresetGesture::Click, "lights.dim", Some(99))]);
    let issues = invalid.check(&module(), &types()).unwrap_err();
    assert!(
        matches!(&issues[..], [PresetIssue::InvalidParameters { reason, .. }] if reason.contains("step")),
        "{issues:?}"
    );

    let mut extra = preset(vec![assign(PresetGesture::Click, "lights.toggle", None)]);
    extra.gestures[0].parameters =
        ActionParameters::new().with("nope", ParameterValue::Boolean(true));
    assert!(matches!(
        &extra.check(&module(), &types()).unwrap_err()[..],
        [PresetIssue::InvalidParameters { .. }]
    ));
}

#[test]
fn gestures_are_unique_and_release_never_joins_double_or_long() {
    let twice = preset(vec![
        assign(PresetGesture::Click, "lights.dim", None),
        assign(PresetGesture::Click, "lights.toggle", None),
    ]);
    assert!(twice.check(&module(), &types()).unwrap_err().contains(
        &PresetIssue::DuplicateGesture {
            preset: "lights.dimmer".into(),
            gesture: PresetGesture::Click,
        }
    ));
    let mixed = preset(vec![
        assign(PresetGesture::Release, "lights.dim", None),
        assign(PresetGesture::LongPress, "lights.dim", None),
    ]);
    assert!(mixed.check(&module(), &types()).unwrap_err().contains(
        &PresetIssue::ReleaseWithGestures {
            preset: "lights.dimmer".into()
        }
    ));
    assert!(matches!(
        &preset(Vec::new()).check(&module(), &types()).unwrap_err()[..],
        [PresetIssue::Empty { .. }]
    ));
}

#[test]
fn an_action_must_run_from_its_gesture_somewhere() {
    // `lights.toggle` reacts to press and click only: no long press.
    let held = preset(vec![assign(
        PresetGesture::LongPress,
        "lights.toggle",
        None,
    )]);
    assert!(matches!(
        &held.check(&module(), &types()).unwrap_err()[..],
        [PresetIssue::UnsupportedGesture { .. }]
    ));
    let turn = preset(vec![assign(
        PresetGesture::RotateClockwise,
        "lights.dim",
        None,
    )]);
    assert_eq!(turn.check(&module(), &[dim(true)]), Ok(()));
    assert!(turn.check(&module(), &[dim(false)]).is_err());
}

#[test]
fn texts_are_required_and_bounded() {
    let mut nameless = dimmer();
    nameless.name = "  ".into();
    nameless.gestures[0].label = "x".repeat(MAX_PRESET_LABEL + 1);
    let issues = nameless.check(&module(), &types()).unwrap_err();
    assert_eq!(issues.len(), 2, "{issues:?}");
}

fn key(index: u16) -> ControlId {
    ControlId::Key { index }
}

fn caps(model: &deckpilot_device::DeviceModel, control: ControlId) -> ControlCapabilities {
    model.capabilities.control_capabilities(control).unwrap()
}

#[test]
fn on_an_mk2_key_every_gesture_goes_on_in_gesture_mode() {
    let mk2 = models::stream_deck_mk2();
    let plan = dimmer()
        .plan(key(0), &caps(&mk2, key(0)), &types())
        .unwrap();
    assert_eq!(
        plan.applied,
        vec![
            (PresetGesture::Click, Trigger::Click),
            (PresetGesture::DoubleClick, Trigger::DoubleClick),
            (PresetGesture::LongPress, Trigger::LongPress),
        ]
    );
    assert_eq!(plan.skipped, Vec::new());
}

#[test]
fn single_press_and_release_stay_raw() {
    let mk2 = models::stream_deck_mk2();
    let raw = preset(vec![
        assign(PresetGesture::Click, "lights.dim", None),
        assign(PresetGesture::Release, "lights.dim", Some(1)),
    ]);
    let (bindings, skipped) = raw
        .bindings(key(4), &caps(&mk2, key(4)), &types(), |_| {
            ActionInstanceId::new()
        })
        .unwrap();
    assert_eq!(skipped, Vec::new());
    let triggers: Vec<_> = bindings.iter().map(|b| b.trigger).collect();
    assert_eq!(triggers, vec![Trigger::Press, Trigger::Release]);
    assert!(
        bindings
            .iter()
            .all(|b| b.control == key(4) && b.label.is_none())
    );
}

#[test]
fn unsupported_gestures_refuse_atomically_or_are_skipped_when_partial() {
    // A Stream Deck + dial: rotation, press, click. `lights.dim` steps on
    // dials (rotate, press, click), so double and long press cannot go on.
    let plus = models::stream_deck_plus();
    let dial = ControlId::Encoder { index: 0 };
    let mut both = preset(vec![
        assign(PresetGesture::Click, "lights.dim", None),
        assign(PresetGesture::LongPress, "lights.dim", None),
        assign(PresetGesture::RotateClockwise, "lights.dim", None),
    ]);
    assert_eq!(
        both.plan(dial, &caps(&plus, dial), &types()),
        Err(PresetRefusal::Unsupported {
            control: dial,
            gestures: vec![PresetGesture::LongPress],
        })
    );
    both.allow_partial = true;
    let plan = both.plan(dial, &caps(&plus, dial), &types()).unwrap();
    // Without the long press the dial stays raw: the press fires at once.
    assert_eq!(
        plan.applied,
        vec![
            (PresetGesture::Click, Trigger::Press),
            (PresetGesture::RotateClockwise, Trigger::RotateClockwise),
        ]
    );
    assert_eq!(plan.skipped, vec![PresetGesture::LongPress]);

    // Nothing at all fits a key for a rotation-only preset.
    let mut turn = preset(vec![assign(
        PresetGesture::RotateClockwise,
        "lights.dim",
        None,
    )]);
    turn.allow_partial = true;
    let mk2 = models::stream_deck_mk2();
    assert_eq!(
        turn.plan(key(0), &caps(&mk2, key(0)), &types()),
        Err(PresetRefusal::Nothing { control: key(0) })
    );
}
