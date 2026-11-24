//! Commands of the projects module: managing the projects of a profile,
//! switching the active project and checking project resources.
//!
//! Every command names the profile it works on, and every project id is
//! looked up inside that profile, so a command can never change a project of
//! another profile. Copying a project to another profile is an explicit
//! command that creates an independent copy.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use tauri::State;

use deckpilot_core::{
    ActiveProjectContext, CycleDirection, MachineResourceBinding, ProfileId, Project,
    ProjectCatalog, ProjectId, ProjectKey, ProjectSwitcher, ResourceKey, ResourceKind,
};
use deckpilot_ipc::{ClientError, Request, Response};
use deckpilot_storage::ConfigStore;

use crate::agent;
use crate::commands::agent::unexpected;
use crate::commands::modify_profile;
use crate::error::{CommandError, CommandResult};
use crate::state::{AppState, PROJECTS_PLUGIN, load_profile, machine_config};

/// Changes a profile's projects after checking that the Projects feature
/// is on ([`AppState::require_feature`]).
fn modify_projects<T>(
    state: &AppState,
    profile_id: ProfileId,
    change: impl FnOnce(&mut deckpilot_core::Profile) -> CommandResult<T>,
) -> CommandResult<T> {
    state.require_feature(PROJECTS_PLUGIN)?;
    modify_profile(state, profile_id, change)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInput {
    key: String,
    name: String,
    #[serde(default)]
    resources: BTreeMap<String, String>,
}

fn build_project(id: ProjectId, input: ProjectInput) -> CommandResult<Project> {
    let key = ProjectKey::new(input.key.trim())?;
    let name = match input.name.trim() {
        "" => key.to_string(),
        name => name.to_owned(),
    };
    let mut project = Project {
        id,
        key,
        name,
        resources: BTreeMap::new(),
    };
    for (role, resource) in input.resources {
        let role = role.trim();
        if role.is_empty() {
            return Err(CommandError::invalid(
                "resourceRoleEmpty",
                "resource role must not be empty",
            ));
        }
        project
            .resources
            .insert(role.to_owned(), ResourceKey::new(resource.trim())?);
    }
    Ok(project)
}

#[tauri::command]
pub fn create_project(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    project: ProjectInput,
) -> CommandResult<Project> {
    let project = build_project(ProjectId::new(), project)?;
    modify_projects(&state, profile_id, |profile| {
        profile.add_project(project.clone())?;
        Ok(project)
    })
}

#[tauri::command]
pub fn update_project(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    project_id: ProjectId,
    project: ProjectInput,
) -> CommandResult<Project> {
    let project = build_project(project_id, project)?;
    modify_projects(&state, profile_id, |profile| {
        profile.update_project(project.clone())?;
        Ok(project)
    })
}

/// Moves a project within the cycle order.
#[tauri::command]
pub fn move_project(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    project_id: ProjectId,
    index: usize,
) -> CommandResult<()> {
    modify_projects(&state, profile_id, |profile| {
        Ok(profile.move_project(project_id, index)?)
    })
}

#[tauri::command]
pub fn delete_project(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    project_id: ProjectId,
) -> CommandResult<()> {
    modify_projects(&state, profile_id, |profile| {
        profile.delete_project(project_id)?;
        Ok(())
    })
}

#[tauri::command]
pub fn set_default_project(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    project_id: Option<ProjectId>,
) -> CommandResult<()> {
    modify_projects(&state, profile_id, |profile| {
        Ok(profile.set_default_project(project_id)?)
    })
}

/// Copies a project into another profile. The copy gets a new id and keeps
/// key, name and resource roles; nothing is shared afterwards.
#[tauri::command]
pub fn copy_project(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    project_id: ProjectId,
    target_profile_id: ProfileId,
) -> CommandResult<Project> {
    state.require_feature(PROJECTS_PLUGIN)?;
    let source = load_profile(&*state.store()?, profile_id)?;
    modify_profile(&state, target_profile_id, |target| {
        let id = source.copy_project_to(project_id, target)?;
        target
            .project(id)
            .cloned()
            .ok_or(CommandError::NotFound("project"))
    })
}

/// Switching acts on the active profile, which is what the agent runs.
fn ensure_active(state: &AppState, profile_id: ProfileId) -> CommandResult<()> {
    state.require_feature(PROJECTS_PLUGIN)?;
    if state.store()?.active_profile()? == Some(profile_id) {
        Ok(())
    } else {
        Err(CommandError::invalid(
            "profileNotActive",
            "the profile is not active",
        ))
    }
}

#[tauri::command]
pub async fn cycle_project(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    direction: CycleDirection,
) -> CommandResult<Option<ProjectId>> {
    ensure_active(&state, profile_id)?;
    match agent::request(&state.paths, Request::CycleProject { direction }).await {
        Ok(Response::ActiveProject { project }) => Ok(project.map(|p| p.id)),
        Ok(other) => Err(unexpected(&other)),
        Err(ClientError::NotRunning) => {
            // The Projects plugin's `wrapAround` lives in the agent's
            // plugin store: without the agent, its default (on).
            let store = state.store()?;
            let profile = load_profile(&store, profile_id)?;
            let context =
                ActiveProjectContext::restore(&profile, store.active_project(profile_id)?);
            let active =
                ProjectSwitcher::step(&profile.projects, context.project_id(), direction, true)
                    .map_err(|e| CommandError::Invalid(e.to_string()))?;
            store.set_active_project(profile_id, Some(active))?;
            Ok(Some(active))
        }
        Err(error) => Err(error.into()),
    }
}

#[tauri::command]
pub async fn select_project(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    project_id: ProjectId,
) -> CommandResult<()> {
    ensure_active(&state, profile_id)?;
    // Ownership is checked here as well as in the agent.
    let profile = load_profile(&*state.store()?, profile_id)?;
    if profile.project(project_id).is_none() {
        return Err(CommandError::NotFound("project"));
    }
    match agent::request(
        &state.paths,
        Request::SelectProject {
            project: project_id,
        },
    )
    .await
    {
        Ok(_) => Ok(()),
        Err(ClientError::NotRunning) => Ok(state
            .store()?
            .set_active_project(profile_id, Some(project_id))?),
        Err(error) => Err(error.into()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ResourceStatus {
    /// Bound and found on this machine.
    Ok,
    /// Bound, but the path does not exist or the URL is malformed.
    Missing,
    /// Used by a project, but not bound on this machine.
    Unbound,
    /// Bound to something DeckPilot cannot check (services, containers).
    Unchecked,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceCheck {
    key: ResourceKey,
    kind: Option<ResourceKind>,
    value: Option<String>,
    status: ResourceStatus,
    /// `PROJECT role` pairs of the profile that use this resource.
    used_by: Vec<String>,
}

/// Whether a binding's value is still there. An application or terminal is
/// checked the way Open Application resolves it (`deckpilot_apps::presence`):
/// a macOS bundle id, a Linux desktop-file id or a Windows App Paths name
/// the pickers store is not a path. A file chosen with the picker (kind
/// `other`) is checked when its value is an absolute path; other values of
/// that kind, and network paths, are not checked.
pub(crate) fn status_of(binding: &MachineResourceBinding) -> ResourceStatus {
    let value = binding.value.trim();
    let path = std::path::Path::new(value);
    let network = value.starts_with("\\\\") || value.starts_with("//");
    match binding.kind {
        ResourceKind::Directory if path.is_dir() => ResourceStatus::Ok,
        ResourceKind::Application | ResourceKind::Terminal if network => ResourceStatus::Unchecked,
        ResourceKind::Application | ResourceKind::Terminal => {
            match deckpilot_apps::presence(value) {
                Some(true) => ResourceStatus::Ok,
                Some(false) => ResourceStatus::Missing,
                None => ResourceStatus::Unchecked,
            }
        }
        ResourceKind::Url if value.starts_with("https://") || value.starts_with("http://") => {
            ResourceStatus::Ok
        }
        ResourceKind::Directory | ResourceKind::Url => ResourceStatus::Missing,
        ResourceKind::Other if path.is_absolute() && !network => {
            if path.exists() {
                ResourceStatus::Ok
            } else {
                ResourceStatus::Missing
            }
        }
        ResourceKind::DockerEnvironment | ResourceKind::Service | ResourceKind::Other => {
            ResourceStatus::Unchecked
        }
    }
}

/// Checks every machine binding and every resource the profile's projects
/// refer to. Not gated on the Projects feature: the bindings are core
/// (Settings › Resources, the `system.open-*` launchers); the projects'
/// own resources are listed only as what uses a binding.
#[tauri::command]
pub fn check_resources(
    state: State<'_, AppState>,
    profile_id: ProfileId,
) -> CommandResult<Vec<ResourceCheck>> {
    let store = state.store()?;
    let profile = load_profile(&store, profile_id)?;
    let config = machine_config(&store)?;
    let mut used: BTreeMap<ResourceKey, Vec<String>> = BTreeMap::new();
    for project in &profile.projects {
        for (role, key) in &project.resources {
            used.entry(key.clone())
                .or_default()
                .push(format!("{} {role}", project.key));
        }
    }
    let keys: BTreeSet<ResourceKey> = config
        .bindings
        .iter()
        .map(|b| b.key.clone())
        .chain(used.keys().cloned())
        .collect();
    Ok(keys
        .into_iter()
        .map(|key| {
            let binding = config.binding(&key);
            ResourceCheck {
                kind: binding.map(|b| b.kind),
                value: binding.map(|b| b.value.clone()),
                status: binding.map_or(ResourceStatus::Unbound, status_of),
                used_by: used.remove(&key).unwrap_or_default(),
                key,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(kind: ResourceKind, value: &str) -> MachineResourceBinding {
        MachineResourceBinding {
            key: ResourceKey::new("picker.test").unwrap(),
            kind,
            value: value.to_owned(),
        }
    }

    #[test]
    fn picked_files_and_folders_are_checked_where_they_were_chosen() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("report.pdf");
        std::fs::write(&file, b"%PDF").unwrap();
        let gone = dir.path().join("moved.pdf");
        let file = file.to_string_lossy();
        let gone = gone.to_string_lossy();
        let folder = dir.path().to_string_lossy();
        let status = |kind, value: &str| status_of(&binding(kind, value));
        assert_eq!(status(ResourceKind::Other, &file), ResourceStatus::Ok);
        assert_eq!(status(ResourceKind::Other, &gone), ResourceStatus::Missing);
        assert_eq!(status(ResourceKind::Directory, &folder), ResourceStatus::Ok);
        assert_eq!(
            status(ResourceKind::Directory, &gone),
            ResourceStatus::Missing
        );
        // Not a path, or a network path: not checked.
        assert_eq!(
            status(ResourceKind::Other, "token-42"),
            ResourceStatus::Unchecked
        );
        assert_eq!(
            status(ResourceKind::Other, "//server/share/a.txt"),
            ResourceStatus::Unchecked
        );
        assert_eq!(
            status(ResourceKind::Other, r"\\server\share\a.txt"),
            ResourceStatus::Unchecked
        );
    }

    #[test]
    fn applications_are_checked_as_open_application_resolves_them() {
        let dir = tempfile::tempdir().unwrap();
        let program = std::env::current_exe().unwrap();
        let gone = dir.path().join("Gone").join("app");
        let status = |value: &str| status_of(&binding(ResourceKind::Application, value));
        assert_eq!(status(&program.to_string_lossy()), ResourceStatus::Ok);
        assert_eq!(status(&gone.to_string_lossy()), ResourceStatus::Missing);
        assert_eq!(
            status_of(&binding(ResourceKind::Terminal, &gone.to_string_lossy())),
            ResourceStatus::Missing
        );
        assert_eq!(status(r"\\server\share\app.exe"), ResourceStatus::Unchecked);
        #[cfg(target_os = "macos")]
        {
            // What the picker stores: a bundle id, never a path.
            assert_eq!(status("com.apple.finder"), ResourceStatus::Ok);
            assert_eq!(
                status("io.github.knittingcode.deckpilot.test-no-such-app"),
                ResourceStatus::Missing
            );
        }
    }
}
