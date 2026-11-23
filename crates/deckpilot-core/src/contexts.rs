//! Where an action makes sense: its execution contexts. See
//! `docs/action-contexts.md`.
//!
//! An [`ActionType`] may declare [`ActionContexts`]: on a key's primary
//! press, on a key's secondary gesture, as a key's live display, as a step
//! of a Multi Action, as a node of a graph workflow. A type without a
//! declaration (every plugin's, a stored definition) gets
//! [`ActionContexts::legacy`]: all five, exactly as before contexts
//! existed. Every built-in type declares its contexts.
//!
//! The context of a use comes from data that already exists, never from a
//! name, a category or an id ([`ActionContext::of_trigger`] for a binding,
//! [`ActionContext::Sequence`] for a step, [`ActionContext::Workflow`] for
//! a graph node). Runs from the app (its Run button, `runAction`) and a
//! plugin's Test run have no context and are never checked.
//!
//! Contexts are independent of [`crate::controls`]: controls decide which
//! control and input an action can be bound to, contexts where its use
//! makes sense. For now they are **advisory**: saving reports a use in an
//! unsupported context ([`ContextIssue`], with whether it was stored
//! before) and refuses nothing, and running one only logs a warning the
//! first time ([`crate::Engine`]); it still runs. Contexts are not
//! permissions and not a security boundary.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

use deckpilot_device::{ControlId, InputCapability};

use crate::graph::NodeId;
use crate::{ActionInstanceId, ActionType, Compatibility, IncompatibilityReason, PageId, Trigger};

/// One place an action can run in.
///
/// Ordered as the app lists them (key, gesture, display, sequence,
/// workflow): a declaration and a reason's `supported` list serialize in
/// this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActionContext {
    /// A key's, dial's or touch zone's primary input: a press, release or
    /// click, a rotation, a touch.
    Key,
    /// A key's secondary gesture: a double click or a long press.
    Gesture,
    /// A key whose face is the point: live state. A type with `display`
    /// but without `key` is display-only: pressing its key does nothing.
    Display,
    /// A step of a Multi Action (a workflow without a graph).
    Sequence,
    /// An action node of a graph workflow.
    Workflow,
}

impl ActionContext {
    /// Every context, in order.
    pub const ALL: [Self; 5] = [
        Self::Key,
        Self::Gesture,
        Self::Display,
        Self::Sequence,
        Self::Workflow,
    ];

    /// The context of a binding for `trigger`: `gesture` for a double
    /// click or a long press, `key` for everything else (press, release,
    /// click, rotations, touches).
    pub fn of_trigger(trigger: Trigger) -> Self {
        match trigger {
            Trigger::DoubleClick | Trigger::LongPress => Self::Gesture,
            Trigger::Press
            | Trigger::Release
            | Trigger::RotateClockwise
            | Trigger::RotateCounterClockwise
            | Trigger::Touch { .. }
            | Trigger::Click => Self::Key,
        }
    }

    /// The identifier, as serialized (`key`, `sequence`…).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Key => "key",
            Self::Gesture => "gesture",
            Self::Display => "display",
            Self::Sequence => "sequence",
            Self::Workflow => "workflow",
        }
    }

    /// Where it is, in words, for messages and logs.
    fn place(self) -> &'static str {
        match self {
            Self::Key => "on a key's press",
            Self::Gesture => "on a double click or long press",
            Self::Display => "as a key's live display",
            Self::Sequence => "as a Multi Action step",
            Self::Workflow => "as a workflow node",
        }
    }
}

impl fmt::Display for ActionContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The contexts an action type declares. Serialized as a list, e.g.
/// `["sequence", "workflow"]`; each context appears once.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ActionContexts(BTreeSet<ActionContext>);

impl ActionContexts {
    /// These contexts.
    pub fn of(contexts: impl IntoIterator<Item = ActionContext>) -> Self {
        Self(contexts.into_iter().collect())
    }

    /// What a type without a declaration gets: all five contexts, as
    /// before contexts existed.
    pub fn legacy() -> Self {
        Self::of(ActionContext::ALL)
    }

    pub fn contains(&self, context: ActionContext) -> bool {
        self.0.contains(&context)
    }

    /// The contexts, in order.
    pub fn iter(&self) -> impl Iterator<Item = ActionContext> + '_ {
        self.0.iter().copied()
    }

    /// The contexts, in order.
    pub fn to_vec(&self) -> Vec<ActionContext> {
        self.iter().collect()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Display-only: `display` without `key`. Pressing its key does
    /// nothing.
    pub fn is_display_only(&self) -> bool {
        self.contains(ActionContext::Display) && !self.contains(ActionContext::Key)
    }

    /// Problems of the declaration on its own: none is declared. (A list
    /// cannot name a context twice: it is a set.) The rules that need the
    /// type's controls are [`ActionType::context_issues`].
    pub fn issues(&self) -> Vec<String> {
        if self.is_empty() {
            vec!["no context is declared".to_owned()]
        } else {
            Vec::new()
        }
    }
}

impl ActionType {
    /// The declared contexts, or [`ActionContexts::legacy`] (all five)
    /// without a declaration.
    pub fn effective_contexts(&self) -> Cow<'_, ActionContexts> {
        self.contexts
            .as_ref()
            .map_or_else(|| Cow::Owned(ActionContexts::legacy()), Cow::Borrowed)
    }

    /// Whether a use of this type in `context` makes sense; see
    /// [`context_compatibility`].
    pub fn context_compatibility(&self, context: ActionContext) -> Compatibility {
        context_compatibility(self, context)
    }

    /// Problems of the type's declared contexts with its controls: none
    /// declared; `gesture` without a double click or long press on any
    /// control kind; `key` without a press, release, click, rotation or
    /// touch input. Nothing without a declaration. (That `display` needs a
    /// live face is checked for the built-ins by their contract test.)
    pub fn context_issues(&self) -> Vec<String> {
        let Some(contexts) = &self.contexts else {
            return Vec::new();
        };
        let mut issues = contexts.issues();
        let controls = self.effective_controls();
        let inputs = || controls.bindings.values().flatten().copied();
        let secondary = |input: InputCapability| {
            matches!(
                input,
                InputCapability::DoubleClick | InputCapability::LongPress
            )
        };
        if contexts.contains(ActionContext::Gesture) && !inputs().any(secondary) {
            issues.push("gesture: the controls declare no double click or long press".to_owned());
        }
        if contexts.contains(ActionContext::Key) && inputs().all(secondary) {
            issues.push(
                "key: the controls declare no press, release, click, rotation or touch".to_owned(),
            );
        }
        issues
    }
}

/// Whether a use of `action` in `context` makes sense: the type declares
/// the context (without a declaration it declares all five). A key's
/// primary input also takes a display-only type (`display` without
/// `key`): its press does nothing. The one decision every check uses: the
/// save reports, the execution guard and the app.
pub fn context_compatibility(action: &ActionType, context: ActionContext) -> Compatibility {
    let contexts = action.effective_contexts();
    let accepted = contexts.contains(context)
        || (context == ActionContext::Key && contexts.contains(ActionContext::Display));
    if accepted {
        Compatibility::Compatible
    } else {
        Compatibility::Incompatible(IncompatibilityReason::ContextUnsupported {
            context,
            supported: contexts.to_vec(),
        })
    }
}

/// The places of [`IncompatibilityReason::ContextUnsupported`]: "as a
/// Multi Action step and as a workflow node".
pub(crate) fn places(contexts: &[ActionContext]) -> String {
    match contexts {
        [] => "nowhere".to_owned(),
        [only] => only.place().to_owned(),
        [first @ .., last] => format!(
            "{} and {}",
            first
                .iter()
                .map(|c| c.place())
                .collect::<Vec<_>>()
                .join(", "),
            last.place()
        ),
    }
}

pub(crate) fn place(context: ActionContext) -> &'static str {
    context.place()
}

/// Where a use of an action is, in a [`ContextIssue`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ContextSite {
    /// A binding of a layout's page.
    Binding {
        page: PageId,
        page_name: String,
        control: ControlId,
        trigger: Trigger,
    },
    /// A step of a Multi Action; `index` counts from 0.
    Step { index: usize },
    /// An action node of a graph workflow.
    Node { node: NodeId },
}

impl fmt::Display for ContextSite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Binding {
                page_name,
                control,
                trigger,
                ..
            } => write!(f, "page `{page_name}`: {control} ({trigger:?})"),
            Self::Step { index } => write!(f, "step {}", index + 1),
            Self::Node { node } => write!(f, "{node}"),
        }
    }
}

/// A use of an action in a context its type does not declare, as saving a
/// layout or a workflow reports it. Reported only: nothing is refused,
/// removed or converted, and the use still runs (with a warning).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextIssue {
    pub site: ContextSite,
    pub action: ActionInstanceId,
    /// Always [`IncompatibilityReason::ContextUnsupported`].
    pub reason: IncompatibilityReason,
    /// The same use is in the version saved before (a binding with the
    /// same page, control, trigger and action; a step with the same action
    /// at the same index or elsewhere, counted as a multiset; a node with
    /// the same id and action). A future policy that refuses new
    /// incompatible uses keeps these.
    pub stored: bool,
}

impl fmt::Display for ContextIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.site, self.reason)
    }
}

/// A run of an action in a context its type does not declare, which the
/// engine let run ([`ContextGuard`]). Logged, never refused, for now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextWarning {
    pub action: ActionInstanceId,
    /// The instance's name in its profile.
    pub name: String,
    pub action_type: crate::ActionTypeId,
    /// The workflow of a step or node; `None` for a key or gesture.
    pub workflow: Option<crate::WorkflowId>,
    /// The device whose control started it, when one did.
    pub device: Option<deckpilot_device::DeviceId>,
    /// Always [`IncompatibilityReason::ContextUnsupported`].
    pub reason: IncompatibilityReason,
}

impl fmt::Display for ContextWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (`{}`): {}", self.name, self.action_type, self.reason)
    }
}

/// Where the engine reports a [`ContextWarning`]: the agent's log.
pub trait ContextWarnings: Send + Sync {
    fn warn(&self, warning: &ContextWarning);
}

/// Reports nothing (the default of an [`crate::Engine`]).
#[derive(Debug, Clone, Copy, Default)]
pub struct NoContextWarnings;

impl ContextWarnings for NoContextWarnings {
    fn warn(&self, _warning: &ContextWarning) {}
}

/// The engine's execution guard for contexts (decision D1 of the action
/// context audit: warn, do not refuse). A run in a context its type does
/// not declare still runs; the first such run of each (context, workflow,
/// action instance) since the engine was made is reported. That is
/// coarser than one binding, step or node: two keys or two steps that use
/// the same instance in the same context share a warning. The agent makes
/// a new engine on every configuration reload, so the warning comes again
/// after each one.
pub struct ContextGuard {
    sink: std::sync::Arc<dyn ContextWarnings>,
    seen: std::sync::Mutex<BTreeSet<(ActionContext, Option<crate::WorkflowId>, ActionInstanceId)>>,
}

impl fmt::Debug for ContextGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ContextGuard").finish_non_exhaustive()
    }
}

impl Default for ContextGuard {
    fn default() -> Self {
        Self::new(std::sync::Arc::new(NoContextWarnings))
    }
}

impl ContextGuard {
    pub fn new(sink: std::sync::Arc<dyn ContextWarnings>) -> Self {
        Self {
            sink,
            seen: std::sync::Mutex::new(BTreeSet::new()),
        }
    }

    /// Checks a run of `instance` (of type `definition`) started by
    /// `invocation`. Never refuses: returns whether the run is outside
    /// the type's contexts, and reports it the first time.
    pub fn check(
        &self,
        definition: &ActionType,
        instance: &crate::ActionInstance,
        invocation: &crate::resolution::Invocation,
    ) -> bool {
        // Runs from the app and Test runs have no context.
        let Some(context) = invocation.context else {
            return false;
        };
        let Compatibility::Incompatible(reason) = context_compatibility(definition, context) else {
            return false;
        };
        let first = self
            .seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert((context, invocation.workflow, instance.id));
        if first {
            self.sink.warn(&ContextWarning {
                action: instance.id,
                name: instance.name.clone(),
                action_type: instance.action_type.clone(),
                workflow: invocation.workflow,
                device: invocation.device.clone(),
                reason,
            });
        }
        true
    }
}

#[cfg(test)]
mod tests;
