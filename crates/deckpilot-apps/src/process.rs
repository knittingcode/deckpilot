//! Running processes, as Close Application sees them: their id, parent,
//! executable, owner and session, never their command line or
//! environment. Which of them may be closed at all ([`protected`]) is
//! decided here, once, for every platform.

use std::path::{Path, PathBuf};

/// One running process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessInfo {
    pub pid: u32,
    pub parent: Option<u32>,
    /// The executable's file name as the system reports it.
    pub name: String,
    pub exe: Option<PathBuf>,
    /// The owner: a user id on Unix, a SID on Windows.
    pub owner: Option<String>,
    /// The login session (Windows), when known.
    pub session: Option<u32>,
    /// When it started, in seconds since the Unix epoch, when known. With
    /// the id, the executable and the owner it tells a process from a later
    /// one the system gave the same id.
    pub started: Option<u64>,
}

/// Every process, and which of them is this one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub processes: Vec<ProcessInfo>,
    /// This process (the agent, or the app).
    pub me: Option<ProcessInfo>,
}

impl Snapshot {
    pub fn get(&self, pid: u32) -> Option<&ProcessInfo> {
        self.processes.iter().find(|p| p.pid == pid)
    }

    /// Owned by the user this process runs as, in the same session where
    /// sessions are known. Unknown owners never count.
    pub fn owned_by_me(&self, process: &ProcessInfo) -> bool {
        let Some(me) = &self.me else {
            return false;
        };
        let owner = matches!((&process.owner, &me.owner), (Some(a), Some(b)) if a == b);
        let session = match (process.session, me.session) {
            (Some(a), Some(b)) => a == b,
            (None, None) => true,
            _ => false,
        };
        owner && session
    }
}

/// Programs that keep the session, the desktop or the system running, by
/// exact executable name (lower case, without `.exe`). Never closed.
pub const PROTECTED_NAMES: &[&str] = &[
    // macOS
    "launchd",
    "kernel_task",
    "windowserver",
    "loginwindow",
    "dock",
    "finder",
    "systemuiserver",
    "controlcenter",
    "coreservicesd",
    // Windows
    "system",
    "registry",
    "smss",
    "csrss",
    "wininit",
    "winlogon",
    "services",
    "lsass",
    "svchost",
    "dwm",
    "explorer",
    "sihost",
    "fontdrvhost",
    "ctfmon",
    "taskhostw",
    "runtimebroker",
    "startmenuexperiencehost",
    "shellexperiencehost",
    "searchhost",
    "textinputhost",
    // Linux
    "init",
    "systemd",
    "xorg",
    "xwayland",
    "x",
    "gnome-shell",
    "gnome-session",
    "gnome-session-binary",
    "gnome-session-service",
    "mutter",
    "kwin_x11",
    "kwin_wayland",
    "plasmashell",
    "ksmserver",
    "sway",
    "weston",
    "hyprland",
    "xfwm4",
    "xfce4-session",
    "lxsession",
    "cinnamon",
    "cinnamon-session",
    "mate-session",
    "dbus-daemon",
    "dbus-broker",
    "gdm",
    "gdm3",
    "sddm",
    "lightdm",
    "pipewire",
    "pulseaudio",
    "wireplumber",
    "polkitd",
    // DeckPilot itself
    "deckpilot",
    "deckpilot-agent",
    "deckpilot-desktop",
    "deckpilot-plugin-host",
    "deckpilot-sandbox-probe",
];

/// macOS bundles that keep the session running, and DeckPilot's own.
pub const PROTECTED_BUNDLES: &[&str] = &[
    "com.apple.finder",
    "com.apple.dock",
    "com.apple.loginwindow",
    "com.apple.systemuiserver",
    "com.apple.controlcenter",
    "com.apple.notificationcenterui",
    "com.apple.windowserver",
    "com.apple.windowmanager",
    "io.github.knittingcode.deckpilot",
    "io.github.knittingcode.deckpilot.agent",
];

/// An executable's name as compared with [`PROTECTED_NAMES`].
pub fn plain_name(name: &str) -> String {
    let lower = name.trim().to_lowercase();
    lower
        .strip_suffix(".exe")
        .map_or_else(|| lower.clone(), str::to_owned)
}

/// The deny-list's verdict for an executable path, whoever runs it.
pub fn protected_executable(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| PROTECTED_NAMES.contains(&plain_name(&name.to_string_lossy()).as_str()))
}

/// A bundle id on the deny-list (case ignored).
pub fn protected_bundle(identifier: &str) -> bool {
    let lower = identifier.to_ascii_lowercase();
    PROTECTED_BUNDLES.contains(&lower.as_str())
}

/// Whether `process` must never be closed: process ids 0 and 1, this
/// process, its parent (the app or the agent that started it), anything
/// running this process's executable or another executable of its
/// installation ([`installation_root`]: the app, the agent, the plugin
/// host), and the deny-list.
///
/// Descendants of this process are not protected as such: an application
/// that Open Application starts directly (Linux, Windows) is a child of
/// the agent, and closing it is the point. DeckPilot's own child, the
/// plugin host, is protected by its folder and its name.
pub fn protected(process: &ProcessInfo, snapshot: &Snapshot) -> bool {
    if process.pid <= 1 {
        return true;
    }
    if let Some(me) = &snapshot.me {
        if process.pid == me.pid || Some(process.pid) == me.parent {
            return true;
        }
        if let (Some(a), Some(b)) = (&process.exe, &me.exe)
            && (same_file(a, b) || installation_root(b).is_some_and(|root| inside(a, &root)))
        {
            return true;
        }
    }
    PROTECTED_NAMES.contains(&plain_name(&process.name).as_str())
        || process.exe.as_deref().is_some_and(protected_executable)
}

/// The folder DeckPilot is installed in, judged from the running
/// executable `exe`: its `.app` bundle on macOS, its own folder on Windows
/// (`C:\Program Files\DeckPilot`). `None` elsewhere: on Linux the agent
/// is often in `/usr/bin` next to every other program, so the names of
/// [`PROTECTED_NAMES`] protect DeckPilot's programs there.
pub fn installation_root(exe: &Path) -> Option<PathBuf> {
    if let Some(bundle) = exe
        .ancestors()
        .find(|a| a.extension().is_some_and(|e| e.eq_ignore_ascii_case("app")))
    {
        return Some(bundle.to_path_buf());
    }
    if cfg!(windows) {
        let folder = exe.parent()?;
        let name = folder.file_name()?.to_string_lossy().to_ascii_lowercase();
        // Never the Windows folder itself.
        if name != "system32" && name != "windows" && name != "syswow64" {
            return Some(folder.to_path_buf());
        }
    }
    None
}

fn inside(path: &Path, root: &Path) -> bool {
    path.starts_with(root)
        || match (std::fs::canonicalize(path), std::fs::canonicalize(root)) {
            (Ok(path), Ok(root)) => path.starts_with(root),
            _ => false,
        }
}

/// The same file: equal paths, or equal after resolving links.
pub fn same_file(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// This computer's processes, read with `sysinfo` (id, parent, name,
/// executable, owner; never command lines or environments).
pub fn snapshot() -> Snapshot {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_exe(UpdateKind::OnlyIfNotSet)
            .with_user(UpdateKind::OnlyIfNotSet)
            .without_tasks(),
    );
    let info = |pid: &sysinfo::Pid, process: &sysinfo::Process| ProcessInfo {
        pid: pid.as_u32(),
        parent: process.parent().map(sysinfo::Pid::as_u32),
        name: process.name().to_string_lossy().into_owned(),
        exe: process.exe().map(Path::to_path_buf),
        owner: process.user_id().map(|uid| (**uid).to_string()),
        session: session_of(pid.as_u32()),
        started: Some(process.start_time()).filter(|t| *t > 0),
    };
    let mut processes: Vec<ProcessInfo> = system
        .processes()
        .iter()
        .map(|(pid, process)| info(pid, process))
        .collect();
    processes.sort_by_key(|p| p.pid);
    let me_pid = std::process::id();
    let me = processes.iter().find(|p| p.pid == me_pid).cloned();
    Snapshot { processes, me }
}

#[cfg(windows)]
fn session_of(pid: u32) -> Option<u32> {
    crate::native_windows::session_of(pid)
}

#[cfg(not(windows))]
fn session_of(_pid: u32) -> Option<u32> {
    None
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn process(pid: u32, name: &str, exe: Option<&str>, owner: &str) -> ProcessInfo {
        ProcessInfo {
            pid,
            parent: Some(1),
            name: name.into(),
            exe: exe.map(PathBuf::from),
            owner: Some(owner.into()),
            session: None,
            started: None,
        }
    }

    pub fn world(processes: Vec<ProcessInfo>) -> Snapshot {
        let me = ProcessInfo {
            parent: Some(900),
            ..process(
                1000,
                "deckpilot-agent",
                Some("/opt/deckpilot/deckpilot-agent"),
                "501",
            )
        };
        let mut all = processes;
        all.push(me.clone());
        Snapshot {
            processes: all,
            me: Some(me),
        }
    }

    #[test]
    fn ownership_needs_the_same_user_and_session() {
        let snapshot = world(Vec::new());
        assert!(snapshot.owned_by_me(&process(5, "a", None, "501")));
        assert!(!snapshot.owned_by_me(&process(5, "a", None, "0")));
        let unknown = ProcessInfo {
            owner: None,
            ..process(5, "a", None, "501")
        };
        assert!(!snapshot.owned_by_me(&unknown));
        let other_session = ProcessInfo {
            session: Some(2),
            ..process(5, "a", None, "501")
        };
        assert!(!snapshot.owned_by_me(&other_session));
        assert!(!Snapshot::default().owned_by_me(&process(5, "a", None, "501")));
    }

    #[test]
    fn protected_processes_are_listed_once_for_every_platform() {
        let snapshot = world(Vec::new());
        for protected_one in [
            process(0, "kernel", None, "0"),
            process(1, "launchd", None, "0"),
            process(1000, "x", None, "501"),
            process(900, "DeckPilot", None, "501"),
            process(50, "explorer.exe", Some(r"C:\Windows\explorer.exe"), "S-1"),
            process(51, "csrss.exe", None, "S-1"),
            process(52, "WindowServer", None, "88"),
            process(53, "gnome-shell", Some("/usr/bin/gnome-shell"), "501"),
            process(54, "renamed", Some("/usr/bin/Xwayland"), "501"),
            process(55, "copy", Some("/opt/deckpilot/deckpilot-agent"), "501"),
            process(
                57,
                "deckpilot-plugin-host",
                Some("/elsewhere/deckpilot-plugin-host"),
                "501",
            ),
        ] {
            assert!(protected(&protected_one, &snapshot), "{protected_one:?}");
        }
        for allowed in [
            process(
                60,
                "Safari",
                Some("/Applications/Safari.app/Contents/MacOS/Safari"),
                "501",
            ),
            // Never substring matches: "explorer" in a longer name is fine.
            process(61, "explorer-helper", Some("/opt/explorer-helper"), "501"),
            process(62, "firefox", Some("/usr/lib/firefox/firefox"), "501"),
        ] {
            assert!(!protected(&allowed, &snapshot), "{allowed:?}");
        }
        assert!(protected_bundle("com.apple.Finder"));
        assert!(protected_bundle("io.github.knittingcode.deckpilot"));
        assert!(!protected_bundle("com.apple.Safari"));
    }

    #[test]
    fn deckpilots_installation_is_protected_but_not_its_folder_neighbours() {
        let agent = "/Applications/DeckPilot.app/Contents/MacOS/deckpilot-agent";
        let me = ProcessInfo {
            parent: Some(900),
            ..process(1000, "deckpilot-agent", Some(agent), "501")
        };
        let snapshot = Snapshot {
            processes: vec![me.clone()],
            me: Some(me),
        };
        let host = process(
            70,
            "host",
            Some("/Applications/DeckPilot.app/Contents/Resources/plugins/host"),
            "501",
        );
        assert!(protected(&host, &snapshot));
        let other = process(
            71,
            "Safari",
            Some("/Applications/Safari.app/Contents/MacOS/Safari"),
            "501",
        );
        assert!(!protected(&other, &snapshot));
        assert_eq!(
            installation_root(Path::new(agent)),
            Some(PathBuf::from("/Applications/DeckPilot.app"))
        );
        // Linux: no folder rule (the agent may be in /usr/bin).
        if !cfg!(windows) {
            assert_eq!(
                installation_root(Path::new("/usr/bin/deckpilot-agent")),
                None
            );
        }
    }

    #[test]
    fn the_real_snapshot_knows_this_process() {
        let snapshot = snapshot();
        let me = snapshot.me.clone().expect("this process is listed");
        assert_eq!(me.pid, std::process::id());
        assert!(me.owner.is_some());
        assert!(snapshot.owned_by_me(&me));
        assert!(protected(&me, &snapshot));
        assert!(me.started.is_some(), "the start time is read");
    }
}
