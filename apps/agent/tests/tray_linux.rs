//! The tray on Linux with and without a `StatusNotifier` host (DP-TRAY-01),
//! through the real agent executable, its real tray library
//! (libayatana-appindicator3) and a private session bus.
//!
//! The test plays a minimal `org.kde.StatusNotifierWatcher` with a host,
//! as KDE Plasma and GNOME's `AppIndicator` extension provide one; it draws
//! nothing, so this checks registration and what the agent reports, not
//! that a panel shows the icon. Without it the bus looks like GNOME
//! without the extension.
//!
//! Needs an X display and a session bus of its own, so it is ignored by
//! default; CI runs it on Linux with
//!
//! ```text
//! xvfb-run -a dbus-run-session -- \
//!   cargo test -p deckpilot-agent --test tray_linux -- --ignored --test-threads 1
//! ```
#![cfg(target_os = "linux")]
// zbus interface methods take `&self`, every argument of the D-Bus method
// (used or not) and the message header by value.
#![allow(
    clippy::needless_pass_by_value,
    clippy::unused_self,
    clippy::used_underscore_binding
)]

use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use deckpilot_ipc::{AgentClient, AgentPaths, ClientKind, Request, Response, TrayState};

const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const TIMEOUT: Duration = Duration::from_secs(30);

/// A `StatusNotifier` watcher with a host: it lists the items registered
/// with it as KDE's watcher does (an item registered by object path is
/// listed as the sender's bus name followed by the path).
struct Watcher {
    items: Arc<Mutex<Vec<String>>>,
}

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    fn register_status_notifier_item(
        &self,
        service: &str,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) {
        let sender = header.sender().map(ToString::to_string).unwrap_or_default();
        let item = if service.starts_with('/') {
            format!("{sender}{service}")
        } else {
            service.to_owned()
        };
        self.items.lock().unwrap().push(item);
    }

    fn register_status_notifier_host(&self, _service: &str) {}

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.items.lock().unwrap().clone()
    }

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }
}

/// Starts the watcher on the session bus; it goes away when dropped.
fn start_watcher() -> (zbus::blocking::Connection, Arc<Mutex<Vec<String>>>) {
    let items = Arc::new(Mutex::new(Vec::new()));
    let connection = zbus::blocking::connection::Builder::session()
        .unwrap()
        .name(WATCHER)
        .unwrap()
        .serve_at(
            WATCHER_PATH,
            Watcher {
                items: items.clone(),
            },
        )
        .unwrap()
        .build()
        .expect("the test's own session bus (run under dbus-run-session)");
    (connection, items)
}

/// The agent executable with a profile of its own, with its tray.
struct TrayAgent {
    child: Child,
    paths: AgentPaths,
    _dir: tempfile::TempDir,
    runtime: tokio::runtime::Runtime,
}

impl TrayAgent {
    fn start() -> Self {
        assert!(
            std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some(),
            "needs a display: run under xvfb-run"
        );
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        for sub in ["home", "xdg-data", "xdg-config", "xdg-cache"] {
            std::fs::create_dir_all(dir.path().join(sub)).unwrap();
        }
        // XDG_RUNTIME_DIR stays the test's: the endpoint's socket is
        // there, and the test connects to it.
        let child = Command::new(env!("CARGO_BIN_EXE_deckpilot-agent"))
            .args(["--no-usb", "--disable-plugins"])
            .env_remove("DECKPILOT_TEST_AGENT_ARGS")
            .env("HOME", dir.path().join("home"))
            .env("XDG_DATA_HOME", dir.path().join("xdg-data"))
            .env("XDG_CONFIG_HOME", dir.path().join("xdg-config"))
            .env("XDG_CACHE_HOME", dir.path().join("xdg-cache"))
            .env("DECKPILOT_DATA_DIR", &data)
            .stdin(Stdio::null())
            .spawn()
            .expect("the agent starts");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        Self {
            child,
            paths: AgentPaths::in_dir(data),
            _dir: dir,
            runtime,
        }
    }

    fn request(&self, request: Request) -> Option<Response> {
        self.runtime.block_on(async {
            let mut client = AgentClient::connect(&self.paths, ClientKind::Other)
                .await
                .ok()?;
            client.request(request).await.ok()
        })
    }

    fn tray(&self) -> Option<TrayState> {
        match self.request(Request::GetStatus)? {
            Response::Status { status } => status.tray,
            _ => None,
        }
    }

    /// Waits until the agent reports `expected`; the states seen until then.
    fn wait_for(&mut self, expected: TrayState) -> Vec<Option<TrayState>> {
        let start = Instant::now();
        let mut seen = Vec::new();
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                panic!("the agent exited ({status}) before reporting {expected:?}; seen {seen:?}");
            }
            let tray = self.tray();
            if seen.last() != Some(&tray) {
                seen.push(tray);
            }
            if tray == Some(expected) {
                return seen;
            }
            assert!(
                start.elapsed() < TIMEOUT,
                "the agent did not report {expected:?} within {TIMEOUT:?}; seen {seen:?}"
            );
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    /// Quits it as the app does without a tray (`shutdown`) and checks
    /// that it ends cleanly.
    fn quit(mut self) {
        assert!(
            matches!(self.request(Request::Shutdown), Some(Response::Ok)),
            "shutdown was not accepted"
        );
        let start = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "the agent exited with {status}");
                return;
            }
            if start.elapsed() > TIMEOUT {
                let _ = self.child.kill();
                panic!("the agent did not stop within {TIMEOUT:?} of shutdown");
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

impl Drop for TrayAgent {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// The process id behind a registered item (`:1.42/org/ayatana/…`).
fn item_pid(connection: &zbus::blocking::Connection, item: &str) -> Option<u32> {
    let name = item.split_once('/').map_or(item, |(name, _)| name);
    let name = zbus::names::BusName::try_from(name).ok()?;
    zbus::blocking::fdo::DBusProxy::new(connection)
        .ok()?
        .get_connection_unix_process_id(name)
        .ok()
}

#[test]
#[ignore = "needs an X display and a private session bus (xvfb-run -a dbus-run-session)"]
fn the_tray_registers_with_a_host_and_the_agent_quits_without_one() {
    // No StatusNotifier host, as on GNOME without an AppIndicator
    // extension: the agent keeps running, says so, and quits on request.
    let mut agent = TrayAgent::start();
    let seen = agent.wait_for(TrayState::NoHost);
    assert!(
        !seen.contains(&Some(TrayState::Shown)),
        "reported shown without a host: {seen:?}"
    );
    agent.quit();

    // With a host: the agent's icon registers, and the agent reports it
    // shown.
    let (watcher, items) = start_watcher();
    let mut agent = TrayAgent::start();
    agent.wait_for(TrayState::Shown);
    let pid = agent.child.id();
    let registered = items.lock().unwrap().clone();
    assert!(
        registered
            .iter()
            .any(|item| item_pid(&watcher, item) == Some(pid)),
        "the agent's item is not among those registered: {registered:?}"
    );

    // The host goes away (the extension is switched off): the agent
    // notices, keeps running and still quits.
    drop(watcher);
    agent.wait_for(TrayState::NoHost);
    agent.quit();
}
