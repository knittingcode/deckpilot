//! A panic in the agent's main loop ends the process (`crash.rs`,
//! DP-AGENT-02): it is logged, the IPC endpoint closes, the instance lock
//! is released, the process exits non-zero, and a next agent starts.
//!
//! The agent is the real executable, made to panic by the test hook
//! `DECKPILOT_TEST_PANIC_AFTER_START_MS` (builds with `test-hooks` only).

mod support;

use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use deckpilot_agent::InstanceLock;
use deckpilot_agent::crash::PANIC_EXIT_CODE;
use deckpilot_agent::crash::testing::PANIC_AFTER_START_ENV;
use deckpilot_ipc::{AgentClient, AgentPaths, ClientError, ClientKind, Request};
use support::{TIMEOUT, TempDir, create_database};

fn data_dir(dir: &TempDir) -> AgentPaths {
    let paths = AgentPaths::in_dir(dir.path().join("data"));
    paths.create_dirs().unwrap();
    // Password-protected: the agent stays locked and never asks the
    // system's credential store for a key.
    create_database(&paths);
    paths
}

/// The agent headless (or with its tray), with a simulated MK.2, no USB
/// and no plugins; it panics `panic_after` after starting.
fn spawn(dir: &TempDir, paths: &AgentPaths, headless: bool, panic_after: Option<u64>) -> Child {
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.path().join("stderr.log"))
        .unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_deckpilot-agent"));
    if headless {
        command.arg("--headless");
    }
    command
        .args(["--no-usb", "--mock-devices", "mk2", "--disable-plugins"])
        .env("DECKPILOT_DATA_DIR", &paths.data_dir)
        .env_remove("DECKPILOT_TEST_AGENT_ARGS")
        .env_remove(PANIC_AFTER_START_ENV)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log);
    if let Some(ms) = panic_after {
        command.env(PANIC_AFTER_START_ENV, ms.to_string());
    }
    command.spawn().expect("the agent starts")
}

fn wait_for_exit(child: &mut Child, within: Duration) -> ExitStatus {
    let deadline = Instant::now() + within;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("the agent did not exit within {within:?}");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Everything the agent wrote to its log files.
fn agent_log(paths: &AgentPaths) -> String {
    let mut text = String::new();
    for entry in std::fs::read_dir(&paths.log_dir).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("agent") {
            text.push_str(&std::fs::read_to_string(entry.path()).unwrap_or_default());
        }
    }
    text
}

async fn connect(paths: &AgentPaths) -> AgentClient {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        match AgentClient::connect(paths, ClientKind::Other).await {
            Ok(client) => return client,
            Err(error) if Instant::now() < deadline => {
                let _ = error;
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(error) => panic!("cannot connect to the agent: {error}"),
        }
    }
}

/// After the panic: non-zero exit, the panic in the log, the lock free and
/// no endpoint left that a client could reach.
async fn assert_crashed(mut child: Child, paths: &AgentPaths) {
    let status = wait_for_exit(&mut child, TIMEOUT);
    assert_eq!(
        status.code(),
        Some(i32::from(PANIC_EXIT_CODE)),
        "exit status {status:?}"
    );
    let log = agent_log(paths);
    assert!(log.contains("agent started"), "{log}");
    assert!(
        log.contains("test hook: the agent's main loop panics"),
        "the panic is in the log: {log}"
    );
    assert!(
        log.contains("the agent's main loop panicked; the agent stops"),
        "{log}"
    );
    // Free in any case once the process is gone (an OS lock). That the lock
    // goes only after the endpoint is the socket check below (Unix) and the
    // unit test `crash::tests::a_panic_is_caught_and_stops_the_runtimes_tasks`.
    assert!(
        InstanceLock::acquire(&paths.lock_file).is_ok(),
        "the instance lock is released"
    );
    #[cfg(unix)]
    assert!(
        !std::path::Path::new(&paths.endpoint().display()).exists(),
        "the socket file is removed"
    );
    assert!(matches!(
        AgentClient::connect(paths, ClientKind::Other).await,
        Err(ClientError::NotRunning)
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_panic_in_the_main_loop_ends_the_headless_agent() {
    let dir = TempDir::new("deckpilot-crash-headless");
    let paths = data_dir(&dir);
    let child = spawn(&dir, &paths, true, Some(500));
    assert_crashed(child, &paths).await;

    // A next agent starts over what the crashed one left, and stops
    // normally with exit code 0.
    let mut next = spawn(&dir, &paths, true, None);
    let mut client = connect(&paths).await;
    client.request(Request::Shutdown).await.unwrap();
    drop(client);
    let status = wait_for_exit(&mut next, TIMEOUT);
    assert_eq!(status.code(), Some(0), "exit status {status:?}");
}

/// With the tray: the process used to stay alive, its tray menu doing
/// nothing. Only where a tray can be created without a display server to
/// set up (macOS and Windows; Linux CI has no display).
#[cfg(any(target_os = "macos", target_os = "windows"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_panic_in_the_main_loop_ends_the_tray_agent() {
    let dir = TempDir::new("deckpilot-crash-tray");
    let paths = data_dir(&dir);
    let child = spawn(&dir, &paths, false, Some(1000));
    assert_crashed(child, &paths).await;
}
