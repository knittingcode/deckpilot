//! Migration 15 (managed layouts, `managed_layouts.rs`) on stores of the
//! 0.13.0 and 0.14.0 releases (schema 13, through migration 14) and on a
//! schema 14 store: it adds one empty table and changes no other table or
//! row; the upgrade is backed up and recorded. Migration 16 (the scripts
//! tables, `scripts.rs`) runs after it in the same upgrade and adds two
//! more empty tables.

use super::*;

const RELEASED: [Fixture; 2] = [repositories::V0_13_0, repositories::V0_14_0];

fn tables(dump: &Dump) -> Vec<&String> {
    dump.tables.keys().collect()
}

#[test]
fn released_stores_get_an_empty_managed_layouts_table() {
    for fixture in RELEASED {
        let copy = copy(fixture);
        let original = dump(&raw(&copy.database(), &copy.key));
        let store = copy.open().unwrap();
        let upgraded = dump(store.connection_for_tests());
        assert_eq!(upgraded.version, CURRENT);
        let (sql, rows) = &upgraded.tables["managed_layouts"];
        assert!(
            sql.contains("REFERENCES layouts (id) ON DELETE RESTRICT"),
            "{sql}"
        );
        assert_eq!(rows.len(), 0, "{fixture:?}: no layout is adopted");
        let added = ["managed_layouts", "script_trust", "scripts"].map(str::to_owned);
        let mut expected = tables(&original);
        expected.extend(&added);
        expected.sort();
        assert_eq!(
            tables(&upgraded),
            expected,
            "{fixture:?}: the managed layouts and the scripts tables added"
        );
        for table in ["script_trust", "scripts"] {
            assert!(upgraded.tables[table].1.is_empty(), "{fixture:?}: {table}");
        }
        // The layouts, their actions and the profiles are as they were.
        for table in [
            "layouts",
            "library_actions",
            "layout_actions",
            "profiles",
            "profile_layouts",
        ] {
            assert_eq!(
                upgraded.tables[table], original.tables[table],
                "{fixture:?}: {table}"
            );
        }
    }
}

/// A store at schema 14 (what a build with migration 14 and without 15
/// wrote) upgrades with migrations 15 and 16, and nothing else changes.
#[test]
fn a_schema_14_store_gets_migrations_15_and_16() {
    let copy = copy(repositories::V0_14_0);
    // Upgrade once, then take the store back to what schema 14 is: the
    // managed layouts and the scripts tables did not exist.
    drop(copy.open().unwrap());
    {
        let connection = raw(&copy.database(), &copy.key);
        connection
            .execute_batch("DROP TABLE script_trust; DROP TABLE scripts; DROP TABLE managed_layouts; PRAGMA user_version = 14;")
            .unwrap();
    }
    let at_14 = dump(&raw(&copy.database(), &copy.key));
    assert_eq!(at_14.version, 14);
    let store = copy.open().unwrap();
    let upgraded = dump(store.connection_for_tests());
    assert_eq!(upgraded.version, CURRENT);
    for (table, (sql, rows)) in &at_14.tables {
        let (new_sql, new_rows) = &upgraded.tables[table];
        assert_eq!(new_sql, sql, "the definition of {table} changed");
        if table == "migration_history" {
            assert_eq!(new_rows.len(), rows.len() + 1);
        } else {
            assert_eq!(new_rows, rows, "rows of {table} changed");
        }
    }
    assert_eq!(upgraded.tables["managed_layouts"].1.len(), 0);
    assert_eq!(upgraded.tables["script_trust"].1.len(), 0);
    assert_eq!(upgraded.tables["scripts"].1.len(), 0);
    let last = store.migration_history().unwrap().pop().unwrap();
    assert_eq!((last.from_version, last.to_version), (14, CURRENT));
    assert_eq!(last.result, "ok");
}

/// A failed or killed upgrade at migration 15 (after 14 ran in the same
/// transaction) changes nothing, and the next start upgrades.
#[test]
fn a_failed_or_killed_migration_15_changes_nothing() {
    for fixture in RELEASED {
        let point = Failpoint::Migration(15);
        let copy = copy(fixture);
        let original = dump(&raw(&copy.database(), &copy.key));
        let failed = failing_at(point, || copy.open());
        assert!(failed.is_err(), "{fixture:?}");
        assert_untouched(&copy, &original);
        assert_next_start_upgrades(&copy);

        let copy = super::copy(fixture);
        let original = dump(&raw(&copy.database(), &copy.key));
        kill_upgrade_at(&copy, point);
        assert_untouched(&copy, &original);
        assert_next_start_upgrades(&copy);
    }
}
