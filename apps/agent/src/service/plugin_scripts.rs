//! The agent's side of the Scripts bridge's store (plugin API 1.13,
//! `docs/scripts-bridge.md`): the [`ScriptHost`] a run of
//! `deckpilot.scripts` reads its script, its grants and the projects from.
//!
//! The executor runs on a plugin's lane thread and never touches the
//! store; it asks the agent's thread ([`AgentScripts`]), which answers from
//! the store (`deckpilot_storage::scripts`) and the runtime's active
//! profile:
//!
//! * Only the **active profile**'s scripts and projects, never another
//!   profile's; nothing while DeckPilot is locked.
//! * A script runs only while it is **trusted** (its source is the one the
//!   user trusted): otherwise `scriptNeedsReview`, and no grants. A script
//!   the profile does not have is `scriptMissing`.
//! * The grants are read again at every call, so a change on the
//!   Permissions tab (or a revoked trust) applies to the next call.
//! * Repositories carry their folder only as the executor needs it (never
//!   handed to a script without `repositories.paths`) and their remotes
//!   with the link to a connection and the provider's repository id (K4:
//!   script objects only, never text variables).

#![allow(
    clippy::result_large_err,
    reason = "the refusal is the ScriptHost's ScriptFault, as in the executor"
)]

use std::path::PathBuf;
use std::sync::mpsc::sync_channel;
use std::time::Duration;

use tokio::sync::mpsc::UnboundedSender;

use deckpilot_core::script_grants::ScriptGrants;
use deckpilot_core::{MachineConfig, ProfileId, Project, ScriptId};
use deckpilot_plugin_abi::scripts::{ScriptFault, ScriptSettings, codes};
use deckpilot_storage::SqliteStore;
use deckpilot_storage::scripts::ScriptForRun;

use super::{Agent, Command, SecurityState};
use crate::plugins::script_host::canonical;
use crate::plugins::script_host::{
    Refusal, RunProject, RunRemote, RunRepository, ScriptHost, StoredScript,
};

/// How long a lane thread waits for the agent's answer.
const ANSWER_WAIT: Duration = Duration::from_secs(2);

/// What a run asks the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ScriptQuery {
    Grants(String),
    Load(String),
    Projects,
    Project(Option<String>),
}

/// The agent's answer.
#[derive(Debug)]
pub(crate) enum ScriptAnswer {
    Grants(Option<ScriptGrants>),
    Load(Result<StoredScript, Refusal>),
    Projects(Result<Vec<RunProject>, Refusal>),
    Project(Result<Option<RunProject>, Refusal>),
}

/// [`ScriptHost`] through the agent's command channel, answered on the
/// agent's thread. Called on a plugin's lane thread, never on the agent's.
#[derive(Debug, Clone)]
pub(crate) struct AgentScripts(UnboundedSender<Command>);

impl AgentScripts {
    pub fn new(commands: UnboundedSender<Command>) -> Self {
        Self(commands)
    }

    fn ask(&self, query: ScriptQuery) -> Option<ScriptAnswer> {
        let (reply, answer) = sync_channel(1);
        self.0.send(Command::PluginScript { query, reply }).ok()?;
        answer.recv_timeout(ANSWER_WAIT).ok()
    }
}

fn unavailable() -> Refusal {
    ScriptFault::new(codes::UNAVAILABLE, "the agent is not answering")
}

impl ScriptHost for AgentScripts {
    fn grants(&self, script: &str) -> Option<ScriptGrants> {
        match self.ask(ScriptQuery::Grants(script.to_owned()))? {
            ScriptAnswer::Grants(grants) => grants,
            _ => None,
        }
    }

    fn load(&self, script: &str) -> Result<StoredScript, Refusal> {
        match self.ask(ScriptQuery::Load(script.to_owned())) {
            Some(ScriptAnswer::Load(answer)) => answer,
            _ => Err(unavailable()),
        }
    }

    fn projects(&self) -> Result<Vec<RunProject>, Refusal> {
        match self.ask(ScriptQuery::Projects) {
            Some(ScriptAnswer::Projects(answer)) => answer,
            _ => Err(unavailable()),
        }
    }

    fn project(&self, id: Option<&str>) -> Result<Option<RunProject>, Refusal> {
        match self.ask(ScriptQuery::Project(id.map(str::to_owned))) {
            Some(ScriptAnswer::Project(answer)) => answer,
            _ => Err(unavailable()),
        }
    }
}

fn missing() -> Refusal {
    ScriptFault::new(
        codes::SCRIPT_MISSING,
        "the active profile has no such script",
    )
}

impl Agent {
    /// A run's question about its script or the projects, on the agent's
    /// thread.
    pub(super) fn plugin_script(&mut self, query: ScriptQuery) -> ScriptAnswer {
        match query {
            ScriptQuery::Grants(id) => ScriptAnswer::Grants(grants_of(self.run_view(&id))),
            ScriptQuery::Load(id) => ScriptAnswer::Load(load_of(self.run_view(&id))),
            ScriptQuery::Projects => ScriptAnswer::Projects(self.run_projects()),
            ScriptQuery::Project(id) => ScriptAnswer::Project(self.run_project(id.as_deref())),
        }
    }

    /// The active profile's script `id`, as a run may see it.
    fn run_view(&self, id: &str) -> Result<ScriptForRun, Refusal> {
        if self.security != SecurityState::Unlocked {
            return Err(ScriptFault::new(codes::UNAVAILABLE, "DeckPilot is locked"));
        }
        let profile = self.runtime.profile().ok_or_else(missing)?.id;
        let store = self.store().map_err(|_| unavailable())?;
        view_in(store, profile, id)
    }

    fn run_projects(&self) -> Result<Vec<RunProject>, Refusal> {
        if self.security != SecurityState::Unlocked {
            return Err(ScriptFault::new(codes::UNAVAILABLE, "DeckPilot is locked"));
        }
        let Some(profile) = self.runtime.profile() else {
            return Ok(Vec::new());
        };
        if !self.project_context_active() {
            return Ok(Vec::new());
        }
        Ok(profile
            .projects
            .iter()
            .map(|p| run_project_of(p, self.runtime.machine()))
            .collect())
    }

    fn run_project(&self, id: Option<&str>) -> Result<Option<RunProject>, Refusal> {
        if self.security != SecurityState::Unlocked {
            return Err(ScriptFault::new(codes::UNAVAILABLE, "DeckPilot is locked"));
        }
        let Some(profile) = self.runtime.profile() else {
            return Ok(None);
        };
        if !self.project_context_active() {
            return Ok(None);
        }
        let project = match id {
            // Never another profile's: an id the active profile does not
            // have is no project.
            Some(id) => profile.projects.iter().find(|p| p.id.to_string() == id),
            None => self
                .runtime
                .active_project_id()
                .and_then(|id| profile.project(id)),
        };
        Ok(project.map(|p| run_project_of(p, self.runtime.machine())))
    }
}

/// A project as a run sees it: its repositories, each with its folder
/// here (canonical, an existing folder, or none) and its remotes.
fn run_project_of(project: &Project, machine: &MachineConfig) -> RunProject {
    let default = project.default_repository().map(|r| r.id);
    RunProject {
        id: project.id.to_string(),
        key: project.key.as_str().to_owned(),
        name: project.name.clone(),
        repositories: project
            .effective_repositories()
            .iter()
            .map(|r| RunRepository {
                id: r.id.to_string(),
                key: r.key.as_str().to_owned(),
                name: r.name.clone(),
                description: r.description.clone(),
                is_default: Some(r.id) == default,
                folder: machine
                    .binding(&r.resource)
                    .filter(|b| b.kind == deckpilot_core::ResourceKind::Directory)
                    .and_then(|b| canonical(&PathBuf::from(&b.value)).ok())
                    .filter(|p| p.is_dir()),
                remote: r.primary_remote().map(|remote| {
                    deckpilot_plugin_abi::scripts::RepositoryRemote {
                        url: remote.remote_url.clone(),
                        default_branch: remote.default_branch.clone(),
                    }
                }),
                remotes: r
                    .remotes
                    .iter()
                    .map(|remote| RunRemote {
                        name: remote.name.clone(),
                        primary: remote.primary,
                        url: remote.remote_url.clone(),
                        default_branch: remote.default_branch.clone(),
                        connection_id: remote.connection_id.clone(),
                        remote_repository_id: remote.remote_repository_id.clone(),
                    })
                    .collect(),
            })
            .collect(),
    }
}

/// Script `id` of `profile` in `store`, as a run may see it.
fn view_in(store: &SqliteStore, profile: ProfileId, id: &str) -> Result<ScriptForRun, Refusal> {
    let id: ScriptId = id.parse().map_err(|_| missing())?;
    store
        .script_for_run(profile, id)
        .map_err(|_| ScriptFault::new(codes::HOST_ERROR, "the script store failed"))?
        .ok_or_else(missing)
}

/// The grants of a run: only a trusted script's.
fn grants_of(view: Result<ScriptForRun, Refusal>) -> Option<ScriptGrants> {
    match view {
        Ok(ScriptForRun::Runnable { grants, .. }) => Some(*grants),
        _ => None,
    }
}

/// What `script.load` gets: a trusted script's source and settings, or
/// why not.
fn load_of(view: Result<ScriptForRun, Refusal>) -> Result<StoredScript, Refusal> {
    match view? {
        ScriptForRun::Runnable { script, .. } => Ok(StoredScript {
            source: script.source,
            settings: ScriptSettings {
                timeout_ms: u64::from(script.settings.timeout_ms),
                stop_on_error: script.settings.stop_on_error,
                memory_mib: script.settings.memory_mib,
            },
        }),
        ScriptForRun::NeedsReview => Err(ScriptFault::new(
            codes::SCRIPT_NEEDS_REVIEW,
            "the script is not trusted: review it on the Scripts page",
        )),
    }
}

#[cfg(test)]
mod tests;
