//! `project.list` and `project.repositories` (grant `projects.read`; a
//! repository's folder only with `repositories.paths`). The active
//! profile only, from the [`ScriptHost`](super::ScriptHost).

use deckpilot_plugin_abi::privileged::{ProjectInfo, ProjectList};
use deckpilot_plugin_abi::scripts::{
    ProjectRepositories, ProjectRepositoriesReply, Repository, ScriptFault, codes,
};
use serde_json::Value;

use super::{Call, RunProject, fault, to_value};

fn granted(call: &Call<'_>) -> Result<(), ScriptFault> {
    if call.grants.projects_read {
        Ok(())
    } else {
        Err(fault(
            codes::NOT_GRANTED,
            "the script may not read projects (grant: projects.read)",
        ))
    }
}

fn info(project: &RunProject) -> ProjectInfo {
    ProjectInfo {
        id: project.id.clone(),
        key: project.key.clone(),
        name: project.name.clone(),
    }
}

pub(super) fn list(call: &Call<'_>) -> Result<Value, ScriptFault> {
    granted(call)?;
    let projects = call.run.host.projects()?;
    to_value(&ProjectList {
        projects: projects.iter().map(info).collect(),
    })
}

pub(super) fn repositories(
    call: &Call<'_>,
    request: &ProjectRepositories,
) -> Result<Value, ScriptFault> {
    granted(call)?;
    let paths = call.grants.repositories_paths;
    let project = match request.project.as_deref() {
        Some(id) => call.run.host.project(Some(id))?,
        None => call.project()?,
    };
    let Some(project) = project else {
        return to_value(&ProjectRepositoriesReply {
            project: None,
            repositories: Vec::new(),
        });
    };
    let repositories = project
        .repositories
        .iter()
        .map(|r| Repository {
            id: r.id.clone(),
            key: r.key.clone(),
            name: r.name.clone(),
            description: r.description.clone(),
            is_default: r.is_default,
            path: r
                .folder
                .as_ref()
                .filter(|_| paths)
                .map(|p| p.to_string_lossy().into_owned()),
            exists: r.folder.is_some(),
            remote: r.remote.clone(),
        })
        .collect();
    to_value(&ProjectRepositoriesReply {
        project: Some(info(&project)),
        repositories,
    })
}
