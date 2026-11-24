//! The close procedure against a fake platform: which processes are
//! matched, asked, forced and left alone.

use std::sync::Mutex;

use super::*;
use crate::process::tests::{process, world};

/// A platform whose processes quit when asked (unless stubborn), and
/// which records what it was asked.
#[derive(Debug)]
struct Fake {
    snapshot: Snapshot,
    bundles: Vec<(String, u32)>,
    aumids: Vec<(u32, String)>,
    /// Ignore a graceful request.
    stubborn: Vec<u32>,
    /// No window to ask (Windows).
    windowless: Vec<u32>,
    /// Asking fails.
    refusing: Vec<u32>,
    gone: Mutex<Vec<u32>>,
    log: Mutex<Vec<String>>,
    /// The run is cancelled (or out of time) at the first wait.
    cancelled: bool,
    elevated: bool,
}

impl Fake {
    fn new(processes: Vec<ProcessInfo>) -> Self {
        Self {
            snapshot: world(processes),
            bundles: Vec::new(),
            aumids: Vec::new(),
            stubborn: Vec::new(),
            windowless: Vec::new(),
            refusing: Vec::new(),
            gone: Mutex::new(Vec::new()),
            log: Mutex::new(Vec::new()),
            cancelled: false,
            elevated: false,
        }
    }

    fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

impl Control for Fake {
    fn snapshot(&self) -> Snapshot {
        self.snapshot.clone()
    }

    fn bundle_pids(&self, id: &str) -> Vec<u32> {
        self.bundles
            .iter()
            .filter(|(b, _)| b == id)
            .map(|(_, pid)| *pid)
            .collect()
    }

    fn aumid(&self, pid: u32) -> Option<String> {
        self.aumids
            .iter()
            .find(|(p, _)| *p == pid)
            .map(|(_, a)| a.clone())
    }

    fn request_quit(&self, _target: &Target, process: &ProcessInfo) -> Result<bool, AppError> {
        self.log
            .lock()
            .unwrap()
            .push(format!("quit {}", process.pid));
        if self.refusing.contains(&process.pid) {
            return Err(AppError::new(codes::CLOSE_FAILED, "refused"));
        }
        if self.windowless.contains(&process.pid) {
            return Ok(false);
        }
        if !self.stubborn.contains(&process.pid) {
            self.gone.lock().unwrap().push(process.pid);
        }
        Ok(true)
    }

    fn force_quit(&self, _target: &Target, process: &ProcessInfo) -> Result<(), AppError> {
        self.log
            .lock()
            .unwrap()
            .push(format!("force {}", process.pid));
        self.gone.lock().unwrap().push(process.pid);
        Ok(())
    }

    fn running(&self, process: &ProcessInfo) -> bool {
        !self.gone.lock().unwrap().contains(&process.pid)
    }

    fn wait(&self, _duration: Duration) -> bool {
        !self.cancelled
    }

    fn elevated(&self) -> bool {
        self.elevated
    }
}

const EDITOR: &str = "/opt/editor/bin/editor";

fn quick(force: bool) -> Options {
    Options {
        force,
        grace: Duration::from_millis(1),
    }
}

fn editor_target() -> Target {
    Target::Executable(EDITOR.into())
}

#[test]
fn every_instance_the_user_owns_is_asked_and_nothing_else() {
    let fake = Fake::new(vec![
        process(10, "editor", Some(EDITOR), "501"),
        process(11, "editor", Some(EDITOR), "501"),
        // Another user's instance.
        process(12, "editor", Some(EDITOR), "502"),
        // Same name, different executable: never matched.
        process(13, "editor", Some("/usr/bin/editor"), "501"),
        // A name that contains it: never matched.
        process(
            14,
            "editor-helper",
            Some("/opt/editor/bin/editor-helper"),
            "501",
        ),
    ]);
    let report = close(&fake, &editor_target(), quick(false)).unwrap();
    assert_eq!(
        report,
        Report {
            instances: 2,
            asked: 2,
            forced: 0,
            remaining: 0,
            skipped: 1,
            interrupted: false,
        }
    );
    assert_eq!(fake.log(), ["quit 10", "quit 11"]);
}

#[test]
fn force_is_only_used_when_asked_for() {
    let mut fake = Fake::new(vec![process(10, "editor", Some(EDITOR), "501")]);
    fake.stubborn = vec![10];
    let report = close(&fake, &editor_target(), quick(false)).unwrap();
    assert_eq!((report.asked, report.forced, report.remaining), (1, 0, 1));
    assert_eq!(fake.log(), ["quit 10"]);

    let report = close(&fake, &editor_target(), quick(true)).unwrap();
    assert_eq!((report.forced, report.remaining), (1, 0));
    assert_eq!(fake.log(), ["quit 10", "quit 10", "force 10"]);
}

#[test]
fn a_windowless_process_is_only_closed_by_force() {
    let mut fake = Fake::new(vec![process(10, "editor", Some(EDITOR), "501")]);
    fake.windowless = vec![10];
    let report = close(&fake, &editor_target(), quick(false)).unwrap();
    assert_eq!((report.asked, report.remaining), (0, 1));
    let report = close(&fake, &editor_target(), quick(true)).unwrap();
    assert_eq!((report.forced, report.remaining), (1, 0));
}

#[test]
fn a_failure_is_reported_when_nothing_could_be_done() {
    let mut fake = Fake::new(vec![process(10, "editor", Some(EDITOR), "501")]);
    fake.refusing = vec![10];
    fake.stubborn = vec![10];
    let error = close(&fake, &editor_target(), quick(false)).unwrap_err();
    assert_eq!(error.code, codes::CLOSE_FAILED);
}

#[test]
fn a_cancelled_or_late_run_never_forces() {
    // The application may be asking to save: the run ends, nothing is
    // killed, and the result says so.
    let mut fake = Fake::new(vec![process(10, "editor", Some(EDITOR), "501")]);
    fake.stubborn = vec![10];
    fake.cancelled = true;
    let options = Options {
        force: true,
        grace: Duration::from_secs(5),
    };
    let report = close(&fake, &editor_target(), options).unwrap();
    assert!(report.interrupted);
    assert_eq!((report.asked, report.forced, report.remaining), (1, 0, 1));
    assert_eq!(fake.log(), ["quit 10"]);
}

#[test]
fn nothing_is_closed_when_running_as_root_or_elevated() {
    let mut fake = Fake::new(vec![process(10, "editor", Some(EDITOR), "501")]);
    fake.elevated = true;
    let error = close(&fake, &editor_target(), quick(true)).unwrap_err();
    assert_eq!(error.code, codes::ELEVATED);
    assert_eq!(fake.log(), Vec::<String>::new());
}

#[test]
fn nothing_running_is_a_success() {
    let fake = Fake::new(Vec::new());
    assert_eq!(
        close(&fake, &editor_target(), quick(true)).unwrap(),
        Report::default()
    );
}

#[test]
fn bundles_and_aumids_match_by_id_with_the_owner_check() {
    let mut fake = Fake::new(vec![
        process(
            20,
            "Safari",
            Some("/Applications/Safari.app/Contents/MacOS/Safari"),
            "501",
        ),
        process(
            21,
            "Safari",
            Some("/Applications/Safari.app/Contents/MacOS/Safari"),
            "0",
        ),
        process(
            30,
            "Calculator",
            Some(r"C:\Program Files\WindowsApps\calc.exe"),
            "501",
        ),
    ]);
    fake.bundles = vec![
        ("com.apple.Safari".into(), 20),
        ("com.apple.Safari".into(), 21),
    ];
    fake.aumids = vec![(30, "Microsoft.Calc_8wekyb3d8bbwe!App".into())];
    let report = close(
        &fake,
        &Target::Bundle("com.apple.Safari".into()),
        quick(false),
    )
    .unwrap();
    assert_eq!((report.instances, report.skipped), (1, 1));
    let report = close(
        &fake,
        &Target::Aumid("Microsoft.Calc_8wekyb3d8bbwe!App".into()),
        quick(false),
    )
    .unwrap();
    assert_eq!(report.instances, 1);
    assert_eq!(fake.log(), ["quit 20", "quit 30"]);
}

#[test]
fn protected_processes_are_never_touched_even_when_they_match() {
    // An executable target that is this process's own executable, and its
    // parent: both protected.
    let fake = Fake::new(vec![ProcessInfo {
        pid: 900,
        ..process(900, "x", Some("/opt/deckpilot/deckpilot-agent"), "501")
    }]);
    let report = close(
        &fake,
        &Target::Executable("/opt/deckpilot/deckpilot-agent".into()),
        quick(true),
    )
    .unwrap();
    assert_eq!((report.instances, report.skipped), (0, 2));
    assert_eq!(fake.log(), Vec::<String>::new());
}

#[test]
fn a_reused_process_id_is_not_the_matched_process() {
    // This process, as the real snapshot reads it, is still itself.
    let me = crate::process::snapshot()
        .me
        .expect("this process is listed");
    assert!(still_same(&me));
    // The same id, executable and owner, but started at another time: a
    // later process the system gave the same id.
    if let Some(started) = me.started {
        let later = ProcessInfo {
            started: Some(started + 1),
            ..me.clone()
        };
        assert!(!still_same(&later));
    } else {
        eprintln!(
            "this system reports no start time; only the id, executable and owner are compared"
        );
    }
    let other_owner = ProcessInfo {
        owner: Some("no-such-user".into()),
        ..me
    };
    assert!(!still_same(&other_owner));
    // Unknown on either side: decided by the id, executable and owner.
    assert!(same_start(None, Some(5)) && same_start(Some(5), None));
    assert!(same_start(Some(5), Some(5)) && !same_start(Some(5), Some(6)));
}

mod targets {
    use super::*;

    fn dirs_with(entries: &[(&str, &str)], bin: &Path) -> (tempfile::TempDir, XdgDirs) {
        let data = tempfile::tempdir().unwrap();
        let apps = data.path().join("applications");
        std::fs::create_dir_all(&apps).unwrap();
        for (id, exec) in entries {
            std::fs::write(
                apps.join(id),
                format!("[Desktop Entry]\nType=Application\nName=N\nExec={exec}\n"),
            )
            .unwrap();
        }
        let dirs = XdgDirs {
            path: Some(std::env::join_paths([bin]).unwrap()),
            ..XdgDirs::only(&[data.path()])
        };
        (data, dirs)
    }

    #[test]
    fn desktop_entries_resolve_to_their_exact_executable() {
        let bin = tempfile::tempdir().unwrap();
        std::fs::write(bin.path().join("editor"), [0x7f, b'E', b'L', b'F']).unwrap();
        std::fs::write(bin.path().join("launcher.sh"), "#!/bin/sh\nexec editor\n").unwrap();
        std::fs::write(bin.path().join("flatpak"), [0x7f, b'E']).unwrap();
        std::fs::write(bin.path().join("gnome-shell"), [0x7f, b'E']).unwrap();
        let (_data, dirs) = dirs_with(
            &[
                ("editor.desktop", "editor %F"),
                ("script.desktop", "launcher.sh"),
                ("sandboxed.desktop", "flatpak run org.example.App"),
                ("shell.desktop", "gnome-shell"),
            ],
            bin.path(),
        );
        let canonical = std::fs::canonicalize(bin.path().join("editor")).unwrap();
        assert_eq!(
            target(Platform::Linux, "editor.desktop", &dirs).unwrap(),
            Target::Executable(canonical)
        );
        for (id, code) in [
            ("script.desktop", codes::NOT_IDENTIFIABLE),
            ("sandboxed.desktop", codes::NOT_IDENTIFIABLE),
            ("shell.desktop", codes::PROTECTED_PROCESS),
            ("missing.desktop", codes::NOT_FOUND),
        ] {
            assert_eq!(
                target(Platform::Linux, id, &dirs).unwrap_err().code,
                code,
                "{id}"
            );
        }
    }

    #[test]
    fn bundles_are_closed_by_identifier() {
        let dirs = XdgDirs::only(&[]);
        assert_eq!(
            target(Platform::MacOs, "com.apple.Safari", &dirs).unwrap(),
            Target::Bundle("com.apple.Safari".into())
        );
        for protected_one in ["com.apple.finder", "io.github.knittingcode.deckpilot"] {
            assert_eq!(
                target(Platform::MacOs, protected_one, &dirs)
                    .unwrap_err()
                    .code,
                codes::PROTECTED_PROCESS
            );
        }
        // A name is not an identity.
        assert_eq!(
            target(Platform::MacOs, "Safari", &dirs).unwrap_err().code,
            codes::NOT_IDENTIFIABLE
        );
        let apps = tempfile::tempdir().unwrap();
        let safari = crate::bundle::tests::fake_bundle(
            apps.path(),
            "Safari",
            Some("com.apple.Safari"),
            true,
        );
        assert_eq!(
            target(Platform::MacOs, &safari.to_string_lossy(), &dirs).unwrap(),
            Target::Bundle("com.apple.Safari".into())
        );
        let gone = apps.path().join("Gone.app");
        assert_eq!(
            target(Platform::MacOs, &gone.to_string_lossy(), &dirs)
                .unwrap_err()
                .code,
            codes::NOT_FOUND
        );
    }

    #[test]
    fn windows_ids_and_shortcuts() {
        let dirs = XdgDirs::only(&[]);
        assert_eq!(
            target(Platform::Windows, "Microsoft.Calc_8wekyb3d8bbwe!App", &dirs).unwrap(),
            Target::Aumid("Microsoft.Calc_8wekyb3d8bbwe!App".into())
        );
        let dir = tempfile::tempdir().unwrap();
        let shortcut = dir.path().join("App.lnk");
        std::fs::write(&shortcut, "").unwrap();
        assert_eq!(
            target(Platform::current(), &shortcut.to_string_lossy(), &dirs)
                .unwrap_err()
                .code,
            codes::NOT_IDENTIFIABLE
        );
        assert_eq!(
            target(Platform::Windows, "deckpilot-none.exe", &dirs)
                .unwrap_err()
                .code,
            codes::NOT_FOUND
        );
    }
}
