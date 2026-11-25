//! A project repository's remotes (decisions K1–K3 of the Git Providers
//! design): the git remotes it has, each optionally linked to a Git
//! Providers connection on this computer (`connectionId`) and to the
//! provider's immutable repository id (`gh:…`, `gl:…`).
//!
//! **Only the app writes the link** (K3), here, after the user confirmed it
//! in the app; a plugin never writes profiles. The app takes a
//! `remoteRepositoryId` only from the plugin's own repository records
//! (`list-repositories`, `get-repository`, `resolve-remote`), never from
//! text the user typed. The core checks every remote (K1: at most 8, unique
//! names, at most one primary; K2: prefixed ids only, URLs without a
//! password, a query or a fragment).

use deckpilot_core::{
    CatalogError, ProfileId, ProjectId, Repository, RepositoryId, RepositoryRemote,
};
use tauri::State;

use super::modify_profile;
use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

/// Whether `id` is a Git Providers connection id: `cn_` and 26 characters
/// of Crockford base32.
fn is_connection_id(id: &str) -> bool {
    id.strip_prefix("cn_").is_some_and(|body| {
        body.len() == 26
            && body
                .bytes()
                .all(|b| b.is_ascii_digit() || (b.is_ascii_uppercase() && !b"ILOU".contains(&b)))
    })
}

/// Replaces the remotes of one repository of a project. A project shown
/// with its derived repository (the `repository` role, R3) stores it first,
/// so the link is kept with it. Answers the repository as saved.
pub(crate) fn set_remotes(
    profile: &mut deckpilot_core::Profile,
    project_id: ProjectId,
    repository_id: RepositoryId,
    remotes: Vec<RepositoryRemote>,
) -> CommandResult<Repository> {
    if let Some(remote) = remotes.iter().find(|r| {
        r.connection_id
            .as_deref()
            .is_some_and(|id| !is_connection_id(id))
    }) {
        return Err(CommandError::invalid(
            "repositoryRemoteConnection",
            format!(
                "remote `{}` names a connection that is not a connection id",
                remote.name
            ),
        ));
    }
    let project = profile
        .projects
        .iter_mut()
        .find(|p| p.id == project_id)
        .ok_or(CommandError::NotFound("project"))?;
    project.materialize_repositories();
    let repository = project
        .repositories
        .iter_mut()
        .find(|r| r.id == repository_id)
        .ok_or(CommandError::NotFound("repository"))?;
    let mut changed = repository.clone();
    changed.remotes = remotes;
    changed
        .check_remotes()
        .map_err(|e| CommandError::from(CatalogError::Repository(e)))?;
    *repository = changed.clone();
    project
        .validate_repositories()
        .map_err(|e| CommandError::from(CatalogError::Repository(e)))?;
    Ok(changed)
}

/// `setRepositoryRemotes` (K3): the link the user confirmed.
#[tauri::command]
pub fn set_repository_remotes(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    project_id: ProjectId,
    repository_id: RepositoryId,
    remotes: Vec<RepositoryRemote>,
) -> CommandResult<Repository> {
    modify_profile(&state, profile_id, |profile| {
        set_remotes(profile, project_id, repository_id, remotes)
    })
}

#[cfg(test)]
mod tests {
    use deckpilot_core::{Profile, Project, ProjectKey, ResourceKey};

    use super::*;

    fn remote(name: &str) -> RepositoryRemote {
        RepositoryRemote {
            name: name.into(),
            primary: false,
            connection_id: Some("cn_01JD7Q0000000000000000000A".into()),
            remote_repository_id: Some("gl:4711".into()),
            remote_url: Some("https://code.example.com/group/app.git".into()),
            default_branch: Some("main".into()),
        }
    }

    fn profile() -> (Profile, ProjectId, RepositoryId) {
        let mut profile = Profile::new("P");
        let mut project = Project::new(ProjectKey::new("SHOP").unwrap(), "Shop");
        project
            .resources
            .insert("repository".into(), ResourceKey::new("shop-repo").unwrap());
        let repository = project.with_derived_repositories().repositories[0].id;
        let id = project.id;
        profile.projects.push(project);
        (profile, id, repository)
    }

    /// The derived repository is stored with its link; the link is
    /// exactly what the app confirmed.
    #[test]
    fn a_confirmed_link_is_stored_with_the_repository() {
        let (mut p, project, repository) = profile();
        let mut origin = remote("origin");
        origin.primary = true;
        let saved = set_remotes(&mut p, project, repository, vec![origin.clone()]).unwrap();
        assert_eq!(saved.remotes, vec![origin]);
        assert_eq!(p.projects[0].repositories.len(), 1);
        assert_eq!(p.projects[0].repositories[0].remotes.len(), 1);
        // Unlinking: no remotes.
        set_remotes(&mut p, project, repository, Vec::new()).unwrap();
        assert_eq!(p.projects[0].repositories[0].remotes, Vec::new());
    }

    /// K1/K2 and the connection id: names are never ids, ids are prefixed,
    /// URLs carry no password, one primary, unique names, a connection id
    /// only; nothing is saved then.
    #[test]
    fn invalid_links_are_refused_and_nothing_changes() {
        let (mut p, project, repository) = profile();
        let mut by_name = remote("origin");
        by_name.remote_repository_id = Some("group/app".into());
        let mut with_password = remote("origin");
        with_password.remote_url = Some("https://user:pw@code.example.com/group/app.git".into());
        let mut bad_connection = remote("origin");
        bad_connection.connection_id = Some("github".into());
        let mut first = remote("origin");
        first.primary = true;
        let mut second = remote("upstream");
        second.primary = true;
        for remotes in [
            vec![by_name],
            vec![with_password],
            vec![bad_connection],
            vec![first, second],
            vec![remote("origin"), remote("origin")],
            (0..9).map(|i| remote(&format!("r{i}"))).collect(),
        ] {
            let before = p.clone();
            assert!(set_remotes(&mut p, project, repository, remotes).is_err());
            assert_eq!(
                p.projects[0]
                    .repositories
                    .iter()
                    .map(|r| r.remotes.len())
                    .sum::<usize>(),
                before.projects[0]
                    .repositories
                    .iter()
                    .map(|r| r.remotes.len())
                    .sum::<usize>()
            );
        }
        assert!(set_remotes(&mut p, ProjectId::new(), repository, Vec::new()).is_err());
        assert!(set_remotes(&mut p, project, RepositoryId::new(), Vec::new()).is_err());
    }
}
