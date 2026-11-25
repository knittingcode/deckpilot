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
use deckpilot_ipc::connections::is_connection_id;
use deckpilot_ipc::{Request, Response};
use tauri::State;

use super::modify_profile;
use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

fn refused(message: impl Into<String>) -> CommandError {
    CommandError::invalid("repositoryRemoteConnection", message)
}

/// Replaces the remotes of one repository of a project. Everything is
/// checked on a copy first, so a refused change leaves `profile` exactly
/// as it was. A project shown with its derived repository (the
/// `repository` role, R3) stores it first, so the link is kept with it.
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
        return Err(refused(format!(
            "remote `{}` names a connection that is not a connection id",
            remote.name
        )));
    }
    let index = profile
        .projects
        .iter()
        .position(|p| p.id == project_id)
        .ok_or(CommandError::NotFound("project"))?;
    let mut candidate = profile.projects[index].clone();
    let repository = |e| CommandError::from(CatalogError::Repository(e));
    candidate
        .set_repository_remotes(repository_id, remotes)
        .map_err(repository)?;
    candidate.validate_repositories().map_err(repository)?;
    let saved = candidate
        .repositories
        .iter()
        .find(|r| r.id == repository_id)
        .cloned()
        .ok_or(CommandError::NotFound("repository"))?;
    profile.projects[index] = candidate;
    Ok(saved)
}

/// `remotes` with `remote` added, or replacing the remote of its name; as
/// the primary one it makes no other primary.
pub(crate) fn merged(
    remotes: &[RepositoryRemote],
    remote: RepositoryRemote,
) -> Vec<RepositoryRemote> {
    let mut out: Vec<RepositoryRemote> = remotes
        .iter()
        .filter(|r| r.name != remote.name)
        .cloned()
        .map(|mut r| {
            if remote.primary {
                r.primary = false;
            }
            r
        })
        .collect();
    out.push(remote);
    out
}

/// The connection type a provider id names (`gh:` GitHub, `gl:` GitLab,
/// `bb:` Bitbucket). A convention shared with the Git Providers plugin,
/// whose connection type ids are `github`, `gitlab` (and later
/// `bitbucket`); the K2 prefixes are the core's.
fn provider_type(remote_repository_id: &str) -> Option<&'static str> {
    match remote_repository_id.split_once(':')?.0 {
        "gh" => Some("github"),
        "gl" => Some("gitlab"),
        "bb" => Some("bitbucket"),
        _ => None,
    }
}

/// Whether every remote that names a link (a connection or a provider id)
/// is exactly a link stored under that name now: the editor may keep, drop
/// or reorder links and edit plain remotes, but a new or changed link goes
/// through [`link_repository_remote`], which checks it (K3: ids only from
/// the plugin's records).
fn links_unchanged(stored: &[RepositoryRemote], remotes: &[RepositoryRemote]) -> bool {
    remotes
        .iter()
        .filter(|r| r.connection_id.is_some() || r.remote_repository_id.is_some())
        .all(|r| {
            stored.iter().any(|s| {
                s.name == r.name
                    && s.connection_id == r.connection_id
                    && s.remote_repository_id == r.remote_repository_id
                    && s.remote_url == r.remote_url
            })
        })
}

/// `setRepositoryRemotes` (K3) for the repository editor: the remotes as
/// the user saved them. Links may only be kept, dropped or reordered
/// ([`links_unchanged`]), so a link made on another computer ("connection
/// not on this computer") can be saved again unchanged, and new links go
/// through [`link_repository_remote`].
#[tauri::command]
pub fn set_repository_remotes(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    project_id: ProjectId,
    repository_id: RepositoryId,
    remotes: Vec<RepositoryRemote>,
) -> CommandResult<Repository> {
    modify_profile(&state, profile_id, |profile| {
        let stored = stored_remotes(profile, project_id, repository_id)?;
        if !links_unchanged(&stored, &remotes) {
            return Err(refused(
                "a new or changed link is made with Link remote, which checks it",
            ));
        }
        set_remotes(profile, project_id, repository_id, remotes)
    })
}

/// The remotes stored now (with the derived repository, R3).
fn stored_remotes(
    profile: &deckpilot_core::Profile,
    project_id: ProjectId,
    repository_id: RepositoryId,
) -> CommandResult<Vec<RepositoryRemote>> {
    Ok(profile
        .projects
        .iter()
        .find(|p| p.id == project_id)
        .ok_or(CommandError::NotFound("project"))?
        .with_derived_repositories()
        .repositories
        .into_iter()
        .find(|r| r.id == repository_id)
        .ok_or(CommandError::NotFound("repository"))?
        .remotes)
}

/// Links one remote the user confirmed (K3): it is added, or replaces the
/// remote of its name, merged with the remotes stored now (never a copy the
/// dialog held). Its connection must be one of `plugin`'s connections on
/// this computer, turned on, with a token, of the provider its id names.
#[tauri::command]
pub async fn link_repository_remote(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    project_id: ProjectId,
    repository_id: RepositoryId,
    plugin: String,
    remote: RepositoryRemote,
) -> CommandResult<Repository> {
    let (Some(connection), Some(id)) = (&remote.connection_id, &remote.remote_repository_id) else {
        return Err(refused("a link names a connection and the provider's id"));
    };
    let answer = super::plugins::plugin_request(
        &state,
        Request::ListPluginConnections {
            plugin: plugin.clone(),
        },
    )
    .await
    .map_err(|e| refused(e.message))?;
    let Response::PluginConnections { connections } = answer else {
        return Err(refused("the agent did not list the connections"));
    };
    let known = connections.connections.iter().find(|c| &c.id == connection);
    match known {
        None => return Err(refused("the connection is not on this computer")),
        Some(c) if !c.enabled || !c.has_secret => {
            return Err(refused("the connection is turned off or has no token"));
        }
        Some(c) if provider_type(id) != Some(c.type_id.as_str()) => {
            return Err(refused(
                "the repository is not of the connection's provider",
            ));
        }
        Some(_) => {}
    }
    modify_profile(&state, profile_id, |profile| {
        let current = stored_remotes(profile, project_id, repository_id)?;
        set_remotes(profile, project_id, repository_id, merged(&current, remote))
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
        let before = p.clone();
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
        assert_eq!(p, before);
    }

    /// The editor keeps, drops or reorders links; it never makes one.
    #[test]
    fn the_editor_never_makes_a_link() {
        let stored = vec![remote("origin"), remote("upstream")];
        assert!(links_unchanged(
            &stored,
            &[remote("upstream"), remote("origin")]
        ));
        assert!(links_unchanged(&stored, &[remote("origin")]));
        let plain = RepositoryRemote {
            name: "mirror".into(),
            primary: false,
            connection_id: None,
            remote_repository_id: None,
            remote_url: Some("https://mirror.example.com/app.git".into()),
            default_branch: None,
        };
        assert!(links_unchanged(&stored, &[plain]));
        let mut other_id = remote("origin");
        other_id.remote_repository_id = Some("gl:9".into());
        let mut other_connection = remote("origin");
        other_connection.connection_id = Some("cn_01JD7Q0000000000000000000C".into());
        for changed in [other_id, other_connection, remote("new")] {
            assert!(!links_unchanged(&stored, &[changed]));
        }
    }

    /// One link replaces the remote of its name and keeps one primary.
    #[test]
    fn a_link_merges_into_the_stored_remotes() {
        let mut origin = remote("origin");
        origin.primary = true;
        let upstream = remote("upstream");
        let mut fork = remote("fork");
        fork.primary = true;
        let out = merged(&[origin.clone(), upstream.clone()], fork.clone());
        assert_eq!(out.iter().filter(|r| r.primary).count(), 1);
        assert_eq!(out.last(), Some(&fork));
        let mut again = remote("origin");
        again.remote_repository_id = Some("gl:9".into());
        let out = merged(&[origin, upstream], again.clone());
        assert_eq!(out.len(), 2);
        assert!(out.contains(&again));
        assert_eq!(provider_type("gl:4711"), Some("gitlab"));
        assert_eq!(provider_type("gh:1"), Some("github"));
        assert_eq!(provider_type("group/app"), None);
    }
}
