//! Workflows as things keys refer to (`docs/data-model.md`, "Workflows")
//! on databases of earlier releases: the storage fixtures (0.5.0, 0.6.0,
//! schema 8 and 0.8.1) open, every profile loads, every workflow's
//! references are found (at least the keys the old count had), every
//! stored key that starts a workflow is kept on a save (none is refused as
//! new, with the Workflows plugin on or off), the device faces of every
//! page are drawn, and saving again keeps every binding, action and
//! workflow byte for byte: existing bindings map 1:1, nothing migrates.

use std::path::{Path, PathBuf};

use deckpilot_core::{ActionRegistry, BindingTarget, Os, WorkflowScope};
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
        "deckpilot-workflow-refs-{fixture}-{}-{}",
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

#[test]
fn stored_workflow_keys_load_and_save_unchanged() {
    let catalog = ModelCatalog::builtin();
    let registry = registry();
    let mut workflow_keys = 0;
    for fixture in FIXTURES {
        let copy = copy(fixture);
        let store = open(&copy);
        for summary in store.list_profiles().unwrap() {
            let id = summary.id.to_string();
            let before = stored(&store, &id);
            let profile = store.load_profile(summary.id).unwrap().unwrap();
            let settings = store.work_session_settings(summary.id).unwrap();
            let session = deckpilot_core::WorkSessionWorkflows {
                start: settings.start_workflow,
                verification: settings.verification_workflow,
            };
            for workflow in &profile.workflows {
                let references = profile.workflow_references_with(workflow.id, &session);
                assert!(
                    references.len() >= profile.workflow_usages(workflow.id).len(),
                    "{fixture}: {}",
                    workflow.name
                );
                assert_eq!(
                    references.len(),
                    profile.workflow_references(workflow.id).len()
                        + session.naming(workflow.id).len()
                );
                let _ = deckpilot_core::workflow_block(workflow, &profile, &registry);
            }
            let scope = WorkflowScope {
                foreign: Some(store.workflows_sharing_layouts(&profile).unwrap()),
                graphs_off: false,
            };
            for layout in &profile.layouts {
                workflow_keys += layout
                    .pages
                    .iter()
                    .flat_map(|p| &p.bindings)
                    .filter(|b| matches!(b.target, BindingTarget::Workflow { .. }))
                    .count();
                for graphs_off in [false, true] {
                    for issue in layout.workflow_binding_issues(
                        Some(layout),
                        &profile,
                        &registry,
                        graphs_off,
                    ) {
                        assert!(issue.stored, "{fixture}: {issue}");
                    }
                }
                let model = catalog.get(&layout.model).unwrap();
                let controls = model.capabilities.controls();
                for page in &layout.pages {
                    let faces = deckpilot_core::page_faces_in(
                        &registry,
                        layout,
                        page.id,
                        &profile,
                        None,
                        &controls,
                        Some(&model.capabilities),
                        &scope,
                    );
                    assert_eq!(faces.len(), controls.len());
                }
            }
            // Saving again keeps every action, workflow and binding byte
            // for byte.
            store.save_profile(&profile).unwrap();
            assert_eq!(stored(&store, &id), before, "{fixture}: {id}");
            assert_eq!(store.load_profile(summary.id).unwrap().unwrap(), profile);
        }
    }
    // The fixtures have keys that start workflows: the check is not empty.
    assert!(workflow_keys > 0);
}
