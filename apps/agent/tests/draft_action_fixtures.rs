//! Draft actions (`deckpilot_core::completeness`) and the databases of
//! released versions: computing completeness and validating for a save
//! never rewrite a configuration that was valid before. Every profile of
//! the 0.5.0-2, 0.6.0, schema 8 and 0.8.1 fixtures of
//! `crates/deckpilot-storage/tests/fixtures` reads back byte for byte after
//! it, and every action that saved before saves exactly as it did.

use std::path::{Path, PathBuf};

use deckpilot_core::completeness::{incomplete_actions, incomplete_uses};
use deckpilot_core::{Os, Profile};
use deckpilot_storage::crypto::Password;
use deckpilot_storage::vault::Vault;
use deckpilot_storage::{ConfigStore, Encryption, SqliteStore};

const FIXTURES: [&str; 4] = ["v0.5.0-2", "v0.6.0", "schema8", "v0.8.1"];
/// The development password the fixtures were created with (a test
/// fixture, protecting demonstration data only).
const PASSWORD: &str = "deckpilot-dev";

fn fixture_dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/deckpilot-storage/tests/fixtures")
        .join(name)
}

struct Copy(PathBuf);

impl Drop for Copy {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn copy(name: &str) -> Copy {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "deckpilot-drafts-{name}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    for file in ["deckpilot.db", "deckpilot.vault.json"] {
        std::fs::copy(fixture_dir(name).join(file), dir.join(file)).unwrap();
    }
    Copy(dir)
}

fn profiles(copy: &Copy) -> Vec<Profile> {
    let database = copy.0.join("deckpilot.db");
    let key = Vault::new(&database)
        .unlock(&Password::new(PASSWORD))
        .unwrap();
    let store = SqliteStore::open(&database, &Encryption::Key(key)).unwrap();
    store
        .list_profiles()
        .unwrap()
        .into_iter()
        .map(|summary| store.load_profile(summary.id).unwrap().unwrap())
        .collect()
}

#[test]
fn released_configurations_are_complete_and_never_rewritten() {
    let registry = deckpilot_modules::builtin(Os::current()).action_registry();
    for name in FIXTURES {
        let originals: Vec<Vec<u8>> = ["deckpilot.db", "deckpilot.vault.json"]
            .iter()
            .map(|file| std::fs::read(fixture_dir(name).join(file)).unwrap())
            .collect();
        let copy = copy(name);
        let loaded = profiles(&copy);
        assert!(!loaded.is_empty(), "{name}");
        let mut checked = 0;
        for profile in &loaded {
            let before = serde_json::to_vec(profile).unwrap();
            // Nothing of a released seed needs configuration.
            assert!(
                incomplete_actions(profile, &registry).is_empty(),
                "{name}: {:?}",
                incomplete_actions(profile, &registry)
            );
            assert!(incomplete_uses(profile, &registry).is_empty(), "{name}");
            for action in &profile.actions {
                let Some(definition) = registry.definition(&action.action_type) else {
                    // Moved to a plugin: unavailable here, never touched.
                    continue;
                };
                // Saving it again is what it was before drafts existed.
                if let Ok(strict) = registry.normalize(&action.action_type, &action.parameters) {
                    let mut again = action.clone();
                    assert_eq!(registry.validate_instance(&mut again), Ok(Vec::new()));
                    assert_eq!(again.parameters, strict, "{name}: {}", action.name);
                    let draft = definition.normalize_draft(&action.parameters).unwrap();
                    assert_eq!(draft.parameters, strict);
                    checked += 1;
                }
            }
            // Computing completeness changed nothing.
            assert_eq!(serde_json::to_vec(profile).unwrap(), before, "{name}");
        }
        assert!(checked > 0, "{name}: no action was checked");
        // A second read is byte for byte the first one.
        let again = profiles(&copy);
        assert_eq!(
            serde_json::to_vec(&again).unwrap(),
            serde_json::to_vec(&loaded).unwrap(),
            "{name}"
        );
        // The fixture itself is untouched.
        for (file, original) in ["deckpilot.db", "deckpilot.vault.json"]
            .iter()
            .zip(originals)
        {
            assert_eq!(
                std::fs::read(fixture_dir(name).join(file)).unwrap(),
                original,
                "{name}/{file}"
            );
        }
    }
}
