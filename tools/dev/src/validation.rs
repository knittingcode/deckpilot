//! `deckpilot-dev validation`: helpers for the reproducible update test
//! (`docs/release-validation.md`). They act on an isolated data directory,
//! never on `.dev` or the real configuration, and do what a user would do
//! in the app: set up a configuration, unlock it, look at the agent.

use std::path::{Path, PathBuf};

use deckpilot_ipc::{AgentClient, AgentPaths, ClientKind, KeyMaterial, Request, Response};
use deckpilot_storage::{ConfigStore as _, DatabaseKey, Encryption, SqliteStore};
use deckpilot_update::policy::{PREFERENCES_SETTING, UpdateMode, UpdatePreferences};

use crate::seed;
use crate::session::DevPaths;

fn data_dir(args: &[String]) -> Result<PathBuf, String> {
    let dir = PathBuf::from(args.first().ok_or("expected a data directory")?);
    let production = AgentPaths::resolve_default().map(|p| p.data_dir).ok();
    if production.is_some_and(|p| p == dir) {
        return Err("refusing to use the real DeckPilot data directory".into());
    }
    Ok(dir)
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
}

async fn request(dir: &Path, request: Request) -> Result<Response, String> {
    let mut client = AgentClient::connect(&AgentPaths::in_dir(dir), ClientKind::Other)
        .await
        .map_err(|e| e.to_string())?;
    client.request(request).await.map_err(|e| e.to_string())
}

/// `seed <dir>`: a password-protected configuration (development password)
/// with the demonstration profiles and automatic updates ("install when
/// idle"), so that an update test build updates without any clicks.
fn seed(dir: &Path) -> Result<(), String> {
    let paths = DevPaths::isolated(dir);
    seed::seed(&paths, seed::MissingDatabase::CreateWithDevPassword).map_err(|e| e.to_string())?;
    let key = seed::dev_key(&paths)
        .ok_or("the configuration does not open with the development password")?;
    let store = SqliteStore::open(dir.join("deckpilot.db"), &Encryption::Key(key))
        .map_err(|e| e.to_string())?;
    update_automatically(&store)?;
    println!(
        "seeded {} (password protected, updates: install when idle)",
        dir.display()
    );
    Ok(())
}

/// The key of the running, unlocked agent (as the configuration app gets it).
fn agent_key(dir: &Path) -> Result<DatabaseKey, String> {
    match runtime().block_on(request(dir, Request::GetDatabaseKey))? {
        Response::DatabaseKey { key } => {
            DatabaseKey::from_hex(key.expose()).map_err(|e| e.to_string())
        }
        other => Err(format!("unexpected {other:?}")),
    }
}

/// Automatic updates ("install when idle"), so that an update test build
/// updates without any clicks.
fn update_automatically(store: &SqliteStore) -> Result<(), String> {
    let preferences = UpdatePreferences {
        mode: UpdateMode::InstallWhenIdle,
        automatic_checks: true,
        ..UpdatePreferences::default()
    };
    store
        .set_setting(
            PREFERENCES_SETTING,
            &serde_json::to_string(&preferences).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())
}

/// `populate <dir>`: fills the configuration the running agent created
/// (with a system-managed key) with the demonstration profiles and
/// automatic updates, using the key from the agent.
fn populate(dir: &Path) -> Result<(), String> {
    let key = agent_key(dir)?;
    seed::seed_with_key(&DevPaths::isolated(dir), key.clone()).map_err(|e| e.to_string())?;
    let store = SqliteStore::open(dir.join("deckpilot.db"), &Encryption::Key(key))
        .map_err(|e| e.to_string())?;
    update_automatically(&store)?;
    runtime().block_on(request(dir, Request::ReloadConfiguration))?;
    println!("populated {} through the agent", dir.display());
    Ok(())
}

/// `inventory <dir>`: what the configuration contains, read with the key
/// from the agent.
fn inventory(dir: &Path) -> Result<(), String> {
    let store = SqliteStore::open(dir.join("deckpilot.db"), &Encryption::Key(agent_key(dir)?))
        .map_err(|e| e.to_string())?;
    let (mut projects, mut actions, mut workflows, mut layouts) = (0, 0, 0, 0);
    let summaries = store.list_profiles().map_err(|e| e.to_string())?;
    for summary in &summaries {
        if let Some(profile) = store.load_profile(summary.id).map_err(|e| e.to_string())? {
            projects += profile.projects.len();
            actions += profile.actions.len();
            workflows += profile.workflows.len();
            layouts += profile.layouts.len();
        }
    }
    let protection = deckpilot_storage::vault::Vault::new(dir.join("deckpilot.db"))
        .protection()
        .map_err(|e| e.to_string())?
        .map(|p| format!("{:?}", p.mode()));
    println!(
        "profiles={} projects={projects} actions={actions} workflows={workflows} layouts={layouts} protection={}",
        summaries.len(),
        protection.unwrap_or_else(|| "-".into())
    );
    Ok(())
}

/// `stored <dir>`: the active profile's active project as stored in the
/// database. The agent reports an active project only while the Projects
/// plugin provides one; an upgrade must keep the stored choice either way.
fn stored(dir: &Path) -> Result<(), String> {
    let store = SqliteStore::open(dir.join("deckpilot.db"), &Encryption::Key(agent_key(dir)?))
        .map_err(|e| e.to_string())?;
    let project = match store.active_profile().map_err(|e| e.to_string())? {
        Some(id) => {
            let chosen = store.active_project(id).map_err(|e| e.to_string())?;
            let profile = store.load_profile(id).map_err(|e| e.to_string())?;
            chosen.and_then(|project| {
                profile.and_then(|profile| {
                    profile
                        .projects
                        .iter()
                        .find(|p| p.id == project)
                        .map(|p| p.key.to_string())
                })
            })
        }
        None => None,
    };
    println!(
        "stored_active_project={}",
        project.unwrap_or_else(|| "-".into())
    );
    Ok(())
}

/// `unlock <dir>`: unlocks the agent with the development password, as a
/// user would by typing it.
fn unlock(dir: &Path) -> Result<(), String> {
    let key = seed::dev_key(&DevPaths::isolated(dir))
        .ok_or("the configuration does not open with the development password")?;
    runtime().block_on(request(
        dir,
        Request::Unlock {
            key: KeyMaterial::new(key.to_hex()),
        },
    ))?;
    println!("agent unlocked");
    Ok(())
}

/// `activate <dir> <profile name>`: makes the named profile the active one
/// through the running agent (`SetActiveProfile`), as choosing it in the
/// app does. The agent stores the choice in the configuration, so the
/// running agent, of whatever version, writes it.
fn activate(dir: &Path, name: &str) -> Result<(), String> {
    let runtime = runtime();
    let Response::Profiles { profiles, .. } =
        runtime.block_on(request(dir, Request::ListProfiles))?
    else {
        return Err("unexpected response to ListProfiles".into());
    };
    let id = profiles
        .iter()
        .find(|p| p.name == name)
        .map(|p| p.id)
        .ok_or_else(|| format!("the agent has no profile named `{name}`"))?;
    match runtime.block_on(request(dir, Request::SetActiveProfile { profile: id }))? {
        Response::ActiveProfile { profile: Some(p) } if p.id == id => {
            println!("active profile {}", p.name);
            Ok(())
        }
        other => Err(format!("unexpected {other:?}")),
    }
}

/// `agent <dir>`: the running agent's version and protocol, the protocol
/// this client negotiated with it and its own.
fn agent(dir: &Path) -> Result<(), String> {
    let client = runtime()
        .block_on(AgentClient::connect(
            &AgentPaths::in_dir(dir),
            ClientKind::Other,
        ))
        .map_err(|e| e.to_string())?;
    println!(
        "agent_version={} agent_protocol={} negotiated_protocol={} client_protocol={}",
        client.agent_version(),
        client.agent_protocol(),
        client.protocol(),
        deckpilot_ipc::PROTOCOL_VERSION,
    );
    Ok(())
}

/// `schema <dir>`: the configuration's schema and the one this build
/// supports, its recorded schema upgrades, the update mode, and the newest
/// upgrade's backup: a copy of it (in a temporary folder; the backup itself
/// is not touched) must open with the agent's key and hold the profiles.
/// Opened with the key from the running agent, like the app opens it, so a
/// configuration of an older schema is upgraded first: run it after the
/// agent of this build has opened the configuration.
fn schema(dir: &Path) -> Result<(), String> {
    let key = agent_key(dir)?;
    let store = SqliteStore::open(dir.join("deckpilot.db"), &Encryption::Key(key.clone()))
        .map_err(|e| e.to_string())?;
    let history = store.migration_history().map_err(|e| e.to_string())?;
    let upgrades = history
        .iter()
        .map(|r| format!("{}->{}:{}", r.from_version, r.to_version, r.result))
        .collect::<Vec<_>>();
    let updates = store
        .setting(PREFERENCES_SETTING)
        .map_err(|e| e.to_string())?
        .and_then(|json| serde_json::from_str::<UpdatePreferences>(&json).ok())
        .map_or_else(|| "-".into(), |p| format!("{:?}", p.mode));
    let backup = match history.iter().rev().find_map(|r| r.backup.as_deref()) {
        None => "-".to_owned(),
        Some(path) => {
            let path = Path::new(path);
            let name = path
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            let profiles = backup_profiles(path, &key)?;
            format!("{name} backup_profiles={profiles}")
        }
    };
    println!(
        "schema={} supported={} upgrades={} updates={updates} backup={backup}",
        store.schema_version().map_err(|e| e.to_string())?,
        SqliteStore::supported_schema_version(),
        if upgrades.is_empty() {
            "-".into()
        } else {
            upgrades.join(",")
        },
    );
    Ok(())
}

/// The number of profiles in a copy of the backup at `path`, opened with
/// `key` in a temporary folder that is removed afterwards.
fn backup_profiles(path: &Path, key: &DatabaseKey) -> Result<usize, String> {
    let copy = std::env::temp_dir().join(format!(
        "deckpilot-backup-check-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_millis()
    ));
    std::fs::create_dir_all(&copy).map_err(|e| e.to_string())?;
    let result = std::fs::copy(path, copy.join("deckpilot.db"))
        .map_err(|e| format!("cannot copy the backup {}: {e}", path.display()))
        .and_then(|_| {
            SqliteStore::open(copy.join("deckpilot.db"), &Encryption::Key(key.clone()))
                .map_err(|e| format!("the backup does not open with the agent's key: {e}"))
        })
        .and_then(|store| store.list_profiles().map_err(|e| e.to_string()))
        .map(|profiles| profiles.len());
    let _ = std::fs::remove_dir_all(&copy);
    result
}

/// `shutdown <dir>`: asks the running agent to stop, as Quit does.
fn shutdown(dir: &Path) -> Result<(), String> {
    runtime().block_on(request(dir, Request::Shutdown))?;
    println!("agent asked to stop");
    Ok(())
}

/// `status <dir>`: one line with what the update test compares.
fn status(dir: &Path) -> Result<(), String> {
    let Response::Status { status } = runtime().block_on(request(dir, Request::GetStatus))? else {
        return Err("unexpected response".into());
    };
    println!(
        "version={} pid={} security={:?} protection={:?} waiting_for_key_store={} profiles={} active_profile={} active_project={}",
        status.version,
        status.pid,
        status.security,
        status.protection,
        status.waiting_for_key_store,
        status.profiles.len(),
        status.active_profile.map_or_else(|| "-".into(), |p| p.name),
        status
            .active_project
            .map_or_else(|| "-".into(), |p| p.key.to_string()),
    );
    Ok(())
}

pub fn command(args: &[String]) -> Result<(), String> {
    let (name, rest) = args
        .split_first()
        .ok_or("expected seed, unlock or status")?;
    let dir = data_dir(rest)?;
    match name.as_str() {
        "seed" => seed(&dir),
        "populate" => populate(&dir),
        "inventory" => inventory(&dir),
        "unlock" => unlock(&dir),
        "status" => status(&dir),
        "stored" => stored(&dir),
        "activate" => activate(&dir, rest.get(1).ok_or("expected a profile name")?),
        "agent" => agent(&dir),
        "schema" => schema(&dir),
        "shutdown" => shutdown(&dir),
        other => Err(format!("unknown validation command `{other}`")),
    }
}
