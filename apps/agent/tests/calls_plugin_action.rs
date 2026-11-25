//! "Back to work" followed to the real end of a plugin's action, through a
//! spawned agent, the real sandboxed plugin host and a plugin built to wasm
//! (C9, the Calls release blocker T-R3 at agent level):
//!
//! * the test fixture `plugins/first-party/test-fixtures/example-default`
//!   is built with the `guest` profile and bundled, as in
//!   `plugins_bundled.rs`; the agent seeds and enables it;
//! * the agent runs headless with a simulated MK.2 and Calls installed with
//!   the mock provider, already in a meeting (`--test-calls-mock`, which
//!   only a `test-hooks` build has; `plugins_feature.rs` checks a normal
//!   build has neither it nor the mock);
//! * "Back to work" is in mode B (completion): only a qualifying run's end
//!   brings the call back, never a timer. The user's key 0 runs the
//!   plugin's `greet` action, detached on its own thread through the host;
//!   its `DetachedFinished` is what brings the call back.
//!
//! No real call app is involved, and a Stream Deck plugged into the machine
//! is never opened (`--no-usb`).
#![cfg(feature = "plugins")]

mod support;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use deckpilot_core::{
    ActionInstance, ActionInstanceId, ActionParameters, ActionTypeId, BindingTarget,
    ControlBinding, DeviceLayout, ParameterValue, Profile, Trigger,
};
use deckpilot_device::{ControlId, ControlInput, DeviceId};
use deckpilot_ipc::plugins::PluginState;
use deckpilot_ipc::{
    AgentClient, AgentPaths, ClientError, ClientKind, KeyMaterial, Request, Response,
};
use deckpilot_modules::general;
use deckpilot_plugin_bundled::{BundleClass, Pins};
use deckpilot_plugin_manifest::PluginId;
use deckpilot_storage::{ConfigStore, DatabaseKey};
use support::TempDir;

const ID: &str = "deckpilot.example-default";
const STARTUP: Duration = Duration::from_secs(30);
const WAIT: Duration = Duration::from_secs(60);
/// The user's key that runs the plugin's action.
const GREET: u16 = 0;

const WENT_BACK: &str = "calls: the device went back to work";
const CAME_BACK: &str = "calls: back on the call after a run ended";

/// The keys of the overlay's Active page on an MK.2, from the built-in
/// template: "Back to work"'s, and another the user's layout leaves blank
/// (when it lights up, the overlay is shown).
fn overlay_keys() -> (u16, u16) {
    use deckpilot_calls::layouts::{Page, Shape, builtin_templates, generate};
    let mk2 = deckpilot_device::models::stream_deck_mk2();
    let shape = Shape::of(&mk2.capabilities);
    let template = builtin_templates()
        .into_iter()
        .find(|t| t.shape == shape)
        .unwrap();
    let generated = generate(&template, &mk2.id, &mk2.capabilities).unwrap();
    let page = generated
        .layout
        .page(generated.pages[&Page::Active])
        .unwrap();
    let key = |b: &ControlBinding| match b.control {
        ControlId::Key { index } => Some(index),
        _ => None,
    };
    let back = page
        .bindings
        .iter()
        .find(|b| {
            b.target.action_id().and_then(|id| generated.action_of(id)) == Some("return-to-work")
        })
        .and_then(key)
        .unwrap();
    let other = page
        .bindings
        .iter()
        .filter_map(key)
        .find(|k| *k != back && *k != GREET)
        .unwrap();
    (back, other)
}

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn runtime() -> PathBuf {
    repo().join("plugins/runtime")
}

fn fixture() -> PathBuf {
    repo().join("plugins/first-party/test-fixtures/example-default")
}

fn cargo_in_runtime(args: &[&str]) {
    let status = Command::new(option_env!("CARGO").unwrap_or("cargo"))
        .current_dir(runtime())
        .args(args)
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("CARGO_BUILD_TARGET")
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .status()
        .expect("cargo runs");
    assert!(status.success(), "cargo {args:?} in plugins/runtime failed");
}

struct Artifacts {
    host: PathBuf,
    module: PathBuf,
}

/// The host and the fixture's module, built from `plugins/runtime` (or
/// taken from `DECKPILOT_E2E_PLUGIN_HOST` and `DECKPILOT_E2E_WASM_DIR`).
fn artifacts() -> &'static Artifacts {
    static ARTIFACTS: OnceLock<Artifacts> = OnceLock::new();
    ARTIFACTS.get_or_init(|| {
        let wasm_dir = std::env::var_os("DECKPILOT_E2E_WASM_DIR")
            .map(PathBuf::from)
            .filter(|dir| dir.join("deckpilot_example_default.wasm").is_file())
            .unwrap_or_else(|| {
                cargo_in_runtime(&[
                    "build",
                    "--locked",
                    "-p",
                    "deckpilot-example-default",
                    "--target",
                    "wasm32-unknown-unknown",
                    "--profile",
                    "guest",
                ]);
                runtime().join("target/wasm32-unknown-unknown/guest")
            });
        let host = std::env::var_os("DECKPILOT_E2E_PLUGIN_HOST").map_or_else(
            || {
                cargo_in_runtime(&[
                    "build",
                    "--locked",
                    "-p",
                    "deckpilot-plugin-host",
                    "--bin",
                    "deckpilot-plugin-host",
                ]);
                runtime().join(format!(
                    "target/debug/deckpilot-plugin-host{}",
                    std::env::consts::EXE_SUFFIX
                ))
            },
            PathBuf::from,
        );
        Artifacts {
            host,
            module: wasm_dir.join("deckpilot_example_default.wasm"),
        }
    })
}

/// The profile: key 0 greets through the plugin, the key under "Back to
/// work" is a log action of the user's own, so its face tells the overlay
/// from the user's layout. Nothing else is bound.
fn profile(back: u16) -> Profile {
    let parameters = ActionParameters::new().with("name", ParameterValue::Text("Ada".into()));
    let greet = ActionInstance {
        id: ActionInstanceId::new(),
        name: "Greet Ada".into(),
        action_type: ActionTypeId::new(format!("{ID}.greet")).unwrap(),
        parameters,
        description: String::new(),
        icon: None,
        label: None,
    };
    let mine = support::registry()
        .create_instance(
            "Mine",
            general::log_id(),
            &ActionParameters::new().with("message", ParameterValue::Text("mine".into())),
        )
        .unwrap();
    let mut layout = DeviceLayout::new("Main", deckpilot_device::models::stream_deck_mk2().id);
    let home = layout.home_page;
    layout.page_mut(home).unwrap().bindings.extend([
        ControlBinding::new(
            ControlId::Key { index: GREET },
            Trigger::Press,
            BindingTarget::action(greet.id),
        ),
        ControlBinding::new(
            ControlId::Key { index: back },
            Trigger::Press,
            BindingTarget::action(mine.id),
        ),
    ]);
    let mut profile = Profile::new("Work");
    profile.actions.extend([greet, mine]);
    profile.layouts.push(layout);
    profile
}

/// A data directory with the profile, and application resources with the
/// fixture bundled and pinned.
struct Env {
    dir: TempDir,
    paths: AgentPaths,
    key: DatabaseKey,
}

impl Env {
    fn new(name: &str, back: u16) -> Self {
        let dir = TempDir::new(name);
        let paths = AgentPaths::in_dir(dir.path().join("data"));
        paths.create_dirs().unwrap();
        let key = support::create_database(&paths);
        let store = support::open_store(&paths, &key);
        let profile = profile(back);
        store.save_profile(&profile).unwrap();
        store.set_active_profile(Some(profile.id)).unwrap();
        drop(store);
        let env = Self { dir, paths, key };
        env.stage();
        env
    }

    fn bundled(&self) -> PathBuf {
        self.dir.path().join("resources/bundled")
    }

    fn pins_file(&self) -> PathBuf {
        self.dir.path().join("compiled-pins.json")
    }

    /// `bundled/deckpilot.example-default/` and the pins the agent "was
    /// compiled with" (as `plugins_bundled.rs` stages it).
    fn stage(&self) {
        let dir = self.bundled().join(ID);
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        for file in ["manifest.json", "LICENSE", "assets/icon-128.png"] {
            std::fs::copy(fixture().join(file), dir.join(file)).unwrap();
        }
        std::fs::copy(&artifacts().module, dir.join("plugin.wasm")).unwrap();
        let pins = Pins::compute(
            &self.bundled(),
            &[(
                PluginId::parse(ID).unwrap(),
                semver::Version::new(0, 1, 0),
                BundleClass::Default,
            )],
        )
        .unwrap();
        std::fs::write(self.bundled().join("pins.json"), pins.to_json_pretty()).unwrap();
        std::fs::write(self.pins_file(), pins.to_json_pretty()).unwrap();
    }

    async fn start(&self, back: &str) -> Agent {
        let log = self.dir.path().join("agent.log");
        let stderr = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log)
            .unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_deckpilot-agent"))
            .args(["--headless", "--no-usb", "--mock-devices", "mk2"])
            .args(["--log-level", "debug", "--enable-plugins"])
            .arg("--plugin-host")
            .arg(&artifacts().host)
            .arg("--bundled-plugins")
            .arg(self.bundled())
            .args(["--test-calls-mock", back])
            .env("DECKPILOT_DATA_DIR", &self.paths.data_dir)
            .env("DECKPILOT_TEST_BUNDLED_PINS", self.pins_file())
            .env_remove("DECKPILOT_ENABLE_PLUGINS")
            .env_remove("DECKPILOT_PLUGIN_REGISTRY")
            .env_remove("DECKPILOT_PLUGIN_REGISTRY_URL")
            .env_remove("DECKPILOT_PLUGIN_REGISTRY_ROOTS")
            .env_remove("DECKPILOT_TEST_AGENT_ARGS")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(stderr)
            .spawn()
            .expect("the agent starts");
        let agent = Agent {
            child,
            paths: self.paths.clone(),
            log,
        };
        agent
            .request(Request::Unlock {
                key: KeyMaterial::new(self.key.to_hex()),
            })
            .await
            .unwrap();
        agent
    }
}

struct Agent {
    child: Child,
    paths: AgentPaths,
    log: PathBuf,
}

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Agent {
    async fn client(&self) -> AgentClient {
        let deadline = Instant::now() + STARTUP;
        loop {
            match AgentClient::connect(&self.paths, ClientKind::Desktop).await {
                Ok(client) => return client,
                Err(e) => {
                    assert!(Instant::now() < deadline, "the agent did not start: {e}");
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        }
    }

    async fn request(&self, request: Request) -> Result<Response, ClientError> {
        let what = request.name();
        tokio::time::timeout(Duration::from_secs(120), async {
            self.client().await.request(request).await
        })
        .await
        .unwrap_or_else(|_| panic!("{what} took too long"))
    }

    fn log(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }

    /// Waits until `check` holds, or fails with the agent's log.
    async fn until<T>(&self, what: &str, mut check: impl AsyncFnMut(&Self) -> Option<T>) -> T {
        let deadline = Instant::now() + WAIT;
        loop {
            if let Some(value) = check(self).await {
                return value;
            }
            assert!(
                Instant::now() < deadline,
                "{what} did not happen\n{}",
                self.log()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn plugin_enabled(&self) -> bool {
        match self.request(Request::ListPlugins).await.unwrap() {
            Response::Plugins { plugins } => plugins
                .iter()
                .any(|p| p.id == ID && p.state == PluginState::Enabled),
            other => panic!("unexpected {other:?}"),
        }
    }

    async fn device(&self) -> Option<DeviceId> {
        match self.request(Request::GetStatus).await.unwrap() {
            Response::Status { status } => status
                .devices
                .iter()
                .find(|d| d.page.is_some())
                .map(|d| d.id.clone()),
            other => panic!("unexpected {other:?}"),
        }
    }

    /// The hash of what key `index` of `device` shows.
    async fn face(&self, device: &DeviceId, index: u16) -> Option<String> {
        match self
            .request(Request::InspectMockDevice {
                device: device.clone(),
            })
            .await
            .unwrap()
        {
            Response::MockDevice { inspection } => inspection
                .frames
                .into_iter()
                .find(|f| f.control == ControlId::Key { index })
                .and_then(|f| f.hash),
            other => panic!("unexpected {other:?}"),
        }
    }

    async fn tap(&self, device: &DeviceId, index: u16) {
        for input in [ControlInput::Press, ControlInput::Release] {
            let answer = self
                .request(Request::SimulateInput {
                    device: device.clone(),
                    control: ControlId::Key { index },
                    input,
                })
                .await;
            assert!(answer.is_ok(), "{answer:?}");
        }
    }

    fn logged(&self, line: &str) -> usize {
        self.log().matches(line).count()
    }
}

/// T-R3 at agent level: a device that went back to work comes back to the
/// call when the plugin action its key started really ends (mode B, so no
/// timer can do it), and not before.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn back_to_work_follows_a_plugin_action_to_its_end() {
    let (back, lit) = overlay_keys();
    let env = Env::new("calls-plugin-action", back);
    let agent = env.start("completion").await;
    agent
        .until("the plugin enabled", async |a| {
            a.plugin_enabled().await.then_some(())
        })
        .await;
    let device = agent.until("the device", async |a| a.device().await).await;
    assert!(
        agent
            .log()
            .contains("Calls installed with the mock provider"),
        "{}",
        agent.log()
    );

    // The overlay of the meeting: a key the user's layout leaves blank
    // lights up, and "Back to work" is drawn.
    agent
        .until("the overlay", async |a| a.face(&device, lit).await)
        .await;
    let overlay = agent
        .until("the overlay's Back to work face", async |a| {
            a.face(&device, back).await
        })
        .await;

    // Back to work: the user's own key shows ("Mine"). Its release
    // running nothing is T-R10 (`calls.rs`).
    agent.tap(&device, back).await;
    agent
        .until("going back to work", async |a| {
            (a.logged(WENT_BACK) == 1).then_some(())
        })
        .await;
    assert_eq!(agent.face(&device, lit).await, None, "the user's layout");
    let mine = agent
        .until("the user's own key", async |a| {
            a.face(&device, back).await.filter(|f| *f != overlay)
        })
        .await;

    // Mode B: no timer brings it back.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(agent.logged(CAME_BACK), 0, "{}", agent.log());
    assert_eq!(
        agent.face(&device, back).await.as_deref(),
        Some(mine.as_str())
    );

    // The plugin's action, detached through the host: its real end brings
    // the call back.
    agent.tap(&device, GREET).await;
    agent
        .until("back on the call after the plugin action", async |a| {
            (a.logged(CAME_BACK) == 1).then_some(())
        })
        .await;
    let log = agent.log();
    let line = log
        .lines()
        .find(|l| l.contains(CAME_BACK))
        .unwrap_or_default();
    assert!(line.contains("Succeeded"), "{line}");
    agent
        .until("the overlay's face again", async |a| {
            (a.face(&device, back).await.as_deref() == Some(overlay.as_str())).then_some(())
        })
        .await;
}
