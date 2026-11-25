//! The Scripts host bridge, the agent's side (plugin API 1.13,
//! `docs/scripts-bridge.md`, `design-projects-scripts.md` §4.5, §5).
//!
//! The broker allowed the call for `deckpilot.scripts` within its
//! `scripts` ceiling ([`ScriptCall`]). Here the agent decides it again,
//! before anything happens:
//!
//! 1. the plugin is `deckpilot.scripts` and the invocation carries a
//!    script run ([`ScriptRun`], the invocation's own `script` parameter);
//! 2. the script's **current** machine-local grants
//!    ([`ScriptHost::grants`]), read at every call, so a revocation applies
//!    to the next one; a script that is not trusted has none;
//! 3. the run's origin (whether the user started it) and its time;
//! 4. the operation's own rules: the file system's scopes, protected
//!    roots and confirmation token (`fs/`, `confirm.rs`).
//!
//! Every call but `script.poll` and `script.log` is audited
//! ([`PrivilegedRecord`]): the operation, the scope's label, the outcome
//! and counts, never a path, a file's content or an environment value.
//!
//! Where the scripts, their trust and their grants come from is the
//! [`ScriptHost`]. Until the Scripts page and its storage land (P7), the
//! agent's is [`NoScripts`]: no script exists, so every run fails with
//! `scriptMissing` and every other operation is refused.

#![allow(
    clippy::result_large_err,
    reason = "a ScriptFault is the reply's script detail, built once per refused call; boxing it everywhere only adds noise"
)]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use deckpilot_core::script_grants::ScriptGrants;
use deckpilot_plugin_abi::scripts::{RepositoryRemote, ScriptFault, ScriptSettings, codes};
use deckpilot_plugin_abi::{CallErrorCode, CallResult, CapabilityRequest};
use deckpilot_plugin_broker::{PluginId, SCRIPTS_PLUGIN, ScriptCall};
use deckpilot_plugin_supervisor::CallInterrupt;
use serde_json::Value;

use super::{Executor, PrivilegedRecord};

mod confirm;
pub(crate) mod fs;
mod projects;
mod run;
#[cfg(test)]
mod tests;

pub(crate) use fs::protected::Protected;

/// A script as the store holds it, for `script.load`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredScript {
    pub source: String,
    pub settings: ScriptSettings,
}

/// A project of the active profile, as a script run sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunProject {
    pub id: String,
    pub key: String,
    pub name: String,
    pub repositories: Vec<RunRepository>,
}

/// A repository of a [`RunProject`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunRepository {
    pub id: String,
    pub key: String,
    pub name: String,
    pub description: Option<String>,
    pub is_default: bool,
    /// Its folder on this computer: canonical, an existing directory, or
    /// `None` (not bound here, or missing). Never handed to a script
    /// without `repositories.paths`.
    pub folder: Option<PathBuf>,
    pub remote: Option<RepositoryRemote>,
}

/// Why the host refused (a script `ErrorCode` and a message).
pub(crate) type Refusal = ScriptFault;

/// Where scripts, their trust and their grants come from (the agent's
/// script store; tests use their own).
pub(crate) trait ScriptHost: Send + Sync + std::fmt::Debug {
    /// The script's machine-local grants **now**: `None` when it does not
    /// exist, is not trusted (its source does not match the reviewed
    /// hash) or has none. Read at every call.
    fn grants(&self, script: &str) -> Option<ScriptGrants>;
    /// The script of the active profile, trusted: `scriptMissing` or
    /// `scriptNeedsReview` otherwise.
    fn load(&self, script: &str) -> Result<StoredScript, Refusal>;
    /// The active profile's projects (id, key, name).
    fn projects(&self) -> Result<Vec<RunProject>, Refusal>;
    /// A project of the active profile with its repositories (`None`: the
    /// active project); `Ok(None)` when there is none.
    fn project(&self, id: Option<&str>) -> Result<Option<RunProject>, Refusal>;
}

/// No script store yet (P7): nothing loads, nothing is granted.
#[derive(Debug, Default)]
pub(crate) struct NoScripts;

impl ScriptHost for NoScripts {
    fn grants(&self, _script: &str) -> Option<ScriptGrants> {
        None
    }

    fn load(&self, _script: &str) -> Result<StoredScript, Refusal> {
        Err(ScriptFault::new(
            codes::SCRIPT_MISSING,
            "this version of DeckPilot keeps no scripts yet",
        ))
    }

    fn projects(&self) -> Result<Vec<RunProject>, Refusal> {
        Ok(Vec::new())
    }

    fn project(&self, _id: Option<&str>) -> Result<Option<RunProject>, Refusal> {
        Ok(None)
    }
}

/// The script run of one invocation of `deckpilot.scripts`.
pub(crate) struct ScriptRun {
    host: Arc<dyn ScriptHost>,
    /// The invocation's own `script` parameter: the only script this run
    /// may load and whose grants it uses.
    script: Option<String>,
    /// A random id for the run (logs and `deckpilot.run.id`).
    id: String,
    started: Instant,
    protected: Arc<Protected>,
    state: Mutex<RunState>,
    /// Tests: what `script.confirm`'s dialog answers.
    #[cfg(test)]
    answer: Mutex<Option<bool>>,
}

impl std::fmt::Debug for ScriptRun {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptRun")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// What a run keeps between its calls.
#[derive(Default)]
struct RunState {
    loaded: Option<StoredScript>,
    /// The run's project, pinned by `script.load`: switching the active
    /// project during the run changes nothing for it (security review of
    /// #469, L5).
    project: Option<String>,
    /// When the run's time is up (from the script's `timeoutMs`, never past
    /// the invocation's deadline); set by `script.load`.
    deadline: Option<Instant>,
    tokens: confirm::Tokens,
    writes: fs::Writes,
    /// Bytes `fs.copy` copied in this run.
    copied: u64,
    log: run::LogBuffer,
}

impl ScriptRun {
    pub fn new(host: Arc<dyn ScriptHost>, script: Option<String>) -> Self {
        Self::with_protected(host, script, Arc::new(Protected::system()))
    }

    pub fn with_protected(
        host: Arc<dyn ScriptHost>,
        script: Option<String>,
        protected: Arc<Protected>,
    ) -> Self {
        Self {
            host,
            script,
            id: random_hex(16).unwrap_or_else(|| "run".into()),
            started: Instant::now(),
            protected,
            state: Mutex::new(RunState::default()),
            #[cfg(test)]
            answer: Mutex::new(None),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, RunState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Drop for ScriptRun {
    /// The run ended: open chunked writes are abandoned (their temporary
    /// files removed) and every confirmation token is dropped.
    fn drop(&mut self) {
        let state = self.state.get_mut().unwrap_or_else(|e| e.into_inner());
        state.writes.abort_all();
        state.tokens.clear();
    }
}

/// `n` random bytes as hex.
pub(super) fn random_hex(n: usize) -> Option<String> {
    use std::fmt::Write as _;
    let mut bytes = vec![0u8; n];
    getrandom::fill(&mut bytes).ok()?;
    Some(bytes.iter().fold(String::with_capacity(n * 2), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    }))
}

/// The agent's wall clock in µs since the Unix epoch (`nowUs`).
pub(super) fn now_us() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_micros()).unwrap_or(u64::MAX))
}

pub(super) fn fault(code: &str, message: impl Into<String>) -> ScriptFault {
    ScriptFault::new(code, message)
}

/// The call error a script fault travels with.
fn call_code(code: &str) -> CallErrorCode {
    match code {
        codes::INVALID_ARGUMENT | codes::TOO_LARGE => CallErrorCode::BadRequest,
        codes::LIMIT => CallErrorCode::Throttled,
        codes::UNAVAILABLE => CallErrorCode::Unavailable,
        codes::NOT_GRANTED
        | codes::OUTSIDE_SCOPE
        | codes::PROTECTED_PATH
        | codes::SYMLINK_ESCAPE
        | codes::NEEDS_CONFIRMATION
        | codes::NOT_USER_INITIATED
        | codes::SCRIPT_NEEDS_REVIEW => CallErrorCode::NotPermitted,
        _ => CallErrorCode::Failed,
    }
}

fn reply(result: Result<Value, ScriptFault>) -> CallResult {
    match result {
        Ok(value) => CallResult::Ok(value),
        Err(fault) => CallResult::script_error(call_code(&fault.code), fault),
    }
}

pub(super) fn to_value<T: serde::Serialize>(value: &T) -> Result<Value, ScriptFault> {
    serde_json::to_value(value).map_err(|e| fault(codes::HOST_ERROR, e.to_string()))
}

/// What an audit record of a bridge call says besides its outcome.
#[derive(Default)]
pub(super) struct Notes(pub Vec<(&'static str, String)>);

impl Notes {
    pub fn add(&mut self, name: &'static str, value: impl Into<String>) {
        self.0.push((name, value.into()));
    }
}

/// The audit's grant key of a call.
fn grant_key(request: &CapabilityRequest) -> &'static str {
    use deckpilot_plugin_abi::scripts::ScriptScope;
    match request.script_scope() {
        Some(ScriptScope::Fs) => "scripts:fs",
        Some(ScriptScope::Process) => "scripts:process",
        Some(ScriptScope::Shell) => "scripts:shell",
        Some(ScriptScope::Projects) | None => "scripts:projects",
        Some(ScriptScope::Run) => "scripts",
    }
}

/// One call's context: the run, its grants as read now, the interrupt.
pub(super) struct Call<'a> {
    pub run: &'a ScriptRun,
    pub script: &'a str,
    pub grants: ScriptGrants,
    pub user_initiated: bool,
    pub origin: &'static str,
    pub interrupt: Option<&'a CallInterrupt>,
}

impl Call<'_> {
    /// The run's time left, or the fault when it ran out or was stopped.
    pub fn alive(&self) -> Result<Duration, ScriptFault> {
        if self.interrupt.is_some_and(CallInterrupt::is_aborted) {
            return Err(fault(codes::CANCELLED, "the run was stopped"));
        }
        let left = self.remaining();
        if left.is_zero() {
            return Err(fault(codes::TIMED_OUT, "the script ran out of time"));
        }
        Ok(left)
    }

    /// The run's project (pinned at `script.load`) with its repositories.
    pub fn project(&self) -> Result<Option<RunProject>, ScriptFault> {
        let pinned = self.run.state().project.clone();
        let project = match pinned {
            Some(id) => self.run.host.project(Some(&id))?,
            None => None,
        };
        // One path form for every comparison (no `\\?\` on Windows).
        Ok(project.map(|mut p| {
            for r in &mut p.repositories {
                r.folder = r.folder.take().map(fs::protected::simplify);
            }
            p
        }))
    }

    /// Time left until the run's deadline (or the invocation's).
    pub fn remaining(&self) -> Duration {
        let now = Instant::now();
        let mut until = self.interrupt.map(CallInterrupt::deadline);
        if let Some(deadline) = self.run.state().deadline {
            until = Some(until.map_or(deadline, |d| d.min(deadline)));
        }
        until.map_or(Duration::from_secs(600), |d| {
            d.saturating_duration_since(now)
        })
    }
}

/// `deckpilot.run.origin` for the run's trigger.
fn origin(run: &deckpilot_plugin_abi::capability::RunContext) -> &'static str {
    use deckpilot_plugin_abi::capability::{RunCause, RunTrigger};
    match run.trigger {
        RunTrigger::Key => "key",
        RunTrigger::Gesture => "gesture",
        RunTrigger::App | RunTrigger::Test => "editor",
        RunTrigger::Workflow if run.cause == Some(RunCause::User) => "workflow",
        RunTrigger::Workflow | RunTrigger::Other => "trigger",
    }
}

/// The executor of an invocation of `plugin` with `params`: for
/// `deckpilot.scripts`, with the run of the script its `script` parameter
/// names (from the agent's store, [`NoScripts`] until P7); every other
/// plugin's unchanged.
pub(crate) fn attach(
    executor: Executor,
    plugin: &str,
    params: &deckpilot_plugin_abi::Params,
) -> Executor {
    if plugin != SCRIPTS_PLUGIN {
        return executor;
    }
    let script = match params.get("script") {
        Some(deckpilot_plugin_abi::ParamValue::String(id)) => Some(id.clone()),
        _ => None,
    };
    executor.with_scripts(Arc::new(NoScripts), script)
}

impl Executor {
    /// A script run for this invocation of `deckpilot.scripts`, with the
    /// invocation's own `script` parameter.
    #[must_use]
    pub fn with_scripts(mut self, host: Arc<dyn ScriptHost>, script: Option<String>) -> Self {
        self.scripts = Some(ScriptRun::new(host, script));
        self
    }

    /// Tests: a run whose protected roots are the test's.
    #[cfg(test)]
    #[must_use]
    pub fn with_script_run(mut self, run: ScriptRun) -> Self {
        self.scripts = Some(run);
        self
    }

    /// A Scripts bridge call the broker allowed.
    pub(super) fn script(
        &self,
        plugin: &PluginId,
        call: &ScriptCall,
        interrupt: Option<&CallInterrupt>,
    ) -> CallResult {
        let started = Instant::now();
        let request = &call.request;
        let quiet = matches!(
            request,
            CapabilityRequest::ScriptPoll(_) | CapabilityRequest::ScriptLog(_)
        );
        let mut notes = Notes::default();
        let result = self.script_call(plugin, request, interrupt, &mut notes);
        if !quiet {
            if let Some(run) = &self.scripts {
                notes.0.insert(0, ("run", run.id.clone()));
            }
            self.audit_privileged(&PrivilegedRecord {
                plugin: plugin.to_string(),
                op: request.op(),
                grant: grant_key(request),
                outcome: result.as_ref().err().map(|f| static_code(&f.code)),
                elapsed: started.elapsed(),
                details: notes.0,
            });
        }
        reply(result)
    }

    fn script_call(
        &self,
        plugin: &PluginId,
        request: &CapabilityRequest,
        interrupt: Option<&CallInterrupt>,
        notes: &mut Notes,
    ) -> Result<Value, ScriptFault> {
        if plugin.as_str() != SCRIPTS_PLUGIN {
            return Err(fault(
                codes::NOT_GRANTED,
                "only DeckPilot's Scripts plugin runs scripts",
            ));
        }
        let Some(run) = &self.scripts else {
            return Err(fault(codes::UNAVAILABLE, "this invocation runs no script"));
        };
        let Some(script) = run.script.as_deref() else {
            return Err(fault(codes::SCRIPT_MISSING, "the action names no script"));
        };
        // Read now: a revoked or no longer trusted script has none.
        let grants = run
            .host
            .grants(script)
            .filter(|g| g.validate().is_ok())
            .unwrap_or_default();
        let call = Call {
            run,
            script,
            grants,
            user_initiated: self.run.is_user_initiated(),
            origin: origin(&self.run),
            interrupt,
        };
        if let CapabilityRequest::ScriptLoad(load) = request {
            return run::load(&call, load, notes);
        }
        // Everything else needs the run's own script loaded first.
        if run.state().loaded.is_none() {
            return Err(fault(codes::INVALID_ARGUMENT, "script.load comes first"));
        }
        if let CapabilityRequest::ScriptPoll(_) = request {
            return run::poll(&call);
        }
        call.alive()?;
        match request {
            CapabilityRequest::ScriptLog(log) => Ok(run::log(&call, plugin, log)),
            CapabilityRequest::ScriptSleep(sleep) => run::sleep(&call, *sleep),
            CapabilityRequest::ScriptConfirm(confirm) => {
                self.script_confirm(plugin, &call, confirm, notes)
            }
            CapabilityRequest::ProjectList(_) => projects::list(&call),
            CapabilityRequest::ProjectRepositories(r) => projects::repositories(&call, r),
            CapabilityRequest::ProcessRun(_)
            | CapabilityRequest::ShellRun(_)
            | CapabilityRequest::ShellAvailable(_) => Err(fault(
                codes::UNAVAILABLE,
                "processes are not available to scripts in this version",
            )),
            other
                if other.script_scope() == Some(deckpilot_plugin_abi::scripts::ScriptScope::Fs) =>
            {
                fs::call(&call, other, notes)
            }
            other => Err(fault(
                codes::INVALID_ARGUMENT,
                format!("{} is not an operation of the Scripts bridge", other.op()),
            )),
        }
    }
}

/// The audit's outcome code (a `&'static str`) of a script code.
fn static_code(code: &str) -> &'static str {
    const ALL: &[&str] = &[
        codes::NOT_GRANTED,
        codes::OUTSIDE_SCOPE,
        codes::PROTECTED_PATH,
        codes::SYMLINK_ESCAPE,
        codes::NOT_FOUND,
        codes::EXISTS,
        codes::TOO_LARGE,
        codes::COMMAND_NOT_FOUND,
        codes::GIT_UNAVAILABLE,
        codes::DOCKER_UNAVAILABLE,
        codes::NON_ZERO_EXIT,
        codes::TIMED_OUT,
        codes::CANCELLED,
        codes::NEEDS_CONFIRMATION,
        codes::NOT_USER_INITIATED,
        codes::LIMIT,
        codes::INVALID_ARGUMENT,
        codes::NO_PROJECT,
        codes::SCRIPT_MISSING,
        codes::SCRIPT_NEEDS_REVIEW,
        codes::UNAVAILABLE,
    ];
    ALL.iter()
        .find(|c| **c == code)
        .copied()
        .unwrap_or(codes::HOST_ERROR)
}
