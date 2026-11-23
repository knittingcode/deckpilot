use std::collections::BTreeMap;

use serde::Serialize;
use tauri::State;

use deckpilot_core::{
    ActionInstanceId, ActiveProjectContext, Profile, ProfileId, ProjectId, ReferenceFingerprint,
    Usage, WorkflowBlock, WorkflowId,
};
use deckpilot_storage::{ConfigStore, ProfileSummary};

use super::modify_profile;
use crate::error::{CommandError, CommandResult};
use crate::state::{AppState, load_profile};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfilesView {
    profiles: Vec<ProfileSummary>,
    active_profile_id: Option<ProfileId>,
}

#[tauri::command]
pub fn list_profiles(state: State<'_, AppState>) -> CommandResult<ProfilesView> {
    let store = state.store()?;
    Ok(ProfilesView {
        profiles: store.list_profiles()?,
        active_profile_id: store.active_profile()?,
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileView {
    profile: Profile,
    active_project_id: Option<ProjectId>,
    issues: Vec<String>,
    action_usages: BTreeMap<ActionInstanceId, Vec<Usage>>,
    /// Key and gesture bindings only; kept for one release, use
    /// `workflow_references`.
    workflow_usages: BTreeMap<WorkflowId, Vec<Usage>>,
    /// Every reference of each workflow: keys and gestures, Call and Loop
    /// nodes, Random and Switch actions, the Work Session settings
    /// (`Profile::workflow_references_with`).
    workflow_references: BTreeMap<WorkflowId, Vec<Usage>>,
    /// The workflows as entries a key can start, in the profile's order.
    workflow_entries: Vec<WorkflowEntry>,
    /// Actions that need configuration, with their missing parameters
    /// (`deckpilot_core::completeness`). Left out when there are none, so
    /// the frontend reads the answer as before.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    incomplete_actions: BTreeMap<ActionInstanceId, Vec<String>>,
    /// Bindings, Multi Action steps and workflow nodes whose effective
    /// parameters leave required ones without a value. Left out when empty.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    incomplete_uses: Vec<deckpilot_core::IncompleteUse>,
}

/// A workflow as the Action Library lists it: graph workflows in the
/// Workflows plugin's section, step workflows (Multi Actions) in the
/// core's Sequences. Derived, never stored.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowEntry {
    id: WorkflowId,
    name: String,
    kind: WorkflowKind,
    has_triggers: bool,
    /// Why a key cannot start it (a required input, an invalid graph);
    /// one with a required input cannot be dropped on a key.
    block: Option<WorkflowBlock>,
    /// How many places refer to it.
    references: usize,
    /// The fingerprint of those references: sent back to confirm deleting
    /// it (`delete_workflow`), which fails if they changed meanwhile.
    fingerprint: ReferenceFingerprint,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum WorkflowKind {
    Graph,
    Steps,
}

fn workflow_entries(
    profile: &Profile,
    references: &BTreeMap<WorkflowId, Vec<Usage>>,
    registry: &deckpilot_core::ActionRegistry,
) -> Vec<WorkflowEntry> {
    profile
        .workflows
        .iter()
        .map(|workflow| {
            let usages = references.get(&workflow.id).map_or(&[][..], Vec::as_slice);
            WorkflowEntry {
                id: workflow.id,
                name: workflow.name.clone(),
                kind: if workflow.graph.is_some() {
                    WorkflowKind::Graph
                } else {
                    WorkflowKind::Steps
                },
                has_triggers: !workflow.triggers.is_empty(),
                block: deckpilot_core::workflow_block(workflow, profile, registry),
                references: usages.len(),
                fingerprint: ReferenceFingerprint::of(usages),
            }
        })
        .collect()
}

#[tauri::command]
pub fn get_profile(
    state: State<'_, AppState>,
    profile_id: ProfileId,
) -> CommandResult<ProfileView> {
    let store = state.store()?;
    let profile = load_profile(&store, profile_id)?;
    let context = ActiveProjectContext::restore(&profile, store.active_project(profile_id)?);
    let issues = profile
        .validate(&state.catalog)
        .iter()
        .map(ToString::to_string)
        .collect();
    // Uses in layouts other profiles use (shared actions): changing such
    // an action changes those keys too, so they count.
    let others: Vec<deckpilot_core::DeviceLayout> = store
        .shared_layouts()?
        .into_iter()
        .map(|stored| stored.layout)
        .filter(|layout| profile.layout(layout.id).is_none())
        .collect();
    let action_usages = profile
        .actions
        .iter()
        .map(|a| {
            let mut usages = profile.action_usages(a.id);
            for layout in &others {
                for page in &layout.pages {
                    for binding in page
                        .bindings
                        .iter()
                        .filter(|b| b.target.action_id() == Some(a.id))
                    {
                        usages.push(Usage::LayoutBinding {
                            layout: layout.id,
                            layout_name: layout.name.clone(),
                            page: page.id,
                            page_name: page.name.clone(),
                            control: binding.control,
                            trigger: Some(binding.trigger),
                        });
                    }
                }
            }
            (a.id, usages)
        })
        .collect();
    let workflow_usages = profile
        .workflows
        .iter()
        .map(|w| (w.id, profile.workflow_usages(w.id)))
        .collect();
    let actions = state.actions();
    let incomplete_actions = deckpilot_core::completeness::incomplete_actions(&profile, &actions);
    let incomplete_uses = deckpilot_core::completeness::incomplete_uses(&profile, &actions);
    // The Work Session settings name workflows too; a missing or locked
    // settings table reads as none set.
    let session = store
        .work_session_settings(profile_id)
        .map(|settings| super::workflows::session_workflows(&settings))
        .unwrap_or_default();
    let workflow_references = profile
        .workflows
        .iter()
        .map(|w| (w.id, profile.workflow_references_with(w.id, &session)))
        .collect();
    let workflow_entries = workflow_entries(&profile, &workflow_references, &actions);
    Ok(ProfileView {
        incomplete_actions,
        incomplete_uses,
        active_project_id: context.project_id(),
        issues,
        action_usages,
        workflow_usages,
        workflow_references,
        workflow_entries,
        profile,
    })
}

fn summary(store: &deckpilot_storage::SqliteStore, id: ProfileId) -> CommandResult<ProfileSummary> {
    store
        .list_profiles()?
        .into_iter()
        .find(|summary| summary.id == id)
        .ok_or(CommandError::NotFound("profile"))
}

fn checked_name(name: &str) -> CommandResult<String> {
    let name = name.trim();
    if name.is_empty() {
        Err(CommandError::invalid(
            "profileNameEmpty",
            "profile name must not be empty",
        ))
    } else {
        Ok(name.to_owned())
    }
}

#[tauri::command]
pub fn create_profile(
    state: State<'_, AppState>,
    name: String,
    description: String,
) -> CommandResult<ProfileSummary> {
    let profile = Profile {
        description: description.trim().to_owned(),
        ..Profile::new(checked_name(&name)?)
    };
    let store = state.store()?;
    store.save_profile(&profile)?;
    if store.active_profile()?.is_none() {
        store.set_active_profile(Some(profile.id))?;
    }
    crate::agent::notify_reload(&state.paths);
    summary(&store, profile.id)
}

#[tauri::command]
pub fn update_profile_details(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    name: String,
    description: String,
) -> CommandResult<()> {
    let name = checked_name(&name)?;
    modify_profile(&state, profile_id, |profile| {
        profile.name = name;
        description.trim().clone_into(&mut profile.description);
        Ok(())
    })
}

/// Copies a profile with new identifiers for everything in it, so editing
/// the copy never affects the original.
#[tauri::command]
pub fn duplicate_profile(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    name: String,
) -> CommandResult<ProfileSummary> {
    let name = checked_name(&name)?;
    let store = state.store()?;
    let copy = load_profile(&store, profile_id)?.duplicate(&name);
    store.save_profile(&copy)?;
    // As after creating one: the agent publishes its profile list again.
    crate::agent::notify_reload(&state.paths);
    summary(&store, copy.id)
}

#[tauri::command]
pub fn delete_profile(state: State<'_, AppState>, profile_id: ProfileId) -> CommandResult<()> {
    if state.store()?.delete_profile(profile_id)? {
        crate::agent::notify_reload(&state.paths);
        Ok(())
    } else {
        Err(CommandError::NotFound("profile"))
    }
}

#[cfg(test)]
mod tests {
    use deckpilot_core::graph::{ValueKind, WorkflowInput};
    use deckpilot_core::{BindingTarget, ControlBinding, DeviceLayout, Trigger, Workflow};
    use deckpilot_device::{ControlId, DeviceModelId, models};

    use super::*;

    fn graph(name: &str) -> Workflow {
        let mut workflow = Workflow::new(name);
        workflow.graph = Some(
            serde_json::from_value(serde_json::json!({
                "version": 1,
                "nodes": [{"id": "start", "type": "trigger"}, {"id": "end", "type": "stop"}],
                "edges": [{"from": "start", "to": "end"}],
            }))
            .unwrap(),
        );
        workflow
    }

    #[test]
    fn workflow_entries_tell_graphs_from_sequences_and_block_inputs() {
        let mut profile = Profile::new("Work");
        let deploy = graph("Deploy");
        let mut needs = graph("Needs");
        needs.inputs.push(WorkflowInput {
            required: true,
            ..WorkflowInput::new("target", ValueKind::Text)
        });
        let multi = Workflow::new("Multi");
        let (deploy_id, multi_id) = (deploy.id, multi.id);
        profile.workflows.extend([deploy, needs, multi]);
        let mut layout = DeviceLayout::new("Main", DeviceModelId::new(models::STREAM_DECK_MK2_ID));
        for (index, workflow) in [(0, deploy_id), (1, deploy_id), (2, multi_id)] {
            layout.pages[0].bindings.push(ControlBinding::new(
                ControlId::Key { index },
                Trigger::Press,
                BindingTarget::Workflow { workflow },
            ));
        }
        profile.layouts.push(layout);
        let references: BTreeMap<WorkflowId, Vec<Usage>> = profile
            .workflows
            .iter()
            .map(|w| (w.id, profile.workflow_references(w.id)))
            .collect();
        let entries = workflow_entries(
            &profile,
            &references,
            &deckpilot_core::ActionRegistry::default(),
        );
        let json = serde_json::to_value(&entries).unwrap();
        assert_eq!(json[0]["name"], "Deploy");
        assert_eq!(json[0]["kind"], "graph");
        assert_eq!(json[0]["references"], 2);
        assert_eq!(json[0]["block"], serde_json::Value::Null);
        assert_eq!(json[0]["hasTriggers"], false);
        assert_eq!(
            json[0]["fingerprint"],
            profile.workflow_fingerprint(deploy_id).to_string()
        );
        assert_eq!(json[1]["block"]["code"], "needsInputs");
        assert!(
            json[1]["block"]["message"]
                .as_str()
                .unwrap()
                .contains("Sub-workflow")
        );
        assert_eq!(json[2]["kind"], "steps");
        assert_eq!(json[2]["references"], 1);
    }
}
