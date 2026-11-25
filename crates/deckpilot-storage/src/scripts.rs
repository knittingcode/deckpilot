//! Schema migration 16: the scripts of a profile and their machine-local
//! trust (`deckpilot_core::script`, `docs/scripts-bridge.md`).
//!
//! Additive: two new, empty tables. `scripts` is portable and goes with its
//! profile. `script_trust` is machine-local: the source hash the user
//! trusted, when, and the script's grants. It is never exported, never
//! synced and never written by an import; only the app's own commands
//! below change it.
//!
//! The trust lifecycle (`deckpilot_core::script`):
//! - [`SqliteStore::create_script`]: trusted, bound to its hash, no grants.
//! - [`SqliteStore::save_script`] (the editor): a trusted script is bound to
//!   its new hash and keeps its grants; one that is not trusted stays so.
//! - [`SqliteStore::put_untrusted_script`] (an import, an overwrite): not
//!   trusted and with no grants (an existing record is removed);
//!   [`SqliteStore::duplicate_script`]: not trusted, no grants.
//! - [`SqliteStore::trust_script`]: the user reviewed the source with this
//!   hash; refused when the stored source has another.

use rusqlite::{OptionalExtension, params};

use deckpilot_core::script::{
    MAX_SCRIPTS, Script, ScriptSettings, ScriptTrust, TrustState, source_sha256,
};
use deckpilot_core::script_grants::ScriptGrants;
use deckpilot_core::{ActionInstanceId, ParameterValue, Profile, ProfileId, ScriptId};

use crate::references::{ActionReference, action_references};
use crate::sqlite::{SqliteStore, constraint_violation, now, parse_id};
use crate::{Result, StorageError};

pub(crate) const SCHEMA: &str = r"
    CREATE TABLE scripts (
        profile_id TEXT NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
        id         TEXT NOT NULL,
        name       TEXT NOT NULL,
        source     TEXT NOT NULL,
        settings   TEXT NOT NULL,
        revision   INTEGER NOT NULL CHECK (revision >= 1),
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        PRIMARY KEY (profile_id, id)
    );
    CREATE UNIQUE INDEX scripts_name ON scripts (profile_id, name COLLATE NOCASE);
    CREATE TABLE script_trust (
        profile_id    TEXT NOT NULL,
        script_id     TEXT NOT NULL,
        source_sha256 TEXT CHECK (source_sha256 IS NULL OR length(source_sha256) = 64),
        reviewed_at   INTEGER,
        grants        TEXT NOT NULL,
        PRIMARY KEY (profile_id, script_id),
        FOREIGN KEY (profile_id, script_id) REFERENCES scripts (profile_id, id) ON DELETE CASCADE
    );
";

/// The action type that runs a script.
pub const RUN_ACTION: &str = "deckpilot.scripts.run";

/// A script as the Scripts page lists it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredScript {
    pub script: Script,
    pub trust: TrustState,
    /// Its grants (dormant while it is not trusted), for the Permissions
    /// tab; `None` when it has none or the record is invalid.
    pub grants: Option<ScriptGrants>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// What a run of a script gets ([`SqliteStore::script_for_run`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptForRun {
    /// Trusted: it runs, with these grants (none when its record is
    /// invalid).
    Runnable {
        script: Box<Script>,
        grants: Box<ScriptGrants>,
    },
    /// Not trusted: it must not run until the user reviews it.
    NeedsReview,
}

/// Where a script is used: an action that runs it, and that action's
/// usages.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptUsage {
    pub action: ActionInstanceId,
    pub action_name: String,
    pub references: Vec<ActionReference>,
}

fn invalid(error: impl std::fmt::Display) -> StorageError {
    StorageError::Invalid(error.to_string())
}

fn row_script(row: &rusqlite::Row<'_>) -> rusqlite::Result<(Script, i64, i64)> {
    let settings: String = row.get(3)?;
    let revision: i64 = row.get(4)?;
    Ok((
        Script {
            id: parse_id(&row.get::<_, String>(0)?)?,
            name: row.get(1)?,
            source: row.get(2)?,
            settings: serde_json::from_str::<ScriptSettings>(&settings).unwrap_or_default(),
            revision: u64::try_from(revision).unwrap_or(1),
        },
        row.get(5)?,
        row.get(6)?,
    ))
}

const SELECT: &str =
    "SELECT id, name, source, settings, revision, created_at, updated_at FROM scripts";

fn trust_of(
    connection: &rusqlite::Connection,
    profile: ProfileId,
    id: ScriptId,
) -> Result<Option<ScriptTrust>> {
    let row: Option<(Option<String>, Option<i64>, String)> = connection
        .query_row(
            "SELECT source_sha256, reviewed_at, grants FROM script_trust
             WHERE profile_id = ?1 AND script_id = ?2",
            params![profile.to_string(), id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    Ok(row.map(|(hash, reviewed_at, grants)| ScriptTrust {
        source_sha256: hash,
        reviewed_at,
        // A record that does not parse grants nothing (never a partial
        // grant); the trust still says whether the script may run.
        grants: serde_json::from_str(&grants).unwrap_or_default(),
    }))
}

fn write_trust(
    tx: &rusqlite::Transaction<'_>,
    profile: ProfileId,
    id: ScriptId,
    trust: &ScriptTrust,
) -> Result<()> {
    tx.execute(
        "INSERT INTO script_trust (profile_id, script_id, source_sha256, reviewed_at, grants)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (profile_id, script_id) DO UPDATE SET
             source_sha256 = excluded.source_sha256,
             reviewed_at = excluded.reviewed_at,
             grants = excluded.grants",
        params![
            profile.to_string(),
            id.to_string(),
            trust.source_sha256,
            trust.reviewed_at,
            serde_json::to_string(&trust.grants)?,
        ],
    )?;
    Ok(())
}

/// A constraint the name check above did not catch: an id (or a name
/// NOCASE folds) that is taken.
fn name_taken(error: &rusqlite::Error, name: &str) -> Option<StorageError> {
    constraint_violation(error).then(|| {
        StorageError::Invalid(format!(
            "a script like “{name}” already exists in this profile"
        ))
    })
}

/// A name not used by `taken` (without case): `base`, or `base (copy)`,
/// `base (copy 2)`, …, cut to the name limit.
fn copy_name(base: &str, taken: &[String]) -> String {
    let used = |n: &str| taken.iter().any(|t| t.to_lowercase() == n.to_lowercase());
    let cut = |suffix: &str| {
        let room = deckpilot_core::script::MAX_NAME.saturating_sub(suffix.chars().count());
        let head: String = base.chars().take(room).collect();
        format!("{}{suffix}", head.trim_end())
    };
    let mut candidate = cut(" (copy)");
    let mut n = 2;
    while used(&candidate) {
        candidate = cut(&format!(" (copy {n})"));
        n += 1;
    }
    candidate
}

impl SqliteStore {
    fn require_profile(&self, profile: ProfileId) -> Result<()> {
        let exists: bool = self.connection().query_row(
            "SELECT EXISTS (SELECT 1 FROM profiles WHERE id = ?1)",
            [profile.to_string()],
            |row| row.get(0),
        )?;
        if exists {
            Ok(())
        } else {
            Err(StorageError::UnknownProfile)
        }
    }

    /// The scripts of `profile`, by name.
    pub fn scripts(&self, profile: ProfileId) -> Result<Vec<StoredScript>> {
        let rows = self
            .connection()
            .prepare(&format!(
                "{SELECT} WHERE profile_id = ?1 ORDER BY name COLLATE NOCASE, id"
            ))?
            .query_map([profile.to_string()], row_script)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(script, created_at, updated_at)| {
                let trust = trust_of(self.connection(), profile, script.id)?;
                Ok(StoredScript {
                    trust: ScriptTrust::state(trust.as_ref(), &script),
                    grants: trust
                        .map(|t| t.grants)
                        .filter(|g| g.validate().is_ok() && *g != ScriptGrants::default()),
                    script,
                    created_at,
                    updated_at,
                })
            })
            .collect()
    }

    /// One script of `profile`.
    pub fn script(&self, profile: ProfileId, id: ScriptId) -> Result<Option<Script>> {
        Ok(self
            .connection()
            .query_row(
                &format!("{SELECT} WHERE profile_id = ?1 AND id = ?2"),
                params![profile.to_string(), id.to_string()],
                row_script,
            )
            .optional()?
            .map(|(script, ..)| script))
    }

    /// What a run of the script gets (`None`: there is no such script):
    /// its source, settings and grants only while it is trusted.
    pub fn script_for_run(&self, profile: ProfileId, id: ScriptId) -> Result<Option<ScriptForRun>> {
        let Some(script) = self.script(profile, id)? else {
            return Ok(None);
        };
        let trust = trust_of(self.connection(), profile, id)?;
        if ScriptTrust::state(trust.as_ref(), &script) != TrustState::Trusted {
            return Ok(Some(ScriptForRun::NeedsReview));
        }
        // Trusted with an invalid record: it runs with none.
        let grants = ScriptTrust::effective_grants(trust.as_ref(), &script).unwrap_or_default();
        Ok(Some(ScriptForRun::Runnable {
            script: Box::new(script),
            grants: Box::new(grants),
        }))
    }

    /// A new script from the editor: trusted (its author is the user),
    /// with no grants.
    pub fn create_script(&self, profile: ProfileId, script: &Script) -> Result<Script> {
        script.validate().map_err(invalid)?;
        self.require_profile(profile)?;
        let tx = self.write_transaction()?;
        let stored = Script {
            revision: 1,
            ..script.clone()
        };
        insert(&tx, profile, &stored)?;
        write_trust(
            &tx,
            profile,
            stored.id,
            &ScriptTrust {
                source_sha256: Some(stored.source_sha256()),
                reviewed_at: Some(now()),
                grants: ScriptGrants::default(),
            },
        )?;
        tx.commit()?;
        Ok(stored)
    }

    /// An editor save of `script` (name, source, settings), made from
    /// revision `script.revision`; refused when the stored one is newer.
    /// A trusted script is bound to its new source and keeps its grants;
    /// one that is not trusted stays so (its review comes first).
    pub fn save_script(&self, profile: ProfileId, script: &Script) -> Result<Script> {
        script.validate().map_err(invalid)?;
        let tx = self.write_transaction()?;
        let Some((stored, ..)) = tx
            .query_row(
                &format!("{SELECT} WHERE profile_id = ?1 AND id = ?2"),
                params![profile.to_string(), script.id.to_string()],
                row_script,
            )
            .optional()?
        else {
            return Err(StorageError::UnknownScript);
        };
        if stored.revision != script.revision {
            return Err(StorageError::ScriptRevision(stored.revision));
        }
        let trust = trust_of(&tx, profile, script.id)?;
        let was_trusted = ScriptTrust::state(trust.as_ref(), &stored) == TrustState::Trusted;
        let saved = Script {
            revision: stored.revision + 1,
            ..script.clone()
        };
        check_name(&tx, profile, &saved)?;
        tx.execute(
            "UPDATE scripts SET name = ?3, source = ?4, settings = ?5, revision = ?6,
                 updated_at = ?7
             WHERE profile_id = ?1 AND id = ?2",
            params![
                profile.to_string(),
                saved.id.to_string(),
                saved.name,
                saved.source,
                serde_json::to_string(&saved.settings)?,
                i64::try_from(saved.revision).unwrap_or(i64::MAX),
                now(),
            ],
        )
        .map_err(|e| name_taken(&e, &saved.name).unwrap_or(e.into()))?;
        if was_trusted && let Some(mut trust) = trust {
            trust.source_sha256 = Some(saved.source_sha256());
            write_trust(&tx, profile, saved.id, &trust)?;
        }
        tx.commit()?;
        Ok(saved)
    }

    /// A script that did not come from this computer's editor (an import,
    /// an overwrite by one): created or replaced, never trusted, with no
    /// grants (an existing record is removed).
    pub fn put_untrusted_script(&self, profile: ProfileId, script: &Script) -> Result<()> {
        script.validate().map_err(invalid)?;
        self.require_profile(profile)?;
        let tx = self.write_transaction()?;
        put_untrusted(&tx, profile, script)?;
        tx.commit()?;
        Ok(())
    }

    /// A copy of the script: a new id, the name "… (copy)", the same
    /// source and settings, and no trust and no grants.
    pub fn duplicate_script(&self, profile: ProfileId, id: ScriptId) -> Result<Script> {
        let Some(original) = self.script(profile, id)? else {
            return Err(StorageError::UnknownScript);
        };
        let taken: Vec<String> = self
            .scripts(profile)?
            .into_iter()
            .map(|s| s.script.name)
            .collect();
        let copy = Script {
            id: ScriptId::new(),
            name: copy_name(&original.name, &taken),
            revision: 1,
            ..original
        };
        let tx = self.write_transaction()?;
        insert(&tx, profile, &copy)?;
        tx.commit()?;
        Ok(copy)
    }

    /// Deletes the script and its trust. Its bindings are kept and show
    /// "Script missing"; the app clears them first when the user asks.
    pub fn delete_script(&self, profile: ProfileId, id: ScriptId) -> Result<()> {
        let tx = self.write_transaction()?;
        let removed = tx.execute(
            "DELETE FROM scripts WHERE profile_id = ?1 AND id = ?2",
            params![profile.to_string(), id.to_string()],
        )?;
        if removed == 0 {
            return Err(StorageError::UnknownScript);
        }
        tx.commit()?;
        Ok(())
    }

    /// The user reviewed the script's source, whose hash is `reviewed`,
    /// and its dormant grants, `shown` (the review lists them), and trusts
    /// it. Refused when the stored source has another hash or the grants
    /// are not the ones shown (either changed after the review): dormant
    /// grants never come back unseen.
    pub fn trust_script(
        &self,
        profile: ProfileId,
        id: ScriptId,
        reviewed: &str,
        shown: &ScriptGrants,
    ) -> Result<()> {
        let tx = self.write_transaction()?;
        let Some(script) = tx
            .query_row(
                &format!("{SELECT} WHERE profile_id = ?1 AND id = ?2"),
                params![profile.to_string(), id.to_string()],
                row_script,
            )
            .optional()?
            .map(|(s, ..)| s)
        else {
            return Err(StorageError::UnknownScript);
        };
        if source_sha256(&script.source) != reviewed {
            return Err(StorageError::ScriptChanged);
        }
        let mut trust = trust_of(&tx, profile, id)?.unwrap_or_default();
        if trust.grants != *shown {
            return Err(StorageError::ScriptChanged);
        }
        trust.source_sha256 = Some(reviewed.to_owned());
        trust.reviewed_at = Some(now());
        write_trust(&tx, profile, id, &trust)?;
        tx.commit()?;
        Ok(())
    }

    /// Sets the script's grants (the Permissions tab). Only a trusted
    /// script's: an untrusted one is reviewed first. The grants are
    /// checked whole.
    pub fn set_script_grants(
        &self,
        profile: ProfileId,
        id: ScriptId,
        grants: &ScriptGrants,
    ) -> Result<()> {
        grants.validate().map_err(invalid)?;
        let tx = self.write_transaction()?;
        let Some(script) = tx
            .query_row(
                &format!("{SELECT} WHERE profile_id = ?1 AND id = ?2"),
                params![profile.to_string(), id.to_string()],
                row_script,
            )
            .optional()?
            .map(|(s, ..)| s)
        else {
            return Err(StorageError::UnknownScript);
        };
        let Some(mut trust) = trust_of(&tx, profile, id)? else {
            return Err(StorageError::ScriptNotTrusted);
        };
        if ScriptTrust::state(Some(&trust), &script) != TrustState::Trusted {
            return Err(StorageError::ScriptNotTrusted);
        }
        trust.grants = grants.clone();
        write_trust(&tx, profile, id, &trust)?;
        tx.commit()?;
        Ok(())
    }
}

/// Refuses a name another script of the profile has (without case, also
/// beyond ASCII, which the index's NOCASE does not fold).
fn check_name(tx: &rusqlite::Transaction<'_>, profile: ProfileId, script: &Script) -> Result<()> {
    let wanted = script.name.to_lowercase();
    let mut statement = tx.prepare("SELECT id, name FROM scripts WHERE profile_id = ?1")?;
    let clash = statement
        .query_map([profile.to_string()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .any(|(id, name)| id != script.id.to_string() && name.to_lowercase() == wanted);
    if clash {
        return Err(StorageError::ScriptNameTaken(script.name.clone()));
    }
    Ok(())
}

fn insert(tx: &rusqlite::Transaction<'_>, profile: ProfileId, script: &Script) -> Result<()> {
    let count: i64 = tx.query_row(
        "SELECT COUNT(*) FROM scripts WHERE profile_id = ?1",
        [profile.to_string()],
        |row| row.get(0),
    )?;
    if usize::try_from(count).unwrap_or(usize::MAX) >= MAX_SCRIPTS {
        return Err(StorageError::Invalid(format!(
            "a profile holds at most {MAX_SCRIPTS} scripts"
        )));
    }
    check_name(tx, profile, script)?;
    let at = now();
    tx.execute(
        "INSERT INTO scripts (profile_id, id, name, source, settings, revision, created_at,
             updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
        params![
            profile.to_string(),
            script.id.to_string(),
            script.name,
            script.source,
            serde_json::to_string(&script.settings)?,
            i64::try_from(script.revision.max(1)).unwrap_or(i64::MAX),
            at,
        ],
    )
    .map_err(|e| name_taken(&e, &script.name).unwrap_or(e.into()))?;
    Ok(())
}

/// [`SqliteStore::put_untrusted_script`] in a transaction (an import
/// applies it with the rest of its changes).
pub(crate) fn put_untrusted(
    tx: &rusqlite::Transaction<'_>,
    profile: ProfileId,
    script: &Script,
) -> Result<()> {
    let revision: Option<i64> = tx
        .query_row(
            "SELECT revision FROM scripts WHERE profile_id = ?1 AND id = ?2",
            params![profile.to_string(), script.id.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    match revision {
        None => insert(
            tx,
            profile,
            &Script {
                revision: 1,
                ..script.clone()
            },
        )?,
        Some(revision) => {
            check_name(tx, profile, script)?;
            tx.execute(
                "UPDATE scripts SET name = ?3, source = ?4, settings = ?5, revision = ?6,
                     updated_at = ?7
                 WHERE profile_id = ?1 AND id = ?2",
                params![
                    profile.to_string(),
                    script.id.to_string(),
                    script.name,
                    script.source,
                    serde_json::to_string(&script.settings)?,
                    revision.saturating_add(1),
                    now(),
                ],
            )
            .map_err(|e| name_taken(&e, &script.name).unwrap_or(e.into()))?;
        }
    }
    // Never trusted, whatever the source, and with no grants: a script
    // from elsewhere never inherits what a local one was allowed.
    tx.execute(
        "DELETE FROM script_trust WHERE profile_id = ?1 AND script_id = ?2",
        params![profile.to_string(), script.id.to_string()],
    )?;
    Ok(())
}

/// Every action of `profile` that runs script `id`, with its usages (the
/// guarded delete lists them).
pub fn script_usages(profile: &Profile, id: ScriptId) -> Vec<ScriptUsage> {
    let needle = id.to_string();
    profile
        .actions
        .iter()
        .filter(|a| a.action_type.as_str() == RUN_ACTION)
        .filter(|a| {
            matches!(
                a.parameters.get("script"),
                Some(ParameterValue::Text(v) | ParameterValue::Choice(v)) if *v == needle
            )
        })
        .map(|a| ScriptUsage {
            action: a.id,
            action_name: a.name.clone(),
            references: action_references(profile, a.id),
        })
        .collect()
}

#[cfg(test)]
mod tests;
