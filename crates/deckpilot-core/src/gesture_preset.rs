//! Gesture presets: library items that put several existing actions on the
//! gestures of one control at once (`docs/gestures-and-work-session.md`,
//! section 3.4).
//!
//! A preset is data only. It names, per gesture (single press, double
//! press, long press, release, either rotation), an action type of the
//! module that declares it and the parameters to give it. Applying one
//! creates ordinary action instances and ordinary [`ControlBinding`]s: there
//! is no preset at run time, no execution system of its own, and nothing
//! of a preset is stored. After it was applied every gesture is edited,
//! replaced or removed on its own, and every action still runs through the
//! registry's parameter validation, capability checks and the plugin's
//! permissions, consent and allow-lists. A preset never grants anything.
//!
//! Gesture timing stays the [`GestureEngine`](crate::gesture::GestureEngine)'s:
//! a preset only decides which triggers the bindings use. A preset that
//! binds a double or long press puts the control in gesture mode (single
//! press = [`Trigger::Click`]); one that binds only the single press and the
//! release keeps it raw ([`Trigger::Press`] fires at once, as before
//! gestures, and [`Trigger::Release`]). The two never mix on a control
//! ([`crate::LayoutIssue::MixedTriggers`]), so a preset may not declare a
//! release next to a double or long press.
//!
//! Presets are declared by feature modules ([`FeatureModule::gesture_presets`])
//! and by plugin manifests (`contributes.gesturePresets`, plugin API 1.10),
//! which the agent turns into the plugin module's presets; the
//! [`ModuleRegistry`](crate::ModuleRegistry) checks both the same way when the
//! module is registered ([`GesturePreset::check`]).
//!
//! [`FeatureModule::gesture_presets`]: crate::FeatureModule::gesture_presets

use std::collections::BTreeSet;
use std::fmt;

use deckpilot_device::{ControlCapabilities, ControlId, InputCapability};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    ActionContext, ActionInstanceId, ActionParameters, ActionType, ActionTypeId, BindingTarget,
    ControlBinding, IdentifierError, ModuleId, Trigger,
};

/// Most gestures one preset declares.
pub const MAX_PRESET_GESTURES: usize = 6;
/// Longest preset name, in characters.
pub const MAX_PRESET_NAME: usize = 48;
/// Longest preset description, in characters.
pub const MAX_PRESET_DESCRIPTION: usize = 200;
/// Longest gesture label (the name of the action it creates), in
/// characters.
pub const MAX_PRESET_LABEL: usize = 64;

/// Identifier of a gesture preset, e.g. `deck.brightness-control` or
/// `acme.lights.dimmer`: like an action type id, everything before the last
/// `.` is the module (or plugin) that declares it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct GesturePresetId(String);

impl GesturePresetId {
    pub fn new(id: impl Into<String>) -> Result<Self, IdentifierError> {
        let id = id.into();
        crate::id::validate_dotted("gesture preset id", &id)?;
        Ok(Self(id))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The module that declares the preset: everything before the last `.`.
    pub fn owner(&self) -> Option<ModuleId> {
        let (namespace, _) = self.0.rsplit_once('.')?;
        ModuleId::new(namespace).ok()
    }
}

impl TryFrom<String> for GesturePresetId {
    type Error = IdentifierError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<GesturePresetId> for String {
    fn from(value: GesturePresetId) -> Self {
        value.0
    }
}

impl fmt::Display for GesturePresetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A gesture a preset can assign. The names are those of the manifest
/// (`click`, `doubleClick`, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PresetGesture {
    /// A single press: [`Trigger::Press`] on a raw control, [`Trigger::Click`]
    /// on a control with a double or long press.
    Click,
    DoubleClick,
    LongPress,
    /// The raw release ([`Trigger::Release`]); never next to a double or
    /// long press.
    Release,
    RotateClockwise,
    RotateCounterClockwise,
}

impl PresetGesture {
    /// Every gesture, in the order the editor shows them.
    pub const ALL: [Self; 6] = [
        Self::Click,
        Self::DoubleClick,
        Self::LongPress,
        Self::Release,
        Self::RotateClockwise,
        Self::RotateCounterClockwise,
    ];

    /// Whether binding it puts the control in gesture mode.
    pub fn needs_gesture_mode(self) -> bool {
        matches!(self, Self::DoubleClick | Self::LongPress)
    }

    /// The trigger it binds on a control in gesture mode (`gestures`) or
    /// raw mode.
    pub fn trigger(self, gestures: bool) -> Trigger {
        match self {
            Self::Click if gestures => Trigger::Click,
            Self::Click => Trigger::Press,
            Self::DoubleClick => Trigger::DoubleClick,
            Self::LongPress => Trigger::LongPress,
            Self::Release => Trigger::Release,
            Self::RotateClockwise => Trigger::RotateClockwise,
            Self::RotateCounterClockwise => Trigger::RotateCounterClockwise,
        }
    }

    /// The context ([`ActionContext::of_trigger`]) of the binding it makes,
    /// the same in either mode: `gesture` for a double or long press, `key`
    /// for everything else.
    pub fn context(self) -> ActionContext {
        ActionContext::of_trigger(self.trigger(self.needs_gesture_mode()))
    }

    /// The inputs a control must produce for it, in either mode.
    fn inputs(self) -> &'static [InputCapability] {
        match self {
            Self::Click => &[InputCapability::Press, InputCapability::Click],
            Self::DoubleClick => &[InputCapability::DoubleClick],
            Self::LongPress => &[InputCapability::LongPress],
            Self::Release => &[InputCapability::Release],
            Self::RotateClockwise => &[InputCapability::RotateClockwise],
            Self::RotateCounterClockwise => &[InputCapability::RotateCounterClockwise],
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Click => "click",
            Self::DoubleClick => "doubleClick",
            Self::LongPress => "longPress",
            Self::Release => "release",
            Self::RotateClockwise => "rotateClockwise",
            Self::RotateCounterClockwise => "rotateCounterClockwise",
        }
    }
}

impl fmt::Display for PresetGesture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What one gesture of a preset does: an action type of the declaring
/// module with predefined parameters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GestureAssignment {
    pub gesture: PresetGesture,
    pub action: ActionTypeId,
    /// Over the type's defaults; checked like an action instance's.
    #[serde(default)]
    pub parameters: ActionParameters,
    /// What the gesture does, in words ("Decrease brightness by 10%"): the
    /// name of the action instance the preset creates.
    pub label: String,
}

/// A library item that maps several gestures of a control to existing
/// action types with predefined parameters. See the module documentation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GesturePreset {
    pub id: GesturePresetId,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// The Action Library category it is listed in (as `ActionType::category`).
    pub category: String,
    /// An icon of the app's icon set, like `NodeSpec::icon`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Whether the gestures a control cannot produce may be left out
    /// (applying the others); otherwise such a control refuses the preset.
    #[serde(default)]
    pub allow_partial: bool,
    pub gestures: Vec<GestureAssignment>,
}

/// Why a declared preset is refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PresetIssue {
    #[error("gesture preset `{preset}` must belong to module `{module}` (`{module}.<name>`)")]
    Namespace { preset: String, module: String },
    #[error("gesture preset `{0}` is declared twice")]
    Duplicate(String),
    #[error("gesture preset `{preset}`: {field} must be 1 to {max} characters")]
    Text {
        preset: String,
        field: String,
        max: usize,
    },
    #[error("gesture preset `{preset}` assigns no gesture")]
    Empty { preset: String },
    #[error("gesture preset `{preset}` assigns more than {MAX_PRESET_GESTURES} gestures")]
    TooMany { preset: String },
    #[error("gesture preset `{preset}` assigns {gesture} twice")]
    DuplicateGesture {
        preset: String,
        gesture: PresetGesture,
    },
    #[error(
        "gesture preset `{preset}` assigns the release next to a double or long press; a control \
         cannot have both"
    )]
    ReleaseWithGestures { preset: String },
    #[error(
        "gesture preset `{preset}` ({gesture}): unknown action `{action}` of module `{module}`"
    )]
    UnknownAction {
        preset: String,
        gesture: PresetGesture,
        action: String,
        module: String,
    },
    #[error("gesture preset `{preset}` ({gesture}): invalid parameters for `{action}`: {reason}")]
    InvalidParameters {
        preset: String,
        gesture: PresetGesture,
        action: String,
        reason: String,
    },
    #[error("gesture preset `{preset}` ({gesture}): action `{action}` cannot run from {gesture}")]
    UnsupportedGesture {
        preset: String,
        gesture: PresetGesture,
        action: String,
    },
    /// The action declares contexts (`docs/action-contexts.md`) without the
    /// one its gesture binds it in: `key` for a single press, a release or
    /// a rotation, `gesture` for a double or long press. An action without
    /// a declaration has all five and never gets this.
    #[error(
        "gesture preset `{preset}` ({gesture}): action `{action}` does not declare the \
         `{context}` context its gesture binds it in"
    )]
    UnsupportedContext {
        preset: String,
        gesture: PresetGesture,
        action: String,
        context: ActionContext,
    },
}

/// Why a preset cannot go on a control.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PresetRefusal {
    #[error("{control} cannot produce {}", list(.gestures))]
    Unsupported {
        control: ControlId,
        gestures: Vec<PresetGesture>,
    },
    #[error("nothing of the preset can go on {control}")]
    Nothing { control: ControlId },
}

fn list(gestures: &[PresetGesture]) -> String {
    gestures
        .iter()
        .map(|g| g.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// What applying a preset to one control does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetPlan {
    /// The gestures that are bound, each with the trigger it uses.
    pub applied: Vec<(PresetGesture, Trigger)>,
    /// The gestures the control cannot produce, left out (only with
    /// [`GesturePreset::allow_partial`]).
    pub skipped: Vec<PresetGesture>,
}

impl GesturePreset {
    /// The assignment of `gesture`, if the preset has one.
    pub fn assignment(&self, gesture: PresetGesture) -> Option<&GestureAssignment> {
        self.gestures.iter().find(|a| a.gesture == gesture)
    }

    /// Checks the declaration of `module`, whose action types are `types`:
    /// the id is in the module's namespace, the texts fit, each gesture is
    /// assigned once, never a release next to a double or long press, and
    /// every assignment names one of the module's own action types, with
    /// parameters its definition accepts and a gesture it can run from on
    /// some control. Returns every problem.
    pub fn check(&self, module: &ModuleId, types: &[ActionType]) -> Result<(), Vec<PresetIssue>> {
        let preset = self.id.to_string();
        let mut issues = Vec::new();
        if self.id.owner().as_ref() != Some(module) {
            issues.push(PresetIssue::Namespace {
                preset: preset.clone(),
                module: module.to_string(),
            });
        }
        let mut text = |field: &str, value: &str, max: usize, required: bool| {
            let count = value.trim().chars().count();
            if (required && count == 0) || value.chars().count() > max {
                issues.push(PresetIssue::Text {
                    preset: preset.clone(),
                    field: field.into(),
                    max,
                });
            }
        };
        text("the name", &self.name, MAX_PRESET_NAME, true);
        text(
            "the description",
            &self.description,
            MAX_PRESET_DESCRIPTION,
            false,
        );
        text("the category", &self.category, MAX_PRESET_NAME, true);
        for assignment in &self.gestures {
            text(
                &format!("the label of {}", assignment.gesture),
                &assignment.label,
                MAX_PRESET_LABEL,
                true,
            );
        }
        if self.gestures.is_empty() {
            issues.push(PresetIssue::Empty {
                preset: preset.clone(),
            });
        }
        if self.gestures.len() > MAX_PRESET_GESTURES {
            issues.push(PresetIssue::TooMany {
                preset: preset.clone(),
            });
        }
        let mut seen = BTreeSet::new();
        for assignment in &self.gestures {
            if !seen.insert(assignment.gesture) {
                issues.push(PresetIssue::DuplicateGesture {
                    preset: preset.clone(),
                    gesture: assignment.gesture,
                });
            }
        }
        if seen.contains(&PresetGesture::Release) && seen.iter().any(|g| g.needs_gesture_mode()) {
            issues.push(PresetIssue::ReleaseWithGestures {
                preset: preset.clone(),
            });
        }
        for assignment in &self.gestures {
            check_assignment(&preset, module, assignment, types, &mut issues);
        }
        if issues.is_empty() {
            Ok(())
        } else {
            Err(issues)
        }
    }

    /// Which gestures go on a control with `capabilities`, and with which
    /// trigger, for an empty control. A gesture is supported when the
    /// control produces its trigger and the assigned action can run from
    /// it there (`types` are the action types, for their `controls`; an
    /// assignment whose type is missing counts as unsupported). Without
    /// [`GesturePreset::allow_partial`] any unsupported gesture refuses the
    /// whole preset.
    pub fn plan(
        &self,
        control: ControlId,
        capabilities: &ControlCapabilities,
        types: &[ActionType],
    ) -> Result<PresetPlan, PresetRefusal> {
        let supported = |assignment: &GestureAssignment, gestures: bool| {
            let trigger = assignment.gesture.trigger(gestures);
            capabilities.supports(trigger.input_capability())
                && types
                    .iter()
                    .find(|t| t.id == assignment.action)
                    .is_some_and(|t| t.compatibility(capabilities, trigger).is_compatible())
        };
        // Gesture mode exactly when a double or long press goes on the
        // control (as the editor converts a raw key).
        let gestures = self
            .gestures
            .iter()
            .any(|a| a.gesture.needs_gesture_mode() && supported(a, true));
        let mut applied = Vec::new();
        let mut skipped = Vec::new();
        for assignment in &self.gestures {
            if supported(assignment, gestures) {
                applied.push((assignment.gesture, assignment.gesture.trigger(gestures)));
            } else {
                skipped.push(assignment.gesture);
            }
        }
        if !skipped.is_empty() && !self.allow_partial {
            return Err(PresetRefusal::Unsupported {
                control,
                gestures: skipped,
            });
        }
        if applied.is_empty() {
            return Err(PresetRefusal::Nothing { control });
        }
        Ok(PresetPlan { applied, skipped })
    }

    /// The bindings applying the preset to the empty `control` makes, with
    /// `instance` giving the action instance of each applied assignment
    /// (created or reused by the caller), and the gestures left out.
    pub fn bindings(
        &self,
        control: ControlId,
        capabilities: &ControlCapabilities,
        types: &[ActionType],
        mut instance: impl FnMut(&GestureAssignment) -> ActionInstanceId,
    ) -> Result<(Vec<ControlBinding>, Vec<PresetGesture>), PresetRefusal> {
        let plan = self.plan(control, capabilities, types)?;
        let bindings = plan
            .applied
            .iter()
            .filter_map(|&(gesture, trigger)| {
                let assignment = self.assignment(gesture)?;
                Some(ControlBinding::new(
                    control,
                    trigger,
                    BindingTarget::action(instance(assignment)),
                ))
            })
            .collect();
        Ok((bindings, plan.skipped))
    }
}

/// The checks of one assignment of `preset` ([`GesturePreset::check`]).
fn check_assignment(
    preset: &str,
    module: &ModuleId,
    assignment: &GestureAssignment,
    types: &[ActionType],
    issues: &mut Vec<PresetIssue>,
) {
    let action = assignment.action.to_string();
    let own = ModuleId::owner_of(&assignment.action).as_ref() == Some(module);
    let Some(definition) = types
        .iter()
        .find(|t| t.id == assignment.action)
        .filter(|_| own)
    else {
        issues.push(PresetIssue::UnknownAction {
            preset: preset.to_owned(),
            gesture: assignment.gesture,
            action,
            module: module.to_string(),
        });
        return;
    };
    if let Err(errors) = definition.normalize(&assignment.parameters) {
        issues.push(PresetIssue::InvalidParameters {
            preset: preset.to_owned(),
            gesture: assignment.gesture,
            action: action.clone(),
            reason: errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; "),
        });
    }
    let controls = definition.effective_controls();
    let runs = controls.kinds().into_iter().any(|kind| {
        assignment
            .gesture
            .inputs()
            .iter()
            .any(|&input| controls.supports(kind, input) && input.applies_to(kind))
    });
    if !runs {
        issues.push(PresetIssue::UnsupportedGesture {
            preset: preset.to_owned(),
            gesture: assignment.gesture,
            action: action.clone(),
        });
    }
    // A declaration check, like the controls above: what the preset puts on
    // a control must be a use its action declares. Stored bindings are
    // never checked here (contexts stay advisory for them).
    let context = assignment.gesture.context();
    if !definition.effective_contexts().contains(context) {
        issues.push(PresetIssue::UnsupportedContext {
            preset: preset.to_owned(),
            gesture: assignment.gesture,
            action,
            context,
        });
    }
}

#[cfg(test)]
mod tests;
