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

use serde::{Deserialize, Serialize};
use tauri::State;

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
pub fn create_script(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    script: NewScript,
) -> CommandResult<Script> {
    let mut new = Script::new(script.name.trim(), script.source);
    if let Some(settings) = script.settings {
        new.settings = settings;
    }
    Ok(state.store()?.create_script(profile_id, &new)?)
}

/// The editor's save of `script` (made from its `revision`).
#[tauri::command]
pub fn save_script(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    script: Script,
) -> CommandResult<Script> {
    let script = Script {
        name: script.name.trim().to_owned(),
        ..script
    };
    Ok(state.store()?.save_script(profile_id, &script)?)
}

#[tauri::command]
pub fn duplicate_script(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    script_id: ScriptId,
) -> CommandResult<Script> {
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
pub fn delete_script(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    script_id: ScriptId,
    clear_keys: bool,
) -> CommandResult<()> {
    script_of(&state, profile_id, script_id)?;
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
    drop(store);
    crate::agent::notify_reload(&state.paths);
    Ok(())
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
    Ok(ScriptReview {
        trust: ScriptTrust::state(record.as_ref(), &script),
        source_sha256: source_sha256(&script.source),
        grants: record.map(|r| r.grants).unwrap_or_default(),
        script,
    })
}

/// "Trust this script": the user reviewed the source with this hash and
/// the grants listed with it.
#[tauri::command]
pub fn trust_script(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    script_id: ScriptId,
    source_sha256: String,
    grants: ScriptGrants,
) -> CommandResult<()> {
    Ok(state
        .store()?
        .trust_script(profile_id, script_id, &source_sha256, &grants)?)
}

/// The Permissions tab's save.
#[tauri::command]
pub fn set_script_grants(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    script_id: ScriptId,
    grants: ScriptGrants,
) -> CommandResult<()> {
    Ok(state
        .store()?
        .set_script_grants(profile_id, script_id, &grants)?)
}

/// Revokes every grant of the script (it stays trusted).
#[tauri::command]
pub fn revoke_script_grants(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    script_id: ScriptId,
) -> CommandResult<()> {
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
