//! Real launches and closes, only ever of the test helper this crate
//! builds (`src/bin/deckpilot-apps-test-helper.rs`), started by the test
//! itself. No other program is opened, matched or signalled: every target
//! is the helper's exact executable, and the procedure only touches
//! processes owned by the user that runs the tests.

use std::io::{BufRead as _, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use deckpilot_apps::Platform;
use deckpilot_apps::close::{self, Options, SystemControl, Target};
use deckpilot_apps::xdg::XdgDirs;

const HELPER: &str = env!("CARGO_BIN_EXE_deckpilot-apps-test-helper");

/// A started helper: its process id, and a channel that receives once it
/// has exited (it is reaped at once, so it never lingers as a zombie).
struct Started {
    pid: u32,
    exited: mpsc::Receiver<()>,
}

fn start(program: &std::path::Path, arguments: &[String]) -> Started {
    let mut child = Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("the helper starts");
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert_eq!(line.trim(), "ready");
    let pid = child.id();
    let (sender, exited) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = child.wait();
        let _ = sender.send(());
    });
    Started { pid, exited }
}

fn helper_target() -> Target {
    close::target(Platform::current(), HELPER, &XdgDirs::only(&[])).unwrap()
}

fn has_exited(started: &Started) -> bool {
    started.exited.recv_timeout(Duration::from_secs(5)).is_ok()
}

fn quick(force: bool) -> Options {
    Options {
        force,
        grace: Duration::from_millis(1500),
    }
}

/// One test at a time: every helper matches the same target. Ends any
/// helper still running when the test ends, even when it failed.
struct Serial(#[allow(dead_code)] std::sync::MutexGuard<'static, ()>);

static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serial() -> Serial {
    Serial(
        LOCK.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    )
}

impl Drop for Serial {
    fn drop(&mut self) {
        let _ = close::close(&SystemControl, &helper_target(), quick(true));
    }
}

#[test]
fn the_helper_is_matched_by_its_exact_executable_only() {
    let target = helper_target();
    let Target::Executable(path) = &target else {
        panic!("{target:?}");
    };
    assert_eq!(path, &std::fs::canonicalize(HELPER).unwrap());
}

#[test]
fn closing_without_force_never_kills_a_program_that_stays() {
    let _serial = serial();
    let started = start(PathBuf::from(HELPER).as_path(), &["stubborn".to_owned()]);
    let control = SystemControl;
    let (instances, _) = close::instances(&control, &helper_target());
    assert!(
        instances.iter().any(|p| p.pid == started.pid),
        "{instances:?}"
    );

    // Unix: SIGTERM is ignored. Windows: the helper has no window to close.
    let report = close::close(&control, &helper_target(), quick(false)).unwrap();
    assert!(report.remaining >= 1, "{report:?}");
    assert_eq!(report.forced, 0);
    assert!(started.exited.try_recv().is_err(), "it was not killed");

    // With force it ends.
    let report = close::close(&control, &helper_target(), quick(true)).unwrap();
    assert!(report.forced >= 1, "{report:?}");
    assert!(has_exited(&started));
}

#[cfg(unix)]
#[test]
fn a_graceful_close_ends_a_program_that_quits_when_asked() {
    let _serial = serial();
    let started = start(PathBuf::from(HELPER).as_path(), &["sleep".to_owned()]);
    let report = close::close(&SystemControl, &helper_target(), quick(false)).unwrap();
    assert!(report.asked >= 1, "{report:?}");
    assert_eq!(report.forced, 0);
    assert!(has_exited(&started));
}

#[cfg(windows)]
#[test]
fn a_graceful_close_sends_wm_close_to_the_window() {
    let _serial = serial();
    let started = start(PathBuf::from(HELPER).as_path(), &["window".to_owned()]);
    let report = close::close(&SystemControl, &helper_target(), quick(false)).unwrap();
    assert!(report.asked >= 1, "{report:?}");
    assert_eq!(report.forced, 0);
    assert!(has_exited(&started));
}

/// Linux: the helper installed as a desktop entry is launched and closed
/// by its desktop-file id.
#[cfg(target_os = "linux")]
#[test]
fn a_desktop_entry_is_launched_and_closed_by_its_id() {
    use deckpilot_apps::{Launchable, launchable_in};

    let _serial = serial();
    let data = tempfile::tempdir().unwrap();
    let apps = data.path().join("applications");
    std::fs::create_dir_all(&apps).unwrap();
    std::fs::write(
        apps.join("io.github.knittingcode.DeckPilotTestHelper.desktop"),
        format!("[Desktop Entry]\nType=Application\nName=Helper\nExec={HELPER} %u\n"),
    )
    .unwrap();
    let dirs = XdgDirs::only(&[data.path()]);
    let id = "io.github.knittingcode.DeckPilotTestHelper.desktop";
    let Launchable::Program { program, arguments } =
        launchable_in(Platform::Linux, id, &["sleep".to_owned()], &dirs).unwrap()
    else {
        panic!("a desktop entry starts its program");
    };
    assert_eq!(arguments, ["sleep"]);
    let started = start(&program, &arguments);
    let target = close::target(Platform::Linux, id, &dirs).unwrap();
    assert_eq!(target, helper_target());
    let report = close::close(&SystemControl, &target, quick(false)).unwrap();
    assert!(report.asked >= 1, "{report:?}");
    assert!(has_exited(&started));
}

#[test]
fn nothing_is_closed_when_the_helper_is_not_running() {
    // Another test may have a helper running; this one only checks that a
    // target nobody runs closes nothing.
    let dir = tempfile::tempdir().unwrap();
    let copy = dir.path().join(if cfg!(windows) {
        "deckpilot-apps-idle.exe"
    } else {
        "deckpilot-apps-idle"
    });
    std::fs::copy(HELPER, &copy).unwrap();
    let target = close::target(
        Platform::current(),
        &copy.to_string_lossy(),
        &XdgDirs::only(&[]),
    )
    .unwrap();
    let report = close::close(&SystemControl, &target, quick(true)).unwrap();
    assert_eq!(report, close::Report::default());
}
