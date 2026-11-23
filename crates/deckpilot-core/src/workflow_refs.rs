//! Workflows as things keys refer to (`docs/data-model.md`, "Workflows"):
//! every place that refers to a workflow, deleting one that is referenced
//! after the user confirmed exactly those references, why a workflow
//! cannot be started from a key, and how a key that refers to one that
//! cannot run here looks.
//!
//! A key refers to a workflow by its id ([`BindingTarget::Workflow`]); the
//! graph or steps are never copied, so a rename or an edit reaches every
//! key at once, and a workflow that is gone leaves its keys in place,
//! shown as missing.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::graph::NodeKind;
use crate::i18n::SystemText;
use crate::shared_layouts::mentions;
use deckpilot_device::ControlId;

use crate::{
    ActionRegistry, BindingTarget, LayoutId, PageId, Profile, ProfileIssue, Trigger, Usage,
    Workflow, WorkflowId,
};

/// A digest of the places that refer to a workflow
/// ([`Profile::workflow_references`]), so that a delete the user confirmed
/// fails when they changed meanwhile (another window bound a key, say).
/// Identifies the places only (not their names): renaming a layout does
/// not invalidate a confirmation. Not a security measure, only a check
/// for concurrent edits.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ReferenceFingerprint(String);

impl ReferenceFingerprint {
    /// The fingerprint of `usages`, in any order.
    pub fn of(usages: &[Usage]) -> Self {
        let mut sites: Vec<String> = usages.iter().map(site_key).collect();
        sites.sort_unstable();
        // FNV-1a, 64 bits: stable across builds and platforms.
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for site in &sites {
            for byte in site.bytes().chain(std::iter::once(b'\n')) {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0100_0000_01b3);
            }
        }
        Self(format!("{:016x}-{}", hash, sites.len()))
    }

    /// The fingerprint as the app received it.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ReferenceFingerprint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The identity of the place a usage names, without display names.
fn site_key(usage: &Usage) -> String {
    fn json(value: &impl Serialize) -> String {
        serde_json::to_string(value).unwrap_or_default()
    }
    match usage {
        Usage::WorkflowStep {
            workflow, position, ..
        } => format!("step:{workflow}:{position}"),
        Usage::LayoutBinding {
            layout,
            page,
            control,
            trigger,
            ..
        } => format!(
            "binding:{layout}:{page}:{}:{}",
            json(control),
            trigger.as_ref().map(json).unwrap_or_default()
        ),
        Usage::WorkflowNode { workflow, node, .. } => format!("node:{workflow}:{node}"),
        Usage::ActionParameter { action, .. } => format!("action:{action}"),
        Usage::WorkSessionSetting { setting } => format!("workSession:{}", json(setting)),
    }
}

/// A setting of the profile's Work Session that names a workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkSessionSetting {
    /// Runs when a session starts.
    StartWorkflow,
    /// Verifies a session before it finishes.
    VerificationWorkflow,
}

/// The workflows the profile's Work Session settings name (they are
/// stored apart from the profile document, `work_session_settings`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WorkSessionWorkflows {
    pub start: Option<WorkflowId>,
    pub verification: Option<WorkflowId>,
}

impl WorkSessionWorkflows {
    /// The settings that name `id`.
    pub fn naming(&self, id: WorkflowId) -> Vec<WorkSessionSetting> {
        [
            (self.start, WorkSessionSetting::StartWorkflow),
            (self.verification, WorkSessionSetting::VerificationWorkflow),
        ]
        .into_iter()
        .filter(|(set, _)| *set == Some(id))
        .map(|(_, setting)| setting)
        .collect()
    }

    /// Clears the settings that name `id`; returns them.
    pub fn clear(&mut self, id: WorkflowId) -> Vec<WorkSessionSetting> {
        let cleared = self.naming(id);
        if self.start == Some(id) {
            self.start = None;
        }
        if self.verification == Some(id) {
            self.verification = None;
        }
        cleared
    }
}

/// What a confirmed delete may also change ([`Profile::delete_workflow_confirmed`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeleteOptions {
    /// Remove the key and gesture bindings that start it.
    pub clear_bindings: bool,
    /// Clear the Work Session settings that name it. Without it, a
    /// workflow they name is not deleted.
    pub clear_work_session: bool,
}

/// What a confirmed delete removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletedWorkflow {
    pub workflow: Workflow,
    /// The key and gesture bindings that ran it and were cleared (empty
    /// unless clearing was asked for).
    pub cleared: Vec<Usage>,
    /// The Work Session settings that were cleared.
    pub cleared_work_session: Vec<WorkSessionSetting>,
}

impl Profile {
    /// Every place of the profile that refers to workflow `id`: key and
    /// gesture bindings that run it, nodes of other graph workflows that
    /// run it (Call, Loop) or name it in their overrides, steps whose
    /// overrides name it, and actions whose parameters name it (a Random
    /// Action's candidates, a Multi Action Switch's states, stored as
    /// `workflow:<id>` lines) or keys whose overrides of such an action
    /// do. The workflow's own nodes are not counted: they go with it. The
    /// Work Session settings are not in the profile document: see
    /// [`Profile::workflow_references_with`].
    pub fn workflow_references(&self, id: WorkflowId) -> Vec<Usage> {
        self.workflow_references_with(id, &WorkSessionWorkflows::default())
    }

    /// [`Profile::workflow_references`], and the Work Session settings
    /// (`session`) that name it.
    pub fn workflow_references_with(
        &self,
        id: WorkflowId,
        session: &WorkSessionWorkflows,
    ) -> Vec<Usage> {
        let needle = id.to_string();
        let names = |json: String| mentions(&json, &needle);
        let mut usages = self.target_usages(|t| *t == BindingTarget::Workflow { workflow: id });
        // Keys that override a Random or Switch action's parameters with
        // the workflow.
        for layout in &self.layouts {
            for page in &layout.pages {
                for binding in &page.bindings {
                    if let BindingTarget::Action { action, overrides } = &binding.target
                        && !overrides.is_empty()
                        && names(serde_json::to_string(overrides).unwrap_or_default())
                    {
                        usages.push(Usage::ActionParameter {
                            action: *action,
                            action_name: self
                                .action(*action)
                                .map_or_else(|| action.to_string(), |a| a.name.clone()),
                        });
                    }
                }
            }
        }
        for workflow in self.workflows.iter().filter(|w| w.id != id) {
            if let Some(graph) = &workflow.graph {
                for node in &graph.nodes {
                    let refers = match &node.kind {
                        NodeKind::Call { workflow, .. } | NodeKind::Loop { workflow, .. } => {
                            *workflow == id
                        }
                        // Fixed values and mappings (a literal or template
                        // that fills a Random or Switch parameter).
                        NodeKind::Action {
                            overrides,
                            mappings,
                            ..
                        } => {
                            (!overrides.is_empty()
                                && names(serde_json::to_string(overrides).unwrap_or_default()))
                                || (!mappings.is_empty()
                                    && names(serde_json::to_string(mappings).unwrap_or_default()))
                        }
                        _ => false,
                    };
                    if refers {
                        usages.push(Usage::WorkflowNode {
                            workflow: workflow.id,
                            workflow_name: workflow.name.clone(),
                            node: node.id.clone(),
                        });
                    }
                }
            }
            for (index, step) in workflow.steps.iter().enumerate() {
                if !step.overrides.is_empty()
                    && names(serde_json::to_string(&step.overrides).unwrap_or_default())
                {
                    usages.push(Usage::WorkflowStep {
                        workflow: workflow.id,
                        workflow_name: workflow.name.clone(),
                        position: index + 1,
                    });
                }
            }
        }
        for action in &self.actions {
            if names(serde_json::to_string(&action.parameters).unwrap_or_default()) {
                usages.push(Usage::ActionParameter {
                    action: action.id,
                    action_name: action.name.clone(),
                });
            }
        }
        usages.extend(
            session
                .naming(id)
                .into_iter()
                .map(|setting| Usage::WorkSessionSetting { setting }),
        );
        usages
    }

    /// The fingerprint of [`Profile::workflow_references`] now.
    pub fn workflow_fingerprint(&self, id: WorkflowId) -> ReferenceFingerprint {
        ReferenceFingerprint::of(&self.workflow_references(id))
    }

    /// Deletes workflow `id` although it may be referenced, after the user
    /// confirmed its references, with the Work Session settings `session`,
    /// as they were when `expected` was taken.
    ///
    /// - Fails with [`ProfileIssue::ReferencesChanged`] when the references
    ///   are no longer those (nothing changes).
    /// - Fails with [`ProfileIssue::UsedByWorkSession`] when a Work Session
    ///   setting names it and `options.clear_work_session` is not set: a
    ///   start or verification workflow is never cleared implicitly.
    ///   Otherwise those settings are cleared in `session`, which the
    ///   caller stores with the profile.
    /// - With `options.clear_bindings`, the key and gesture bindings that
    ///   run it are removed too; otherwise they stay and show "Workflow
    ///   missing".
    /// - Graphs and action parameters that name it are never edited:
    ///   validation then reports them.
    pub fn delete_workflow_confirmed(
        &mut self,
        id: WorkflowId,
        expected: &ReferenceFingerprint,
        options: DeleteOptions,
        session: &mut WorkSessionWorkflows,
    ) -> Result<DeletedWorkflow, ProfileIssue> {
        let index = self
            .workflows
            .iter()
            .position(|workflow| workflow.id == id)
            .ok_or(ProfileIssue::NotFound("workflow"))?;
        if ReferenceFingerprint::of(&self.workflow_references_with(id, session)) != *expected {
            return Err(ProfileIssue::ReferencesChanged);
        }
        if !session.naming(id).is_empty() && !options.clear_work_session {
            return Err(ProfileIssue::UsedByWorkSession);
        }
        let cleared_work_session = session.clear(id);
        let target = BindingTarget::Workflow { workflow: id };
        let cleared = if options.clear_bindings {
            let cleared = self.target_usages(|t| *t == target);
            for page in self.layouts.iter_mut().flat_map(|l| &mut l.pages) {
                page.bindings.retain(|binding| binding.target != target);
            }
            cleared
        } else {
            Vec::new()
        };
        Ok(DeletedWorkflow {
            workflow: self.workflows.remove(index),
            cleared,
            cleared_work_session,
        })
    }
}

/// Why a workflow cannot be started from a key at all, whatever the
/// moment ([`workflow_block`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkflowBlockCode {
    /// A required input: it runs only from a Sub-workflow node.
    NeedsInputs,
    /// Its graph has errors.
    InvalidGraph,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowBlock {
    pub code: WorkflowBlockCode,
    /// The English explanation, as the engine refuses a run.
    pub message: String,
}

/// Whether `workflow` of `profile` can be started without input values,
/// as a key starts it: the check the engine makes before a run
/// ([`crate::Engine::check_runnable`]), in the same order (inputs, then
/// the graph). `None`: it can.
pub fn workflow_block(
    workflow: &Workflow,
    profile: &Profile,
    registry: &ActionRegistry,
) -> Option<WorkflowBlock> {
    let graph = workflow.graph.as_ref()?;
    if let Some(input) = workflow.inputs.iter().find(|input| input.required) {
        return Some(WorkflowBlock {
            code: WorkflowBlockCode::NeedsInputs,
            message: format!(
                "{} needs a value for its input `{}`; run it from a Sub-workflow node",
                workflow.name, input.key
            ),
        });
    }
    let issues = graph.validate_as(workflow, profile, registry);
    if crate::graph::WorkflowGraph::is_runnable(&issues) {
        return None;
    }
    let first = issues
        .iter()
        .find(|i| i.severity == crate::graph::IssueSeverity::Error)
        .map(|i| i.message.clone())
        .unwrap_or_default();
    Some(WorkflowBlock {
        code: WorkflowBlockCode::InvalidGraph,
        message: first,
    })
}

/// What decides how keys that start workflows look and whether they run,
/// beyond the profile: which absent workflows belong to another profile,
/// and whether graph workflows may run from keys.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkflowScope {
    /// For each of this profile's layouts, the workflows of the other
    /// profiles that use it too (a shared layout). On a key of a layout,
    /// an absent workflow among its set is "Not in this profile", any
    /// other is "Workflow missing" (deleted). `None`: not known, every
    /// absent one is "Not in this profile", as before.
    pub foreign: Option<BTreeMap<LayoutId, BTreeSet<WorkflowId>>>,
    /// The Workflows plugin (`deckpilot.workflows`) is disabled: keys and
    /// gestures that start a graph workflow are kept and do not run (step
    /// workflows, the core's Multi Actions, still do).
    pub graphs_off: bool,
}

impl WorkflowScope {
    /// How a key of `layout` shows absent workflow `id`; without a layout
    /// (a start from the app), by every layout of the profile.
    pub fn absent_text(&self, layout: Option<LayoutId>, id: WorkflowId) -> SystemText {
        let Some(foreign) = &self.foreign else {
            return SystemText::NotInThisProfile;
        };
        let theirs = match layout {
            Some(layout) => foreign.get(&layout).is_some_and(|set| set.contains(&id)),
            None => foreign.values().any(|set| set.contains(&id)),
        };
        if theirs {
            SystemText::NotInThisProfile
        } else {
            SystemText::WorkflowMissing
        }
    }

    /// Why a key of `layout` may not start `id` of `profile` now (before
    /// the engine's own checks), if it may not.
    pub fn refusal(
        &self,
        profile: &Profile,
        layout: Option<LayoutId>,
        id: WorkflowId,
    ) -> Option<WorkflowRefusal> {
        match profile.workflow(id) {
            None => Some(match self.absent_text(layout, id) {
                SystemText::WorkflowMissing => WorkflowRefusal::Missing,
                _ => WorkflowRefusal::NotInThisProfile,
            }),
            Some(workflow) if self.graphs_off && workflow.graph.is_some() => {
                Some(WorkflowRefusal::PluginOff)
            }
            Some(_) => None,
        }
    }
}

/// Why a key or gesture did not start its workflow. Each is reported to
/// the user with its code (`notification.code`, an IPC error's `reason`).
/// Serialized as its code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum WorkflowRefusal {
    /// It was deleted: the key is kept and shows "Workflow missing".
    #[serde(rename = "workflowMissing")]
    Missing,
    /// It belongs to another profile that shares the layout.
    #[serde(rename = "workflowNotInProfile")]
    NotInThisProfile,
    /// A graph workflow while the Workflows plugin is disabled.
    #[serde(rename = "workflowPluginOff")]
    PluginOff,
    /// Its graph has errors.
    #[serde(rename = "workflowInvalid")]
    Invalid,
    /// It has a required input.
    #[serde(rename = "workflowNeedsInputs")]
    NeedsInputs,
    /// It is running already.
    #[serde(rename = "workflowAlreadyRunning")]
    AlreadyRunning,
    /// Too many workflows are running.
    #[serde(rename = "workflowLimit")]
    Limit,
}

impl WorkflowRefusal {
    /// The stable code (`workflowMissing`…).
    pub fn code(self) -> &'static str {
        match self {
            Self::Missing => "workflowMissing",
            Self::NotInThisProfile => "workflowNotInProfile",
            Self::PluginOff => "workflowPluginOff",
            Self::Invalid => "workflowInvalid",
            Self::NeedsInputs => "workflowNeedsInputs",
            Self::AlreadyRunning => "workflowAlreadyRunning",
            Self::Limit => "workflowLimit",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        [
            Self::Missing,
            Self::NotInThisProfile,
            Self::PluginOff,
            Self::Invalid,
            Self::NeedsInputs,
            Self::AlreadyRunning,
            Self::Limit,
        ]
        .into_iter()
        .find(|refusal| refusal.code() == code)
    }

    /// The English sentence the user reads.
    pub fn message(self) -> &'static str {
        match self {
            Self::Missing => "the workflow was deleted; the key is kept, choose another one for it",
            Self::NotInThisProfile => "the workflow belongs to another profile",
            Self::PluginOff => {
                "the Workflows plugin is off; turn it on in Settings › Plugins to run it"
            }
            Self::Invalid => "the workflow has errors; fix them in Workflows",
            Self::NeedsInputs => "the workflow needs input values; run it from a Sub-workflow node",
            Self::AlreadyRunning => "the workflow is already running",
            Self::Limit => "too many workflows are running; wait for one to finish",
        }
    }
}

/// Why saving a layout refuses a new key or gesture binding that starts a
/// workflow ([`crate::DeviceLayout::workflow_binding_issues`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkflowBindingReason {
    /// Not one of the saving profile's workflows (another profile's, or
    /// deleted): a key never runs another profile's workflow.
    WorkflowNotInProfile,
    /// A graph workflow while the Workflows plugin is off.
    PluginFeatureOff,
    /// It has a required input and cannot run from a key.
    WorkflowNeedsInputs,
}

impl WorkflowBindingReason {
    pub fn code(self) -> &'static str {
        match self {
            Self::WorkflowNotInProfile => "workflowNotInProfile",
            Self::PluginFeatureOff => "pluginFeatureOff",
            Self::WorkflowNeedsInputs => "workflowNeedsInputs",
        }
    }
}

/// A binding that starts a workflow it may not start from the saving
/// profile: refused when new, kept and reported when it was stored like
/// this already.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowBindingIssue {
    pub page: PageId,
    pub page_name: String,
    pub control: ControlId,
    pub trigger: Trigger,
    pub workflow: WorkflowId,
    pub reason: WorkflowBindingReason,
    /// It was saved like this before: kept, only reported.
    pub stored: bool,
}

impl std::fmt::Display for WorkflowBindingIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let what = match self.reason {
            WorkflowBindingReason::WorkflowNotInProfile => {
                "starts a workflow that is not in this profile"
            }
            WorkflowBindingReason::PluginFeatureOff => {
                "starts a graph workflow while the Workflows plugin is off"
            }
            WorkflowBindingReason::WorkflowNeedsInputs => {
                "starts a workflow that needs input values; use it from a Sub-workflow node"
            }
        };
        write!(f, "page `{}`: {} {what}", self.page_name, self.control)
    }
}

impl crate::DeviceLayout {
    /// The bindings of this layout that start a workflow the saving
    /// `profile` may not start from a key: another profile's or a deleted
    /// one, a graph workflow while `graphs_off`, or one with a required
    /// input. Each is `stored` when `stored` (the saved version of this
    /// layout) already starts the same workflow somewhere, so moving or
    /// duplicating a key or page is not a new reference: those are kept
    /// and reported, the ones to a workflow the layout did not start are
    /// refused
    /// (`docs/data-model.md`, "Workflows").
    pub fn workflow_binding_issues(
        &self,
        stored: Option<&crate::DeviceLayout>,
        profile: &Profile,
        registry: &ActionRegistry,
        graphs_off: bool,
    ) -> Vec<WorkflowBindingIssue> {
        let mut found = Vec::new();
        for page in &self.pages {
            for binding in &page.bindings {
                let BindingTarget::Workflow { workflow } = binding.target else {
                    continue;
                };
                let reason = match profile.workflow(workflow) {
                    None => WorkflowBindingReason::WorkflowNotInProfile,
                    Some(definition) if graphs_off && definition.graph.is_some() => {
                        WorkflowBindingReason::PluginFeatureOff
                    }
                    Some(definition)
                        if workflow_block(definition, profile, registry)
                            .is_some_and(|b| b.code == WorkflowBlockCode::NeedsInputs) =>
                    {
                        WorkflowBindingReason::WorkflowNeedsInputs
                    }
                    Some(_) => continue,
                };
                // Moving, swapping or duplicating a page or key adds no
                // new reference: stored is any binding of the saved
                // layout to the same workflow.
                let stored = stored.is_some_and(|stored| {
                    stored
                        .pages
                        .iter()
                        .flat_map(|saved| &saved.bindings)
                        .any(|b| b.target == binding.target)
                });
                found.push(WorkflowBindingIssue {
                    page: page.id,
                    page_name: page.name.clone(),
                    control: binding.control,
                    trigger: binding.trigger,
                    workflow,
                    reason,
                    stored,
                });
            }
        }
        found
    }
}

#[cfg(test)]
#[path = "workflow_refs/tests.rs"]
mod tests;
