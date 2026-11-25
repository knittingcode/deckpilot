use deckpilot_core::script::Script;
use deckpilot_core::script_grants::ScriptGrants;
use deckpilot_core::{
    MachineConfig, MachineResourceBinding, Os, Profile, Project, ProjectKey, Repository,
    RepositoryId, RepositoryKey, RepositoryRemote, ResourceKey, ResourceKind,
};
use deckpilot_plugin_abi::scripts::codes;
use deckpilot_storage::{ConfigStore, SqliteStore};

use super::{grants_of, load_of, run_project_of, view_in};

fn store() -> (SqliteStore, Profile) {
    let store = SqliteStore::open_in_memory().unwrap();
    let profile = Profile::new("Work");
    store.save_profile(&profile).unwrap();
    (store, profile)
}

#[test]
fn only_a_trusted_script_of_the_profile_loads_and_gets_grants() {
    let (store, profile) = store();
    let script = store
        .create_script(profile.id, &Script::new("Deploy", "deckpilot.log(1)"))
        .unwrap();
    let granted = ScriptGrants {
        projects_read: true,
        ..ScriptGrants::default()
    };
    store
        .set_script_grants(profile.id, script.id, &granted)
        .unwrap();
    let id = script.id.to_string();
    let loaded = load_of(view_in(&store, profile.id, &id)).unwrap();
    assert_eq!(loaded.source, "deckpilot.log(1)");
    assert_eq!(loaded.settings.timeout_ms, 60_000);
    assert_eq!(loaded.settings.memory_mib, 64);
    assert_eq!(grants_of(view_in(&store, profile.id, &id)), Some(granted));
    // Imported over: it must not run and gets nothing.
    store.put_untrusted_script(profile.id, &script).unwrap();
    let refused = load_of(view_in(&store, profile.id, &id)).unwrap_err();
    assert_eq!(refused.code, codes::SCRIPT_NEEDS_REVIEW);
    assert_eq!(grants_of(view_in(&store, profile.id, &id)), None);
    // Another profile's, an unknown id and a malformed one are missing.
    let other = Profile::new("Other");
    store.save_profile(&other).unwrap();
    for (profile, id) in [
        (other.id, id.as_str()),
        (profile.id, &deckpilot_core::ScriptId::new().to_string()),
        (profile.id, "../x"),
    ] {
        let refused = load_of(view_in(&store, profile, id)).unwrap_err();
        assert_eq!(refused.code, codes::SCRIPT_MISSING, "{id}");
        assert_eq!(grants_of(view_in(&store, profile, id)), None);
    }
}

#[test]
fn a_repository_has_its_folder_here_and_all_its_remotes_with_their_ids() {
    let folder = tempfile::tempdir().unwrap();
    let mut project = Project::new(ProjectKey::new("SHOP").unwrap(), "Shop");
    let remote = |name: &str, primary: bool| RepositoryRemote {
        name: name.into(),
        primary,
        connection_id: primary.then(|| "conn-1".to_owned()),
        remote_repository_id: Some(format!("gh:{name}")),
        remote_url: Some(format!("https://example.com/{name}.git")),
        default_branch: Some("main".into()),
    };
    let repository = |key: &str, resource: &str| Repository {
        id: RepositoryId::new(),
        key: RepositoryKey::new(key).unwrap(),
        name: key.to_uppercase(),
        resource: ResourceKey::new(resource).unwrap(),
        description: None,
        remotes: vec![remote("origin", true), remote("upstream", false)],
    };
    project.repositories = vec![
        repository("web", "shop.web"),
        repository("api", "shop.api"),
        repository("ops", "shop.ops"),
    ];
    let mut machine = MachineConfig::new(Os::current());
    machine.set_binding(MachineResourceBinding {
        key: ResourceKey::new("shop.web").unwrap(),
        kind: ResourceKind::Directory,
        value: folder.path().display().to_string(),
    });
    // A file is not a repository's folder, nor is a missing one.
    machine.set_binding(MachineResourceBinding {
        key: ResourceKey::new("shop.api").unwrap(),
        kind: ResourceKind::Directory,
        value: folder.path().join("missing").display().to_string(),
    });
    let run = run_project_of(&project, &machine);
    assert_eq!(run.key, "SHOP");
    let web = &run.repositories[0];
    assert_eq!(
        web.folder.as_deref(),
        Some(
            crate::plugins::script_host::canonical(folder.path())
                .unwrap()
                .as_path()
        )
    );
    assert_eq!(run.repositories[1].folder, None);
    assert_eq!(run.repositories[2].folder, None);
    // K4: every remote, with the connection and the provider's id.
    assert_eq!(web.remotes.len(), 2);
    assert_eq!(web.remotes[0].connection_id.as_deref(), Some("conn-1"));
    assert_eq!(
        web.remotes[0].remote_repository_id.as_deref(),
        Some("gh:origin")
    );
    assert!(web.remotes[0].primary);
    assert_eq!(web.remotes[1].connection_id, None);
    assert_eq!(
        web.remote.as_ref().and_then(|r| r.url.as_deref()),
        Some("https://example.com/origin.git")
    );
}
