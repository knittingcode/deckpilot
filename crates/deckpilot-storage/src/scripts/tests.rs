use deckpilot_core::script::{Script, TrustState, source_sha256};
use deckpilot_core::script_grants::{ProcessCwd, ProcessEnv, ProcessGrant, ScriptGrants};
use deckpilot_core::{
    ActionInstance, ActionInstanceId, ActionParameters, ActionTypeId, ParameterValue, Profile,
    ProfileId, ScriptId,
};

use super::{RUN_ACTION, ScriptForRun, script_usages};
use crate::{ConfigStore, SqliteStore, StorageError};

fn store() -> (SqliteStore, ProfileId) {
    let store = SqliteStore::open_in_memory().unwrap();
    let profile = Profile::new("Work");
    store.save_profile(&profile).unwrap();
    (store, profile.id)
}

fn granted() -> ScriptGrants {
    ScriptGrants {
        projects_read: true,
        ..ScriptGrants::default()
    }
}

/// The grants a run gets, `None` when it must not run.
fn run_grants(store: &SqliteStore, profile: ProfileId, id: ScriptId) -> Option<ScriptGrants> {
    match store.script_for_run(profile, id).unwrap().unwrap() {
        ScriptForRun::Runnable { grants, .. } => Some(*grants),
        ScriptForRun::NeedsReview => None,
    }
}

fn source(store: &SqliteStore, profile: ProfileId, id: ScriptId) -> String {
    store.script(profile, id).unwrap().unwrap().source
}

fn state(store: &SqliteStore, profile: ProfileId, id: ScriptId) -> TrustState {
    store
        .scripts(profile)
        .unwrap()
        .into_iter()
        .find(|s| s.script.id == id)
        .unwrap()
        .trust
}

#[test]
fn a_new_script_is_trusted_with_no_grants_and_saving_grants_nothing() {
    let (store, profile) = store();
    let script = store
        .create_script(profile, &Script::new("Deploy", "one()"))
        .unwrap();
    assert_eq!(script.revision, 1);
    assert_eq!(state(&store, profile, script.id), TrustState::Trusted);
    assert_eq!(
        run_grants(&store, profile, script.id),
        Some(ScriptGrants::default()),
        "runs, with none"
    );
    // The editor's save keeps it trusted and its grants.
    store
        .set_script_grants(profile, script.id, &granted())
        .unwrap();
    let saved = store
        .save_script(
            profile,
            &Script {
                source: "two()".into(),
                ..script.clone()
            },
        )
        .unwrap();
    assert_eq!(saved.revision, 2);
    assert_eq!(state(&store, profile, script.id), TrustState::Trusted);
    assert_eq!(source(&store, profile, script.id), "two()");
    assert_eq!(run_grants(&store, profile, script.id), Some(granted()));
    // An older revision is refused.
    assert!(matches!(
        store.save_script(profile, &script),
        Err(StorageError::ScriptRevision(2))
    ));
}

#[test]
fn an_import_never_trusts_and_brings_no_grants() {
    let (store, profile) = store();
    let script = store
        .create_script(profile, &Script::new("Deploy", "one()"))
        .unwrap();
    store
        .set_script_grants(profile, script.id, &granted())
        .unwrap();
    // Overwritten by an import with another source: no trust, no grants.
    let imported = Script {
        source: "evil()".into(),
        ..script.clone()
    };
    store.put_untrusted_script(profile, &imported).unwrap();
    assert_eq!(state(&store, profile, script.id), TrustState::NeedsReview);
    assert_eq!(
        run_grants(&store, profile, script.id),
        None,
        "it never runs"
    );
    assert!(store.scripts(profile).unwrap()[0].grants.is_none());
    // Even the same source as before is not trusted again by an import.
    store.put_untrusted_script(profile, &script).unwrap();
    assert_eq!(state(&store, profile, script.id), TrustState::NeedsReview);
    // Grants cannot be set before the review; an editor save keeps it
    // untrusted.
    assert!(matches!(
        store.set_script_grants(profile, script.id, &granted()),
        Err(StorageError::ScriptNotTrusted)
    ));
    let current = store.script(profile, script.id).unwrap().unwrap();
    store.save_script(profile, &current).unwrap();
    assert_eq!(state(&store, profile, script.id), TrustState::NeedsReview);
    // A review of an older source, or one that saw grants that are not
    // there, is refused; of the stored source, trusts it with none.
    assert!(matches!(
        store.trust_script(
            profile,
            script.id,
            &source_sha256("evil()"),
            &ScriptGrants::default()
        ),
        Err(StorageError::ScriptChanged)
    ));
    assert!(matches!(
        store.trust_script(profile, script.id, &source_sha256("one()"), &granted()),
        Err(StorageError::ScriptChanged)
    ));
    store
        .trust_script(
            profile,
            script.id,
            &source_sha256("one()"),
            &ScriptGrants::default(),
        )
        .unwrap();
    assert_eq!(
        run_grants(&store, profile, script.id),
        Some(ScriptGrants::default())
    );
    // A new script from an import has no record at all.
    let fresh = Script::new("Fresh", "x()");
    store.put_untrusted_script(profile, &fresh).unwrap();
    assert_eq!(state(&store, profile, fresh.id), TrustState::NeedsReview);
    assert_eq!(run_grants(&store, profile, fresh.id), None);
}

#[test]
fn a_dormant_grant_comes_back_only_as_the_review_showed_it() {
    let (store, profile) = store();
    let script = store
        .create_script(profile, &Script::new("Deploy", "one()"))
        .unwrap();
    store
        .set_script_grants(profile, script.id, &granted())
        .unwrap();
    // A source that no longer matches its hash (changed outside the
    // editor): not trusted, its grants dormant.
    store
        .connection()
        .execute("UPDATE scripts SET source = 'two()'", [])
        .unwrap();
    assert_eq!(run_grants(&store, profile, script.id), None);
    assert!(matches!(
        store.trust_script(
            profile,
            script.id,
            &source_sha256("two()"),
            &ScriptGrants::default()
        ),
        Err(StorageError::ScriptChanged)
    ));
    store
        .trust_script(profile, script.id, &source_sha256("two()"), &granted())
        .unwrap();
    assert_eq!(run_grants(&store, profile, script.id), Some(granted()));
}

#[test]
fn a_duplicate_has_a_new_id_a_copy_name_and_no_trust() {
    let (store, profile) = store();
    let script = store
        .create_script(profile, &Script::new("Deploy", "one()"))
        .unwrap();
    store
        .set_script_grants(profile, script.id, &granted())
        .unwrap();
    let copy = store.duplicate_script(profile, script.id).unwrap();
    assert_ne!(copy.id, script.id);
    assert_eq!(copy.name, "Deploy (copy)");
    assert_eq!(copy.source, "one()");
    assert_eq!(state(&store, profile, copy.id), TrustState::NeedsReview);
    assert_eq!(run_grants(&store, profile, copy.id), None);
    let again = store.duplicate_script(profile, script.id).unwrap();
    assert_eq!(again.name, "Deploy (copy 2)");
}

#[test]
fn names_are_unique_without_case_and_scripts_are_checked() {
    let (store, profile) = store();
    store
        .create_script(profile, &Script::new("Deploy", "one()"))
        .unwrap();
    assert!(matches!(
        store.create_script(profile, &Script::new("deploy", "two()")),
        Err(StorageError::ScriptNameTaken(_))
    ));
    assert!(matches!(
        store.create_script(profile, &Script::new(" ", "two()")),
        Err(StorageError::Invalid(_))
    ));
    // Beyond ASCII too, which SQLite's NOCASE does not fold.
    store
        .create_script(profile, &Script::new("Édition", "x()"))
        .unwrap();
    assert!(matches!(
        store.create_script(profile, &Script::new("édition", "y()")),
        Err(StorageError::ScriptNameTaken(_))
    ));
    // And in another Unicode form of the same name (NFD).
    assert!(matches!(
        store.create_script(profile, &Script::new("E\u{0301}dition", "z()")),
        Err(StorageError::ScriptNameTaken(_))
    ));
    // An import or a duplicate never passes the profile's limit either.
    for n in 0..deckpilot_core::script::MAX_SCRIPTS - 2 {
        store
            .create_script(profile, &Script::new(format!("s{n}"), ""))
            .unwrap();
    }
    assert!(matches!(
        store.put_untrusted_script(profile, &Script::new("one too many", "")),
        Err(StorageError::Invalid(_))
    ));
    assert!(matches!(
        store.create_script(ProfileId::new(), &Script::new("X", "")),
        Err(StorageError::UnknownProfile)
    ));
}

#[test]
fn invalid_grants_are_refused_and_container_runtimes_never_stored() {
    let (store, profile) = store();
    let script = store
        .create_script(profile, &Script::new("Deploy", "one()"))
        .unwrap();
    let docker = ScriptGrants {
        process: vec![ProcessGrant {
            command: "docker".into(),
            cwd: ProcessCwd::Repositories,
            env: ProcessEnv::None,
            unrestricted_args: false,
        }],
        ..ScriptGrants::default()
    };
    assert!(matches!(
        store.set_script_grants(profile, script.id, &docker),
        Err(StorageError::Invalid(_))
    ));
}

#[test]
fn deleting_a_script_removes_its_trust_and_a_profile_its_scripts() {
    let (store, profile) = store();
    let script = store
        .create_script(profile, &Script::new("Deploy", "one()"))
        .unwrap();
    store.delete_script(profile, script.id).unwrap();
    assert!(store.script_for_run(profile, script.id).unwrap().is_none());
    let count: i64 = store
        .connection()
        .query_row("SELECT COUNT(*) FROM script_trust", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
    assert!(matches!(
        store.delete_script(profile, script.id),
        Err(StorageError::UnknownScript)
    ));
    let other = store
        .create_script(profile, &Script::new("Other", "x()"))
        .unwrap();
    store.delete_profile(profile).unwrap();
    assert!(store.script(profile, other.id).unwrap().is_none());
}

#[test]
fn usages_are_the_actions_that_run_the_script() {
    let id = ScriptId::new();
    let mut profile = Profile::new("Work");
    let run = |script: &str| ActionInstance {
        id: ActionInstanceId::new(),
        name: "Run".into(),
        action_type: ActionTypeId::new(RUN_ACTION).unwrap(),
        parameters: ActionParameters::new().with("script", ParameterValue::Text(script.into())),
        description: String::new(),
        icon: None,
        label: None,
    };
    let mine = run(&id.to_string());
    profile.actions = vec![mine.clone(), run(&ScriptId::new().to_string())];
    let usages = script_usages(&profile, id);
    assert_eq!(usages.len(), 1);
    assert_eq!(usages[0].action, mine.id);
}
