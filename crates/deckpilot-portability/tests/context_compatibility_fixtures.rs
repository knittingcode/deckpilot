//! Action contexts on databases of earlier releases
//! (`docs/action-contexts.md`): with the built-ins' declared contexts, the
//! storage fixtures (0.5.0, 0.6.0, schema 8 and 0.8.1) open, every profile
//! loads, every use outside its action's contexts is reported as stored
//! (and none is refused: nothing in the save path looks at contexts), and
//! saving again keeps every binding, action and workflow byte for byte.

use std::path::{Path, PathBuf};

use deckpilot_core::{
    ActionContext, ActionRegistry, ContextIssue, ContextSite, IncompatibilityReason, Os,
};
use deckpilot_device::ModelCatalog;
use deckpilot_storage::crypto::Password;
use deckpilot_storage::vault::Vault;
use deckpilot_storage::{ConfigStore, Encryption, SqliteStore};
use serde_json::Value;

/// The development password the fixtures were created with (test
/// fixtures, protecting demonstration data only).
const PASSWORD: &str = "deckpilot-dev";

const FIXTURES: [&str; 4] = ["v0.5.0-2", "v0.6.0", "schema8", "v0.8.1"];

struct Copy(PathBuf);

impl Drop for Copy {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn copy(fixture: &str) -> Copy {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../deckpilot-storage/tests/fixtures")
        .join(fixture);
    let mut bytes = [0u8; 8];
    deckpilot_storage::crypto::random_bytes(&mut bytes).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "deckpilot-contexts-{fixture}-{}-{}",
        std::process::id(),
        u64::from_le_bytes(bytes)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    for file in ["deckpilot.db", "deckpilot.vault.json"] {
        std::fs::copy(source.join(file), dir.join(file)).unwrap();
    }
    Copy(dir)
}

fn open(copy: &Copy) -> SqliteStore {
    let database = copy.0.join("deckpilot.db");
    let key = Vault::new(database.clone())
        .unlock(&Password::new(PASSWORD))
        .unwrap();
    SqliteStore::open(database, &Encryption::Key(key)).unwrap()
}

/// Every built-in module enabled, so that every built-in type is checked.
fn registry() -> ActionRegistry {
    let mut modules = deckpilot_modules::builtin(Os::current());
    for info in modules.modules() {
        let _ = modules.set_enabled(&info.manifest.id, true);
    }
    modules.action_registry()
}

/// The stored profile document's actions and workflows, and every binding
/// of its layouts, each serialized.
fn stored(store: &SqliteStore, id: &str) -> Vec<String> {
    let connection = store.connection_for_tests();
    let document: String = connection
        .query_row("SELECT document FROM profiles WHERE id = ?1", [id], |row| {
            row.get(0)
        })
        .unwrap();
    let document: Value = serde_json::from_str(&document).unwrap();
    let layouts: Vec<String> = connection
        .prepare(
            "SELECT l.document FROM profile_layouts pl JOIN layouts l ON l.id = pl.layout_id
             WHERE pl.profile_id = ?1 ORDER BY pl.position",
        )
        .unwrap()
        .query_map([id], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let mut items: Vec<String> = ["actions", "workflows"]
        .iter()
        .flat_map(|key| document[*key].as_array().cloned().unwrap_or_default())
        .map(|item| serde_json::to_string(&item).unwrap())
        .collect();
    for layout in layouts {
        let layout: Value = serde_json::from_str(&layout).unwrap();
        for page in layout["pages"].as_array().into_iter().flatten() {
            for binding in page["bindings"].as_array().into_iter().flatten() {
                items.push(serde_json::to_string(binding).unwrap());
            }
        }
    }
    items
}

fn context(issue: &ContextIssue) -> ActionContext {
    match &issue.reason {
        IncompatibilityReason::ContextUnsupported { context, .. } => *context,
        other => panic!("{other:?}"),
    }
}

#[test]
fn stored_uses_outside_their_contexts_are_reported_as_stored_and_kept() {
    let catalog = ModelCatalog::builtin();
    let registry = registry();
    let mut reported = Vec::new();
    for fixture in FIXTURES {
        let copy = copy(fixture);
        let store = open(&copy);
        for summary in store.list_profiles().unwrap() {
            let id = summary.id.to_string();
            let before = stored(&store, &id);
            let profile = store.load_profile(summary.id).unwrap().unwrap();
            for layout in &profile.layouts {
                for issue in layout.context_issues_for_save(Some(layout), &registry, &profile) {
                    assert!(issue.stored, "{fixture}: {issue}");
                    assert!(matches!(issue.site, ContextSite::Binding { .. }));
                    let name = &profile.action(issue.action).unwrap().action_type;
                    reported.push(format!("{fixture}: {name} {}", context(&issue)));
                }
                // The contexts change nothing a save refuses.
                let (refused, _) =
                    layout.compatibility_for_save(Some(layout), &catalog, &registry, &profile);
                assert_eq!(refused, Vec::new(), "{fixture}");
            }
            for workflow in &profile.workflows {
                for issue in workflow.context_issues(Some(workflow), &profile, &registry) {
                    assert!(issue.stored, "{fixture}: {issue}");
                    let name = &profile.action(issue.action).unwrap().action_type;
                    reported.push(format!("{fixture}: {name} {}", context(&issue)));
                }
            }
            // Saving again keeps every action, workflow and binding byte
            // for byte.
            store.save_profile(&profile).unwrap();
            assert_eq!(stored(&store, &id), before, "{fixture}: {id}");
            assert_eq!(store.load_profile(summary.id).unwrap().unwrap(), profile);
        }
    }
    reported.sort();
    reported.dedup();
    // What the fixtures' seeds bind outside the built-ins' contexts: Log
    // and the workflow summary on keys. Reported, kept, and run as before
    // (with a warning, the first time).
    assert_eq!(
        reported,
        [
            "schema8: general.log key",
            "schema8: notifications.workflow-summary key",
            "v0.5.0-2: general.log key",
            "v0.6.0: general.log key",
            "v0.6.0: notifications.workflow-summary key",
            "v0.8.1: general.log key",
            "v0.8.1: notifications.workflow-summary key",
        ]
    );
}
