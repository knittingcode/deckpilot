//! Display templates: variables in the texts a key shows (`docs/variables.md`,
//! "Variables in key titles").
//!
//! A display template is plain text with references in the reference
//! grammar of the engine, `{{ <root>.<path> }}` ([`super::expression`]), and
//! the older key label placeholders `{profile}`, `{project.key}` and
//! `{project.name}`, which keep working and are resolved through the engine
//! as `system.profile.current.name`, `system.project.current.key` and
//! `system.project.current.name`. `{page.…}` placeholders are the layout's,
//! expanded afterwards by [`crate::render_page_label`] as before.
//!
//! [`resolve_display_template`] is the **one** function that turns a
//! template into the text a key shows. The agent calls it for the device and
//! for the editor's preview (`resolveDisplayTemplates`), so both show the
//! same text; the app never resolves anything itself. It is written for any
//! number of texts per key: the title is [`TextSlot::Title`], and further
//! labels get slots of their own, each with its own dependencies
//! ([`TextKey`]).
//!
//! Display is not a run. Only **display-safe** sources resolve
//! ([`display_safe`]):
//!
//! - core variables of the profile and the project: `system.profile.current.name`,
//!   `system.project.current.key`, `system.project.current.name` and
//!   `system.platform`;
//! - module and plugin variables (`projects.current.key`,
//!   `plugins.<publisher>.<name>.<id>`) declared non-sensitive, with a
//!   text value, whose value depends on the machine, the profile or the
//!   project only (never a device, a workflow, a run or the work session),
//!   while their module or plugin is enabled and registered;
//! - shared variables (`shared.<key>`) that are not sensitive;
//! - the display-safe scalar members of a structured variable
//!   (`projects.current.repository.name`, [`super::Member::display_safe`]),
//!   never the whole object or map, an id or a folder path (OD-4).
//!
//! Everything else is refused before anything is resolved where the
//! declaration says so, and otherwise as soon as the resolution reveals it:
//! sensitive values, secret references (credentials), node outputs,
//! `workflow.…`, the device and the work session. Only string, integer and
//! boolean values become text. A refused or missing value never reaches
//! the text: the reference is shown as [`UNRESOLVED_MARKER`] and reported
//! in [`DisplayText::unresolved`] (by its name and a reason, never a
//! value). The template itself is never changed.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::registry::{DeclaredScope, Requirement, VariableRegistry, module_variable_alias};
use super::resolution::{
    MissingReason, Resolution, ResolutionContext, ResolutionPhase, VariableResolver,
};
use super::{SystemVariable, VariableRef, VariableType, VariableValue};
use crate::{PageId, ProfileId, ProjectId, Trigger};
use deckpilot_device::ControlId;

/// What a reference without a displayable value shows: the same mark the
/// key label placeholders always showed without a project.
pub const UNRESOLVED_MARKER: &str = "-";

/// Longest value, in characters, one reference puts into a text; longer
/// values are cut with `…`. Layout (wrapping, fitting) happens afterwards.
pub const MAX_DISPLAY_VALUE: usize = 256;

/// The older key label placeholders and the variables they are.
pub const LEGACY_PLACEHOLDERS: [(&str, SystemVariable); 3] = [
    ("{profile}", SystemVariable::ProfileCurrentName),
    ("{project.key}", SystemVariable::ProjectCurrentKey),
    ("{project.name}", SystemVariable::ProjectCurrentName),
];

/// Which text of a key a template is for: its title, or one of its labels
/// ([`crate::KeyLabel`]) by id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TextSlot {
    /// The key's title (the binding's label, else its action's).
    Title,
    /// A label of the binding's look.
    Label { id: crate::LabelId },
}

/// One text of one binding: the unit whose dependencies the agent tracks.
/// A binding is its page, control and trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextKey {
    pub page: PageId,
    pub control: ControlId,
    pub trigger: Trigger,
    pub slot: TextSlot,
}

/// What a resolved text depends on: when it changes, the text is resolved
/// again.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DisplayDependency {
    /// The profile shown (its name).
    Profile,
    /// The active project of the profile.
    Project,
    /// A variable, by the reference as written (`shared.team.name`,
    /// `projects.current.key`).
    Variable { reference: String },
}

impl DisplayDependency {
    /// Whether a change announced as `key` (a `sharedVariablesChanged` key:
    /// a shared variable's key without `shared.`, or a core or module
    /// variable's id, canonical or legacy) changes this dependency.
    pub fn changed_by(&self, key: &str) -> bool {
        let Self::Variable { reference } = self else {
            return false;
        };
        let Ok(parsed) = VariableRef::parse(reference) else {
            return false;
        };
        spellings(&parsed).iter().any(|name| name == key)
    }
}

/// Every name a change of `reference` may be announced by.
fn spellings(reference: &VariableRef) -> Vec<String> {
    let mut names = vec![reference.to_string()];
    match reference {
        VariableRef::Shared(key) => names.push(key.to_string()),
        VariableRef::System(variable) => names.push(variable.canonical().to_string()),
        VariableRef::Module(_) | VariableRef::Step(_) => {}
    }
    for name in names.clone() {
        if let Some(alias) = super::VariableId::new(name)
            .ok()
            .as_ref()
            .and_then(module_variable_alias)
        {
            names.push(alias.canonical.to_owned());
        }
    }
    names
}

/// Why a reference of a template shows [`UNRESOLVED_MARKER`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "reason",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum UnresolvedReason {
    /// Not a display-safe source, or its value is sensitive or a secret:
    /// it never resolves on a key.
    NotDisplayable,
    /// It has no value now.
    Missing { missing: MissingReason },
    /// Its module or plugin is disabled, not supported here or not
    /// installed.
    Unavailable { owner: String },
    /// Its value is not text (a resource, an id, JSON).
    NotText { found: VariableType },
    /// It exists only while a workflow runs.
    RuntimeOnly,
}

/// A reference of a template that has no displayable value: its name as
/// written, and why. Never a value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnresolvedReference {
    /// The reference without braces (`shared.team.name`), or the legacy
    /// placeholder's name (`project.key`).
    pub reference: String,
    #[serde(flatten)]
    pub reason: UnresolvedReason,
}

/// Where one [`UNRESOLVED_MARKER`] is in [`DisplayText::text`], so the
/// editor can tell it from a value that happens to read `-`. Offsets are
/// in UTF-16 code units (as JavaScript strings count), `end` exclusive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkerSpan {
    pub start: usize,
    pub end: usize,
    /// The reference the marker stands for, as in
    /// [`UnresolvedReference::reference`].
    pub reference: String,
}

/// A template resolved for display.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayText {
    /// What the key shows (before `{page.…}` and layout).
    pub text: String,
    /// The references shown as [`UNRESOLVED_MARKER`], in order, each once.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unresolved: Vec<UnresolvedReference>,
    /// Every marker in `text`, in order (a reference used twice has two).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub markers: Vec<MarkerSpan>,
    /// Text between `{{` and `}}` that is not a reference, kept as typed
    /// (as before display templates), each once.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub invalid: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub dependencies: BTreeSet<DisplayDependency>,
}

impl DisplayText {
    /// A text without references.
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Self::default()
        }
    }

    /// Whether the text depends on anything.
    pub fn is_dynamic(&self) -> bool {
        !self.dependencies.is_empty()
    }
}

/// Everything a display template is resolved with: the profile's display
/// context and the engine's resolver for that profile.
#[derive(Clone, Copy)]
pub struct DisplayEnvironment<'a> {
    /// A preview context of the profile with its active project
    /// ([`display_context`]).
    pub context: &'a ResolutionContext,
    /// The engine's resolver for the context's profile
    /// ([`crate::resolution::ContextResolver`]).
    pub resolver: &'a dyn VariableResolver,
    /// The core's and the modules' variables, for their declarations.
    pub registry: &'a VariableRegistry,
}

impl std::fmt::Debug for DisplayEnvironment<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DisplayEnvironment")
            .field("context", self.context)
            .finish_non_exhaustive()
    }
}

/// The context texts are displayed in: `profile` with its active
/// `project`, as a preview (nothing runs), without a device (a preview in
/// the editor has none, and both must show the same).
pub fn display_context(profile: ProfileId, project: Option<ProjectId>) -> ResolutionContext {
    ResolutionContext::preview(profile)
        .with_project(project, super::resolution::ProjectSource::Application)
}

/// Whether `reference` is from a display-safe source, judged by what is
/// known before resolving it: its root and, for core and module variables,
/// their declaration. Shared variables pass here and are checked on their
/// value (sensitivity, type). `Err` carries why not.
pub fn display_safe(
    reference: &VariableRef,
    registry: &VariableRegistry,
) -> Result<(), UnresolvedReason> {
    match reference {
        VariableRef::Step(_) => Err(UnresolvedReason::RuntimeOnly),
        VariableRef::Shared(_) => Ok(()),
        VariableRef::System(variable) => match variable {
            SystemVariable::ProfileCurrentName
            | SystemVariable::ProjectCurrentKey
            | SystemVariable::ProjectCurrentName
            | SystemVariable::Platform => Ok(()),
            SystemVariable::WorkflowExecutionId => Err(UnresolvedReason::RuntimeOnly),
            _ => Err(UnresolvedReason::NotDisplayable),
        },
        // Ids are never text for people, whoever provides them.
        VariableRef::Module(id) if IDENTIFIERS.contains(&id.as_str()) => {
            Err(UnresolvedReason::NotDisplayable)
        }
        VariableRef::Module(id) => {
            // Not registered: its owner is not here, which the resolution
            // reports; no value can come of it.
            let Some((registered, member)) = registry.declaration_of(id) else {
                return Ok(());
            };
            let declaration = registered.declaration;
            let scope_ok = matches!(
                declaration.scope,
                DeclaredScope::Machine | DeclaredScope::Profile | DeclaredScope::Project
            );
            let requires_ok = declaration
                .requires
                .iter()
                .all(|requirement| *requirement == Requirement::Project);
            if declaration.sensitive || declaration.identifier || !scope_ok || !requires_ok {
                return Err(UnresolvedReason::NotDisplayable);
            }
            // Structured values never render; of their members only those
            // the shape marks display-safe do (never an id or a path).
            if declaration.value_type.is_structured() {
                let Some(member) = member else {
                    return Err(UnresolvedReason::NotDisplayable);
                };
                return match super::member_target(declaration.value_type, &member) {
                    super::MemberTarget::Scalar(member) if member.display_safe => Ok(()),
                    super::MemberTarget::Scalar(_) | super::MemberTarget::Object(_) => {
                        Err(UnresolvedReason::NotDisplayable)
                    }
                    super::MemberTarget::Unknown => Err(UnresolvedReason::Missing {
                        missing: MissingReason::UnknownMember,
                    }),
                };
            }
            if !is_text_type(declaration.value_type) {
                return Err(if declaration.value_type == VariableType::SecretRef {
                    UnresolvedReason::NotDisplayable
                } else {
                    UnresolvedReason::NotText {
                        found: declaration.value_type,
                    }
                });
            }
            Ok(())
        }
    }
}

/// Whether the picker of a key title lists `entry`: a display-safe,
/// non-sensitive reference with a text value that can have one outside a
/// run.
pub fn display_listed(entry: &super::CatalogEntry, registry: &VariableRegistry) -> bool {
    !entry.sensitive
        && is_text_type(entry.value_type)
        && entry.availability != super::ReferenceAvailability::RuntimeOnly
        && display_safe(&entry.reference, registry).is_ok()
}

/// Variables that hold an id, refused on keys by name too: the project id
/// under its legacy, module and plugin spellings, and the work session's.
/// A plugin's own ids are refused by their declaration
/// ([`super::VariableDeclaration::identifier`], a `host:project.id` source).
pub const IDENTIFIERS: &[&str] = &[
    "projects.current.id",
    "plugins.deckpilot.projects.current.id",
    "workSession.id",
    "workSession.projectId",
];

fn is_text_type(value_type: VariableType) -> bool {
    matches!(
        value_type,
        VariableType::String | VariableType::Integer | VariableType::Boolean
    )
}

/// What the context part of a reference depends on.
fn context_dependencies(reference: &VariableRef, into: &mut BTreeSet<DisplayDependency>) {
    match reference {
        VariableRef::System(SystemVariable::ProfileCurrentName) => {
            into.insert(DisplayDependency::Profile);
        }
        VariableRef::System(SystemVariable::Platform) | VariableRef::Step(_) => {}
        // The project's own values, and shared or module values, which may
        // be set per project.
        VariableRef::System(_) | VariableRef::Shared(_) | VariableRef::Module(_) => {
            into.insert(DisplayDependency::Project);
        }
    }
}

/// The text of one display-safe value, or why there is none.
fn display_value(
    reference: &VariableRef,
    environment: &DisplayEnvironment<'_>,
) -> Result<String, UnresolvedReason> {
    display_safe(reference, environment.registry)?;
    // Display is never a run, whatever the caller's context says.
    let context = ResolutionContext {
        phase: ResolutionPhase::Preview,
        device: None,
        workflow: None,
        execution: None,
        ..environment.context.clone()
    };
    match environment.resolver.resolve(reference, &context) {
        Resolution::Value {
            sensitive: true, ..
        }
        | Resolution::Value {
            value: VariableValue::SecretRef(_),
            ..
        }
        | Resolution::PermissionDenied => Err(UnresolvedReason::NotDisplayable),
        Resolution::Value { value, .. } if value.value_type().is_structured() => {
            Err(UnresolvedReason::NotDisplayable)
        }
        Resolution::Value { value, .. } => {
            if !is_text_type(value.value_type()) {
                return Err(UnresolvedReason::NotText {
                    found: value.value_type(),
                });
            }
            let text = value.render_text().unwrap_or_default();
            Ok(shorten(text))
        }
        Resolution::UnavailableUntilRuntime => Err(UnresolvedReason::RuntimeOnly),
        Resolution::Missing(missing) => Err(UnresolvedReason::Missing { missing }),
        Resolution::Unavailable { owner } => Err(UnresolvedReason::Unavailable { owner }),
    }
}

fn shorten(text: String) -> String {
    if text.chars().count() <= MAX_DISPLAY_VALUE {
        return text;
    }
    let mut short: String = text.chars().take(MAX_DISPLAY_VALUE - 1).collect();
    short.push('…');
    short
}

/// Resolves `template` for display in `environment`: references of
/// display-safe sources become their values, everything else
/// [`UNRESOLVED_MARKER`]; text between `{{` and `}}` that is not a reference
/// (and an unclosed `{{`) stays as typed. Values are inserted as they are,
/// never expanded again. A template without references comes back
/// unchanged, with no dependencies.
pub fn resolve_display_template(
    environment: &DisplayEnvironment<'_>,
    template: &str,
) -> DisplayText {
    let mut out = DisplayText::default();
    if !template.contains('{') {
        template.clone_into(&mut out.text);
        return out;
    }
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        literal(environment, &rest[..start], &mut out);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            // Unclosed: the rest is text, as typed.
            literal(environment, &rest[start..], &mut out);
            return out;
        };
        let raw = &after[..end];
        if let Ok(reference) = VariableRef::parse(raw) {
            reference_value(environment, &reference, raw.trim(), &mut out);
        } else {
            let typed = &rest[start..start + 2 + end + 2];
            if !out.invalid.iter().any(|seen| seen == raw.trim()) {
                out.invalid.push(raw.trim().to_owned());
            }
            literal(environment, typed, &mut out);
        }
        rest = &after[end + 2..];
    }
    literal(environment, rest, &mut out);
    out
}

fn reference_value(
    environment: &DisplayEnvironment<'_>,
    reference: &VariableRef,
    name: &str,
    out: &mut DisplayText,
) {
    context_dependencies(reference, &mut out.dependencies);
    out.dependencies.insert(DisplayDependency::Variable {
        reference: reference.to_string(),
    });
    match display_value(reference, environment) {
        Ok(text) => out.text.push_str(&text),
        Err(reason) => {
            let start = out.text.encode_utf16().count();
            out.text.push_str(UNRESOLVED_MARKER);
            out.markers.push(MarkerSpan {
                start,
                end: start + UNRESOLVED_MARKER.encode_utf16().count(),
                reference: name.to_owned(),
            });
            if !out.unresolved.iter().any(|seen| seen.reference == name) {
                out.unresolved.push(UnresolvedReference {
                    reference: name.to_owned(),
                    reason,
                });
            }
        }
    }
}

/// Literal text, with the legacy placeholders expanded through the engine.
fn literal(environment: &DisplayEnvironment<'_>, mut text: &str, out: &mut DisplayText) {
    loop {
        let next = LEGACY_PLACEHOLDERS
            .iter()
            .filter_map(|(token, variable)| text.find(token).map(|at| (at, *token, *variable)))
            .min_by_key(|(at, ..)| *at);
        let Some((at, token, variable)) = next else {
            out.text.push_str(text);
            return;
        };
        out.text.push_str(&text[..at]);
        let name = &token[1..token.len() - 1];
        reference_value(environment, &VariableRef::System(variable), name, out);
        text = &text[at + token.len()..];
    }
}

/// Resolves the display templates of a page's keys: the device's faces and
/// the editor's previews ask it for every text they show, naming the text
/// ([`TextKey`]), so an implementation can record what each one depends on.
pub trait DisplayTemplates {
    fn display(&self, key: &TextKey, template: &str) -> DisplayText;
}

impl DisplayTemplates for DisplayEnvironment<'_> {
    fn display(&self, _key: &TextKey, template: &str) -> DisplayText {
        resolve_display_template(self, template)
    }
}

/// Records what each text resolved by `inner` depends on.
pub struct RecordingTemplates<'a> {
    inner: &'a dyn DisplayTemplates,
    texts: std::cell::RefCell<Vec<(TextKey, DisplayText)>>,
}

impl std::fmt::Debug for RecordingTemplates<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordingTemplates").finish_non_exhaustive()
    }
}

impl<'a> RecordingTemplates<'a> {
    pub fn new(inner: &'a dyn DisplayTemplates) -> Self {
        Self {
            inner,
            texts: std::cell::RefCell::default(),
        }
    }

    /// Every text resolved so far, in order, with its key (the last
    /// resolution of a key wins).
    pub fn into_texts(self) -> Vec<(TextKey, DisplayText)> {
        let mut texts = self.texts.into_inner();
        let mut seen = std::collections::HashSet::new();
        texts.reverse();
        texts.retain(|(key, _)| seen.insert(*key));
        texts.reverse();
        texts
    }
}

impl DisplayTemplates for RecordingTemplates<'_> {
    fn display(&self, key: &TextKey, template: &str) -> DisplayText {
        let text = self.inner.display(key, template);
        self.texts.borrow_mut().push((*key, text.clone()));
        text
    }
}

/// Texts resolved elsewhere (by the agent, `resolveDisplayTemplates`), by
/// template; a template it did not answer falls back to `fallback`.
pub struct ResolvedTemplates<'a> {
    pub resolved: &'a std::collections::BTreeMap<String, DisplayText>,
    pub fallback: &'a dyn DisplayTemplates,
}

impl std::fmt::Debug for ResolvedTemplates<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedTemplates")
            .field("templates", &self.resolved.len())
            .finish_non_exhaustive()
    }
}

impl DisplayTemplates for ResolvedTemplates<'_> {
    fn display(&self, key: &TextKey, template: &str) -> DisplayText {
        self.resolved
            .get(template)
            .cloned()
            .unwrap_or_else(|| self.fallback.display(key, template))
    }
}

#[cfg(test)]
mod tests;
