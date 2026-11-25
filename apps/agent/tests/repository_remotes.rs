//! `listRepositoryRemotes` through the real agent (feature `plugins`): the
//! git remotes of a project repository's bound folder, read with the
//! system's real `git` in **temporary repositories only**, for "Link
//! remote" (Git Providers design §9.2).
//!
//! * The folder comes from the profile and this computer's bindings, never
//!   from the request: the app names a profile, a project and a repository.
//! * Read-only: the repository is byte for byte as it was, nothing was
//!   fetched; the URLs carry no credentials.
//! * A folder that is not a repository, a repository without a folder,
//!   unknown ids, and the Projects feature off are refused with their
//!   reasons.
//!
//! Skipped when the system has no `git`. Unix only.
#![cfg(all(feature = "plugins", unix))]

#[path = "support/first_party.rs"]
mod first_party;
mod support;

use std::path::{Path, PathBuf};

use deckpilot_core::{
    MachineConfig, MachineResourceBinding, Os, Profile, Project, ProjectId, ProjectKey,
    RepositoryId, ResourceKey, ResourceKind,
};
use deckpilot_ipc::connections::DetectedRemote;
use deckpilot_ipc::{ClientError, Request, Response};
use deckpilot_plugin_bundled::BundleClass;
use first_party::{Agent, Env, FirstParty};

const PLUGIN: FirstParty = FirstParty {
    id: "deckpilot.projects",
    released: "migration",
};

fn real_git() -> Option<PathBuf> {
    [
        "/usr/bin/git",
        "/opt/homebrew/bin/git",
        "/usr/local/bin/git",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|p| p.is_file())
}

/// A step of the setup, with no user configuration.
fn setup(git: &Path, home: &Path, dir: &Path, args: &[&str]) {
    let status = std::process::Command::new(git)
        .arg("-C")
        .arg(dir)
        .args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "a git setup step failed");
}

struct Fixture {
    profile: Profile,
    machine: MachineConfig,
    repo: PathBuf,
    shop: ProjectId,
    plain: ProjectId,
    unbound: ProjectId,
}

fn fixture(git: &Path, dir: &Path) -> Fixture {
    let home = dir.join("home");
    let repo = dir.join("src/shop");
    let plain = dir.join("src/plain");
    for d in [&home, &repo, &plain] {
        std::fs::create_dir_all(d).unwrap();
    }
    let repo = repo.canonicalize().unwrap();
    setup(git, &home, &repo, &["init", "-q"]);
    setup(
        git,
        &home,
        &repo,
        &[
            "remote",
            "add",
            "origin",
            "https://user:pw@gitlab.com/shop/main.git",
        ],
    );
    setup(
        git,
        &home,
        &repo,
        &["remote", "add", "upstream", "git@github.com:acme/main.git"],
    );
    let mut machine = MachineConfig::new(Os::current());
    for (key, value) in [
        ("shop.repo", repo.display().to_string()),
        (
            "plain.repo",
            plain.canonicalize().unwrap().display().to_string(),
        ),
    ] {
        machine.set_binding(MachineResourceBinding {
            key: ResourceKey::new(key).unwrap(),
            kind: ResourceKind::Directory,
            value,
        });
    }
    let mut profile = Profile::new("Work");
    for (key, resource) in [
        ("SHOP", "shop.repo"),
        ("PLAIN", "plain.repo"),
        ("UNBOUND", "unbound.repo"),
    ] {
        let mut project = Project::new(ProjectKey::new(key).unwrap(), key);
        project
            .resources
            .insert("repository".into(), ResourceKey::new(resource).unwrap());
        profile.projects.push(project);
    }
    let ids: Vec<ProjectId> = profile.projects.iter().map(|p| p.id).collect();
    Fixture {
        profile,
        machine,
        repo,
        shop: ids[0],
        plain: ids[1],
        unbound: ids[2],
    }
}

/// Every file under `dir` with its bytes.
fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        for entry in std::fs::read_dir(next).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                files.push((path.clone(), std::fs::read(&path).unwrap()));
            }
        }
    }
    files.sort();
    files
}

async fn remotes(
    agent: &Agent,
    profile: &Profile,
    project: ProjectId,
    repository: RepositoryId,
) -> Result<Vec<DetectedRemote>, (Option<String>, String)> {
    match agent
        .request(Request::ListRepositoryRemotes {
            profile: profile.id,
            project,
            repository,
        })
        .await
    {
        Ok(Response::RepositoryRemotes { remotes }) => Ok(remotes),
        Ok(other) => panic!("unexpected {other:?}"),
        Err(ClientError::Agent(error)) => Err((error.reason, error.message)),
        Err(other) => panic!("{other}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_project_repositorys_remotes_are_read_from_its_folder_only() {
    let Some(git) = real_git() else { return };
    let dir = support::TempDir::new("repository-remotes-fixture");
    let f = fixture(&git, dir.path());
    let env = Env::new(
        "repository-remotes",
        PLUGIN,
        BundleClass::Default,
        Some((&f.profile, Some(f.shop))),
        Some(&f.machine),
    );
    let agent = env.start().await;
    let derived = RepositoryId::derived;

    // The Projects feature off: nothing is read.
    agent.seeded().await;
    let (reason, _) = remotes(&agent, &f.profile, f.shop, derived(f.shop))
        .await
        .unwrap_err();
    assert_eq!(reason.as_deref(), Some("projectsOff"), "{}", agent.log());

    agent.enable().await;
    let before = snapshot(&f.repo.join(".git"));
    let found = remotes(&agent, &f.profile, f.shop, derived(f.shop))
        .await
        .unwrap_or_else(|e| panic!("{e:?}\n{}", agent.log()));
    assert_eq!(
        found,
        vec![
            DetectedRemote {
                name: "origin".into(),
                url: "https://gitlab.com/shop/main.git".into(),
                push_url: None,
            },
            DetectedRemote {
                name: "upstream".into(),
                url: "git@github.com:acme/main.git".into(),
                push_url: None,
            },
        ]
    );
    assert_eq!(
        snapshot(&f.repo.join(".git")),
        before,
        "the repository changed"
    );
    assert!(!f.repo.join(".git/FETCH_HEAD").exists());

    // A folder that is no repository; a repository without a folder.
    let (reason, _) = remotes(&agent, &f.profile, f.plain, derived(f.plain))
        .await
        .unwrap_err();
    assert_eq!(reason.as_deref(), Some("not-a-repository"));
    let (reason, _) = remotes(&agent, &f.profile, f.unbound, derived(f.unbound))
        .await
        .unwrap_err();
    assert_eq!(reason.as_deref(), Some("no-folder"));
    // Unknown ids name nothing.
    for (project, repository) in [
        (ProjectId::new(), derived(f.shop)),
        (f.shop, RepositoryId::new()),
        // Another project's repository is not this project's.
        (f.shop, derived(f.plain)),
    ] {
        let (_, message) = remotes(&agent, &f.profile, project, repository)
            .await
            .unwrap_err();
        assert!(message.contains("does not exist"), "{message}");
    }
    let mut other = f.profile.clone();
    other.id = deckpilot_core::ProfileId::new();
    let (_, message) = remotes(&agent, &other, f.shop, derived(f.shop))
        .await
        .unwrap_err();
    assert!(message.contains("does not exist"), "{message}");
    agent.stop().await;
}
