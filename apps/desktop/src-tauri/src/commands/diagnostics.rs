//! Export diagnostics: a support bundle the user saves to a file.
//!
//! The bundle comes from the agent executable of this app's own bundle,
//! `deckpilot-agent diagnose --bundle` (`apps/agent/src/diagnose/bundle.rs`):
//! the diagnose report and the end of the recent agent and update logs,
//! through one redaction. That command is read-only, starts and opens
//! nothing and works while the configuration is locked; it never reads the
//! database or the IPC token. The app adds its own version and update
//! channel (only while the configuration is open: the locked defaults would
//! be wrong), and nothing else.
//!
//! [`prepare_diagnostics`] returns the whole bundle for the window to show;
//! [`save_diagnostics`] opens the native save dialog itself and writes
//! exactly those bytes to the path the user picks there. The window never
//! sends content or a path, only the token of a prepared bundle. Nothing is
//! uploaded anywhere.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Value, json};
use tauri::{Manager, Runtime, State, WebviewWindow};

use crate::error::{CommandError, CommandResult};
use crate::state::{APP_VERSION, AppState};

/// The bundle's schema (`deckpilot_agent::diagnose::bundle::SCHEMA`).
const BUNDLE_SCHEMA: &str = "deckpilot.support-bundle";
/// A larger output is refused (the agent bounds its logs well below it).
const MAX_BUNDLE_BYTES: usize = 8 * 1024 * 1024;
const BUNDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// The prepared bundle, until it is saved again or discarded.
#[derive(Debug, Default)]
pub struct PendingDiagnostics {
    slot: Mutex<Option<Prepared>>,
    next: AtomicU64,
}

#[derive(Debug)]
struct Prepared {
    token: u64,
    file_name: String,
    bytes: Vec<u8>,
}

impl PendingDiagnostics {
    fn slot(&self) -> std::sync::MutexGuard<'_, Option<Prepared>> {
        self.slot.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// One log in the bundle, for the list of what it contains.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LogSummary {
    file: String,
    lines: usize,
    truncated: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsPreview {
    token: String,
    /// A suggested file name.
    file_name: String,
    /// The whole bundle, exactly as it will be saved.
    text: String,
    bytes: usize,
    logs: Vec<LogSummary>,
    /// Findings of the report (a device without HID, a missing permission).
    findings: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedDiagnostics {
    path: String,
    bytes: usize,
}

/// Runs `deckpilot-agent diagnose --bundle` and keeps its bundle for
/// [`save_diagnostics`].
#[tauri::command]
pub async fn prepare_diagnostics(
    state: State<'_, AppState>,
    pending: State<'_, PendingDiagnostics>,
) -> CommandResult<DiagnosticsPreview> {
    let executable = crate::agent::agent_executable().ok_or_else(|| {
        CommandError::Invalid("the DeckPilot agent executable was not found".into())
    })?;
    let output = tokio::task::spawn_blocking(move || run_bundle(&executable, BUNDLE_TIMEOUT))
        .await
        .map_err(|error| CommandError::Invalid(error.to_string()))?
        .map_err(CommandError::Invalid)?;
    let channel = state
        .store()
        .is_ok()
        .then(|| crate::updates::stored_channel(&state));
    let (bytes, preview) = finish(&output, &app_section(channel))?;
    let token = pending.next.fetch_add(1, Ordering::Relaxed) + 1;
    *pending.slot() = Some(Prepared {
        token,
        file_name: preview.file_name.clone(),
        bytes,
    });
    Ok(DiagnosticsPreview {
        token: token.to_string(),
        ..preview
    })
}

/// Asks where to save the prepared bundle in a native save dialog (`title`
/// and `file_type` are its labels) and writes it there. `None` when the
/// user cancelled.
#[tauri::command]
pub async fn save_diagnostics<R: Runtime>(
    window: WebviewWindow<R>,
    pending: State<'_, PendingDiagnostics>,
    token: String,
    title: String,
    file_type: String,
) -> CommandResult<Option<SavedDiagnostics>> {
    use tauri_plugin_dialog::DialogExt;
    let file_name = pending
        .slot()
        .as_ref()
        .filter(|prepared| prepared.token.to_string() == token)
        .map(|prepared| prepared.file_name.clone())
        .ok_or_else(|| CommandError::Invalid("prepare the diagnostics again".into()))?;
    if window
        .try_state::<tauri_plugin_dialog::Dialog<R>>()
        .is_none()
    {
        return Err(CommandError::Invalid("no file dialog is available".into()));
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    window
        .dialog()
        .file()
        .set_parent(&window)
        .set_title(title)
        .set_file_name(file_name)
        .add_filter(file_type, &["json"])
        .save_file(move |file| {
            let _ = tx.send(file);
        });
    let Some(path) = rx
        .await
        .ok()
        .flatten()
        .and_then(|file| file.into_path().ok())
    else {
        return Ok(None);
    };
    save_prepared(&pending, &token, &path).map(Some)
}

fn save_prepared(
    pending: &PendingDiagnostics,
    token: &str,
    path: &Path,
) -> CommandResult<SavedDiagnostics> {
    let slot = pending.slot();
    let prepared = slot
        .as_ref()
        .filter(|prepared| prepared.token.to_string() == token)
        .ok_or_else(|| CommandError::Invalid("prepare the diagnostics again".into()))?;
    if path.as_os_str().is_empty() {
        return Err(CommandError::Invalid("no file was chosen".into()));
    }
    super::portability::write_export(path, &prepared.bytes)
        .map_err(|error| CommandError::Invalid(error.to_string()))?;
    Ok(SavedDiagnostics {
        path: path.display().to_string(),
        bytes: prepared.bytes.len(),
    })
}

/// Forgets the prepared bundle (the dialog closed).
#[tauri::command]
pub fn discard_diagnostics(pending: State<'_, PendingDiagnostics>) {
    *pending.slot() = None;
}

fn app_section(update_channel: Option<deckpilot_update::channel::Channel>) -> Value {
    json!({
        "version": APP_VERSION,
        "updateChannel": update_channel,
        "debugBuild": cfg!(debug_assertions),
    })
}

/// The agent's output checked and completed with the app's section: the
/// bytes to save and the preview without its token.
fn finish(output: &[u8], app: &Value) -> CommandResult<(Vec<u8>, DiagnosticsPreview)> {
    let invalid = || CommandError::Invalid("the agent did not produce a diagnostics bundle".into());
    let mut bundle: Value = serde_json::from_slice(output).map_err(|_| invalid())?;
    if bundle.get("schema").and_then(Value::as_str) != Some(BUNDLE_SCHEMA) {
        return Err(invalid());
    }
    let object = bundle.as_object_mut().ok_or_else(invalid)?;
    object.insert("app".into(), app.clone());
    let logs = bundle["logs"]
        .as_array()
        .map(|logs| {
            logs.iter()
                .map(|log| LogSummary {
                    file: log["file"].as_str().unwrap_or_default().to_owned(),
                    lines: log["lines"].as_array().map_or(0, Vec::len),
                    truncated: log["truncated"].as_bool().unwrap_or(false),
                })
                .collect()
        })
        .unwrap_or_default();
    let findings = bundle["report"]["devices"]["findings"]
        .as_array()
        .map_or(0, Vec::len);
    let stamp: String = bundle["createdAt"]
        .as_str()
        .unwrap_or_default()
        .chars()
        .filter(char::is_ascii_digit)
        .take(14)
        .collect();
    let file_name = if stamp.len() == 14 {
        format!("deckpilot-diagnostics-{}-{}.json", &stamp[..8], &stamp[8..])
    } else {
        "deckpilot-diagnostics.json".to_owned()
    };
    let mut text =
        serde_json::to_string_pretty(&bundle).map_err(|e| CommandError::Invalid(e.to_string()))?;
    text.push('\n');
    let bytes = text.clone().into_bytes();
    Ok((
        bytes,
        DiagnosticsPreview {
            token: String::new(),
            file_name,
            bytes: text.len(),
            text,
            logs,
            findings,
        },
    ))
}

/// Runs `<executable> diagnose --bundle` with no input and returns its
/// output; ends it after `timeout`.
fn run_bundle(executable: &Path, timeout: Duration) -> Result<Vec<u8>, String> {
    let mut command = Command::new(executable);
    command
        .args(["diagnose", "--bundle"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // The agent is a console program: no console window flashes up.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("cannot start the agent's diagnostics: {error}"))?;
    let stdout = child.stdout.take().ok_or("no output from the agent")?;
    let (sender, output) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let limit = u64::try_from(MAX_BUNDLE_BYTES).unwrap_or(u64::MAX) + 1;
        let _ = sender.send(stdout.take(limit).read_to_end(&mut bytes).map(|_| bytes));
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("the agent's diagnostics did not finish in time".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.to_string());
            }
        }
    };
    // The output ends with the process; a program it left behind that
    // keeps the pipe open is not waited for.
    let bytes = output
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| "cannot read the agent's diagnostics".to_owned())?
        .map_err(|error| error.to_string())?;
    if bytes.len() > MAX_BUNDLE_BYTES {
        return Err("the agent's diagnostics are too large".into());
    }
    if !status.success() {
        return Err(format!("the agent's diagnostics failed ({status})"));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent_output() -> Vec<u8> {
        serde_json::to_vec(&json!({
            "schema": BUNDLE_SCHEMA,
            "schemaVersion": 1,
            "createdAt": "2026-11-26T10:20:30Z",
            "report": { "devices": { "findings": [{ "code": "no-permission" }] } },
            "logs": [
                { "file": "agent.2026-11-26.log", "size": 10, "truncated": true, "lines": ["a", "b"] },
                { "file": "updates.log", "size": 1, "truncated": false, "lines": ["c"] }
            ],
            "redaction": { "rules": ["secret-fields"] }
        }))
        .unwrap()
    }

    #[test]
    fn the_preview_is_exactly_what_is_saved() {
        let app = app_section(Some(deckpilot_update::channel::Channel::Stable));
        let (bytes, preview) = finish(&agent_output(), &app).unwrap();
        assert_eq!(preview.text.as_bytes(), bytes.as_slice());
        assert_eq!(preview.bytes, bytes.len());
        assert_eq!(
            preview.file_name,
            "deckpilot-diagnostics-20261126-102030.json"
        );
        assert_eq!(preview.findings, 1);
        assert_eq!(
            preview.logs,
            [
                LogSummary {
                    file: "agent.2026-11-26.log".into(),
                    lines: 2,
                    truncated: true
                },
                LogSummary {
                    file: "updates.log".into(),
                    lines: 1,
                    truncated: false
                },
            ]
        );
        let saved: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(saved["app"]["version"], APP_VERSION);
        assert_eq!(saved["app"]["updateChannel"], "stable");
        assert_eq!(saved["logs"][0]["lines"][1], "b");
    }

    #[test]
    fn a_locked_configuration_reports_no_update_channel() {
        let (bytes, _) = finish(&agent_output(), &app_section(None)).unwrap();
        let saved: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(saved["app"]["updateChannel"].is_null());
    }

    #[test]
    fn other_output_is_refused() {
        let app = app_section(None);
        assert!(finish(b"not json", &app).is_err());
        assert!(finish(br#"{"schema":"deckpilot.diagnose"}"#, &app).is_err());
        assert!(finish(b"[]", &app).is_err());
    }

    #[cfg(unix)]
    fn script(dir: &Path, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("fake-agent");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    #[test]
    fn the_agent_is_asked_for_a_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let agent = script(
            dir.path(),
            r#"[ "$1 $2" = "diagnose --bundle" ] || exit 3; printf '{"schema":"deckpilot.support-bundle"}'"#,
        );
        let output = run_bundle(&agent, Duration::from_secs(10)).unwrap();
        assert_eq!(output, br#"{"schema":"deckpilot.support-bundle"}"#);
    }

    #[cfg(unix)]
    #[test]
    fn a_failing_or_hanging_agent_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let failing = script(dir.path(), "exit 1");
        assert!(run_bundle(&failing, Duration::from_secs(10)).is_err());
        let hanging = script(dir.path(), "exec sleep 30");
        let started = Instant::now();
        assert!(run_bundle(&hanging, Duration::from_millis(300)).is_err());
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn saving_writes_the_prepared_bytes_for_its_token_only() {
        let pending = PendingDiagnostics::default();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bundle.json");
        assert!(
            save_prepared(&pending, "1", &path).is_err(),
            "nothing prepared"
        );
        *pending.slot() = Some(Prepared {
            token: 7,
            file_name: "d.json".into(),
            bytes: b"{\"a\":1}".to_vec(),
        });
        assert!(
            save_prepared(&pending, "8", &path).is_err(),
            "another token"
        );
        assert!(!path.exists());
        assert!(
            save_prepared(&pending, "7", Path::new("")).is_err(),
            "no path"
        );
        let saved = save_prepared(&pending, "7", &path).unwrap();
        assert_eq!(saved.bytes, 7);
        assert_eq!(std::fs::read(&path).unwrap(), b"{\"a\":1}");
        // Saved again (another place) until discarded.
        let again = dir.path().join("again.json");
        assert!(save_prepared(&pending, "7", &again).is_ok());
        *pending.slot() = None;
        assert!(save_prepared(&pending, "7", &path).is_err(), "discarded");
    }
}
