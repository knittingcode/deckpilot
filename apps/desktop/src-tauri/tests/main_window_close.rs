//! The main window's close button on a build of the app (Windows, Linux):
//! the window closes and, as the last window, ends the app, while the agent
//! keeps running (`lifecycle::main_close`). From 0.10.0 to 0.12.0 a page
//! listener held every close back and the button did nothing.
//!
//! The app runs as built (`DECKPILOT_E2E_APP`, desktop-e2e.yml) with an
//! empty data directory and starts the built agent (`DECKPILOT_AGENT_BIN`,
//! default `target/debug`), which creates the configuration with a key in
//! the system store (Credential Manager; on Linux a Secret Service,
//! `tools/ci/with-keyring.sh`), so the main window is shown as on a first
//! launch. The close comes from outside, as the title bar's button sends it
//! (`tools/ci/close-window.ps1`: `WM_SYSCOMMAND` `SC_CLOSE`;
//! `tools/ci/close-window.py`: `WM_DELETE_WINDOW` under Xvfb). macOS hides
//! the window instead and is not covered here.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

/// Until the page has loaded and set up its listeners.
const PAGE_LOAD: Duration = Duration::from_secs(15);

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

/// `find` or `close` the app's window titled DeckPilot.
fn window(pid: u32, mode: &str) -> Output {
    let tools = repo_root().join("tools/ci");
    let mut command = if cfg!(windows) {
        let mut c = Command::new("powershell");
        c.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(tools.join("close-window.ps1"))
            .args(["-AppPid", &pid.to_string(), "-Mode", mode]);
        c
    } else {
        let mut c = Command::new("python3");
        c.arg(tools.join("close-window.py"))
            .args([&pid.to_string(), mode]);
        c
    };
    command.output().expect("the window helper runs")
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout).trim(),
        String::from_utf8_lossy(&output.stderr).trim()
    )
}

fn log_of(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// Whether a `deckpilot-agent` process runs.
fn agent_running() -> bool {
    let output = if cfg!(windows) {
        Command::new("tasklist")
            .args(["/FI", "IMAGENAME eq deckpilot-agent.exe", "/NH"])
            .output()
    } else {
        Command::new("pgrep")
            .args(["-x", "deckpilot-agent"])
            .output()
    };
    output.is_ok_and(|o| {
        o.status.success()
            && String::from_utf8_lossy(&o.stdout).contains(|c: char| c.is_ascii_digit())
    })
}

/// Stops the agents this test's app started (it keeps them running).
fn stop_agents() {
    let _ = if cfg!(windows) {
        Command::new("taskkill")
            .args(["/F", "/IM", "deckpilot-agent.exe"])
            .output()
    } else {
        Command::new("pkill")
            .args(["-x", "deckpilot-agent"])
            .output()
    };
}

/// Ends the app and its agent however they are, for a failing test.
fn kill(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
    stop_agents();
}

#[test]
#[ignore = "needs the built app (DECKPILOT_E2E_APP), a display and a key store (desktop-e2e.yml)"]
fn the_close_button_closes_the_main_window_and_ends_the_app() {
    if cfg!(target_os = "macos") {
        eprintln!("macOS hides the main window instead (lifecycle::main_close): not covered");
        return;
    }
    let app = std::env::var_os("DECKPILOT_E2E_APP")
        .map(PathBuf::from)
        .expect("DECKPILOT_E2E_APP is not set");
    let agent = std::env::var_os("DECKPILOT_AGENT_BIN").map_or_else(
        || {
            repo_root()
                .join("target/debug")
                .join(format!("deckpilot-agent{}", std::env::consts::EXE_SUFFIX))
        },
        PathBuf::from,
    );
    assert!(agent.is_file(), "no agent at {}", agent.display());
    assert!(!agent_running(), "an agent runs already");
    let scratch = std::env::temp_dir().join(format!("deckpilot-close-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(scratch.join("data")).unwrap();
    let log = scratch.join("app.log");
    let file = std::fs::File::create(&log).unwrap();
    let mut child = Command::new(&app)
        .env("DECKPILOT_DATA_DIR", scratch.join("data"))
        .env("DECKPILOT_AGENT_BIN", &agent)
        .env_remove("DECKPILOT_NO_AGENT_AUTOSTART")
        .stdin(Stdio::null())
        .stdout(file.try_clone().unwrap())
        .stderr(file)
        .spawn()
        .expect("the app starts");
    let pid = child.id();

    let deadline = Instant::now() + Duration::from_secs(120);
    let shown = loop {
        let found = window(pid, "find");
        if found.status.success() {
            break text(&found);
        }
        if let Ok(Some(status)) = child.try_wait() {
            stop_agents();
            panic!(
                "the app ended ({status}) before showing its window: {}",
                log_of(&log)
            );
        }
        if Instant::now() > deadline {
            kill(&mut child);
            panic!(
                "no window 'DeckPilot' of pid {pid} within 120 s (agent running: {}): {}",
                agent_running(),
                log_of(&log)
            );
        }
        std::thread::sleep(Duration::from_millis(500));
    };
    eprintln!("[close] {shown}");
    std::thread::sleep(PAGE_LOAD);
    assert!(
        child.try_wait().unwrap().is_none(),
        "the app ended by itself: {}",
        log_of(&log)
    );
    assert!(
        agent_running(),
        "the app started no agent: {}",
        log_of(&log)
    );

    let sent = window(pid, "close");
    if !sent.status.success() {
        kill(&mut child);
        panic!("cannot close the window: {}", text(&sent));
    }
    eprintln!("[close] {}", text(&sent));

    let closed = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if closed.elapsed() > Duration::from_secs(30) {
            let still = text(&window(pid, "find"));
            kill(&mut child);
            panic!(
                "the main window's close button did not end the app within 30 s \
                 (window still there: {still}); log: {}",
                log_of(&log)
            );
        }
        std::thread::sleep(Duration::from_millis(250));
    };
    eprintln!(
        "[close] the app (pid {pid}) ended with its last window: {status}, {:.1} s after the close",
        closed.elapsed().as_secs_f64()
    );
    std::thread::sleep(Duration::from_secs(2));
    let agent_stays = agent_running();
    stop_agents();
    assert!(agent_stays, "the agent ended with the window");
    eprintln!("[close] the agent keeps running after the window closed");
    let _ = std::fs::remove_dir_all(&scratch);
}
