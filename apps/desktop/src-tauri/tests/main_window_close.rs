//! The main window's close button on a build of the app (Windows, Linux):
//! the window closes and, as the last window, ends the app
//! (`lifecycle::main_close`). From 0.10.0 to 0.12.0 a page listener held
//! every close back and the button did nothing.
//!
//! The app runs as built (`DECKPILOT_E2E_APP`, desktop-e2e.yml), without an
//! agent, with an empty data directory; the close comes from outside, as the
//! title bar's button sends it (`tools/ci/close-window.ps1`: `WM_SYSCOMMAND`
//! `SC_CLOSE`; `tools/ci/close-window.py`: `WM_DELETE_WINDOW` under Xvfb). macOS
//! hides the window instead and is not covered here.

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

/// Ends the app however it is, for a failing test.
fn kill(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
#[ignore = "needs the built app (DECKPILOT_E2E_APP) and a display (desktop-e2e.yml)"]
fn the_close_button_closes_the_main_window_and_ends_the_app() {
    if cfg!(target_os = "macos") {
        eprintln!("macOS hides the main window instead (lifecycle::main_close): not covered");
        return;
    }
    let app = std::env::var_os("DECKPILOT_E2E_APP")
        .map(PathBuf::from)
        .expect("DECKPILOT_E2E_APP is not set");
    let scratch = std::env::temp_dir().join(format!("deckpilot-close-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(scratch.join("data")).unwrap();
    let log = scratch.join("app.log");
    let file = std::fs::File::create(&log).unwrap();
    let mut child = Command::new(&app)
        .env("DECKPILOT_DATA_DIR", scratch.join("data"))
        .env("DECKPILOT_NO_AGENT_AUTOSTART", "1")
        .stdin(Stdio::null())
        .stdout(file.try_clone().unwrap())
        .stderr(file)
        .spawn()
        .expect("the app starts");
    let pid = child.id();

    let deadline = Instant::now() + Duration::from_secs(90);
    let shown = loop {
        let found = window(pid, "find");
        if found.status.success() {
            break text(&found);
        }
        if let Ok(Some(status)) = child.try_wait() {
            panic!(
                "the app ended ({status}) before showing its window: {}",
                log_of(&log)
            );
        }
        if Instant::now() > deadline {
            kill(&mut child);
            panic!(
                "no window 'DeckPilot' of pid {pid} within 90 s: {}",
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

    let sent = window(pid, "close");
    if !sent.status.success() {
        kill(&mut child);
        panic!("cannot close the window: {}", text(&sent));
    }
    eprintln!("[close] {}", text(&sent));

    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() > deadline {
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
        30.0 - deadline
            .saturating_duration_since(Instant::now())
            .as_secs_f64()
    );
    let _ = std::fs::remove_dir_all(&scratch);
}
