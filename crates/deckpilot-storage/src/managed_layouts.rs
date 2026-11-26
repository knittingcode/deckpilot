//! Managed layouts: layouts a plugin generates and keeps (the Calls
//! plugin's overlay layouts, one per device model; `docs/calls.md`,
//! "Managed layouts").
//!
//! The `provenance` table names items by profile, and a managed layout
//! belongs to no profile, so it gets a table of its own, with the same
//! rules (D2, `docs/action-architecture.md` §3.3):
//!
//! - **Created only when there is none** for the owner and model. Its
//!   layout, its actions and its row are written in one transaction. No
//!   other layout is touched, and a layout without a row is the user's,
//!   whatever its name: it is never adopted.
//! - **Edited** means the canonical JSON of the layout with its actions
//!   differs from the snapshot taken when it was generated.
//! - **A new template version** regenerates an unedited layout in place.
//!   An edited one is never rewritten; the caller offers *Restore defaults*.
//! - **Restore defaults** first keeps the user's version as an ordinary
//!   layout of their own (assigned to no profile, with copies of its
//!   actions), then regenerates.
//! - **Removing** the owner's layouts takes the unedited ones, and the
//!   edited ones only when asked for.
//! - Imports never delete a managed layout, and deleting one as an
//!   ordinary layout is refused.

use std::collections::HashMap;

use deckpilot_core::{ActionInstance, ActionInstanceId, DeviceLayout, LayoutId};
use deckpilot_device::DeviceModelId;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::layouts::{layout_row, library, upsert_action, upsert_layout};
use crate::sqlite::{SqliteStore, now};
use crate::{Result, StorageError};

#[cfg(test)]
#[path = "managed_layouts/tests.rs"]
mod tests;

/// Schema 15: the managed layouts.
pub(crate) const SCHEMA: &str = r"
    CREATE TABLE IF NOT EXISTS managed_layouts (
        layout_id        TEXT PRIMARY KEY NOT NULL REFERENCES layouts (id) ON DELETE RESTRICT,
        owner            TEXT NOT NULL,
        model            TEXT NOT NULL,
        seed_key         TEXT NOT NULL,
        template_version TEXT NOT NULL,
        snapshot         TEXT NOT NULL,
        created_at       INTEGER NOT NULL,
        updated_at       INTEGER NOT NULL,
        UNIQUE (owner, model)
    );
";

/// One managed layout, with its actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedLayout {
    pub owner: String,
    pub model: DeviceModelId,
    pub template_version: String,
    pub layout: DeviceLayout,
    pub actions: Vec<ActionInstance>,
    /// The user changed it since it was generated.
    pub edited: bool,
}

/// What [`SqliteStore::seed_managed_layout`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seeded {
    Created,
    /// It is there and up to date (or edited, of the same version).
    Unchanged,
    /// It was unedited and is generated again.
    Regenerated,
    /// The user edited it and a newer template exists: nothing was
    /// rewritten.
    UpdateAvailable,
}

/// What a snapshot is taken of.
#[derive(Serialize)]
struct Content<'a> {
    layout: &'a DeviceLayout,
    actions: Vec<&'a ActionInstance>,
}

fn snapshot(layout: &DeviceLayout, actions: &[ActionInstance]) -> Result<String> {
    let mut actions: Vec<&ActionInstance> = actions.iter().collect();
    actions.sort_by_key(|a| a.id.to_string());
    crate::provenance::snapshot(&Content { layout, actions })
}

/// Whether layout `id` is a managed layout.
pub(crate) fn is_managed(connection: &Connection, id: &str) -> Result<bool> {
    // Migrations before this one store profiles before the table exists.
    let exists: bool = connection.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'managed_layouts')",
        [],
        |row| row.get(0),
    )?;
    if !exists {
        return Ok(false);
    }
    Ok(connection.query_row(
        "SELECT EXISTS (SELECT 1 FROM managed_layouts WHERE layout_id = ?1)",
        [id],
        |row| row.get(0),
    )?)
}

struct Row {
    layout_id: String,
    model: String,
    template_version: String,
    snapshot: String,
}

fn row(connection: &Connection, owner: &str, model: &str) -> Result<Option<Row>> {
    Ok(connection
        .query_row(
            "SELECT layout_id, model, template_version, snapshot FROM managed_layouts
             WHERE owner = ?1 AND model = ?2",
            params![owner, model],
            |r| {
                Ok(Row {
                    layout_id: r.get(0)?,
                    model: r.get(1)?,
                    template_version: r.get(2)?,
                    snapshot: r.get(3)?,
                })
            },
        )
        .optional()?)
}

/// The layout of `row` with the actions it binds, as stored now.
fn load(connection: &Connection, owner: &str, row: &Row) -> Result<ManagedLayout> {
    let layout = layout_row(connection, &row.layout_id)?.ok_or(StorageError::UnknownLayout)?;
    let ids: Vec<String> = connection
        .prepare("SELECT action_id FROM layout_actions WHERE layout_id = ?1")?
        .query_map([&row.layout_id], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut actions: Vec<ActionInstance> = library(connection)?
        .into_iter()
        .filter(|a| ids.contains(&a.id.to_string()))
        .collect();
    actions.sort_by_key(|a| a.id.to_string());
    let edited = snapshot(&layout, &actions)? != row.snapshot;
    Ok(ManagedLayout {
        owner: owner.to_owned(),
        model: DeviceModelId::new(&row.model),
        template_version: row.template_version.clone(),
        layout,
        actions,
        edited,
    })
}

/// Writes the generated `layout` and `actions` and their row.
fn write(
    tx: &Connection,
    owner: &str,
    layout: &DeviceLayout,
    actions: &[ActionInstance],
    template_version: &str,
    created: bool,
) -> Result<()> {
    let stamp = now();
    for action in actions {
        upsert_action(tx, action, stamp)?;
    }
    upsert_layout(tx, layout, Some(actions), stamp)?;
    let snapshot = snapshot(layout, actions)?;
    if created {
        tx.execute(
            "INSERT INTO managed_layouts
                (layout_id, owner, model, seed_key, template_version, snapshot, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
            params![
                layout.id.to_string(),
                owner,
                layout.model.as_str(),
                format!("{owner}.layout.{}", layout.model.as_str()),
                template_version,
                snapshot,
                stamp
            ],
        )?;
    } else {
        tx.execute(
            "UPDATE managed_layouts SET template_version = ?2, snapshot = ?3, updated_at = ?4
             WHERE layout_id = ?1",
            params![layout.id.to_string(), template_version, snapshot, stamp],
        )?;
    }
    Ok(())
}

impl SqliteStore {
    /// The managed layouts of `owner`, by model.
    pub fn managed_layouts(&self, owner: &str) -> Result<Vec<ManagedLayout>> {
        let connection = self.connection();
        let models: Vec<String> = connection
            .prepare("SELECT model FROM managed_layouts WHERE owner = ?1 ORDER BY model")?
            .query_map([owner], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        models
            .iter()
            .filter_map(|model| row(connection, owner, model).transpose())
            .map(|row| load(connection, owner, &row?))
            .collect()
    }

    /// The managed layout of `owner` for `model`.
    pub fn managed_layout(
        &self,
        owner: &str,
        model: &DeviceModelId,
    ) -> Result<Option<ManagedLayout>> {
        let connection = self.connection();
        row(connection, owner, model.as_str())?
            .map(|row| load(connection, owner, &row))
            .transpose()
    }

    /// Makes sure `owner` has its layout for `layout.model`, generated as
    /// `layout` with `actions` from template `template_version`: created
    /// when there is none, generated again when it is unedited and the
    /// template changed, never rewritten when the user edited it.
    ///
    /// # Errors
    ///
    /// [`StorageError::Invalid`] when a layout of that id exists without
    /// being managed (a layout of the user's is never adopted), or the
    /// layout's actions are not all among `actions`.
    pub fn seed_managed_layout(
        &self,
        owner: &str,
        layout: &DeviceLayout,
        actions: &[ActionInstance],
        template_version: &str,
    ) -> Result<Seeded> {
        check_actions(layout, actions)?;
        let tx = self.write_transaction()?;
        let id = layout.id.to_string();
        let outcome = match row(&tx, owner, layout.model.as_str())? {
            None => {
                if layout_row(&tx, &id)?.is_some() {
                    return Err(StorageError::Invalid(
                        "a layout with this id exists and is not managed: it is never adopted"
                            .into(),
                    ));
                }
                write(&tx, owner, layout, actions, template_version, true)?;
                Seeded::Created
            }
            Some(row) => {
                let stored = load(&tx, owner, &row)?;
                let generated = snapshot(layout, actions)?;
                if stored.edited {
                    if row.template_version == template_version {
                        Seeded::Unchanged
                    } else {
                        Seeded::UpdateAvailable
                    }
                } else if row.snapshot == generated && row.template_version == template_version {
                    Seeded::Unchanged
                } else if row.layout_id == id {
                    write(&tx, owner, layout, actions, template_version, false)?;
                    Seeded::Regenerated
                } else {
                    // A new major version: a new stable id. The old one goes
                    // (it is unedited), the new one is created.
                    remove(&tx, &row.layout_id)?;
                    write(&tx, owner, layout, actions, template_version, true)?;
                    Seeded::Regenerated
                }
            }
        };
        tx.commit()?;
        Ok(outcome)
    }

    /// *Restore defaults*: keeps the user's version of `owner`'s layout for
    /// `layout.model` as an ordinary layout named `copy_name` (with copies
    /// of its actions, assigned to no profile) when it was edited, then
    /// generates it again from `layout` and `actions`. Returns the copy's
    /// id, `None` when there was nothing of the user's to keep.
    pub fn restore_managed_layout(
        &self,
        owner: &str,
        layout: &DeviceLayout,
        actions: &[ActionInstance],
        template_version: &str,
        copy_name: &str,
    ) -> Result<Option<LayoutId>> {
        check_actions(layout, actions)?;
        let tx = self.write_transaction()?;
        let Some(row) = row(&tx, owner, layout.model.as_str())? else {
            return Err(StorageError::UnknownLayout);
        };
        let stored = load(&tx, owner, &row)?;
        let mut kept = None;
        if stored.edited {
            let stamp = now();
            // Every action of the layout's closure (a Multi Action's steps
            // too) is copied, and the copies name each other: nothing of the
            // user's version points at the regenerated actions.
            let map: HashMap<ActionInstanceId, ActionInstanceId> = stored
                .actions
                .iter()
                .map(|action| (action.id, ActionInstanceId::new()))
                .collect();
            let mut copies = Vec::new();
            for action in &stored.actions {
                let mut copy = action.clone();
                copy.id = map[&action.id];
                copy.parameters = remap_parameters(&copy.parameters, &map)?;
                upsert_action(&tx, &copy, stamp)?;
                copies.push(copy);
            }
            let copy = stored.layout.duplicated_with_actions(copy_name, &map);
            upsert_layout(&tx, &copy, Some(&copies), stamp)?;
            kept = Some(copy.id);
        }
        if row.layout_id == layout.id.to_string() {
            write(&tx, owner, layout, actions, template_version, false)?;
        } else {
            remove(&tx, &row.layout_id)?;
            write(&tx, owner, layout, actions, template_version, true)?;
        }
        crate::layouts::collect_garbage(&tx)?;
        tx.commit()?;
        Ok(kept)
    }

    /// Sets the parameters of `action`, one of the actions of `owner`'s
    /// managed layout for `model` (the *Back to work* settings): the
    /// user's edit, like an edit in the inspector, so the layout counts as
    /// edited afterwards (a template update then never rewrites it, and
    /// *Restore defaults* keeps it first). Nothing else changes. The
    /// parameters are stored as given: a caller that changes some keys
    /// merges them into the action's own first (the agent does).
    pub fn set_managed_action_parameters(
        &self,
        owner: &str,
        model: &DeviceModelId,
        action: ActionInstanceId,
        parameters: &deckpilot_core::ActionParameters,
    ) -> Result<()> {
        self.set_managed_actions_parameters(owner, model, &[(action, parameters.clone())])
    }

    /// [`SqliteStore::set_managed_action_parameters`] for several actions
    /// of the layout at once, in one transaction: all of them change, or
    /// (one is not the layout's, the write fails) none does.
    pub fn set_managed_actions_parameters(
        &self,
        owner: &str,
        model: &DeviceModelId,
        changes: &[(ActionInstanceId, deckpilot_core::ActionParameters)],
    ) -> Result<()> {
        let tx = self.write_transaction()?;
        let Some(row) = row(&tx, owner, model.as_str())? else {
            return Err(StorageError::UnknownLayout);
        };
        let stored = load(&tx, owner, &row)?;
        let stamp = now();
        for (action, parameters) in changes {
            let Some(mut updated) = stored.actions.iter().find(|a| a.id == *action).cloned() else {
                return Err(StorageError::Invalid(
                    "the action is not one of the managed layout's".into(),
                ));
            };
            updated.parameters = parameters.clone();
            upsert_action(&tx, &updated, stamp)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Removes `owner`'s managed layouts: the unedited ones, and the
    /// edited ones too with `include_edited`. Returns the removed ids.
    pub fn remove_managed_layouts(
        &self,
        owner: &str,
        include_edited: bool,
    ) -> Result<Vec<LayoutId>> {
        let tx = self.write_transaction()?;
        // Read and filter inside the transaction: an edit made meanwhile is
        // seen.
        let models: Vec<String> = tx
            .prepare("SELECT model FROM managed_layouts WHERE owner = ?1 ORDER BY model")?
            .query_map([owner], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let mut layouts = Vec::new();
        for model in &models {
            if let Some(row) = row(&tx, owner, model)? {
                layouts.push(load(&tx, owner, &row)?);
            }
        }
        let mut removed = Vec::new();
        for managed in layouts.iter().filter(|m| include_edited || !m.edited) {
            remove(&tx, &managed.layout.id.to_string())?;
            removed.push(managed.layout.id);
        }
        crate::layouts::collect_garbage(&tx)?;
        tx.commit()?;
        Ok(removed)
    }
}

/// `parameters` with every id of `map` replaced by its copy (references to
/// actions are their ids, as text, anywhere in the parameters).
fn remap_parameters(
    parameters: &deckpilot_core::ActionParameters,
    map: &HashMap<ActionInstanceId, ActionInstanceId>,
) -> Result<deckpilot_core::ActionParameters> {
    let mut text = serde_json::to_string(parameters)?;
    for (old, new) in map {
        text = text.replace(&old.to_string(), &new.to_string());
    }
    Ok(serde_json::from_str(&text)?)
}

/// Every action the layout binds is one of `actions`.
fn check_actions(layout: &DeviceLayout, actions: &[ActionInstance]) -> Result<()> {
    for id in layout.bound_actions() {
        if !actions.iter().any(|a| a.id == id) {
            return Err(StorageError::Invalid(format!(
                "the managed layout binds the action {id}, which it does not bring"
            )));
        }
    }
    Ok(())
}

/// Deletes a managed layout and its row (its actions go with the garbage
/// collection, unless something else uses them).
fn remove(tx: &Connection, id: &str) -> Result<()> {
    tx.execute("DELETE FROM managed_layouts WHERE layout_id = ?1", [id])?;
    tx.execute("DELETE FROM layouts WHERE id = ?1", [id])?;
    Ok(())
}
