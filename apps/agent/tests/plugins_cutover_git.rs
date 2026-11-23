//! The Git cut-over of the extraction (WP-C1, `docs/marketplace/
//! module-migration.md` §11 D4, §7 X13) end to end, through the real
//! agent, the real sandboxed host and the verified `deckpilot.git`
//! release artifact bundled as the release bundles it (class `migration`, pinned
//! with `DECKPILOT_TEST_BUNDLED_PINS`), on a configuration that uses the
//! built-in `git` module: its `modules` row (with the executable) and a
//! stored `git.status` key. Every git is a **fake `git`** that logs its
//! arguments (Unix only).
//!
//! * the migration installs the plugin, backs up `deckpilot.db` once
//!   (`backups/deckpilot-pre-plugin-extraction-*.db`, encrypted) and waits
//!   for the high-tier consent: the stored key is unavailable with the
//!   reason, nothing runs, and the profile is byte for byte as stored;
//! * the user's consent (`decideModuleMigration`) cuts it over: the same
//!   stored key runs through the plugin and the host's `tool.run` (its
//!   work tree check, which the built-in never runs, proves which ran),
//!   with the built-in's executable setting held by the host;
//! * a restart changes nothing and writes no second backup.
#![cfg(all(feature = "plugins", unix))]

#[path = "support/first_party.rs"]
mod first_party;
mod support;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use deckpilot_core::{
    ActionInstanceId, ActionParameters, ActionTypeId, MachineConfig, MachineResourceBinding,
    ModuleId, ModuleSettings, Os, ParameterValue, Profile, Project, ProjectKey, ResourceKey,
    ResourceKind,
};
use deckpilot_ipc::plugins::{
    ConsentDecision, MigrationPhase, MigrationRoute, ModuleMigrationInfo,
};
use deckpilot_ipc::{Request, Response};
use deckpilot_plugin_bundled::BundleClass;
use deckpilot_storage::ConfigStore;
use first_party::{Agent, Env, FirstParty, SETTLE};

const PLUGIN: FirstParty = FirstParty {
    id: "deckpilot.git",
    released: "migration",
};
const OID: &str = "1234567890abcdef1234567890abcdef12345678";

/// A fake `git` that logs its arguments and answers `status` and the
/// host's work tree check like git in a clean repository on `main`.
fn fake_git(bin: &Path, log: &Path) -> PathBuf {
    std::fs::create_dir_all(bin).unwrap();
    let script = format!(
        r#"#!/bin/sh
printf '%s\n' "$@" '@@' >> '{log}'
while [ $# -gt 0 ]; do
  case "$1" in
    -C|-c) shift 2 ;;
    --no-pager|--no-optional-locks) shift ;;
    *) break ;;
  esac
done
here=$(pwd -P)
case "$*" in
  "rev-parse --show-toplevel") echo "$here" ;;
  "rev-parse --show-toplevel --absolute-git-dir") echo "$here"; echo "$here/.git" ;;
  status*) printf '%s\000' '# branch.oid {OID}' '# branch.head main' ;;
  *) echo "fake git: unsupported $*" >&2; exit 1 ;;
esac
"#,
        log = log.display()
    );
    let path = bin.join("git");
    std::fs::write(&path, script).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

/// Each logged run's arguments.
fn runs(log: &Path) -> Vec<Vec<String>> {
    let text = std::fs::read_to_string(log).unwrap_or_default();
    let mut out = Vec::new();
    let mut args = Vec::new();
    for line in text.lines() {
        if line == "@@" {
            out.push(std::mem::take(&mut args));
        } else {
            args.push(line.to_owned());
        }
    }
    out
}

/// The backups the agent wrote before the extraction.
fn backups(env: &Env) -> Vec<String> {
    let dir = env.paths.data_dir.join("backups");
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("deckpilot-pre-plugin-extraction-"))
        .collect();
    names.sort();
    names
}

async fn migration(agent: &Agent) -> ModuleMigrationInfo {
    match agent.ok(Request::ListModuleMigrations).await {
        Response::ModuleMigrations { migrations } => migrations
            .into_iter()
            .find(|m| m.module == "git")
            .unwrap_or_else(|| panic!("no migration of git\n{}", agent.log())),
        other => panic!("unexpected {other:?}"),
    }
}

async fn wait_for(
    agent: &Agent,
    what: &str,
    done: impl Fn(&ModuleMigrationInfo) -> bool,
) -> ModuleMigrationInfo {
    let deadline = Instant::now() + SETTLE;
    loop {
        let info = migration(agent).await;
        if done(&info) {
            return info;
        }
        assert!(
            Instant::now() < deadline,
            "{what}: {info:?}\n{}",
            agent.log()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// The stored key, run as the app's `runAction` does: its message, or the
/// agent's refusal.
async fn run(agent: &Agent, action: ActionInstanceId) -> (bool, String) {
    match agent.request(Request::RunAction { action }).await {
        Ok(Response::ActionResult { result }) => (
            result.status == deckpilot_core::ActionStatus::Succeeded,
            result.message,
        ),
        Ok(other) => panic!("unexpected {other:?}"),
        Err(error) => (false, error.to_string()),
    }
}

/// An alpha-era configuration that uses the built-in `git`: a project
/// whose `repository` role is a clean repository, the legacy `git.status`
/// key, the module saved with a fake `git` as its executable, and the
/// plugin (with Projects) bundled as the release bundles it.
struct Fixture {
    /// Holds the fake `git` and the repository.
    _dir: support::TempDir,
    env: Env,
    status: ActionInstanceId,
    profile: deckpilot_core::ProfileId,
    log: PathBuf,
    repo: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let dir = support::TempDir::new(&format!("{name}-fixture"));
        let repo = dir.path().join("src/shop");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let repo = repo.canonicalize().unwrap();
        let log = dir.path().join("git.log");
        let git = fake_git(&dir.path().join("bin"), &log);
        let mut machine = MachineConfig::new(Os::current());
        machine.set_binding(MachineResourceBinding {
            key: ResourceKey::new("shop.repo").unwrap(),
            kind: ResourceKind::Directory,
            value: repo.display().to_string(),
        });
        let mut profile = Profile::new("Work");
        let mut shop = Project::new(ProjectKey::new("SHOP").unwrap(), "Shop");
        shop.resources
            .insert("repository".into(), ResourceKey::new("shop.repo").unwrap());
        let project = shop.id;
        profile.projects.push(shop);
        // The legacy key, as an alpha-era app stored it.
        let status = ActionInstanceId::new();
        profile.actions.push(deckpilot_core::ActionInstance {
            id: status,
            name: "Status".into(),
            action_type: ActionTypeId::new("git.status").unwrap(),
            parameters: ActionParameters::new(),
            description: String::new(),
            icon: None,
            label: None,
        });
        let env = Env::new(
            name,
            PLUGIN,
            BundleClass::Migration,
            Some((&profile, Some(project))),
            Some(&machine),
        )
        .with_projects(BundleClass::Migration);
        // The built-in `git` saved with the user's executable.
        {
            let store = support::open_store(&env.paths, &env.key);
            store
                .set_module_state(
                    &ModuleId::new("git").unwrap(),
                    &ModuleSettings {
                        enabled: true,
                        config: ActionParameters::new().with(
                            "gitExecutable",
                            ParameterValue::Text(git.display().to_string()),
                        ),
                    },
                )
                .unwrap();
        }
        Self {
            _dir: dir,
            env,
            status,
            profile: profile.id,
            log,
            repo,
        }
    }

    /// The profile as stored, serialized.
    fn stored(&self) -> String {
        let store = support::open_store(&self.env.paths, &self.env.key);
        serde_json::to_string(&store.load_profile(self.profile).unwrap().unwrap()).unwrap()
    }
}

/// Runs the stored key until it succeeds; its message.
async fn run_until_ok(agent: &Agent, action: ActionInstanceId) -> String {
    let deadline = Instant::now() + SETTLE;
    loop {
        let (ok, message) = run(agent, action).await;
        if ok {
            return message;
        }
        assert!(
            Instant::now() < deadline,
            "the key did not run: {message}\n{}",
            agent.log()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // one path, step by step
async fn a_used_git_module_waits_for_consent_then_runs_through_the_plugin() {
    let fixture = Fixture::new("plugins-cutover-git");
    let (env, status, log, repo) = (&fixture.env, fixture.status, &fixture.log, &fixture.repo);
    let stored = || fixture.stored();
    let before = stored();

    // ---- waiting for consent: unavailable, nothing runs, one backup.
    let agent = env.start().await;
    let info = wait_for(&agent, "awaiting consent", |m| {
        m.phase == MigrationPhase::AwaitingConsent
    })
    .await;
    assert_eq!(info.route, MigrationRoute::PluginWithdrawn, "{info:?}");
    assert!(
        info.route_reason
            .as_deref()
            .is_some_and(|r| r.contains("waits for your consent")),
        "{info:?}"
    );
    let pending: Vec<String> = info
        .consent
        .iter()
        .filter(|item| item.decision == ConsentDecision::Pending)
        .map(|item| item.key.clone())
        .collect();
    for key in ["tools:git", "tools:git:network", "resources:open"] {
        assert!(pending.iter().any(|k| k == key), "{key}: {pending:?}");
    }
    let (ok, message) = run(&agent, status).await;
    assert!(
        !ok && message.contains("waits for your consent"),
        "{message}"
    );
    assert_eq!(runs(log), Vec::<Vec<String>>::new(), "nothing ran");
    let first = backups(env);
    assert_eq!(first.len(), 1, "{first:?}\n{}", agent.log());

    // ---- the user's consent: the stored key runs through the plugin.
    match agent
        .ok(Request::DecideModuleMigration {
            module: "git".into(),
            approve: pending,
            decline: Vec::new(),
        })
        .await
    {
        Response::ModuleMigration { .. } => {}
        other => panic!("unexpected {other:?}"),
    }
    wait_for(&agent, "cut over", |m| {
        m.phase == MigrationPhase::CutOver && m.route == MigrationRoute::Plugin
    })
    .await;
    let deadline = Instant::now() + SETTLE;
    let message = loop {
        let (ok, message) = run(&agent, status).await;
        if ok {
            break message;
        }
        assert!(
            Instant::now() < deadline,
            "the key did not run: {message}\n{}",
            agent.log()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert!(message.starts_with("main"), "{message}");
    let ran = runs(log);
    assert!(
        ran.iter().any(|args| args.ends_with(&[
            "rev-parse".to_owned(),
            "--show-toplevel".to_owned(),
            "--absolute-git-dir".to_owned()
        ])),
        "the host's work tree check ran, so the plugin did: {ran:?}"
    );
    assert!(
        ran.iter()
            .any(|args| args.iter().any(|a| a == &repo.display().to_string())),
        "{ran:?}"
    );
    agent.stop().await;

    // ---- a restart: still cut over, no second backup, nothing rewritten.
    let agent = env.start().await;
    wait_for(&agent, "still cut over", |m| {
        m.phase == MigrationPhase::CutOver && m.route == MigrationRoute::Plugin
    })
    .await;
    agent.stop().await;
    assert_eq!(backups(env), first);
    assert_eq!(stored(), before, "the profile is byte for byte as stored");
}

/// The versions v0.10.0 and v0.11.0 bundled (built from source there).
const SHIPPED: [(&str, &str); 2] = [("deckpilot.git", "0.1.0"), ("deckpilot.projects", "0.1.0")];

fn shipped(id: &str, manifest: &mut serde_json::Value) {
    let version = SHIPPED.iter().find(|(p, _)| *p == id).unwrap().1;
    manifest["version"] = version.into();
}

/// Grants of `plugin`: key, status and who decided.
fn grants(env: &Env, plugin: &str) -> Vec<(String, String, String)> {
    env.plugin_store()
        .grants(&deckpilot_plugin_manifest::PluginId::parse(plugin).unwrap())
        .unwrap()
        .into_iter()
        .map(|g| (g.key.to_string(), format!("{:?}", g.status), g.decided_by))
        .collect()
}

async fn version(agent: &Agent, plugin: &str) -> String {
    match agent.ok(Request::ListPlugins).await {
        Response::Plugins { plugins } => {
            plugins
                .into_iter()
                .find(|p| p.id == plugin)
                .unwrap_or_else(|| panic!("{plugin} is not installed\n{}", agent.log()))
                .version
        }
        other => panic!("unexpected {other:?}"),
    }
}

/// An installation of v0.10.0 or v0.11.0 cut over to the bundled
/// `deckpilot.git` 0.1.0 (a privileged plugin: `tools:git`, which is never
/// derived, only the user's consent grants it) upgrades to an app that
/// bundles the signed release artifacts: the copy moves to the released
/// version, offline, and the same stored key runs through it with exactly
/// the grants the user gave (none re-derived, none dropped). An older app
/// never moves it back. A later bundled version that asks for a new
/// privileged permission (`projectContext`) is staged and waits for the user's consent, its key
/// unavailable meanwhile; approved, it runs.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // one upgrade, step by step
async fn a_cut_over_git_installation_upgrades_to_the_released_artifact() {
    let fixture = Fixture::new("plugins-cutover-git-artifact");
    let (env, status) = (&fixture.env, fixture.status);
    let released = support::official::locked_version(PLUGIN.id);
    env.restage(BundleClass::Migration, &shipped);

    // ---- v0.11.0: consent, cut over to 0.1.0.
    let agent = env.start().await;
    let info = wait_for(&agent, "awaiting consent", |m| {
        m.phase == MigrationPhase::AwaitingConsent
    })
    .await;
    assert_eq!(version(&agent, PLUGIN.id).await, "0.1.0");
    // The process permission is never derived: it waits for the user.
    assert!(
        grants(env, PLUGIN.id)
            .iter()
            .all(|(key, _, _)| !key.starts_with("tools:")),
        "{:?}",
        grants(env, PLUGIN.id)
    );
    let pending: Vec<String> = info
        .consent
        .iter()
        .filter(|item| item.decision == ConsentDecision::Pending)
        .map(|item| item.key.clone())
        .collect();
    assert!(pending.iter().any(|k| k == "tools:git"), "{pending:?}");
    agent
        .ok(Request::DecideModuleMigration {
            module: "git".into(),
            approve: pending,
            decline: Vec::new(),
        })
        .await;
    wait_for(&agent, "cut over", |m| {
        m.phase == MigrationPhase::CutOver && m.route == MigrationRoute::Plugin
    })
    .await;
    assert!(run_until_ok(&agent, status).await.starts_with("main"));
    agent
        .ok(Request::SetPluginSettings {
            plugin: PLUGIN.id.into(),
            values: [("networkTimeoutSeconds".to_owned(), serde_json::json!(30))].into(),
        })
        .await;
    agent.stop().await;
    let granted = grants(env, PLUGIN.id);
    let tools: Vec<_> = granted
        .iter()
        .filter(|(k, _, _)| k.starts_with("tools:"))
        .collect();
    assert!(!tools.is_empty(), "{granted:?}");
    for (key, state, by) in &tools {
        assert_eq!(state, "Granted", "{key}");
        assert!(
            !by.starts_with("bundled:") && !by.starts_with("migration:"),
            "{key} decided by {by}: the user's consent, never derived"
        );
    }
    let settings = || {
        env.plugin_store()
            .settings(&deckpilot_plugin_manifest::PluginId::parse(PLUGIN.id).unwrap())
            .unwrap()
    };
    assert_eq!(settings()["networkTimeoutSeconds"], serde_json::json!(30));
    let profile = fixture.stored();

    // ---- the app update with the release artifacts.
    env.restage(BundleClass::Migration, &|_, _| {});
    let agent = env.start().await;
    wait_for(&agent, "still cut over", |m| {
        m.phase == MigrationPhase::CutOver
            && m.route == MigrationRoute::Plugin
            && m.consent.is_empty()
    })
    .await;
    assert!(run_until_ok(&agent, status).await.starts_with("main"));
    assert_eq!(
        version(&agent, PLUGIN.id).await,
        released,
        "{}",
        agent.log()
    );
    assert_eq!(
        version(&agent, "deckpilot.projects").await,
        support::official::locked_version("deckpilot.projects")
    );
    agent.stop().await;
    assert_eq!(
        grants(env, PLUGIN.id),
        granted,
        "the user's grants, exactly"
    );
    assert_eq!(settings()["networkTimeoutSeconds"], serde_json::json!(30));
    assert_eq!(fixture.stored(), profile, "the key binding, unchanged");

    // ---- an older app on the same data: never backwards.
    env.restage(BundleClass::Migration, &shipped);
    let agent = env.start().await;
    agent.ok(Request::GetStatus).await;
    assert_eq!(
        version(&agent, PLUGIN.id).await,
        released,
        "{}",
        agent.log()
    );
    agent.stop().await;
    assert_eq!(grants(env, PLUGIN.id), granted);

    // ---- a later bundled version that asks for a new privileged
    // permission: staged,
    // nothing granted by itself, the key unavailable until the user
    // approves it.
    let broadened = |id: &str, manifest: &mut serde_json::Value| {
        if id != PLUGIN.id {
            return;
        }
        manifest["version"] = "0.1.99".into();
        manifest["permissions"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "kind": "projectContext",
                "select": true,
                "justification": "A privileged permission the released version does not declare."
            }));
    };
    env.restage(BundleClass::Migration, &broadened);
    let agent = env.start().await;
    let deadline = Instant::now() + SETTLE;
    let pending = loop {
        let details = agent.details().await;
        if let Some(pending) = details.pending_update {
            break pending;
        }
        assert!(
            Instant::now() < deadline,
            "no staged update\n{}",
            agent.log()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert_eq!(pending.version, "0.1.99");
    assert!(pending.changes.escalation, "{pending:?}");
    let escalated: Vec<String> = pending
        .changes
        .added
        .iter()
        .chain(&pending.changes.broadened)
        .map(|p| p.key.clone())
        .collect();
    assert!(
        !escalated.is_empty() && escalated.iter().all(|k| k.starts_with("projectContext")),
        "{escalated:?}"
    );
    assert_eq!(version(&agent, PLUGIN.id).await, released);
    assert_eq!(grants(env, PLUGIN.id), granted, "nothing granted by itself");
    let (ok, message) = run(&agent, status).await;
    assert!(!ok, "unavailable until approved: {message}");
    agent
        .ok(Request::ApprovePluginUpdate {
            plugin: PLUGIN.id.into(),
            consent: escalated,
        })
        .await;
    assert!(run_until_ok(&agent, status).await.starts_with("main"));
    assert_eq!(version(&agent, PLUGIN.id).await, "0.1.99");
    agent.stop().await;
}

/// An installation of v0.10.0 or v0.11.0 (both shipped `deckpilot.git`
/// 0.1.0 with the same files) whose Git migration still waits for the
/// user's consent upgrades to the app that bundles the released artifact:
/// at the start the copy moves to the released version with nothing
/// granted (the same declared permissions), the key stays unavailable
/// with the same consent asked, and the user's consent then cuts it over
/// to the released version; a restart keeps it. Before the fix the update
/// was staged and the consent failed: "this agent pins no bundled
/// deckpilot.git 0.1.0".
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // one upgrade, step by step
async fn a_git_installation_waiting_for_consent_upgrades_to_the_released_artifact() {
    let fixture = Fixture::new("plugins-cutover-git-waiting");
    let (env, status) = (&fixture.env, fixture.status);
    let released = support::official::locked_version(PLUGIN.id);
    env.restage(BundleClass::Migration, &shipped);

    // ---- v0.10.0 / v0.11.0: installed, waiting for consent.
    let agent = env.start().await;
    wait_for(&agent, "awaiting consent", |m| {
        m.phase == MigrationPhase::AwaitingConsent
    })
    .await;
    assert_eq!(version(&agent, PLUGIN.id).await, "0.1.0");
    agent.stop().await;
    let before = grants(env, PLUGIN.id);
    assert!(
        before.iter().all(|(key, _, _)| !key.starts_with("tools:")),
        "{before:?}"
    );
    let profile = fixture.stored();

    // ---- the app update with the release artifacts: still waiting.
    env.restage(BundleClass::Migration, &|_, _| {});
    let agent = env.start().await;
    let info = wait_for(&agent, "still awaiting consent", |m| {
        m.phase == MigrationPhase::AwaitingConsent
    })
    .await;
    assert_eq!(
        version(&agent, PLUGIN.id).await,
        released,
        "{}",
        agent.log()
    );
    assert!(
        agent.details().await.pending_update.is_none(),
        "nothing staged\n{}",
        agent.log()
    );
    assert_eq!(grants(env, PLUGIN.id), before, "nothing granted by itself");
    let (ok, message) = run(&agent, status).await;
    assert!(
        !ok && message.contains("waits for your consent"),
        "{message}"
    );
    let pending: Vec<String> = info
        .consent
        .iter()
        .filter(|item| item.decision == ConsentDecision::Pending)
        .map(|item| item.key.clone())
        .collect();
    assert!(pending.iter().any(|k| k == "tools:git"), "{pending:?}");

    // ---- the user's consent: cut over to the released version.
    agent
        .ok(Request::DecideModuleMigration {
            module: "git".into(),
            approve: pending,
            decline: Vec::new(),
        })
        .await;
    wait_for(&agent, "cut over", |m| {
        m.phase == MigrationPhase::CutOver && m.route == MigrationRoute::Plugin
    })
    .await;
    assert!(run_until_ok(&agent, status).await.starts_with("main"));
    assert_eq!(version(&agent, PLUGIN.id).await, released);
    agent.stop().await;
    assert_eq!(fixture.stored(), profile, "the key binding, unchanged");

    // ---- a restart changes nothing.
    let agent = env.start().await;
    wait_for(&agent, "still cut over", |m| {
        m.phase == MigrationPhase::CutOver && m.route == MigrationRoute::Plugin
    })
    .await;
    assert!(run_until_ok(&agent, status).await.starts_with("main"));
    agent.stop().await;
}
