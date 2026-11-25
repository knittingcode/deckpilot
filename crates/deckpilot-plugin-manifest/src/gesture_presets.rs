//! `contributes.gesturePresets` (plugin API 1.10): the rules of
//! `deckpilot_core::GesturePreset::check`, on the manifest, so a broken
//! preset is a manifest error before anything is installed.
//!
//! Each preset has a local id (unique), a name, an optional description, a
//! category and 1 to [`MAX_PRESET_GESTURES`] gestures. Each gesture is one of
//! [`PRESET_GESTURES`], at most once, never a `release` next to a
//! `doubleClick` or `longPress` (a control cannot have both). It names one of
//! the plugin's own actions, gives only parameters that action declares,
//! with values that fit them, and every required parameter without a
//! default, and is a gesture the action can run from (its `controls`, else
//! keys with every key input) and, when the action declares `contexts`
//! (plugin API 1.11), one whose context it declares (`gesture` for a
//! double or long press, `key` otherwise). A preset never names a
//! permission: it grants nothing.

use deckpilot_device::{ControlKind, InputCapability};

use super::{Checker, IssueCode, Manifest, default_value};
use crate::identity::is_local_id;
use crate::model::{
    Action, GESTURE_PRESETS_API, GesturePreset, MAX_GESTURE_PRESETS, MAX_PRESET_GESTURES,
    PRESET_GESTURES, PresetAssignment,
};

/// The inputs a control must produce for `gesture` (a single press is the
/// raw press or the click, as the control's mode decides).
fn inputs(gesture: &str) -> &'static [InputCapability] {
    match gesture {
        "click" => &[InputCapability::Press, InputCapability::Click],
        "doubleClick" => &[InputCapability::DoubleClick],
        "longPress" => &[InputCapability::LongPress],
        "release" => &[InputCapability::Release],
        "rotateClockwise" => &[InputCapability::RotateClockwise],
        "rotateCounterClockwise" => &[InputCapability::RotateCounterClockwise],
        _ => &[],
    }
}

/// Whether `action` can run from `gesture` on some control.
fn runs_from(action: &Action, gesture: &str) -> bool {
    let inputs = inputs(gesture);
    match &action.controls {
        Some(controls) => controls.bindings.iter().any(|(kind, declared)| {
            inputs
                .iter()
                .any(|input| declared.contains(input) && input.applies_to(*kind))
        }),
        // No declaration: keys, with every key input.
        None => inputs
            .iter()
            .any(|input| input.applies_to(ControlKind::Key)),
    }
}

pub(super) fn check(c: &mut Checker, m: &Manifest) {
    let Some(presets) = &m.contributes.gesture_presets else {
        return;
    };
    let required = m.engines.plugin_api;
    if required.minimum < GESTURE_PRESETS_API {
        c.error(
            "contributes.gesturePresets",
            IssueCode::Conflict,
            format!(
                "needs plugin API {GESTURE_PRESETS_API}: set engines.pluginApi to \
                 `^{GESTURE_PRESETS_API}` (it is `{required}`)"
            ),
        );
        return;
    }
    c.count(
        "contributes.gesturePresets",
        presets.len(),
        1,
        MAX_GESTURE_PRESETS,
    );
    for (index, preset) in presets.iter().enumerate() {
        self::preset(
            c,
            m,
            &format!("contributes.gesturePresets[{index}]"),
            preset,
        );
    }
    c.unique("contributes.gesturePresets", presets.iter().map(|p| &p.id));
}

fn preset(c: &mut Checker, m: &Manifest, path: &str, preset: &GesturePreset) {
    if !is_local_id(&preset.id, 48) {
        c.error(
            format!("{path}.id"),
            IssueCode::InvalidId,
            format!(
                "`{}` must be 1-48 lowercase letters and digits, starting with a letter, with \
                 single hyphens",
                preset.id
            ),
        );
    }
    c.text(&format!("{path}.name"), &preset.name, 1, 48);
    c.opt_text(
        &format!("{path}.description"),
        preset.description.as_ref(),
        200,
    );
    c.text(&format!("{path}.category"), &preset.category, 1, 32);
    c.count(
        &format!("{path}.gestures"),
        preset.gestures.len(),
        1,
        MAX_PRESET_GESTURES,
    );
    c.unique(
        &format!("{path}.gestures"),
        preset.gestures.iter().map(|g| &g.gesture),
    );
    let has = |name: &str| preset.gestures.iter().any(|g| g.gesture == name);
    if has("release") && (has("doubleClick") || has("longPress")) {
        c.error(
            format!("{path}.gestures"),
            IssueCode::Conflict,
            "`release` cannot be assigned next to `doubleClick` or `longPress`: a control \
             reacts either to its raw press and release or to gestures",
        );
    }
    for (j, assignment) in preset.gestures.iter().enumerate() {
        self::assignment(c, m, &format!("{path}.gestures[{j}]"), assignment);
    }
}

fn assignment(c: &mut Checker, m: &Manifest, path: &str, assignment: &PresetAssignment) {
    let actions = m.contributes.actions();
    c.text(&format!("{path}.label"), &assignment.label, 1, 64);
    if !PRESET_GESTURES.contains(&assignment.gesture.as_str()) {
        c.error(
            format!("{path}.gesture"),
            IssueCode::InvalidValue,
            format!(
                "`{}` is not a gesture: use one of {}",
                assignment.gesture,
                PRESET_GESTURES.join(", ")
            ),
        );
    }
    let Some(action) = actions.iter().find(|a| a.id == assignment.action) else {
        c.error(
            format!("{path}.action"),
            IssueCode::UnknownReference,
            format!(
                "`{}` is not one of this plugin's actions (contributes.actions[].id)",
                assignment.action
            ),
        );
        return;
    };
    let given = assignment.parameters.as_ref();
    for (key, value) in given.into_iter().flatten() {
        let field = format!("{path}.parameters.{key}");
        match action.parameters.iter().find(|p| &p.key == key) {
            Some(parameter) => default_value(c, &field, &parameter.kind, value),
            None => c.error(
                field,
                IssueCode::UnknownReference,
                format!("`{key}` is not a parameter of the action `{}`", action.id),
            ),
        }
    }
    for parameter in &action.parameters {
        let set = given.is_some_and(|p| p.contains_key(&parameter.key));
        if parameter.required && parameter.default.is_none() && !set {
            c.error(
                format!("{path}.parameters"),
                IssueCode::InvalidValue,
                format!(
                    "the required parameter `{}` of `{}` has no default: give it a value",
                    parameter.key, action.id
                ),
            );
        }
    }
    if PRESET_GESTURES.contains(&assignment.gesture.as_str())
        && !runs_from(action, &assignment.gesture)
    {
        c.error(
            format!("{path}.gesture"),
            IssueCode::Conflict,
            format!(
                "the action `{}` cannot run from `{}` (its controls)",
                action.id, assignment.gesture
            ),
        );
    }
    // Plugin API 1.11: an action that declares contexts must declare the
    // one its gesture binds it in (`GesturePreset::check` in the core).
    let context = if matches!(assignment.gesture.as_str(), "doubleClick" | "longPress") {
        "gesture"
    } else {
        "key"
    };
    if PRESET_GESTURES.contains(&assignment.gesture.as_str())
        && let Some(contexts) = &action.contexts
        && !contexts.iter().any(|declared| declared == context)
    {
        c.error(
            format!("{path}.gesture"),
            IssueCode::Conflict,
            format!(
                "the action `{}` does not declare the `{context}` context `{}` binds it in \
                 (its contexts)",
                action.id, assignment.gesture
            ),
        );
    }
}
