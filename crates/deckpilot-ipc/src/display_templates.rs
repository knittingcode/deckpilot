//! Display templates of protocol 17 (capability `displayTemplates`):
//! variables in key titles (`docs/variables.md`, "Variables in key titles").
//!
//! The agent is the only place that resolves a display template, for the
//! device and for the editor alike, through
//! [`deckpilot_core::variables::resolve_display_template`]:
//!
//! - `listDisplayVariables` lists the references a key title may use in a
//!   context: the catalog of `listVariables` limited to display-safe
//!   sources (no sensitive variables, no secret references, nothing of a
//!   run, a device or the work session). Metadata only, never values.
//! - `resolveDisplayTemplates` resolves templates (a key's title, as
//!   typed) in the display context of a profile and its active project, or
//!   the project the client names, in the order asked: the text the device
//!   shows, the references shown as `-` with why (never a value), the text
//!   between `{{ }}` that is not a reference, and what the text depends on.
//!
//! Both answer for any profile the client names (an inactive one is read
//! from the database); the context is always a preview: a client cannot
//! claim a run, a workflow or a device.
//!
//! Clients re-ask when `projectChanged`, `profileChanged` or a
//! `sharedVariablesChanged` naming one of the dependencies arrives
//! ([`deckpilot_core::variables::DisplayDependency::changed_by`]); the
//! device is updated by the agent on the same changes, without polling.

use serde::{Deserialize, Serialize};

use deckpilot_core::variables::DisplayText;

/// Most templates one `resolveDisplayTemplates` may ask about.
pub const MAX_DISPLAY_TEMPLATES: usize = 200;

/// Longest template, in bytes, `resolveDisplayTemplates` accepts.
pub const MAX_DISPLAY_TEMPLATE_BYTES: usize = 2048;

/// Checks a `resolveDisplayTemplates` request's templates.
pub fn check_templates(templates: &[String]) -> Result<(), String> {
    if templates.len() > MAX_DISPLAY_TEMPLATES {
        return Err(format!(
            "at most {MAX_DISPLAY_TEMPLATES} templates can be resolved at once"
        ));
    }
    if templates
        .iter()
        .any(|template| template.len() > MAX_DISPLAY_TEMPLATE_BYTES)
    {
        return Err(format!(
            "a template has at most {MAX_DISPLAY_TEMPLATE_BYTES} bytes"
        ));
    }
    Ok(())
}

/// One answer of `resolveDisplayTemplates`: the template as asked and what
/// it resolves to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedDisplayTemplate {
    pub template: String,
    #[serde(flatten)]
    pub display: DisplayText,
}
