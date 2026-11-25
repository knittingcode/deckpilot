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
    ProjectCatalog, ProjectId, ProjectKey, ProjectSwitcher, Repository, RepositoryId,
    RepositoryKey, ResourceKey, ResourceKind,
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
    /// The project's repositories, in order. Left out: the stored ones are
    /// kept as they are (an older editor, or a save of other fields).
    #[serde(default)]
    repositories: Option<Vec<RepositoryInput>>,
    /// The default repository; read only together with `repositories`.
    /// Left out: the stored default is kept (when it is still listed);
    /// `null`: no default.
    #[serde(default, deserialize_with = "some")]
    #[allow(clippy::option_option)] // Left out, `null` and an id differ.
    default_repository: Option<Option<RepositoryId>>,
}

/// `Some` for a field that is present, also when it is `null`, so that a
/// left-out field (`None`) can be told apart from `null` (`Some(None)`).
fn some<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

/// A repository as the editor sends it. Its remotes are not part of it:
/// only the app writes them (Git Providers, after the user confirms), and
/// every save keeps them.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryInput {
    /// Left out for a new repository, which gets a new id.
    #[serde(default)]
    id: Option<RepositoryId>,
    key: String,
    name: String,
    /// The resource key whose binding holds the folder. Left out for a new
    /// repository: one is made from the project and repository keys that no
    /// project or binding uses yet.
    #[serde(default)]
    resource: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

fn build_project(id: ProjectId, input: ProjectInput) -> CommandResult<Project> {
    let key = ProjectKey::new(input.key.trim())?;
    let name = match input.name.trim() {
        "" => key.to_string(),
        name => name.to_owned(),
    };
    let mut project = Project::new(key, name);
    project.id = id;
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

/// Fills in `project`'s repositories from `input`, or keeps those of
/// `stored` when the input has none (and the stored default). Remotes are
/// kept by repository id. A repository without a resource gets a new key
/// that `taken` does not hold. When the default changes to a repository,
/// the `repository` role follows (R2).
#[allow(clippy::option_option)] // `default`: left out, `null` or an id.
fn apply_repositories(
    project: &mut Project,
    stored: Option<&Project>,
    repositories: Option<Vec<RepositoryInput>>,
    default: Option<Option<RepositoryId>>,
    taken: &BTreeSet<ResourceKey>,
) -> CommandResult<()> {
    let Some(inputs) = repositories else {
        if let Some(stored) = stored {
            // The repository the migration stored for the role, never
            // edited, is derived again from the project as saved now (its
            // name, its role), so it never goes stale and exports as before.
            if stored.has_only_derived_repository() {
                return Ok(());
            }
            project.repositories.clone_from(&stored.repositories);
            project.default_repository = stored.default_repository;
        }
        return Ok(());
    };
    // No repository while the role exists: the role would bring one back
    // (R3); the role has to go first.
    if inputs.is_empty()
        && project
            .resources
            .contains_key(deckpilot_core::project::REPOSITORY_ROLE)
    {
        let shown = stored.map(Project::with_derived_repositories);
        if shown.is_some_and(|p| !p.repositories.is_empty()) {
            return Err(deckpilot_core::CatalogError::from(
                deckpilot_core::RepositoryError::LastWithRole,
            )
            .into());
        }
    }
    // What the editor showed: the stored repositories, or the derived one.
    let shown = stored.map(Project::with_derived_repositories);
    let mut used = taken.clone();
    used.extend(inputs.iter().filter_map(|r| {
        r.resource
            .as_deref()
            .and_then(|k| ResourceKey::new(k.trim()).ok())
    }));
    let mut built = Vec::with_capacity(inputs.len());
    for input in inputs {
        let key =
            RepositoryKey::new(input.key.trim()).map_err(deckpilot_core::CatalogError::from)?;
        let id = input.id.unwrap_or_default();
        let resource = match input.resource.as_deref().map(str::trim) {
            Some(resource) if !resource.is_empty() => ResourceKey::new(resource)?,
            _ => {
                let resource = project.new_repository_resource_key(&key, |k| used.contains(k));
                used.insert(resource.clone());
                resource
            }
        };
        let name = match input.name.trim() {
            "" => key.to_string(),
            name => name.to_owned(),
        };
        let description = input
            .description
            .map(|d| d.trim().to_owned())
            .filter(|d| !d.is_empty());
        let remotes = shown
            .as_ref()
            .and_then(|p| p.repositories.iter().find(|r| r.id == id))
            .map(|r| r.remotes.clone())
            .unwrap_or_default();
        built.push(Repository {
            id,
            key,
            name,
            resource,
            description,
            remotes,
        });
    }
    project.repositories = built;
    project.default_repository = None;
    project
        .validate_repositories()
        .map_err(deckpilot_core::CatalogError::from)?;
    let before = shown.as_ref().and_then(|p| p.default_repository);
    // Left out: the stored default, while it is still listed.
    let default = default
        .unwrap_or_else(|| before.filter(|id| project.repositories.iter().any(|r| r.id == *id)));
    if default.is_some_and(|id| !project.repositories.iter().any(|r| r.id == id)) {
        return Err(deckpilot_core::CatalogError::from(
            deckpilot_core::RepositoryError::UnknownDefault,
        )
        .into());
    }
    if default.is_some() && default != before {
        project
            .set_default_repository(default)
            .map_err(deckpilot_core::CatalogError::from)?;
    } else {
        project.default_repository = default;
    }
    Ok(())
}

/// Resource keys in use by the profile's projects (roles and repositories)
/// and this computer's bindings: a new repository never gets one of them.
fn taken_resource_keys(
    state: &AppState,
    profile: &deckpilot_core::Profile,
) -> CommandResult<BTreeSet<ResourceKey>> {
    let config = machine_config(&*state.store()?)?;
    // (The guard above is released at the end of that statement.)
    let mut taken: BTreeSet<ResourceKey> = config.bindings.iter().map(|b| b.key.clone()).collect();
    for project in &profile.projects {
        taken.extend(project.resources.values().cloned());
        taken.extend(
            project
                .effective_repositories()
                .iter()
                .map(|r| r.resource.clone()),
        );
    }
    Ok(taken)
}

#[tauri::command]
pub fn create_project(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    project: ProjectInput,
) -> CommandResult<Project> {
    state.require_feature(PROJECTS_PLUGIN)?;
    let profile = load_profile(&*state.store()?, profile_id)?;
    let taken = taken_resource_keys(&state, &profile)?;
    let mut input = project;
    let repositories = input.repositories.take();
    let default = input.default_repository;
    let mut project = build_project(ProjectId::new(), input)?;
    apply_repositories(&mut project, None, repositories, default, &taken)?;
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
    state.require_feature(PROJECTS_PLUGIN)?;
    let mut input = project;
    let repositories = input.repositories.take();
    let default = input.default_repository;
    let project = build_project(project_id, input)?;
    // The store's guard is released before `taken_resource_keys` takes it.
    let profile = load_profile(&*state.store()?, profile_id)?;
    let taken = taken_resource_keys(&state, &profile)?;
    modify_projects(&state, profile_id, |profile| {
        let mut project = project;
        let stored = profile.project(project_id).cloned();
        apply_repositories(&mut project, stored.as_ref(), repositories, default, &taken)?;
        profile.update_project(project.clone())?;
        Ok(project)
    })
}

/// Marks a repository as the project's default, or clears the default
/// (`None`). Marking one also points the `repository` role at its
/// resource, adding the role when it is missing, in the same save (R2);
/// clearing leaves the role as it is.
#[tauri::command]
pub fn set_default_repository(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    project_id: ProjectId,
    repository_id: Option<RepositoryId>,
) -> CommandResult<Project> {
    modify_projects(&state, profile_id, |profile| {
        let mut project = profile
            .project(project_id)
            .cloned()
            .ok_or(CommandError::NotFound("project"))?;
        project
            .set_default_repository(repository_id)
            .map_err(deckpilot_core::CatalogError::from)?;
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
/// and their repositories refer to. A missing folder is reported, never
/// removed. Not gated on the Projects feature: the bindings are core
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
        // Repositories, the derived one too: `PROJECT repositories.KEY`.
        for repository in project.effective_repositories().iter() {
            let entry = used.entry(repository.resource.clone()).or_default();
            let name = format!("{} repositories.{}", project.key, repository.key);
            if !entry.contains(&name) {
                entry.push(name);
            }
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
