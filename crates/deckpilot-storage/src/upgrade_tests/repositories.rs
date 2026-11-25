//! Migration 14 (repositories) on stores of the 0.13.0 and 0.14.0 releases
//! (`tests/fixtures/v0.13.0`, `tests/fixtures/v0.14.0`, schema 13; both
//! seeded by that release's `deckpilot-dev validation seed`, their machine
//! paths replaced with neutral ones).
//!
//! The checks: the repository the `repository` role stands for is derived
//! for every project with that role (key `main`, deterministic id, the
//! default) and for no other; `resources`, the machine configuration and
//! every other row stay byte for byte; a second open is a no-op; the
//! upgrade is backed up and recorded; a failed or killed upgrade changes
//! nothing; and an older build's project upsert keeps the new columns (R5).

use std::collections::BTreeMap;

use rusqlite::params;

use deckpilot_core::{Project, RepositoryId};

use super::*;
use crate::ConfigStore;

const V0_13_0: Fixture = Fixture {
    dir: concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/v0.13.0"),
    version: 13,
    complete: false,
};
const V0_14_0: Fixture = Fixture {
    dir: concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/v0.14.0"),
    version: 13,
    complete: false,
};
const RELEASED: [Fixture; 2] = [V0_13_0, V0_14_0];

/// Adds, as the released version could have stored them, a project with no
/// role and one with roles but no `repository` role, so that 0, 1 and
/// several roles are all present (the seed has projects with one and with
/// several roles, the `repository` role among them).
fn add_projects_without_the_role(copy: &Copy) {
    let connection = raw(&copy.database(), &copy.key);
    let profile: String = connection
        .query_row("SELECT id FROM profiles ORDER BY id LIMIT 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    for (id, key, resources) in [
        ("0b6f3f3e-6c51-4d0e-9a55-2f1f0c1d9a01", "BARE", "{}"),
        (
            "0b6f3f3e-6c51-4d0e-9a55-2f1f0c1d9a02",
            "DOCS",
            r#"{"compose":"docs.compose","docs":"docs.site"}"#,
        ),
    ] {
        connection
            .execute(
                "INSERT INTO projects (id, profile_id, key, name, position, resources)
                 VALUES (?1, ?2, ?3, ?3, 90, ?4)",
                params![id, profile, key, resources],
            )
            .unwrap();
    }
}

/// `(id, repositories, default_repository)` of every project.
fn repository_columns(connection: &Connection) -> Vec<(String, String, Option<String>)> {
    connection
        .prepare("SELECT id, repositories, default_repository FROM projects ORDER BY id")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

#[test]
fn released_stores_get_the_repository_of_their_role_and_nothing_else_changes() {
    for fixture in RELEASED {
        let copy = copy(fixture);
        add_projects_without_the_role(&copy);
        let original = dump(&raw(&copy.database(), &copy.key));
        assert_eq!(original.version, 13);
        let projects = project_columns(&raw(&copy.database(), &copy.key));
        assert!(projects.len() >= 7, "{fixture:?}: {projects:?}");

        let store = copy.open().unwrap();
        let upgraded = dump(store.connection_for_tests());
        assert_eq!(upgraded.version, CURRENT);
        // Ids, keys, names, positions and `resources` (with the
        // `repository` role) are the same, value for value.
        assert_eq!(project_columns(store.connection_for_tests()), projects);
        for (table, (sql, rows)) in &original.tables {
            let (new_sql, new_rows) = &upgraded.tables[table];
            match table.as_str() {
                "projects" => {
                    assert!(new_sql.contains("repositories TEXT NOT NULL DEFAULT '[]'"));
                    assert!(new_sql.contains("default_repository TEXT"));
                }
                "migration_history" => {
                    assert_eq!(new_rows.len(), rows.len() + 1);
                    assert!(rows.iter().all(|row| new_rows.contains(row)));
                }
                // The machine configuration (bindings, paths), the profile
                // documents and everything else: unchanged.
                _ => {
                    assert_eq!(new_sql, sql, "{fixture:?}: the definition of {table}");
                    assert_eq!(new_rows, rows, "{fixture:?}: rows of {table} changed");
                }
            }
        }
        assert_eq!(
            upgraded.tables.keys().collect::<Vec<_>>(),
            original.tables.keys().collect::<Vec<_>>(),
            "no table is added"
        );

        // Recorded and backed up.
        let last = store.migration_history().unwrap().pop().unwrap();
        assert_eq!((last.from_version, last.to_version), (13, CURRENT));
        assert_eq!(last.result, "ok");
        assert!(Path::new(&last.backup.unwrap()).exists());
        assert_eq!(copy.backup_files().len(), 1);
        assert_backup_of(&copy, &copy.backup_files()[0], &original);

        // Each project with the role has exactly its derived repository,
        // as the default; the others have none.
        let mut with_role = 0;
        let mut without = 0;
        for summary in store.list_profiles().unwrap() {
            let profile = store.load_profile(summary.id).unwrap().unwrap();
            for project in &profile.projects {
                let roles = &project.resources;
                if let Some(resource) = roles.get("repository") {
                    with_role += 1;
                    assert_eq!(project.repositories.len(), 1, "{}", project.key);
                    let repository = &project.repositories[0];
                    assert_eq!(repository.id, RepositoryId::derived(project.id));
                    assert_eq!(repository.key.as_str(), "main");
                    assert_eq!(repository.name, project.name);
                    assert_eq!(&repository.resource, resource);
                    assert_eq!(repository.description, None);
                    assert_eq!(repository.remotes, []);
                    assert_eq!(project.default_repository, Some(repository.id));
                    // What R3 would derive is what was stored.
                    assert_eq!(project.derived_repository().as_ref(), Some(repository));
                } else {
                    without += 1;
                    assert_eq!(project.repositories, [], "{}", project.key);
                    assert_eq!(project.default_repository, None);
                }
                project.validate_repositories().unwrap();
            }
            assert_eq!(
                profile.validate(&deckpilot_device::ModelCatalog::builtin()),
                []
            );
        }
        assert!(with_role >= 5 && without >= 2, "{with_role} / {without}");
        drop(store);

        // Opening again writes nothing.
        for _ in 0..2 {
            let store = copy.open().unwrap();
            assert!(
                dump(store.connection_for_tests()) == upgraded,
                "{fixture:?}: reopening changed the database"
            );
        }
        assert_eq!(copy.backup_files().len(), 1);
    }
}

#[test]
fn the_derivation_is_the_same_on_every_upgrade() {
    for fixture in RELEASED {
        let columns: Vec<_> = (0..2)
            .map(|_| {
                let copy = copy(fixture);
                let store = copy.open().unwrap();
                repository_columns(store.connection_for_tests())
            })
            .collect();
        assert_eq!(columns[0], columns[1], "{fixture:?}");
        assert!(
            columns[0]
                .iter()
                .any(|(_, repositories, _)| repositories != "[]")
        );
    }
}

#[test]
fn a_failed_or_killed_upgrade_of_a_released_store_changes_nothing() {
    for fixture in RELEASED {
        for point in [Failpoint::Migration(14), Failpoint::Commit] {
            let copy = copy(fixture);
            let original = dump(&raw(&copy.database(), &copy.key));
            let failed = failing_at(point, || copy.open());
            assert!(failed.is_err(), "{fixture:?} {point:?}");
            assert_untouched(&copy, &original);
            assert_next_start_upgrades(&copy);

            let copy = super::copy(fixture);
            let original = dump(&raw(&copy.database(), &copy.key));
            kill_upgrade_at(&copy, point);
            assert_untouched(&copy, &original);
            assert_next_start_upgrades(&copy);
        }
    }
}

/// R5: an older build refuses the newer schema, but if one wrote the
/// projects table anyway, its upsert names its columns, so the new ones
/// survive.
#[test]
fn an_older_builds_project_upsert_keeps_the_repositories() {
    let copy = copy(V0_14_0);
    let store = copy.open().unwrap();
    let before = repository_columns(store.connection_for_tests());
    let (id, profile, key, name, position, resources): (
        String,
        String,
        String,
        String,
        i64,
        String,
    ) = store
        .connection_for_tests()
        .query_row(
            "SELECT id, profile_id, key, name, position, resources FROM projects
                 WHERE repositories != '[]' ORDER BY id LIMIT 1",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .unwrap();
    // The statement of 0.14.0's `save_projects`, renaming the project.
    store
        .connection_for_tests()
        .execute(
            "INSERT INTO projects (id, profile_id, key, name, position, resources)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (id) DO UPDATE SET
                key = excluded.key,
                name = excluded.name,
                position = excluded.position,
                resources = excluded.resources
             WHERE projects.profile_id = excluded.profile_id",
            params![
                id,
                profile,
                key,
                format!("{name} (renamed)"),
                position,
                resources
            ],
        )
        .unwrap();
    assert_eq!(repository_columns(store.connection_for_tests()), before);
}

/// After the upgrade the repositories are the project's own: saving keeps
/// them, a rename of the project does not rename them, and a project saved
/// with several repositories reads back as it was.
#[test]
fn upgraded_repositories_are_kept_and_edited_like_any_other() {
    let copy = copy(V0_13_0);
    let store = copy.open().unwrap();
    let (summary, mut profile, index) = store
        .list_profiles()
        .unwrap()
        .into_iter()
        .find_map(|summary| {
            let profile = store.load_profile(summary.id).unwrap().unwrap();
            let index = profile
                .projects
                .iter()
                .position(|p| !p.repositories.is_empty())?;
            Some((summary, profile, index))
        })
        .unwrap();
    let derived = profile.projects[index].repositories[0].clone();
    profile.projects[index].name = "Renamed".into();
    store.save_profile(&profile).unwrap();
    let loaded = store.load_profile(summary.id).unwrap().unwrap();
    assert_eq!(
        loaded.projects[index].repositories,
        std::slice::from_ref(&derived)
    );

    let project: &mut Project = &mut profile.projects[index];
    let mut cms = derived.clone();
    cms.id = RepositoryId::new();
    cms.key = deckpilot_core::RepositoryKey::new("cms").unwrap();
    cms.name = "CMS".into();
    cms.resource = deckpilot_core::ResourceKey::new("shop.cms").unwrap();
    project.repositories.push(cms.clone());
    let roles: BTreeMap<_, _> = project.resources.clone();
    project.set_default_repository(Some(cms.id)).unwrap();
    store.save_profile(&profile).unwrap();
    let loaded = store.load_profile(summary.id).unwrap().unwrap();
    let project = &loaded.projects[index];
    assert_eq!(project.repositories, [derived, cms.clone()]);
    assert_eq!(project.default_repository, Some(cms.id));
    // R2: only the `repository` role followed the default.
    let mut expected = roles;
    expected.insert("repository".into(), cms.resource.clone());
    assert_eq!(project.resources, expected);
}
