//! The run's own operations: `script.load`, `script.poll`, `script.log`
//! and `script.sleep`.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use deckpilot_core::script_grants::{FsRoot, ScriptGrants};
use deckpilot_plugin_abi::capability::LogLevel;
use deckpilot_plugin_abi::scripts::{
    Platform, ScriptFault, ScriptLoad, ScriptLoaded, ScriptLog, ScriptPolled, ScriptRunInfo,
    ScriptSleep, ScriptSlept, codes,
};
use deckpilot_plugin_broker::PluginId;
use serde_json::{Value, json};
use tracing::{debug, info, warn};

use super::{Call, Notes, fault, now_us, to_value};

/// Lines a run's log keeps, and bytes (design §5.4); then one marker.
pub(super) const LOG_LINES: usize = 1_000;
pub(super) const LOG_BYTES: usize = 256 << 10;
/// The marker after the last kept line.
pub(super) const LOG_TRUNCATED: &str = "…truncated (the log limit of this run was reached)";
/// How often a sleep looks at the run's interrupt.
const SLEEP_SLICE: Duration = Duration::from_millis(25);

/// The run's log: the first [`LOG_LINES`] lines or [`LOG_BYTES`] bytes,
/// then one marker. Kept for the run only (the editor shows the last
/// run's log; P7 streams it to the app).
#[derive(Debug, Default)]
pub(super) struct LogBuffer {
    pub lines: VecDeque<(LogLevel, String)>,
    pub bytes: usize,
    pub truncated: bool,
}

impl LogBuffer {
    /// Keeps `message`; `false` when the limit was reached (the marker is
    /// kept once instead).
    pub fn push(&mut self, level: LogLevel, message: &str) -> bool {
        if self.truncated {
            return false;
        }
        if self.lines.len() >= LOG_LINES || self.bytes + message.len() > LOG_BYTES {
            self.truncated = true;
            self.lines
                .push_back((LogLevel::Warn, LOG_TRUNCATED.to_owned()));
            return false;
        }
        self.bytes += message.len();
        self.lines.push_back((level, message.to_owned()));
        true
    }
}

fn platform() -> Platform {
    if cfg!(windows) {
        Platform::Windows
    } else if cfg!(target_os = "macos") {
        Platform::Macos
    } else {
        Platform::Linux
    }
}

/// What the guest may expose (`docs/scripts-bridge.md`): never trusted
/// for a decision, and never a path (a directory scope by its label, an
/// absolute command by its file name).
pub(super) fn summary(grants: &ScriptGrants) -> Value {
    let fs: Vec<Value> = grants
        .fs
        .iter()
        .map(|scope| {
            let root = match &scope.root {
                FsRoot::Repository(key) => json!({ "repository": key }),
                FsRoot::Directory { label, .. } => json!({ "directory": label }),
            };
            json!({ "root": root, "access": scope.access })
        })
        .collect();
    let process: Vec<Value> = grants
        .process
        .iter()
        .map(|p| {
            let name = std::path::Path::new(&p.command)
                .file_name()
                .map_or_else(|| p.command.clone(), |n| n.to_string_lossy().into_owned());
            json!({ "command": name })
        })
        .collect();
    json!({
        "projectsRead": grants.projects_read,
        "repositoriesPaths": grants.repositories_paths,
        "fs": fs,
        "process": process,
        "shell": grants.shell.as_ref().map(|s| s.shells.clone()),
        "destructive": grants.destructive,
    })
}

/// `script.load`: the invocation's own script, from the active profile,
/// trusted; once per run.
pub(super) fn load(
    call: &Call<'_>,
    load: &ScriptLoad,
    notes: &mut Notes,
) -> Result<Value, ScriptFault> {
    notes.add("script", call.script);
    if load.script != call.script {
        return Err(fault(
            codes::NOT_GRANTED,
            "a run loads only the script its action names",
        ));
    }
    if call.run.state().loaded.is_some() {
        return Err(fault(
            codes::INVALID_ARGUMENT,
            "the script is already loaded",
        ));
    }
    call.alive()?;
    let stored = call.run.host.load(call.script)?;
    let mut settings = stored.settings;
    settings.timeout_ms = settings
        .timeout_ms
        .clamp(1, deckpilot_core::script_grants::MAX_TIMEOUT_MS);
    {
        let mut state = call.run.state();
        state.deadline = Some(call.run.started + Duration::from_millis(settings.timeout_ms));
        state.loaded = Some(stored.clone());
    }
    to_value(&ScriptLoaded {
        source: stored.source,
        platform: platform(),
        settings,
        grants: summary(&call.grants),
        run: ScriptRunInfo {
            id: call.run.id.clone(),
            origin: call.origin.to_owned(),
            user_initiated: call.user_initiated,
        },
        now_us: now_us(),
    })
}

/// `script.poll`: whether the run was stopped, and its time left.
pub(super) fn poll(call: &Call<'_>) -> Result<Value, ScriptFault> {
    let cancelled = call
        .interrupt
        .is_some_and(deckpilot_plugin_supervisor::CallInterrupt::is_aborted);
    to_value(&ScriptPolled {
        cancelled,
        remaining_ms: u64::try_from(call.remaining().as_millis()).unwrap_or(u64::MAX),
        now_us: now_us(),
    })
}

/// Text fit for the agent's log: control characters (but tabs) shown as
/// escapes, so a script cannot forge lines or colours.
pub(super) fn loggable(text: &str) -> String {
    text.chars()
        .flat_map(|c| {
            if c.is_control() && c != '\t' {
                c.escape_default().collect::<Vec<_>>()
            } else {
                vec![c]
            }
        })
        .collect()
}

/// `script.log`: one line into the run's log and the agent's.
pub(super) fn log(call: &Call<'_>, plugin: &PluginId, log: &ScriptLog) -> Value {
    let kept = call.run.state().log.push(log.level, &log.message);
    if kept {
        let message = loggable(&log.message);
        let (plugin, run) = (plugin.as_str(), call.run.id.as_str());
        match log.level {
            LogLevel::Error | LogLevel::Warn => warn!(plugin, run, "script: {message}"),
            LogLevel::Info => info!(plugin, run, "script: {message}"),
            LogLevel::Debug => debug!(plugin, run, "script: {message}"),
        }
    }
    Value::Null
}

/// `script.sleep`: waits in the agent, at most the run's time left;
/// a Stop ends it at once.
pub(super) fn sleep(call: &Call<'_>, sleep: ScriptSleep) -> Result<Value, ScriptFault> {
    let wanted = Duration::from_millis(sleep.ms);
    let left = call.remaining();
    let until = Instant::now() + wanted.min(left);
    loop {
        call.alive()?;
        let now = Instant::now();
        if now >= until {
            break;
        }
        std::thread::sleep(SLEEP_SLICE.min(until - now));
    }
    if wanted > left {
        return Err(fault(
            codes::TIMED_OUT,
            "the script ran out of time while it slept",
        ));
    }
    to_value(&ScriptSlept { now_us: now_us() })
}
