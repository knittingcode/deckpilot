//! Schema migration 14: the repositories of a project.
//!
//! Additive. Two columns are added to `projects` ([`SCHEMA`]): the ordered
//! repository list (JSON, like `resources`) and the default repository.
//! Then every project whose `resources` has the `repository` role gets the
//! one repository that role stands for ([`Project::derived_repository`]):
//! key `main`, the project's name, the role's resource key, and an id
//! derived from the project id, so this migration, a later derivation
//! (rule R3) and an import agree on it. It becomes the project's default.
//!
//! Nothing else is written: `resources` (and with it the `repository`
//! role), the machine bindings, the profile documents, action parameters
//! and every other row stay exactly as they were. A row this build cannot
//! read (it would not load either) is left as it is.

use std::collections::BTreeMap;

use rusqlite::{Connection, params};

use deckpilot_core::{Project, ProjectId, ProjectKey, ResourceKey};

use crate::Result;

/// Repositories live with their project, read and written whole, like its
/// resources: a column, not a table.
pub(crate) const SCHEMA: &str = r"
    ALTER TABLE projects ADD COLUMN repositories TEXT NOT NULL DEFAULT '[]';
    ALTER TABLE projects ADD COLUMN default_repository TEXT;
";

/// The project a stored row describes, as far as the derivation needs it;
/// `None` when the row does not parse.
fn project_of(id: &str, key: &str, name: &str, resources: &str) -> Option<Project> {
    let mut project = Project::new(ProjectKey::new(key).ok()?, name);
    project.id = id.parse::<ProjectId>().ok()?;
    project.resources = serde_json::from_str::<BTreeMap<String, ResourceKey>>(resources).ok()?;
    Some(project)
}

pub(crate) fn migrate(connection: &Connection) -> Result<()> {
    let rows: Vec<(String, String, String, String)> = connection
        .prepare(
            "SELECT id, key, name, resources FROM projects
             WHERE repositories = '[]' ORDER BY id",
        )?
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    for (id, key, name, resources) in rows {
        let Some(derived) =
            project_of(&id, &key, &name, &resources).and_then(|p| p.derived_repository())
        else {
            continue;
        };
        connection.execute(
            "UPDATE projects SET repositories = ?1, default_repository = ?2 WHERE id = ?3",
            params![
                crate::sqlite::encode(&[&derived])?,
                derived.id.to_string(),
                id
            ],
        )?;
    }
    Ok(())
}
