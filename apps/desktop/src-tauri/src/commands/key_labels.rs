//! Key label templates judged by the agent's variables (`docs/key-text.md`,
//! "Labels") when a layout is saved or an export imported.
//!
//! The core checks a label's template with its own variables
//! ([`deckpilot_core::appearance::check_labels`]): secrets, run values and
//! the core's non-display variables are refused. Plugins and modules
//! declare their variables in the agent; a reference to one the agent
//! knows is sensitive or an identifier, or to a shared variable whose value
//! is sensitive, is refused here too, from the agent's own answer
//! (`resolveDisplayTemplates`, the function the device uses). A reference
//! the agent does not know yet (a plugin that is not installed) is allowed:
//! it shows `-` until its plugin declares it display-safe. Without an agent
//! that answers, the core's check stands.

use deckpilot_core::variables::{ResolutionContext, UnresolvedReason};
use deckpilot_core::{DeviceLayout, ProfileId};
use deckpilot_ipc::display_templates::{MAX_DISPLAY_TEMPLATES, ResolvedDisplayTemplate};

/// The label templates of `layouts` with references (`{{ … }}`), each once.
pub(crate) fn templates<'a>(layouts: impl IntoIterator<Item = &'a DeviceLayout>) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for layout in layouts {
        let bindings = layout
            .pages
            .iter()
            .flat_map(|page| &page.bindings)
            .chain(layout.recovered_bindings());
        for look in bindings.filter_map(|binding| binding.appearance.as_ref()) {
            for label in &look.labels {
                if label.text.contains("{{") && !found.contains(&label.text) {
                    found.push(label.text.clone());
                }
            }
        }
    }
    found
}

/// Why the agent's `answers` refuse a template: one of its references
/// never shows on a key (not display-safe, sensitive, an identifier, or a
/// run's value).
pub(crate) fn refusal(answers: &[ResolvedDisplayTemplate]) -> Option<String> {
    answers.iter().find_map(|answer| {
        answer
            .display
            .unresolved
            .iter()
            .find(|unresolved| {
                matches!(
                    unresolved.reason,
                    UnresolvedReason::NotDisplayable | UnresolvedReason::RuntimeOnly
                )
            })
            .map(|unresolved| {
                format!(
                    "the label “{}” uses {}, which a key never shows (a secret, a sensitive value, an identifier or a run's value)",
                    answer.template, unresolved.reference
                )
            })
    })
}

/// Asks the agent about `templates` in `profile`'s context; `Err` with the
/// reason when one is refused. Without an agent that answers (not running,
/// older than protocol 17, the profile unknown to it), nothing more is
/// refused.
pub(crate) async fn check(
    paths: &deckpilot_ipc::AgentPaths,
    profile: ProfileId,
    templates: Vec<String>,
) -> Result<(), String> {
    for chunk in templates.chunks(MAX_DISPLAY_TEMPLATES) {
        let Ok(answers) = super::variables::resolve_templates(
            paths,
            ResolutionContext::preview(profile),
            chunk.to_vec(),
        )
        .await
        else {
            return Ok(());
        };
        if let Some(reason) = refusal(&answers) {
            return Err(reason);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckpilot_core::variables::{DisplayText, UnresolvedReference};

    fn answer(reference: &str, reason: UnresolvedReason) -> ResolvedDisplayTemplate {
        ResolvedDisplayTemplate {
            template: format!("{{{{ {reference} }}}}"),
            display: DisplayText {
                text: "-".into(),
                unresolved: vec![UnresolvedReference {
                    reference: reference.into(),
                    reason,
                }],
                ..DisplayText::default()
            },
        }
    }

    #[test]
    fn what_the_agent_never_shows_is_refused_what_it_does_not_know_is_not() {
        // A plugin's variable it declares sensitive.
        let refused = refusal(&[answer(
            "plugins.acme.api.token",
            UnresolvedReason::NotDisplayable,
        )])
        .unwrap();
        assert!(refused.contains("plugins.acme.api.token"), "{refused}");
        assert!(refused.contains("never shows"));
        assert!(refusal(&[answer("steps.a.output.b", UnresolvedReason::RuntimeOnly)]).is_some());
        // A plugin that is not installed: allowed, shown as `-`.
        assert_eq!(
            refusal(&[answer(
                "plugins.other.thing.name",
                UnresolvedReason::Unavailable {
                    owner: "other.thing".into()
                }
            )]),
            None
        );
    }
}
