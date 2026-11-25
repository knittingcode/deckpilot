//! Scripts in exports (`docs/data-model.md`, "Scripts"): an export carries
//! a script's name, source and settings, never whether it is trusted nor
//! its grants; an imported script arrives untrusted, with no grants, and
//! never runs until reviewed. Nothing runs on import. An export with
//! scripts is payload version 6; one without keeps its version.

use deckpilot_core::script::{Script, TrustState};
use deckpilot_core::script_grants::ScriptGrants;
use deckpilot_core::{Os, Profile};
use deckpilot_device::ModelCatalog;
use deckpilot_portability::import::{ConflictResolution, preview, validate};
use deckpilot_portability::payload::{self, ExportOptions, Payload};
use deckpilot_portability::{ImportMode, ImportOptions, PAYLOAD_VERSION, PortabilityError, plan};
use deckpilot_storage::crypto::{KdfParams, Password};
use deckpilot_storage::scripts::ScriptForRun;
use deckpilot_storage::{ConfigStore, SqliteStore};

const OPTIONS: ExportOptions = ExportOptions {
    module_settings: false,
    app_settings: false,
};

fn modules() -> deckpilot_core::ModuleRegistry {
    deckpilot_modules::builtin(Os::Linux)
}

fn password() -> Password {
    let mut bytes = [0u8; 16];
    deckpilot_storage::crypto::random_bytes(&mut bytes).unwrap();
    Password::new(format!("test-export-{:032x}", u128::from_le_bytes(bytes)))
}

fn granted() -> ScriptGrants {
    ScriptGrants {
        projects_read: true,
        repositories_paths: true,
        ..ScriptGrants::default()
    }
}

/// A store with a profile and a trusted, granted script.
fn source_store() -> (SqliteStore, Profile, Script) {
    let store = SqliteStore::open_in_memory().unwrap();
    let profile = Profile::new("Work");
    store.save_profile(&profile).unwrap();
    let script = store
        .create_script(profile.id, &Script::new("Deploy", "deckpilot.log('hi')"))
        .unwrap();
    store
        .set_script_grants(profile.id, script.id, &granted())
        .unwrap();
    (store, profile, script)
}

fn exported(store: &SqliteStore) -> (Payload, String) {
    let password = password();
    let payload = payload::export(store, &modules(), None, OPTIONS, 1, "test").unwrap();
    let bytes = payload::seal(&payload, &password, KdfParams::for_tests()).unwrap();
    let opened = payload::open(&bytes, &password).unwrap();
    let json = serde_json::to_string(&opened).unwrap();
    (opened, json)
}

fn import_into(
    store: &SqliteStore,
    payload: &Payload,
    resolution: Option<ConflictResolution>,
) -> deckpilot_portability::ImportSummary {
    validate(payload, &ModelCatalog::builtin()).unwrap();
    let options = ImportOptions {
        mode: ImportMode::Merge,
        profiles: payload.profiles.iter().map(|p| p.id).collect(),
        conflicts: payload
            .profiles
            .iter()
            .filter_map(|p| resolution.map(|r| (p.id, r)))
            .collect(),
        ..ImportOptions::default()
    };
    let (changes, summary) = plan(payload, &options, store, &modules()).unwrap();
    store.apply_changes(&changes).unwrap();
    summary
}

#[test]
fn an_export_carries_the_script_and_never_its_trust_or_grants() {
    let (store, profile, script) = source_store();
    let (payload, json) = exported(&store);
    assert_eq!(payload.version, PAYLOAD_VERSION);
    assert_eq!(PAYLOAD_VERSION, 6);
    assert_eq!(payload.scripts.len(), 1);
    assert_eq!(payload.scripts[0].profile, profile.id);
    assert_eq!(payload.scripts[0].script.source, script.source);
    for word in [
        "grants",
        "projectsRead",
        "trust",
        "sourceSha256",
        "reviewedAt",
    ] {
        assert!(!json.contains(word), "{word} in the export");
    }
    // Without scripts, the version is what it was.
    let empty = SqliteStore::open_in_memory().unwrap();
    empty.save_profile(&Profile::new("Other")).unwrap();
    let (payload, _) = exported(&empty);
    assert!(payload.version < 6);
    assert!(!serde_json::to_string(&payload).unwrap().contains("scripts"));
}

#[test]
fn an_imported_script_is_untrusted_with_no_grants_and_never_runs() {
    let (source, _, script) = source_store();
    let (payload, _) = exported(&source);
    let target = SqliteStore::open_in_memory().unwrap();
    let pv = preview(&payload, &target, &modules(), Vec::new()).unwrap();
    assert_eq!(pv.scripts.len(), 1);
    assert_eq!(pv.scripts[0].name, "Deploy");
    let summary = import_into(&target, &payload, None);
    assert_eq!(summary.scripts, ["Deploy"]);
    let profile = summary.imported[0].0;
    let listed = target.scripts(profile).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].trust, TrustState::NeedsReview);
    assert_eq!(listed[0].grants, None);
    assert_eq!(
        target.script(profile, script.id).unwrap().unwrap().source,
        script.source
    );
    assert_eq!(
        target.script_for_run(profile, script.id).unwrap(),
        Some(ScriptForRun::NeedsReview),
        "it must not run"
    );
}

#[test]
fn importing_over_a_trusted_script_never_trusts_the_new_source() {
    let (store, profile, script) = source_store();
    // An edited export of the same profile and script.
    let (mut payload, _) = exported(&store);
    payload.scripts[0].script.source = "steal()".into();
    import_into(&store, &payload, Some(ConflictResolution::Replace));
    assert_eq!(
        store.script(profile.id, script.id).unwrap().unwrap().source,
        "steal()"
    );
    assert_eq!(
        store.script_for_run(profile.id, script.id).unwrap(),
        Some(ScriptForRun::NeedsReview)
    );
    // As a copy: a new profile, its own untrusted script.
    let summary = import_into(&store, &payload, Some(ConflictResolution::KeepBoth));
    let copy = summary.imported[0].0;
    assert_ne!(copy, profile.id);
    assert_eq!(
        store.scripts(copy).unwrap()[0].trust,
        TrustState::NeedsReview
    );
    // The original keeps its own record untouched by the copy.
    assert_eq!(store.scripts(profile.id).unwrap().len(), 1);
}

#[test]
fn a_crafted_payload_cannot_carry_grants_or_bad_scripts() {
    let (store, _, _) = source_store();
    let (payload, _) = exported(&store);
    // A record with grants does not parse at all.
    let mut value = serde_json::to_value(&payload).unwrap();
    value["scripts"][0]["script"]["grants"] = serde_json::json!({"projectsRead": true});
    assert!(serde_json::from_value::<Payload>(value.clone()).is_err());
    let mut value = serde_json::to_value(&payload).unwrap();
    value["scripts"][0]["trusted"] = serde_json::json!(true);
    assert!(serde_json::from_value::<Payload>(value).is_err());
    // A script of no exported profile, twice, or invalid, is refused.
    let refused = |payload: &Payload| {
        matches!(
            validate(payload, &ModelCatalog::builtin()),
            Err(PortabilityError::Invalid(_))
        )
    };
    let mut orphan = payload.clone();
    orphan.scripts[0].profile = deckpilot_core::ProfileId::new();
    assert!(refused(&orphan));
    let mut twice = payload.clone();
    twice.scripts.push(twice.scripts[0].clone());
    assert!(refused(&twice));
    let mut big = payload;
    big.scripts[0].script.source = "x".repeat(256 * 1024 + 1);
    assert!(refused(&big));
}

#[test]
fn a_name_taken_here_gets_an_imported_suffix() {
    let (source, _, _) = source_store();
    let (payload, _) = exported(&source);
    let target = SqliteStore::open_in_memory().unwrap();
    // The same profile id exists here with its own "Deploy".
    let mut here = Profile::new("Work");
    here.id = payload.profiles[0].id;
    target.save_profile(&here).unwrap();
    target
        .create_script(here.id, &Script::new("deploy", "mine()"))
        .unwrap();
    // Kept as a copy, so the profile here stays.
    let summary = import_into(&target, &payload, Some(ConflictResolution::KeepBoth));
    let copy = summary.imported[0].0;
    assert_eq!(target.scripts(copy).unwrap()[0].script.name, "Deploy");
    assert_eq!(target.scripts(here.id).unwrap()[0].script.source, "mine()");
}
