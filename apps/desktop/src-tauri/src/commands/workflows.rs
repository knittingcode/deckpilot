use tauri::State;

use serde::{Deserialize, Serialize};

use deckpilot_core::graph::{Conversion, GraphIssue, from_steps};
use deckpilot_core::{
    ContextIssue, DeleteOptions, Profile, ProfileId, ProfileIssue, ReferenceFingerprint, Usage,
    WorkSessionWorkflows, Workflow, WorkflowId,
};

use super::reload::Saved;
use super::{modify_profile, modify_profile_and_reload};
use crate::error::{CommandError, CommandResult, Param};
use crate::state::{AppState, WORKFLOWS_PLUGIN, load_profile};

/// Workflow management belongs to the Workflows plugin
/// (`deckpilot.workflows`, decisions D1/D6 of 2026-11-20) and is refused
/// while it is off, except for step workflows without a graph: those are
/// the key editor's Multi Actions, a core feature that keeps working with
/// the plugin off. Keys and gestures that start a graph workflow are kept
/// and do not run while the plugin is off (the agent refuses them, code
/// `workflowPluginOff`); they run again once it is on. No workflow is ever
/// deleted because the plugin is off.
fn require_workflows(state: &AppState, workflow: Option<&Workflow>) -> CommandResult<()> {
    if workflow.is_some_and(|w| w.graph.is_none()) {
        return Ok(());
    }
    state.require_feature(WORKFLOWS_PLUGIN)
}

/// Problems of a workflow's graph, as the engine sees them, for the editor
/// to show on the nodes. The workflow does not have to be saved.
#[tauri::command]
pub fn validate_workflow_graph(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    workflow: Workflow,
) -> CommandResult<Vec<GraphIssue>> {
    state.require_feature(WORKFLOWS_PLUGIN)?;
    let profile = load_profile(&*state.store()?, profile_id)?;
    Ok(workflow.graph_issues(&profile, &state.actions()))
}

/// The graph form of a workflow's steps, with notes where it behaves
/// differently. Nothing is saved: the editor stores the graph when the user
/// saves, and the steps are kept.
#[tauri::command]
pub fn convert_workflow(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    workflow: Workflow,
) -> CommandResult<Conversion> {
    state.require_feature(WORKFLOWS_PLUGIN)?;
    let profile = load_profile(&*state.store()?, profile_id)?;
    Ok(from_steps(&workflow, &profile))
}

/// A saved workflow, with its steps or nodes whose action is not meant
/// for a Multi Action step or a workflow node (`deckpilot_core::contexts`),
/// each marked `stored` when it was saved before. Serialized as the
/// workflow with a `contextIssues` field, left out when empty, so the
/// frontend reads it as before.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedWorkflow {
    #[serde(flatten)]
    workflow: Workflow,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    context_issues: Vec<ContextIssue>,
    /// Steps or nodes that need configuration
    /// (`deckpilot_core::completeness`): saved, refused when they run.
    /// Left out when empty.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    incomplete: Vec<deckpilot_core::IncompleteUse>,
}

/// Creates or replaces a workflow after validating it. Returns once the
/// agent has read it, so it can be started right away. A graph workflow
/// needs the Workflows plugin, and so does replacing one; a step workflow
/// (Multi Action) does not. Uses outside an action's contexts are reported
/// and never refused.
#[tauri::command]
pub async fn save_workflow(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    workflow: Workflow,
) -> CommandResult<Saved<SavedWorkflow>> {
    require_workflows(&state, Some(&workflow))?;
    if workflow.graph.is_none() {
        // Turning a graph back into steps is managing a workflow too.
        let profile = load_profile(&*state.store()?, profile_id)?;
        if let Some(existing) = profile.workflow(workflow.id) {
            require_workflows(&state, Some(existing))?;
        }
    }
    let mut workflow = workflow;
    super::trim_in_place(&mut workflow.name);
    super::trim_in_place(&mut workflow.description);
    modify_profile_and_reload(&state, profile_id, |profile| {
        let actions = state.actions();
        let issues = workflow.validate(profile, &actions);
        if !issues.is_empty() {
            return Err(CommandError::Invalid(issues.join("\n")));
        }
        let context_issues =
            workflow.context_issues(profile.workflow(workflow.id), profile, &actions);
        let incomplete =
            deckpilot_core::completeness::workflow_incomplete(&workflow, profile, &actions);
        match profile.workflows.iter_mut().find(|w| w.id == workflow.id) {
            Some(existing) => *existing = workflow.clone(),
            None => profile.workflows.push(workflow.clone()),
        }
        Ok(SavedWorkflow {
            workflow,
            context_issues,
            incomplete,
        })
    })
    .await
}

#[tauri::command]
pub fn duplicate_workflow(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    workflow_id: WorkflowId,
    name: String,
) -> CommandResult<Workflow> {
    modify_profile(&state, profile_id, |profile| {
        let mut copy = profile
            .workflow(workflow_id)
            .ok_or(CommandError::NotFound("workflow"))?
            .clone();
        require_workflows(&state, Some(&copy))?;
        copy.id = WorkflowId::new();
        name.trim().clone_into(&mut copy.name);
        if copy.name.is_empty() {
            return Err(CommandError::invalid(
                "workflowNameEmpty",
                "workflow name must not be empty",
            ));
        }
        profile.workflows.push(copy.clone());
        Ok(copy)
    })
}

/// What the user confirmed when deleting a referenced workflow: the
/// fingerprint of the references they were shown
/// ([`deckpilot_core::ReferenceFingerprint`]) and whether the keys and
/// gestures that start it are cleared too.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteConfirmation {
    fingerprint: String,
    #[serde(default)]
    clear_bindings: bool,
    /// Also clear the Work Session settings that name it. Without it, a
    /// workflow a Work Session setting names is not deleted.
    #[serde(default)]
    clear_work_session: bool,
}

/// The workflows the profile's Work Session settings name.
pub(crate) fn session_workflows(
    settings: &deckpilot_storage::work_sessions::WorkSessionSettings,
) -> WorkSessionWorkflows {
    WorkSessionWorkflows {
        start: settings.start_workflow,
        verification: settings.verification_workflow,
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// The refusal of a delete that needs (another) confirmation: the
/// references as they are now and their fingerprint, for the dialog to
/// list and send back.
fn needs_confirmation(
    reason: &'static str,
    message: &str,
    usages: &[Usage],
    fingerprint: &ReferenceFingerprint,
) -> CommandError {
    CommandError::invalid_with(
        reason,
        message,
        [
            ("count", usages.len().into()),
            (
                "usages",
                Param::Data(serde_json::to_value(usages).unwrap_or_default()),
            ),
            ("fingerprint", fingerprint.to_string().into()),
        ],
    )
}

/// Deletes a workflow: a graph workflow only while the Workflows plugin
/// is on, a step workflow (Multi Action) always.
///
/// - Without `confirm`, a workflow that something refers to (keys and
///   gestures, Call and Loop nodes, Random and Switch actions, the Work
///   Session settings: `Profile::workflow_references_with`) is refused
///   with `workflowInUse`, which lists the references (`params.usages`)
///   and their `params.fingerprint`. One nothing refers to is deleted as
///   before.
/// - With `confirm`, it is deleted if its references are still those the
///   fingerprint names, else refused with `workflowReferencesChanged`
///   (with the current list and fingerprint). Keys and gestures that start
///   it stay and show "Workflow missing" unless `clearBindings` asks to
///   remove them. A Work Session start or verification workflow is never
///   cleared implicitly: without `clearWorkSession` the delete is refused
///   with `workflowInWorkSession`. Graphs and action parameters are never
///   edited.
///
/// The profile and the Work Session settings are checked and written in
/// one transaction. Returns once the agent has read the change, with how
/// many key and gesture bindings and Work Session settings were cleared.
#[tauri::command]
pub async fn delete_workflow(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    workflow_id: WorkflowId,
    confirm: Option<DeleteConfirmation>,
) -> CommandResult<Saved<DeletedView>> {
    let value = {
        let store = state.store()?;
        store.modify_profile_with_work_session(profile_id, now_ms(), |profile, settings| {
            require_workflows(&state, profile.workflow(workflow_id))?;
            let mut session = session_workflows(settings);
            let done = delete_in(profile, workflow_id, confirm.as_ref(), &mut session)?;
            settings.start_workflow = session.start;
            settings.verification_workflow = session.verification;
            Ok::<_, CommandError>(done)
        })?
    };
    let agent_reload =
        super::reload::reload_agent(&state.paths, super::reload::RELOAD_TIMEOUT).await;
    Ok(Saved {
        value,
        agent_reload,
    })
}

/// What a delete did: the key and gesture bindings it cleared.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeletedView {
    cleared: usize,
    cleared_work_session: usize,
}

fn delete_in(
    profile: &mut Profile,
    workflow_id: WorkflowId,
    confirm: Option<&DeleteConfirmation>,
    session: &mut WorkSessionWorkflows,
) -> CommandResult<DeletedView> {
    if profile.workflow(workflow_id).is_none() {
        return Err(CommandError::NotFound("workflow"));
    }
    let usages = profile.workflow_references_with(workflow_id, session);
    let current = ReferenceFingerprint::of(&usages);
    match confirm {
        None if usages.is_empty() => {
            profile.delete_workflow(workflow_id)?;
            Ok(DeletedView {
                cleared: 0,
                cleared_work_session: 0,
            })
        }
        None => Err(needs_confirmation(
            "workflowInUse",
            &format!(
                "the workflow is still used in {} place(s); confirm to delete it",
                usages.len()
            ),
            &usages,
            &current,
        )),
        Some(confirm) => {
            let expected = ReferenceFingerprint::new(confirm.fingerprint.clone());
            let options = DeleteOptions {
                clear_bindings: confirm.clear_bindings,
                clear_work_session: confirm.clear_work_session,
            };
            match profile.delete_workflow_confirmed(workflow_id, &expected, options, session) {
                Ok(deleted) => Ok(DeletedView {
                    cleared: deleted.cleared.len(),
                    cleared_work_session: deleted.cleared_work_session.len(),
                }),
                Err(ProfileIssue::UsedByWorkSession) => Err(needs_confirmation(
                    "workflowInWorkSession",
                    "the Work Session settings use the workflow; also clear them to delete it",
                    &usages,
                    &current,
                )),
                Err(ProfileIssue::ReferencesChanged) => Err(needs_confirmation(
                    "workflowReferencesChanged",
                    "what uses the workflow changed since you confirmed; review it again",
                    &usages,
                    &current,
                )),
                Err(other) => Err(other.into()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use deckpilot_core::{
        ActionContext, ActionInstanceId, BindingTarget, ContextIssue, ContextSite, ControlBinding,
        DeviceLayout, IncompatibilityReason, OnFailure, Trigger,
    };
    use deckpilot_device::{ControlId, DeviceModelId, models};

    use super::*;
    use crate::commands::reload::AgentReload;

    /// Work, with Deploy on keys 0 and 1 and called by a Multi Action
    /// Switch's state (an action parameter).
    fn referenced() -> (Profile, WorkflowId) {
        let mut profile = Profile::new("Work");
        let deploy = Workflow::new("Deploy");
        let id = deploy.id;
        profile.workflows.push(deploy);
        let mut layout = DeviceLayout::new("Main", DeviceModelId::new(models::STREAM_DECK_MK2_ID));
        for index in [0, 1] {
            layout.pages[0].bindings.push(ControlBinding::new(
                ControlId::Key { index },
                Trigger::Press,
                BindingTarget::Workflow { workflow: id },
            ));
        }
        profile.layouts.push(layout);
        profile.actions.push(deckpilot_core::ActionInstance {
            id: ActionInstanceId::new(),
            name: "Switch".into(),
            action_type: deckpilot_core::ActionTypeId::new("deck.multi-switch").unwrap(),
            parameters: deckpilot_core::ActionParameters::new().with(
                "states",
                deckpilot_core::ParameterValue::Text(format!("workflow:{id}")),
            ),
            description: String::new(),
            icon: None,
            label: None,
        });
        (profile, id)
    }

    fn payload(error: &CommandError) -> serde_json::Value {
        serde_json::to_value(error).unwrap()
    }

    fn confirm(fingerprint: &str, clear_bindings: bool) -> DeleteConfirmation {
        DeleteConfirmation {
            fingerprint: fingerprint.to_owned(),
            clear_bindings,
            clear_work_session: false,
        }
    }

    #[test]
    fn a_work_session_workflow_is_deleted_only_when_its_setting_is_cleared_too() {
        let (mut profile, id) = referenced();
        let mut session = WorkSessionWorkflows {
            start: None,
            verification: Some(id),
        };
        // Listed with the other references.
        let refused = payload(&delete_in(&mut profile, id, None, &mut session).unwrap_err());
        assert_eq!(refused["code"], "invalid.workflowInUse");
        assert_eq!(refused["params"]["count"], 4);
        assert_eq!(
            refused["params"]["usages"][3],
            serde_json::json!({ "type": "workSessionSetting", "setting": "verificationWorkflow" })
        );
        let fingerprint = refused["params"]["fingerprint"]
            .as_str()
            .unwrap()
            .to_owned();
        // Clearing the keys is not enough: never cleared implicitly.
        let before = (profile.clone(), session);
        let refused = payload(
            &delete_in(
                &mut profile,
                id,
                Some(&confirm(&fingerprint, true)),
                &mut session,
            )
            .unwrap_err(),
        );
        assert_eq!(refused["code"], "invalid.workflowInWorkSession");
        assert_eq!((profile.clone(), session), before);
        // Ticked: deleted, the setting cleared, the keys kept.
        let ticked = DeleteConfirmation {
            fingerprint,
            clear_bindings: false,
            clear_work_session: true,
        };
        let done = delete_in(&mut profile, id, Some(&ticked), &mut session).unwrap();
        assert_eq!(
            done,
            DeletedView {
                cleared: 0,
                cleared_work_session: 1
            }
        );
        assert_eq!(session, WorkSessionWorkflows::default());
        assert_eq!(profile.layouts[0].pages[0].bindings.len(), 2);
    }

    #[test]
    fn deleting_a_referenced_workflow_lists_every_reference_first() {
        let (mut profile, id) = referenced();
        let before = profile.clone();
        let refused = payload(
            &delete_in(&mut profile, id, None, &mut WorkSessionWorkflows::default()).unwrap_err(),
        );
        assert_eq!(refused["code"], "invalid.workflowInUse");
        assert_eq!(refused["params"]["count"], 3);
        let usages = refused["params"]["usages"].as_array().unwrap();
        let kinds: Vec<&str> = usages.iter().map(|u| u["type"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["layoutBinding", "layoutBinding", "actionParameter"]);
        assert_eq!(usages[0]["trigger"]["type"], "press");
        assert_eq!(
            refused["params"]["fingerprint"],
            profile.workflow_fingerprint(id).to_string()
        );
        assert_eq!(profile, before);
    }

    #[test]
    fn a_confirmed_delete_is_guarded_and_clears_keys_only_when_asked() {
        // Kept keys: they show "Workflow missing".
        let (mut profile, id) = referenced();
        let fingerprint = profile.workflow_fingerprint(id).to_string();
        let done = delete_in(
            &mut profile,
            id,
            Some(&confirm(&fingerprint, false)),
            &mut WorkSessionWorkflows::default(),
        )
        .unwrap();
        assert_eq!(
            done,
            DeletedView {
                cleared: 0,
                cleared_work_session: 0
            }
        );
        assert!(profile.workflow(id).is_none());
        assert_eq!(profile.layouts[0].pages[0].bindings.len(), 2);
        assert_eq!(profile.actions.len(), 1);

        // Cleared keys; the Switch's parameter is never edited.
        let (mut profile, id) = referenced();
        let fingerprint = profile.workflow_fingerprint(id).to_string();
        let parameters = profile.actions[0].parameters.clone();
        let done = delete_in(
            &mut profile,
            id,
            Some(&confirm(&fingerprint, true)),
            &mut WorkSessionWorkflows::default(),
        )
        .unwrap();
        assert_eq!(
            done,
            DeletedView {
                cleared: 2,
                cleared_work_session: 0
            }
        );
        assert_eq!(profile.layouts[0].pages[0].bindings, Vec::new());
        assert_eq!(profile.actions[0].parameters, parameters);

        // The references changed between the preview and the confirmation.
        let (mut profile, id) = referenced();
        let fingerprint = profile.workflow_fingerprint(id).to_string();
        profile.layouts[0].pages[0]
            .bindings
            .push(ControlBinding::new(
                ControlId::Key { index: 7 },
                Trigger::LongPress,
                BindingTarget::Workflow { workflow: id },
            ));
        let before = profile.clone();
        let refused = payload(
            &delete_in(
                &mut profile,
                id,
                Some(&confirm(&fingerprint, true)),
                &mut WorkSessionWorkflows::default(),
            )
            .unwrap_err(),
        );
        assert_eq!(refused["code"], "invalid.workflowReferencesChanged");
        assert_eq!(refused["params"]["count"], 4);
        assert_eq!(
            refused["params"]["fingerprint"],
            profile.workflow_fingerprint(id).to_string()
        );
        assert_eq!(profile, before);
    }

    #[test]
    fn an_unreferenced_workflow_is_deleted_as_before() {
        let mut profile = Profile::new("Work");
        let alone = Workflow::new("Alone");
        let id = alone.id;
        profile.workflows.push(alone);
        assert_eq!(
            delete_in(
                &mut profile.clone(),
                id,
                None,
                &mut WorkSessionWorkflows::default()
            )
            .unwrap(),
            DeletedView {
                cleared: 0,
                cleared_work_session: 0
            }
        );
        let empty = ReferenceFingerprint::of(&[]).to_string();
        assert_eq!(
            delete_in(
                &mut profile,
                id,
                Some(&confirm(&empty, true)),
                &mut WorkSessionWorkflows::default()
            )
            .unwrap(),
            DeletedView {
                cleared: 0,
                cleared_work_session: 0
            }
        );
        assert_eq!(
            payload(
                &delete_in(&mut profile, id, None, &mut WorkSessionWorkflows::default())
                    .unwrap_err()
            )["code"],
            "notFound.workflow"
        );
    }

    #[test]
    fn the_confirmation_reads_as_the_app_sends_it() {
        let confirm: DeleteConfirmation = serde_json::from_value(
            serde_json::json!({ "fingerprint": "f", "clearBindings": true }),
        )
        .unwrap();
        assert!(confirm.clear_bindings);
        let confirm: DeleteConfirmation =
            serde_json::from_value(serde_json::json!({ "fingerprint": "f" })).unwrap();
        assert!(!confirm.clear_bindings);
        assert!(!confirm.clear_work_session);
    }

    fn workflow() -> Workflow {
        Workflow::new("Steps").step(ActionInstanceId::new(), OnFailure::Stop)
    }

    /// Without issues, a save answers exactly what it did before
    /// `contextIssues` existed: the workflow and `agentReload`, flattened.
    #[test]
    fn a_saved_workflow_without_issues_serializes_as_before() {
        let workflow = workflow();
        let before = serde_json::to_value(Saved {
            value: workflow.clone(),
            agent_reload: AgentReload::Applied,
        })
        .unwrap();
        let now = serde_json::to_value(Saved {
            value: SavedWorkflow {
                workflow: workflow.clone(),
                context_issues: Vec::new(),
                incomplete: Vec::new(),
            },
            agent_reload: AgentReload::Applied,
        })
        .unwrap();
        assert_eq!(now, before);
        assert!(now.get("contextIssues").is_none());

        // With one, only the field is added.
        let action = workflow.steps[0].action;
        let mut reported = serde_json::to_value(Saved {
            value: SavedWorkflow {
                workflow,
                context_issues: vec![ContextIssue {
                    site: ContextSite::Step { index: 0 },
                    action,
                    reason: IncompatibilityReason::ContextUnsupported {
                        context: ActionContext::Sequence,
                        supported: vec![ActionContext::Key, ActionContext::Gesture],
                    },
                    stored: false,
                }],
                incomplete: Vec::new(),
            },
            agent_reload: AgentReload::Applied,
        })
        .unwrap();
        let issues = reported.as_object_mut().unwrap().remove("contextIssues");
        assert_eq!(reported, before);
        assert_eq!(
            issues,
            Some(serde_json::json!([{
                "site": { "kind": "step", "index": 0 },
                "action": action,
                "reason": {
                    "type": "contextUnsupported",
                    "context": "sequence",
                    "supported": ["key", "gesture"],
                },
                "stored": false,
            }]))
        );
    }
}
