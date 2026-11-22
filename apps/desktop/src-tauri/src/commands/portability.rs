//! Encrypted configuration export and import.
//!
//! The file paths come from native save/open dialogs in the UI. The export
//! password is separate from the database password and only used to derive
//! the archive key. An import is previewed first; the decrypted content is
//! kept in memory until the user applies or cancels it.

use std::io::Write as _;
use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::State;

use deckpilot_core::ProfileId;
use deckpilot_portability::archive::MAX_ARCHIVE_BYTES;
use deckpilot_portability::import::{self, ImportPreview};
use deckpilot_portability::{
    ExportOptions, ImportOptions, ImportSummary, Payload, PortabilityError, payload,
};
use deckpilot_storage::crypto::{KdfParams, Password};
use deckpilot_storage::{ConfigStore as _, StorageError};

use crate::error::{CommandError, CommandResult};
use crate::state::{APP_VERSION, AppState};

/// The decrypted export between preview and apply.
#[derive(Debug, Default)]
pub struct PendingImport(Mutex<Option<(u32, Payload)>>);

impl PendingImport {
    fn take(&self, token: u32) -> Option<Payload> {
        let mut pending = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        match pending.take() {
            Some((stored, payload)) if stored == token => Some(payload),
            other => {
                *pending = other;
                None
            }
        }
    }

    /// A random token (32 bits, exact in JavaScript numbers) ties the apply
    /// call to the preview the user saw.
    fn put(&self, payload: Payload) -> u32 {
        let mut bytes = [0u8; 4];
        let _ = deckpilot_storage::crypto::random_bytes(&mut bytes);
        let token = u32::from_le_bytes(bytes);
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some((token, payload));
        token
    }

    fn clear(&self) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

impl From<PortabilityError> for CommandError {
    fn from(error: PortabilityError) -> Self {
        match error {
            PortabilityError::Storage(error) => error.into(),
            other => Self::Invalid(other.to_string()),
        }
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .unwrap_or_default()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    path: String,
    password: Password,
    confirmation: Password,
    /// `None`: all profiles.
    profiles: Option<Vec<ProfileId>>,
    include_module_settings: bool,
    /// Portable application settings (update preferences).
    #[serde(default)]
    include_app_settings: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    path: String,
    profiles: usize,
    modules: usize,
    bytes: usize,
}

/// Writes an encrypted export to `path` (chosen in a save dialog).
#[tauri::command]
pub fn export_configuration(
    state: State<'_, AppState>,
    request: ExportRequest,
) -> CommandResult<ExportResult> {
    request
        .password
        .validate_new(&request.confirmation)
        .map_err(StorageError::from)?;
    state.store()?;
    // Exports contain the whole configuration: a sensitive operation.
    state.take_authorization()?;
    let content = {
        let store = state.store()?;
        let modules = state.modules();
        payload::export(
            &store,
            &modules,
            request.profiles.as_deref(),
            ExportOptions {
                module_settings: request.include_module_settings,
                app_settings: request.include_app_settings,
            },
            now(),
            APP_VERSION,
        )?
    };
    let kdf = KdfParams::generate().map_err(StorageError::from)?;
    let bytes = payload::seal(&content, &request.password, kdf)?;
    // Only encrypted bytes reach the disk.
    write_export(Path::new(&request.path), &bytes)
        .map_err(|error| CommandError::Invalid(error.to_string()))?;
    Ok(ExportResult {
        path: request.path,
        profiles: content.profiles.len(),
        modules: content.modules.len(),
        bytes: bytes.len(),
    })
}

/// The steps of [`write_export`], where tests make it fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
enum ExportStep {
    /// Writing the staging file, after half of the bytes.
    Write,
    /// Replacing the destination with the staging file.
    Replace,
}

/// Replaces `path` with `bytes`: afterwards `path` holds either all of
/// `bytes` or, after an error, exactly what it held before (issue #402).
///
/// The bytes are staged in a file of this export's own, created exclusively
/// (`create_new`) under a random name in the destination's directory,
/// written through that handle and synced, then renamed over `path` — an
/// atomic replacement on the same file system (`rename(2)`; on Windows
/// `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING`). No other file is
/// opened, so a sibling that happens to look like a staging file is never
/// written or removed, and concurrent exports never share a staging file.
/// On every error the staging file, and only it, is removed.
fn write_export(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    write_export_failing_at(path, bytes, None)
}

fn write_export_failing_at(
    path: &Path,
    bytes: &[u8],
    fail_at: Option<ExportStep>,
) -> std::io::Result<()> {
    let injected = |step: ExportStep| {
        if fail_at == Some(step) {
            Err(std::io::Error::other(format!("injected {step:?} failure")))
        } else {
            Ok(())
        }
    };
    let dir = match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    // Removed when dropped, by its own path: every early return below
    // cleans up exactly this file.
    let mut staging = tempfile::Builder::new()
        .prefix(".deckpilot-export-")
        .suffix(".tmp")
        .tempfile_in(dir)?;
    if fail_at == Some(ExportStep::Write) {
        staging.write_all(&bytes[..bytes.len() / 2])?;
    }
    injected(ExportStep::Write)?;
    staging.write_all(bytes)?;
    staging.as_file().sync_all()?;
    injected(ExportStep::Replace)?;
    replace_with(staging, path)?;
    // The rename itself survives a crash once the directory is synced.
    #[cfg(unix)]
    if let Ok(dir) = std::fs::File::open(dir) {
        let _ = dir.sync_all();
    }
    Ok(())
}

/// Renames `staging` over `path`. On Windows, another program (a virus
/// scanner, the search indexer) briefly holding the destination open makes
/// the replacement fail with "access denied" or a sharing violation; that
/// is retried for a bounded time, as `deckpilot_storage` does for the
/// database files.
fn replace_with(mut staging: tempfile::NamedTempFile, path: &Path) -> std::io::Result<()> {
    const RETRY_FOR: Duration = Duration::from_secs(3);
    let in_use = |error: &std::io::Error| {
        // ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION.
        cfg!(windows) && matches!(error.raw_os_error(), Some(5 | 32 | 33))
    };
    let deadline = Instant::now() + RETRY_FOR;
    let mut pause = Duration::from_millis(10);
    loop {
        match staging.persist(path) {
            Ok(_) => return Ok(()),
            Err(error) if in_use(&error.error) && Instant::now() < deadline => {
                staging = error.file;
                std::thread::sleep(pause);
                pause = (pause * 2).min(Duration::from_millis(200));
            }
            // `error.file` is dropped here, which removes the staging file.
            Err(error) => {
                return Err(std::io::Error::new(
                    error.error.kind(),
                    format!("could not write {}: {}", path.display(), error.error),
                ));
            }
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewResult {
    token: u32,
    preview: ImportPreview,
}

/// Decrypts and validates an export and describes what importing it would
/// change. Nothing is written.
#[tauri::command]
pub fn preview_import(
    state: State<'_, AppState>,
    pending: State<'_, PendingImport>,
    path: String,
    password: Password,
) -> CommandResult<PreviewResult> {
    let size = std::fs::metadata(&path)
        .map_err(|e| CommandError::Invalid(e.to_string()))?
        .len();
    if usize::try_from(size).map_or(true, |size| size > MAX_ARCHIVE_BYTES) {
        return Err(PortabilityError::TooLarge.into());
    }
    let bytes = std::fs::read(&path).map_err(|e| CommandError::Invalid(e.to_string()))?;
    let content = payload::open(&bytes, &password)?;
    let mut warnings = import::validate(&content, &state.catalog)?;
    // Bindings whose action cannot run from their control are imported as
    // they are and reported.
    warnings.extend(import::compatibility_warnings(
        &content,
        &state.catalog,
        &state.actions(),
    ));
    let preview = {
        let store = state.store()?;
        let modules = state.modules();
        import::preview(&content, &store, &modules, warnings)?
    };
    let token = pending.put(content);
    Ok(PreviewResult { token, preview })
}

/// Sent to every window after imported data was written.
pub const CONFIGURATION_CHANGED: &str = "deckpilot://configuration-changed";

/// Applies a previewed import in one transaction.
#[tauri::command]
pub fn apply_import<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
    pending: State<'_, PendingImport>,
    token: u32,
    options: ImportOptions,
) -> CommandResult<ImportSummary> {
    let content = pending
        .take(token)
        .ok_or_else(|| CommandError::invalid("previewAgain", "preview the import again"))?;
    let summary = {
        let store = state.store()?;
        let (changes, summary) = {
            let modules = state.modules();
            import::plan(&content, &options, &store, &modules)?
        };
        store.apply_changes(&changes)?;
        let (modules, problems) = deckpilot_modules::configured(&store.module_states()?);
        for problem in problems {
            eprintln!("imported module state was not applied: {problem}");
        }
        *state.modules() = modules;
        summary
    };
    crate::agent::notify_reload(&state.paths);
    let _ = tauri::Emitter::emit(&app, CONFIGURATION_CHANGED, ());
    Ok(summary)
}

#[tauri::command]
pub fn cancel_import(pending: State<'_, PendingImport>) {
    pending.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A new empty directory, removed when dropped.
    struct Dir(std::path::PathBuf);

    impl Dir {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("deckpilot-export-{}", ProfileId::new()));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn join(&self, name: &str) -> std::path::PathBuf {
            self.0.join(name)
        }

        /// The names of the files in the directory, sorted.
        fn names(&self) -> Vec<String> {
            let mut names: Vec<String> = std::fs::read_dir(&self.0)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const SIBLING: &[u8] = b"unrelated sibling marker";

    /// A destination with an earlier export and the sibling the old staging
    /// scheme wrote to (`<stem>.deckpilot.tmp`).
    fn with_earlier_export() -> (Dir, std::path::PathBuf, std::path::PathBuf) {
        let dir = Dir::new();
        let destination = dir.join("backup.deckpilot");
        let sibling = dir.join("backup.deckpilot.tmp");
        std::fs::write(&destination, b"earlier export").unwrap();
        std::fs::write(&sibling, SIBLING).unwrap();
        (dir, destination, sibling)
    }

    #[test]
    fn an_export_replaces_the_destination_and_keeps_the_sibling() {
        let (dir, destination, sibling) = with_earlier_export();
        write_export(&destination, b"new export").unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"new export");
        assert_eq!(std::fs::read(&sibling).unwrap(), SIBLING);
        assert_eq!(dir.names(), ["backup.deckpilot", "backup.deckpilot.tmp"]);

        // A destination that does not exist yet.
        let fresh = dir.join("fresh.deckpilot");
        write_export(&fresh, b"first export").unwrap();
        assert_eq!(std::fs::read(&fresh).unwrap(), b"first export");
    }

    #[test]
    fn a_failed_write_or_replacement_leaves_everything_as_it_was() {
        for step in [ExportStep::Write, ExportStep::Replace] {
            let (dir, destination, sibling) = with_earlier_export();
            let error =
                write_export_failing_at(&destination, b"new export", Some(step)).unwrap_err();
            assert!(error.to_string().contains("injected"), "{step:?}: {error}");
            assert_eq!(
                std::fs::read(&destination).unwrap(),
                b"earlier export",
                "{step:?}"
            );
            assert_eq!(std::fs::read(&sibling).unwrap(), SIBLING, "{step:?}");
            // The staging file, and only it, is gone.
            assert_eq!(
                dir.names(),
                ["backup.deckpilot", "backup.deckpilot.tmp"],
                "{step:?}"
            );
        }
    }

    #[test]
    fn a_real_replacement_failure_cleans_up_only_its_staging_file() {
        let (dir, _, sibling) = with_earlier_export();
        // A directory that holds a file cannot be replaced by a file.
        let blocked = dir.join("blocked.deckpilot");
        std::fs::create_dir_all(&blocked).unwrap();
        std::fs::write(blocked.join("inside"), b"x").unwrap();
        let error = write_export(&blocked, b"new export").unwrap_err();
        assert!(error.to_string().contains("blocked.deckpilot"), "{error}");
        assert_eq!(std::fs::read(blocked.join("inside")).unwrap(), b"x");
        assert_eq!(std::fs::read(&sibling).unwrap(), SIBLING);
        assert_eq!(
            dir.names(),
            [
                "backup.deckpilot",
                "backup.deckpilot.tmp",
                "blocked.deckpilot"
            ]
        );
    }

    #[test]
    fn concurrent_exports_never_share_a_staging_file() {
        let (dir, destination, sibling) = with_earlier_export();
        // `backup.json` derived the same staging name as `backup.deckpilot`
        // in the old scheme.
        let other = dir.join("backup.json");
        let contents: Vec<Vec<u8>> = (0..16u8)
            .map(|i| vec![b'a' + i; 64 * 1024 + usize::from(i)])
            .collect();
        std::thread::scope(|scope| {
            for (i, bytes) in contents.iter().enumerate() {
                let target = if i % 2 == 0 { &destination } else { &other };
                scope.spawn(move || write_export(target, bytes).unwrap());
            }
        });
        // Each destination holds one export completely, of its own writers.
        let written = std::fs::read(&destination).unwrap();
        assert!(contents.iter().step_by(2).any(|c| *c == written));
        let written = std::fs::read(&other).unwrap();
        assert!(contents.iter().skip(1).step_by(2).any(|c| *c == written));
        assert_eq!(std::fs::read(&sibling).unwrap(), SIBLING);
        assert_eq!(
            dir.names(),
            ["backup.deckpilot", "backup.deckpilot.tmp", "backup.json"]
        );
    }
}
