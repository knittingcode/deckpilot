//! Commands of the Scripts page (`docs/data-model.md`, "Scripts"): the
//! scripts of a profile, their review and trust, and their grants.
//!
//! Every command names the profile it works on, and every script id is
//! looked up inside that profile. Nothing here runs a script: a run is an
//! action of the Scripts plugin, which the agent answers only for a
//! trusted script. Trust and grants are this computer's (`script_trust`)
//! and are changed only by the user here:
//!
//! - a script created here is trusted, with no grants; saving grants
//!   nothing;
//! - "Trust this script" names the hash of the source the review showed
//!   and the dormant grants it listed; it is refused when either changed;
//! - grants are set only on a trusted script and checked whole (a
//!   container runtime, a program that acts as another user or a shell as
//!   a command is refused).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tauri::{Runtime, State, WebviewWindow};

use deckpilot_core::script::{Script, ScriptSettings, ScriptTrust, TrustState, source_sha256};
use deckpilot_core::script_grants::{CommandClass, ScriptGrants, command_class};
use deckpilot_core::{ProfileId, ScriptId};
use deckpilot_storage::references::{ReferenceSite, Selection};
use deckpilot_storage::scripts::{ScriptUsage, StoredScript, script_usages};

use crate::error::{CommandError, CommandResult};
use crate::state::{AppState, load_profile};

/// A new script, as the editor creates it.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NewScript {
    name: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    settings: Option<ScriptSettings>,
}

/// What the review shows before "Trust this script".
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptReview {
    pub script: Script,
    pub trust: TrustState,
    /// The hash of `script.source`, which trusting names.
    pub source_sha256: String,
    /// The grants that come back with trust (dormant while untrusted;
    /// none for a script that never had any).
    pub grants: ScriptGrants,
    /// What each granted command is (a launcher runs any program).
    pub command_classes: BTreeMap<String, CommandClass>,
}

/// Trust, grants and the scripts themselves change only from the main
/// window (defense in depth: every window loads the app's own pages).
fn main_only<R: Runtime>(window: &WebviewWindow<R>) -> CommandResult<()> {
    if window.label() == crate::windows::MAIN {
        Ok(())
    } else {
        Err(CommandError::invalid_with(
            "notMainWindow",
            "scripts are changed only from the main window",
            [],
        ))
    }
}

fn script_of(state: &AppState, profile: ProfileId, id: ScriptId) -> CommandResult<Script> {
    state
        .store()?
        .script(profile, id)?
        .ok_or(CommandError::NotFound("script"))
}

#[tauri::command]
pub fn list_scripts(
    state: State<'_, AppState>,
    profile_id: ProfileId,
) -> CommandResult<Vec<StoredScript>> {
    load_profile(&*state.store()?, profile_id)?;
    Ok(state.store()?.scripts(profile_id)?)
}

#[tauri::command]
pub fn create_script<R: Runtime>(
    window: WebviewWindow<R>,
    state: State<'_, AppState>,
    profile_id: ProfileId,
    script: NewScript,
) -> CommandResult<Script> {
    main_only(&window)?;
    let mut new = Script::new(script.name.trim(), script.source);
    if let Some(settings) = script.settings {
        new.settings = settings;
    }
    Ok(state.store()?.create_script(profile_id, &new)?)
}

/// The editor's save of `script` (made from its `revision`).
#[tauri::command]
pub fn save_script<R: Runtime>(
    window: WebviewWindow<R>,
    state: State<'_, AppState>,
    profile_id: ProfileId,
    script: Script,
) -> CommandResult<Script> {
    main_only(&window)?;
    let script = Script {
        name: script.name.trim().to_owned(),
        ..script
    };
    Ok(state.store()?.save_script(profile_id, &script)?)
}

#[tauri::command]
pub fn duplicate_script<R: Runtime>(
    window: WebviewWindow<R>,
    state: State<'_, AppState>,
    profile_id: ProfileId,
    script_id: ScriptId,
) -> CommandResult<Script> {
    main_only(&window)?;
    Ok(state.store()?.duplicate_script(profile_id, script_id)?)
}

/// Every action that runs the script, with its uses (the guarded delete
/// lists them).
#[tauri::command]
pub fn script_usages_of(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    script_id: ScriptId,
) -> CommandResult<Vec<ScriptUsage>> {
    let profile = load_profile(&*state.store()?, profile_id)?;
    Ok(script_usages(&profile, script_id))
}

/// Deletes the script and its trust. Its bindings are kept and show
/// "Script missing", unless `clear_keys`: then the key bindings that run
/// it are cleared first (steps and nodes stay, and show it missing).
#[tauri::command]
pub fn delete_script<R: Runtime>(
    window: WebviewWindow<R>,
    state: State<'_, AppState>,
    profile_id: ProfileId,
    script_id: ScriptId,
    clear_keys: bool,
) -> CommandResult<()> {
    main_only(&window)?;
    script_of(&state, profile_id, script_id)?;
    let result = (|| -> CommandResult<()> {
        let store = state.store()?;
        if clear_keys {
            let profile = load_profile(&store, profile_id)?;
            for usage in script_usages(&profile, script_id) {
                let keys: Vec<ReferenceSite> = usage
                    .references
                    .iter()
                    .map(|r| r.site.clone())
                    .filter(|s| matches!(s, ReferenceSite::Binding { .. }))
                    .collect();
                if !keys.is_empty() {
                    store.unbind_action(profile_id, usage.action, &Selection::Sites(keys))?;
                }
            }
        }
        store.delete_script(profile_id, script_id)?;
        Ok(())
    })();
    // Also when the delete failed after keys were cleared.
    crate::agent::notify_reload(&state.paths);
    result
}

/// What the review shows: the stored source, its hash and the grants that
/// trusting brings back.
#[tauri::command]
pub fn review_script(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    script_id: ScriptId,
) -> CommandResult<ScriptReview> {
    let script = script_of(&state, profile_id, script_id)?;
    let record = state.store()?.script_trust(profile_id, script_id)?;
    let grants = record
        .as_ref()
        .map(|r| r.grants.clone())
        .unwrap_or_default();
    Ok(ScriptReview {
        trust: ScriptTrust::state(record.as_ref(), &script),
        source_sha256: source_sha256(&script.source),
        command_classes: grants
            .process
            .iter()
            .map(|p| (p.command.clone(), command_class(&p.command)))
            .collect(),
        grants,
        script,
    })
}

/// "Trust this script": the user reviewed the source with this hash and
/// the grants listed with it.
#[tauri::command]
pub fn trust_script<R: Runtime>(
    window: WebviewWindow<R>,
    state: State<'_, AppState>,
    profile_id: ProfileId,
    script_id: ScriptId,
    source_sha256: String,
    grants: ScriptGrants,
) -> CommandResult<()> {
    main_only(&window)?;
    Ok(state
        .store()?
        .trust_script(profile_id, script_id, &source_sha256, &grants)?)
}

/// A folder scope's path as granted: canonical (links and junctions
/// resolved now, so a link pointed elsewhere later does not move the
/// grant), an existing folder, in the plain form on Windows.
fn canonical_folder(path: &std::path::Path) -> CommandResult<std::path::PathBuf> {
    let real = std::fs::canonicalize(path)
        .ok()
        .filter(|p| p.is_dir())
        .ok_or_else(|| {
            CommandError::invalid_with(
                "scriptFolderMissing",
                format!("{} is not a folder on this computer", path.display()),
                [],
            )
        })?;
    #[cfg(windows)]
    {
        let text = real.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            return Ok(std::path::PathBuf::from(format!(r"\\{rest}")));
        }
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            return Ok(std::path::PathBuf::from(rest));
        }
    }
    Ok(real)
}

/// The file a granted command stands for, as the agent's executor finds
/// it: an absolute path as it is, a bare name in the absolute `PATH`
/// entries (with `.exe` and `.com` on Windows unless it has one), a file
/// that can run; canonical. `None` when it is not found here.
fn program_file(command: &str) -> Option<std::path::PathBuf> {
    let path = std::path::Path::new(command);
    if path.is_absolute() {
        return std::fs::canonicalize(path).ok();
    }
    let has_ext = std::path::Path::new(command)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("exe") || e.eq_ignore_ascii_case("com"));
    let names: Vec<String> = if cfg!(windows) && !has_ext {
        vec![format!("{command}.exe"), format!("{command}.com")]
    } else {
        vec![command.to_owned()]
    };
    let runnable = |p: &std::path::Path| {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            p.metadata()
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        }
        #[cfg(not(unix))]
        {
            p.is_file()
        }
    };
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .filter(|dir| dir.is_absolute())
            .flat_map(|dir| names.iter().map(move |n| dir.join(n)))
            .find(|candidate| runnable(candidate))
            .and_then(|found| std::fs::canonicalize(found).ok())
    })
}

/// Refuses a program granted twice: the same file started under the same
/// name (`make` and `/usr/bin/make`), or, when it is not found here, the
/// same name. Names compare without case where the system does (macOS,
/// Windows). One file under two names (`xz` and `unxz`) is two programs:
/// many behave by the name they are started as.
fn no_program_twice(grants: &ScriptGrants) -> CommandResult<()> {
    let fold = |text: &str| {
        if cfg!(any(target_os = "macos", windows)) {
            text.to_lowercase()
        } else {
            text.to_owned()
        }
    };
    let mut seen: Vec<String> = Vec::new();
    for grant in &grants.process {
        let name = std::path::Path::new(&grant.command)
            .file_name()
            .map_or_else(
                || grant.command.clone(),
                |n| n.to_string_lossy().into_owned(),
            );
        let key = match program_file(&grant.command) {
            Some(file) => format!("{}\0{}", file.display(), fold(&name)),
            None => fold(&grant.command),
        };
        if seen.contains(&key) {
            return Err(CommandError::invalid_with(
                "scriptProgramTwice",
                format!("{} is granted twice", grant.command),
                [],
            ));
        }
        seen.push(key);
    }
    Ok(())
}

/// The Permissions tab's save. Folder scopes are stored canonical.
#[tauri::command]
pub fn set_script_grants<R: Runtime>(
    window: WebviewWindow<R>,
    state: State<'_, AppState>,
    profile_id: ProfileId,
    script_id: ScriptId,
    grants: ScriptGrants,
) -> CommandResult<()> {
    main_only(&window)?;
    let mut grants = grants;
    no_program_twice(&grants)?;
    for scope in &mut grants.fs {
        if let deckpilot_core::script_grants::FsRoot::Directory { path, .. } = &mut scope.root {
            *path = canonical_folder(path)?;
        }
    }
    Ok(state
        .store()?
        .set_script_grants(profile_id, script_id, &grants)?)
}

/// Revokes every grant of the script (it stays trusted).
#[tauri::command]
pub fn revoke_script_grants<R: Runtime>(
    window: WebviewWindow<R>,
    state: State<'_, AppState>,
    profile_id: ProfileId,
    script_id: ScriptId,
) -> CommandResult<()> {
    main_only(&window)?;
    Ok(state
        .store()?
        .set_script_grants(profile_id, script_id, &ScriptGrants::default())?)
}

/// What a command a grant would name is (the Permissions tab shows why a
/// container runtime, a program acting as another user or a shell is not
/// available, and that a launcher runs any program).
#[tauri::command]
pub fn script_command_class(command: String) -> CommandClass {
    command_class(&command)
}

#[cfg(test)]
mod tests {
    use super::{canonical_folder, no_program_twice};
    use deckpilot_core::script_grants::{ProcessCwd, ProcessEnv, ProcessGrant, ScriptGrants};

    fn grants(commands: &[&str]) -> ScriptGrants {
        ScriptGrants {
            process: commands
                .iter()
                .map(|c| ProcessGrant {
                    command: (*c).into(),
                    cwd: ProcessCwd::Repositories,
                    env: ProcessEnv::None,
                    unrestricted_args: false,
                })
                .collect(),
            ..ScriptGrants::default()
        }
    }

    #[test]
    fn a_program_is_granted_once_however_it_is_named() {
        // Not found here: by name (without case on macOS and Windows).
        assert!(no_program_twice(&grants(&["no-such-tool-x", "no-such-tool-x"])).is_err());
        assert_eq!(
            no_program_twice(&grants(&["no-such-tool-x", "NO-SUCH-TOOL-X"])).is_err(),
            cfg!(any(target_os = "macos", windows))
        );
        assert!(no_program_twice(&grants(&["no-such-tool-x", "no-such-tool-y"])).is_ok());
        // Found here: one file under one name is one program, however it
        // is reached; under another name it is another.
        #[cfg(unix)]
        {
            let dir = tempfile::tempdir().unwrap();
            let tool = dir.path().join("tool");
            std::fs::write(&tool, "#!/bin/sh\n").unwrap();
            let link_dir = dir.path().join("links");
            std::fs::create_dir(&link_dir).unwrap();
            std::os::unix::fs::symlink(&tool, link_dir.join("tool")).unwrap();
            std::os::unix::fs::symlink(&tool, link_dir.join("other")).unwrap();
            let via_link = link_dir.join("tool");
            let other = link_dir.join("other");
            let direct = tool.to_string_lossy().into_owned();
            let linked = via_link.to_string_lossy().into_owned();
            let renamed = other.to_string_lossy().into_owned();
            assert!(no_program_twice(&grants(&[&direct, &linked])).is_err());
            assert!(no_program_twice(&grants(&[&direct, &renamed])).is_ok());
        }
    }

    #[test]
    fn a_folder_grant_is_the_folder_itself_now() {
        let dir = tempfile::tempdir().unwrap();
        let work = dir.path().join("work");
        std::fs::create_dir(&work).unwrap();
        let real = canonical_folder(&work).unwrap();
        assert_eq!(real, std::fs::canonicalize(&work).unwrap());
        // A link is resolved now: pointing it elsewhere later moves nothing.
        #[cfg(unix)]
        {
            let link = dir.path().join("link");
            std::os::unix::fs::symlink(&work, &link).unwrap();
            assert_eq!(canonical_folder(&link).unwrap(), real);
        }
        // `..` is resolved away; a file or a missing folder is refused.
        let dotted = work.join("..").join("work");
        assert_eq!(canonical_folder(&dotted).unwrap(), real);
        std::fs::write(dir.path().join("file"), b"x").unwrap();
        assert!(canonical_folder(&dir.path().join("file")).is_err());
        assert!(canonical_folder(&dir.path().join("missing")).is_err());
    }
}
