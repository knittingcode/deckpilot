//! `contributes.actions[].contexts` (plugin API 1.11): where an action
//! makes sense, as `docs/action-contexts.md` describes for the built-ins.
//!
//! The field is optional; without it the action has all five contexts,
//! exactly as before. A declaration names at least one of
//! [`ACTION_CONTEXTS`], each at most once, and follows the rules the
//! built-ins' contract test holds them to:
//!
//! - `gesture` needs a double click or long press among the action's
//!   controls (without `controls`: keys with every key input, so it has
//!   both);
//! - `key` needs a press, release, click, rotation or touch among them;
//! - `display` needs a live face, which for a plugin action is its key
//!   state (`state`, plugin API 1.2).
//!
//! These rules check the declaration only. Contexts stay advisory where
//! they are used: saving a layout or workflow reports a use outside them,
//! and running it logs a warning and still runs.

use std::collections::BTreeSet;

use deckpilot_device::InputCapability;

use super::{Checker, IssueCode, Manifest};
use crate::model::{ACTION_CONTEXTS, ACTION_CONTEXTS_API, Action};

/// Whether `input` is a double click or a long press (the `gesture`
/// context's inputs); every other input is a `key` input.
fn secondary(input: InputCapability) -> bool {
    matches!(
        input,
        InputCapability::DoubleClick | InputCapability::LongPress
    )
}

/// The inputs the action can be bound to on any control kind: its
/// `controls`, or without them the legacy default (keys with press,
/// release, click, double click and long press).
fn inputs(action: &Action) -> Vec<InputCapability> {
    match &action.controls {
        Some(controls) => controls.bindings.values().flatten().copied().collect(),
        None => vec![
            InputCapability::Press,
            InputCapability::Release,
            InputCapability::Click,
            InputCapability::DoubleClick,
            InputCapability::LongPress,
        ],
    }
}

pub(super) fn check(c: &mut Checker, m: &Manifest) {
    let required = m.engines.plugin_api;
    for (index, action) in m.contributes.actions().iter().enumerate() {
        let Some(contexts) = &action.contexts else {
            continue;
        };
        let path = format!("contributes.actions[{index}].contexts");
        if required.minimum < ACTION_CONTEXTS_API {
            c.error(
                &path,
                IssueCode::Conflict,
                format!(
                    "needs plugin API {ACTION_CONTEXTS_API}: set engines.pluginApi to \
                     `^{ACTION_CONTEXTS_API}` (it is `{required}`)"
                ),
            );
            continue;
        }
        if contexts.is_empty() {
            c.error(
                &path,
                IssueCode::Count,
                "declare at least one context, or leave the field out for all five",
            );
            continue;
        }
        let mut seen = BTreeSet::new();
        for (at, context) in contexts.iter().enumerate() {
            if !ACTION_CONTEXTS.contains(&context.as_str()) {
                c.error(
                    format!("{path}[{at}]"),
                    IssueCode::InvalidValue,
                    format!(
                        "`{context}` is not a context: use one of {}",
                        ACTION_CONTEXTS.join(", ")
                    ),
                );
            } else if !seen.insert(context.as_str()) {
                c.error(
                    format!("{path}[{at}]"),
                    IssueCode::Duplicate,
                    format!("`{context}` is declared twice"),
                );
            }
        }
        let inputs = inputs(action);
        if seen.contains("gesture") && !inputs.iter().copied().any(secondary) {
            c.error(
                &path,
                IssueCode::Conflict,
                "`gesture` needs a double click or long press among the action's controls",
            );
        }
        if seen.contains("key") && inputs.iter().copied().all(secondary) {
            c.error(
                &path,
                IssueCode::Conflict,
                "`key` needs a press, release, click, rotation or touch among the action's \
                 controls",
            );
        }
        if seen.contains("display") && action.state.is_none() {
            c.error(
                &path,
                IssueCode::Conflict,
                "`display` needs a live face: declare the action's `state`",
            );
        }
    }
}
