//! Project repositories in exports (`docs/data-model.md`, "Repositories"):
//! the folder of a repository stays a machine binding and is never
//! exported, nor is a remote's `connectionId`. A project whose only
//! repository is the one its `repository` role stands for exports as
//! before (payload version 2) and gets the same repository back on import;
//! other repositories make the export version 5 and come back as they
//! were. An import as a copy gives the copy's repositories ids of their own.

use deckpilot_core::{
    MachineConfig, MachineResourceBinding, Os, Profile, Project, ProjectKey, Repository,
    RepositoryId, RepositoryKey, RepositoryRemote, ResourceKey, ResourceKind,
};
use deckpilot_device::ModelCatalog;
use deckpilot_portability::import::{ConflictResolution, preview, validate};
use deckpilot_portability::payload::{self, ExportOptions, Payload};
use deckpilot_portability::{
    ImportMode, ImportOptions, PAYLOAD_VERSION, PAYLOAD_VERSION_WITHOUT_LIBRARY, plan,
};
use deckpilot_storage::crypto::{KdfParams, Password};
use deckpilot_storage::{ConfigStore, SqliteStore};

const OPTIONS: ExportOptions = ExportOptions {
    module_settings: false,
    app_settings: false,
};
const SHOP_PATH: &str = "/home/someone/src/shop-checkout";

fn modules() -> deckpilot_core::ModuleRegistry {
    deckpilot_modules::builtin(Os::Linux)
}

fn password() -> Password {
    let mut bytes = [0u8; 16];
    deckpilot_storage::crypto::random_bytes(&mut bytes).unwrap();
    Password::new(format!("test-export-{:032x}", u128::from_le_bytes(bytes)))
}

fn key(text: &str) -> ResourceKey {
    ResourceKey::new(text).unwrap()
}

fn repository(repo_key: &str, resource: &str) -> Repository {
    Repository {
        id: RepositoryId::new(),
        key: RepositoryKey::new(repo_key).unwrap(),
        name: repo_key.to_uppercase(),
        resource: key(resource),
        description: Some(format!("The {repo_key} repository")),
        remotes: Vec::new(),
    }
}

/// A store with one profile holding `project`, and a binding of
/// `shop.repository` to a folder on this computer.
fn store_with(project: Project) -> (SqliteStore, Profile) {
    let store = SqliteStore::open_in_memory().unwrap();
    let mut profile = Profile::new("Work");
    profile.projects.push(project);
    store.save_profile(&profile).unwrap();
    let mut machine = MachineConfig::new(Os::Linux);
    machine.set_binding(MachineResourceBinding {
        key: key("shop.repository"),
        kind: ResourceKind::Directory,
        value: SHOP_PATH.into(),
    });
    store.save_machine_config(&machine).unwrap();
    let profile = store.load_profile(profile.id).unwrap().unwrap();
    (store, profile)
}

fn shop() -> Project {
    let mut project = Project::new(ProjectKey::new("SHOP").unwrap(), "Shop");
    project
        .resources
        .insert("repository".into(), key("shop.repository"));
    project
}

/// Exported, sealed and opened again.
fn exported(store: &SqliteStore) -> (Payload, serde_json::Value) {
    let password = password();
    let payload = payload::export(store, &modules(), None, OPTIONS, 1, "test").unwrap();
    let bytes = payload::seal(&payload, &password, KdfParams::for_tests()).unwrap();
    let opened = payload::open(&bytes, &password).unwrap();
    let json = serde_json::to_value(&opened).unwrap();
    (opened, json)
}

fn import_into(store: &SqliteStore, payload: &Payload, resolution: Option<ConflictResolution>) {
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
    let (changes, _) = plan(payload, &options, store, &modules()).unwrap();
    store.apply_changes(&changes).unwrap();
}

#[test]
fn the_derived_repository_exports_as_before_and_comes_back_the_same() {
    let mut project = shop();
    // Stored, as the migration stores it.
    project.materialize_repositories();
    let derived = project.repositories[0].clone();
    let (store, _) = store_with(project);
    let (payload, json) = exported(&store);
    assert_eq!(payload.version, PAYLOAD_VERSION_WITHOUT_LIBRARY);
    let exported_project = &json["profiles"][0]["projects"][0];
    assert!(exported_project.get("repositories").is_none());
    assert!(exported_project.get("defaultRepository").is_none());
    assert!(!json.to_string().contains(SHOP_PATH), "a path was exported");

    let target = SqliteStore::open_in_memory().unwrap();
    import_into(&target, &payload, None);
    let imported = target.list_profiles().unwrap().remove(0);
    let profile = target.load_profile(imported.id).unwrap().unwrap();
    // R3: the same repository, derived again (same id, key, resource).
    assert_eq!(
        profile.projects[0].effective_repositories().as_ref(),
        std::slice::from_ref(&derived)
    );
    assert_eq!(
        profile.projects[0].effective_default_repository_id(),
        Some(derived.id)
    );
}

#[test]
fn several_repositories_round_trip_without_paths_or_connection_ids() {
    let mut project = shop();
    project.materialize_repositories();
    let mut cms = repository("cms", "shop.cms");
    cms.remotes = vec![
        RepositoryRemote {
            name: "origin".into(),
            primary: true,
            connection_id: Some("local-connection-7".into()),
            remote_repository_id: Some("gl:4711".into()),
            remote_url: Some("git@gitlab.example.com:shop/cms.git".into()),
            default_branch: Some("main".into()),
        },
        RepositoryRemote {
            name: "upstream".into(),
            ..RepositoryRemote::default()
        },
    ];
    let cms_id = cms.id;
    project.repositories.push(cms.clone());
    project.repositories.push(repository("api", "shop.api"));
    project.set_default_repository(Some(cms_id)).unwrap();
    let (store, original) = store_with(project);
    let original = original.projects[0].clone();

    let (payload, json) = exported(&store);
    assert_eq!(payload.version, PAYLOAD_VERSION);
    let text = json.to_string();
    assert!(!text.contains(SHOP_PATH), "a path was exported");
    assert!(!text.contains("connectionId") && !text.contains("local-connection-7"));
    let repositories = &json["profiles"][0]["projects"][0]["repositories"];
    assert_eq!(repositories.as_array().unwrap().len(), 3);
    assert_eq!(
        repositories[1]["remotes"][0]["remoteRepositoryId"],
        "gl:4711"
    );
    assert_eq!(
        json["profiles"][0]["projects"][0]["defaultRepository"],
        cms_id.to_string()
    );
    // The repositories' folders are listed as resources of kind directory,
    // also where this computer has no binding; their values never are.
    for resource in ["shop.repository", "shop.cms", "shop.api"] {
        assert_eq!(
            payload.resources[&key(resource)],
            Some(ResourceKind::Directory)
        );
    }

    // Into an empty installation: everything as it was, but the
    // connection id.
    let target = SqliteStore::open_in_memory().unwrap();
    let preview = preview(&payload, &target, &modules(), Vec::new()).unwrap();
    let missing: Vec<&str> = preview
        .missing_resources
        .iter()
        .map(|m| m.key.as_str())
        .collect();
    assert_eq!(missing, ["shop.api", "shop.cms", "shop.repository"]);
    import_into(&target, &payload, None);
    let profile = target
        .load_profile(target.list_profiles().unwrap()[0].id)
        .unwrap()
        .unwrap();
    let mut expected = original.clone();
    expected.repositories[1].remotes[0].connection_id = None;
    assert_eq!(profile.projects[0], expected);
    // Nothing was bound on the other computer.
    assert_eq!(target.load_machine_config().unwrap(), None);

    // Into the same installation, as a copy: new project id, repository
    // ids derived from it, the default follows; the original is unchanged.
    import_into(&store, &payload, Some(ConflictResolution::KeepBoth));
    let profiles = store.list_profiles().unwrap();
    assert_eq!(profiles.len(), 2);
    let copy = profiles
        .iter()
        .map(|s| store.load_profile(s.id).unwrap().unwrap())
        .find(|p| p.projects[0].id != original.id)
        .unwrap();
    let copied = &copy.projects[0];
    assert_eq!(copied.repositories[0].id, RepositoryId::derived(copied.id));
    for (new, old) in copied.repositories.iter().zip(&original.repositories) {
        assert_ne!(new.id, old.id);
        assert_eq!((&new.key, &new.resource), (&old.key, &old.resource));
    }
    assert_eq!(copied.default_repository, Some(copied.repositories[1].id));
    assert_eq!(copied.repositories[1].remotes[0].connection_id, None);
    let kept = profiles
        .iter()
        .map(|s| store.load_profile(s.id).unwrap().unwrap())
        .find(|p| p.projects[0].id == original.id)
        .unwrap();
    assert_eq!(kept.projects[0], original);
}

#[test]
fn crafted_repositories_are_refused_or_cleaned() {
    let mut project = shop();
    project.repositories.push(repository("cms", "shop.cms"));
    let (store, _) = store_with(project);
    let (payload, _) = exported(&store);

    // Duplicate keys: refused before anything is written.
    let mut twice = payload.clone();
    let repositories = &mut twice.profiles[0].projects[0].repositories;
    let mut again = repositories[0].clone();
    again.id = RepositoryId::new();
    repositories.push(again);
    assert!(validate(&twice, &ModelCatalog::builtin()).is_err());

    // A default that is no repository of the project: refused.
    let mut foreign = payload.clone();
    foreign.profiles[0].projects[0].default_repository = Some(RepositoryId::new());
    assert!(validate(&foreign, &ModelCatalog::builtin()).is_err());

    // A name of a remote id (K2): refused.
    let mut named = payload.clone();
    named.profiles[0].projects[0].repositories[0].remotes = vec![RepositoryRemote {
        name: "origin".into(),
        remote_repository_id: Some("group/proj".into()),
        ..RepositoryRemote::default()
    }];
    assert!(validate(&named, &ModelCatalog::builtin()).is_err());

    // An invalid key in the JSON: the payload does not even parse.
    let mut json = serde_json::to_value(&payload).unwrap();
    json["profiles"][0]["projects"][0]["repositories"][0]["key"] = "Not-Valid".into();
    assert!(serde_json::from_value::<Payload>(json).is_err());

    // A connection id that came with the archive is dropped.
    let mut with_connection = payload.clone();
    with_connection.profiles[0].projects[0].repositories[0].remotes = vec![RepositoryRemote {
        name: "origin".into(),
        connection_id: Some("smuggled".into()),
        ..RepositoryRemote::default()
    }];
    let target = SqliteStore::open_in_memory().unwrap();
    import_into(&target, &with_connection, None);
    let profile = target
        .load_profile(target.list_profiles().unwrap()[0].id)
        .unwrap()
        .unwrap();
    assert_eq!(
        profile.projects[0].repositories[0].remotes[0].connection_id,
        None
    );
}

/// An export of an earlier build (payload version 2, projects without
/// `repositories`) imports with the derived repository, as R3 shows it.
#[test]
fn an_older_export_gets_the_derived_repository() {
    let (store, profile) = store_with(shop());
    let (payload, mut json) = exported(&store);
    assert_eq!(payload.version, 2);
    json["appVersion"] = "0.14.0".into();
    let older: Payload = serde_json::from_value(json).unwrap();
    let target = SqliteStore::open_in_memory().unwrap();
    import_into(&target, &older, None);
    let imported = target.load_profile(profile.id).unwrap().unwrap();
    let project = &imported.projects[0];
    assert_eq!(project.repositories, []);
    let derived = project.derived_repository().unwrap();
    assert_eq!(derived.id, RepositoryId::derived(profile.projects[0].id));
    assert_eq!(project.default_repository().unwrap(), derived);
}
