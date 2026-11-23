use tauri::State;

use serde::Serialize;

use deckpilot_core::graph::{Conversion, GraphIssue, from_steps};
use deckpilot_core::{ContextIssue, ProfileId, Workflow, WorkflowId};

use super::reload::Saved;
use super::{modify_profile, modify_profile_and_reload};
use crate::error::{CommandError, CommandResult};
use crate::state::{AppState, WORKFLOWS_PLUGIN, load_profile};

/// Workflow management belongs to the Workflows plugin
/// (`deckpilot.workflows`, decisions D1/D6 of 2026-11-20) and is refused
/// while it is off, except for step workflows without a graph: those are
/// the key editor's Multi Actions, a core feature that keeps working with
/// the plugin off. Keys that run workflows run them either way (the
/// engine stays in the core), and no workflow is ever deleted because the
/// plugin is off.
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
        match profile.workflows.iter_mut().find(|w| w.id == workflow.id) {
            Some(existing) => *existing = workflow.clone(),
            None => profile.workflows.push(workflow.clone()),
        }
        Ok(SavedWorkflow {
            workflow,
            context_issues,
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

/// Deletes a workflow that no layout uses: a graph workflow only while
/// the Workflows plugin is on, a step workflow (Multi Action) always.
#[tauri::command]
pub fn delete_workflow(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    workflow_id: WorkflowId,
) -> CommandResult<()> {
    modify_profile(&state, profile_id, |profile| {
        require_workflows(&state, profile.workflow(workflow_id))?;
        profile.delete_workflow(workflow_id)?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use deckpilot_core::{
        ActionContext, ActionInstanceId, ContextIssue, ContextSite, IncompatibilityReason,
        OnFailure,
    };

    use super::*;
    use crate::commands::reload::AgentReload;

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
