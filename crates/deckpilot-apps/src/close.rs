//! Close Application: which running processes are the application, and
//! closing them gracefully first, forced only when asked.
//!
//! A [`Target`] is identified by a stable id or an exact executable only;
//! process names are never compared. Of the matching processes only those
//! the current user owns are touched, never a protected one
//! ([`crate::process::protected`]), and every instance is closed (the
//! documented semantic for several instances). The platform calls are
//! behind [`Control`], so the whole procedure is tested with a fake one.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::process::{self, ProcessInfo, Snapshot};
use crate::xdg::{self, XdgDirs};
use crate::{AppError, AppId, Platform, bundle, codes, windows_apps};

/// How long a graceful close may take before force (or before it is
/// reported as still running).
pub const GRACE: Duration = Duration::from_secs(5);
/// How long a forced close may take to show.
pub const FORCE_WAIT: Duration = Duration::from_secs(2);
/// How often the processes are looked at while waiting.
pub const POLL: Duration = Duration::from_millis(100);

/// What a Close Application action closes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// macOS: every running application with this bundle id.
    Bundle(String),
    /// Windows: every process with this `AppUserModelID`.
    Aumid(String),
    /// Every process running exactly this executable (canonical path).
    Executable(PathBuf),
}

impl Target {
    /// How the target is named to the user.
    pub fn describe(&self) -> String {
        match self {
            Self::Bundle(id) | Self::Aumid(id) => id.clone(),
            Self::Executable(path) => path.display().to_string(),
        }
    }
}

fn not_identifiable(message: impl Into<String>) -> AppError {
    AppError::new(codes::NOT_IDENTIFIABLE, message)
}

/// Launchers and interpreters: a program started through one cannot be
/// told apart from every other program it starts.
const WRAPPERS: &[&str] = &[
    "env",
    "sh",
    "bash",
    "dash",
    "zsh",
    "fish",
    "ksh",
    "csh",
    "tcsh",
    "flatpak",
    "snap",
    "flatpak-spawn",
    "gtk-launch",
    "gio",
    "xdg-open",
    "kioclient",
    "kioclient5",
    "kde-open",
    "kde-open5",
    "exo-open",
    "python",
    "python3",
    "perl",
    "ruby",
    "node",
    "java",
    "mono",
    "dotnet",
    "wine",
    "wine64",
    "sudo",
    "pkexec",
    "nohup",
    "setsid",
    "systemd-run",
    "cmd",
    "powershell",
    "pwsh",
    "wscript",
    "cscript",
    "rundll32",
    "open",
];

fn wrapper(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|n| process::plain_name(&n.to_string_lossy()))
        .unwrap_or_default();
    let base = name.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    WRAPPERS.contains(&name.as_str()) || WRAPPERS.contains(&base)
}

/// A file that starts with `#!`: a script run by an interpreter.
fn script(path: &Path) -> bool {
    use std::io::Read as _;
    let mut head = [0u8; 2];
    std::fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut head))
        .is_ok_and(|()| head == *b"#!")
}

/// The canonical executable `program` is, refusing what cannot identify
/// an application.
fn executable(program: &Path, what: &str) -> Result<Target, AppError> {
    let real = std::fs::canonicalize(program).map_err(|_| {
        AppError::new(
            codes::NOT_FOUND,
            format!(
                "{} does not exist; it may have been moved or uninstalled",
                program.display()
            ),
        )
    })?;
    if !real.is_file() {
        return Err(not_identifiable(format!(
            "{what} is not an executable file"
        )));
    }
    if wrapper(&real) || wrapper(program) || script(&real) {
        return Err(not_identifiable(format!(
            "{what} starts through {} (a script, an interpreter or a launcher), so its processes \
             cannot be told apart from others; bind the application's own executable instead",
            real.display()
        )));
    }
    if process::protected_executable(&real) {
        return Err(AppError::new(
            codes::PROTECTED_PROCESS,
            format!("{what} is part of the system or of DeckPilot and is never closed"),
        ));
    }
    Ok(Target::Executable(real))
}

/// What application `value` (an application resource's value) is, to
/// close it on `platform`.
pub fn target(platform: Platform, value: &str, xdg_dirs: &XdgDirs) -> Result<Target, AppError> {
    let id = AppId::parse(platform, value)?;
    let target = match id {
        AppId::Bundle(bundle) => {
            if !bundle::valid_identifier(&bundle) {
                return Err(AppError::new(
                    codes::INVALID_APPLICATION,
                    format!("“{bundle}” is not a bundle identifier"),
                ));
            }
            Target::Bundle(bundle)
        }
        AppId::Aumid(aumid) => Target::Aumid(aumid),
        AppId::DesktopFile(desktop) => {
            let found = xdg::find(&desktop, xdg_dirs)?;
            let argv = found.entry.command(&[], "")?;
            let program =
                xdg::find_program(&argv[0], xdg_dirs.path.as_deref()).ok_or_else(|| {
                    AppError::new(
                        codes::NOT_FOUND,
                        format!("{desktop} starts “{}”, which is not installed", argv[0]),
                    )
                })?;
            executable(&program, &desktop)?
        }
        AppId::AppPaths(name) => {
            let program = windows_apps::app_path(&name).ok_or_else(|| {
                AppError::new(
                    codes::NOT_FOUND,
                    format!("{name} is not registered on this computer (App Paths)"),
                )
            })?;
            executable(&program, &name)?
        }
        AppId::Path(path) => {
            let is_bundle = path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("app"));
            if is_bundle {
                let read = bundle::read(&path).ok_or_else(|| {
                    AppError::new(
                        codes::NOT_FOUND,
                        format!(
                            "the application {} does not exist; it may have been moved or \
                             uninstalled",
                            path.display()
                        ),
                    )
                })?;
                match read.identifier {
                    Some(id) => Target::Bundle(id),
                    None => match read.executable {
                        Some(exe) => executable(&exe, &read.name)?,
                        None => {
                            return Err(not_identifiable(format!(
                                "{} has neither a bundle identifier nor an executable",
                                path.display()
                            )));
                        }
                    },
                }
            } else if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("lnk"))
            {
                return Err(not_identifiable(
                    "a shortcut can be opened but not closed; bind the application's executable \
                     instead",
                ));
            } else {
                executable(&path, &path.display().to_string())?
            }
        }
        AppId::Program(name) => {
            if platform == Platform::MacOs && !name.contains('.') {
                return Err(not_identifiable(format!(
                    "“{name}” is an application name, which does not identify one application; \
                     choose it again so that its bundle identifier is stored"
                )));
            }
            let program = xdg::find_program(&name, std::env::var_os("PATH").as_deref())
                .ok_or_else(|| {
                    AppError::new(
                        codes::NOT_FOUND,
                        format!("the program “{name}” was not found on PATH"),
                    )
                })?;
            executable(&program, &name)?
        }
    };
    if let Target::Bundle(id) = &target
        && process::protected_bundle(id)
    {
        return Err(AppError::new(
            codes::PROTECTED_PROCESS,
            format!("{id} is part of the system or of DeckPilot and is never closed"),
        ));
    }
    Ok(target)
}

/// The platform side of closing. Every method acts on one process the
/// procedure already matched; implementations re-check its identity right
/// before they act, where the platform allows it.
pub trait Control: std::fmt::Debug {
    /// Every running process.
    fn snapshot(&self) -> Snapshot;
    /// The process ids of the running applications with bundle id `id`
    /// (macOS; empty elsewhere).
    fn bundle_pids(&self, id: &str) -> Vec<u32>;
    /// The `AppUserModelID` of `pid` (Windows; `None` elsewhere).
    fn aumid(&self, pid: u32) -> Option<String>;
    /// Asks the process to quit, as the user would. `Ok(false)` when there
    /// is no way to ask (a Windows process without a window).
    fn request_quit(&self, target: &Target, process: &ProcessInfo) -> Result<bool, AppError>;
    /// Ends the process now.
    fn force_quit(&self, target: &Target, process: &ProcessInfo) -> Result<(), AppError>;
    /// Whether the process is still running.
    fn running(&self, process: &ProcessInfo) -> bool;
    /// Waits `duration`; `false` when the run was cancelled or its deadline
    /// passed.
    fn wait(&self, duration: Duration) -> bool {
        std::thread::sleep(duration);
        true
    }
    /// This process runs as root (Unix) or elevated (Windows).
    fn elevated(&self) -> bool;
}

/// How to close.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// Force the instances that are still running after [`Options::grace`].
    pub force: bool,
    pub grace: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            force: false,
            grace: GRACE,
        }
    }
}

/// What closing did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// The matching instances owned by the current user.
    pub instances: usize,
    /// How many were asked to quit.
    pub asked: usize,
    /// How many were forced.
    pub forced: usize,
    /// How many are still running at the end.
    pub remaining: usize,
    /// Matching instances left alone: another user's, or protected.
    pub skipped: usize,
    /// The run was cancelled or ran out of time while the application was
    /// given time to quit, so nothing was forced.
    pub interrupted: bool,
}

/// The current user's instances of `target`, and how many were skipped.
pub fn instances(control: &dyn Control, target: &Target) -> (Vec<ProcessInfo>, usize) {
    let snapshot = control.snapshot();
    let matching: Vec<&ProcessInfo> = match target {
        Target::Bundle(id) => {
            let pids = control.bundle_pids(id);
            snapshot
                .processes
                .iter()
                .filter(|p| pids.contains(&p.pid))
                .collect()
        }
        Target::Aumid(aumid) => snapshot
            .processes
            .iter()
            .filter(|p| control.aumid(p.pid).as_deref() == Some(aumid.as_str()))
            .collect(),
        Target::Executable(path) => snapshot
            .processes
            .iter()
            .filter(|p| {
                p.exe
                    .as_deref()
                    .is_some_and(|exe| same_executable(exe, path))
            })
            .collect(),
    };
    let mut skipped = 0;
    let mut mine = Vec::new();
    for process in matching {
        if snapshot.owned_by_me(process) && !process::protected(process, &snapshot) {
            mine.push(process.clone());
        } else {
            skipped += 1;
        }
    }
    (mine, skipped)
}

/// The same executable file: the names first (cheap), then the real
/// paths.
fn same_executable(exe: &Path, target: &Path) -> bool {
    if exe == target {
        return true;
    }
    let names = exe
        .file_name()
        .zip(target.file_name())
        .is_some_and(|(a, b)| a.eq_ignore_ascii_case(b));
    names && process::same_file(exe, target)
}

/// Closes every instance of `target` the current user owns: asks each to
/// quit, waits up to `options.grace`, then with `options.force` ends those
/// still running. Without force, an application that keeps running (it
/// may be asking to save) is reported in [`Report::remaining`].
pub fn close(control: &dyn Control, target: &Target, options: Options) -> Result<Report, AppError> {
    if control.elevated() {
        return Err(AppError::new(
            codes::ELEVATED,
            "DeckPilot runs as root or as an administrator (elevated), so it does not close \
             applications: they could belong to another account. Run DeckPilot as yourself",
        ));
    }
    let (instances, skipped) = instances(control, target);
    let mut report = Report {
        instances: instances.len(),
        skipped,
        ..Report::default()
    };
    if instances.is_empty() {
        return Ok(report);
    }
    let mut failure = None;
    for process in &instances {
        match control.request_quit(target, process) {
            Ok(true) => report.asked += 1,
            Ok(false) => {}
            Err(error) => failure = Some(error),
        }
    }
    let (mut left, completed) = wait_for_exit(control, &instances, options.grace);
    if !completed && !left.is_empty() {
        // Cancelled, or out of time, while the application may be asking
        // to save: never force then.
        report.interrupted = true;
        report.remaining = left.len();
        return Ok(report);
    }
    if options.force && !left.is_empty() {
        for process in &left {
            match control.force_quit(target, process) {
                Ok(()) => report.forced += 1,
                Err(error) => failure = Some(error),
            }
        }
        left = wait_for_exit(control, &left, FORCE_WAIT).0;
    }
    report.remaining = left.len();
    match failure {
        Some(error) if report.asked == 0 && report.forced == 0 => Err(error),
        _ => Ok(report),
    }
}

/// The processes still running after waiting up to `limit` for them to
/// exit, and `false` when the wait was cut short (cancelled, deadline).
fn wait_for_exit(
    control: &dyn Control,
    processes: &[ProcessInfo],
    limit: Duration,
) -> (Vec<ProcessInfo>, bool) {
    let until = Instant::now() + limit;
    loop {
        let left: Vec<ProcessInfo> = processes
            .iter()
            .filter(|p| control.running(p))
            .cloned()
            .collect();
        if left.is_empty() || Instant::now() >= until {
            return (left, true);
        }
        if !control.wait(POLL) {
            return (left, false);
        }
    }
}

/// The real platform.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemControl;

impl Control for SystemControl {
    fn snapshot(&self) -> Snapshot {
        process::snapshot()
    }

    fn bundle_pids(&self, id: &str) -> Vec<u32> {
        #[cfg(target_os = "macos")]
        {
            crate::native_macos::bundle_pids(id)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = id;
            Vec::new()
        }
    }

    fn aumid(&self, pid: u32) -> Option<String> {
        #[cfg(windows)]
        {
            crate::native_windows::aumid_of(pid)
        }
        #[cfg(not(windows))]
        {
            let _ = pid;
            None
        }
    }

    fn request_quit(&self, target: &Target, process: &ProcessInfo) -> Result<bool, AppError> {
        #[cfg(target_os = "macos")]
        if let Target::Bundle(id) = target {
            return crate::native_macos::terminate(id, process.pid, false);
        }
        #[cfg(windows)]
        {
            let _ = target;
            crate::native_windows::close_windows(process)
        }
        #[cfg(unix)]
        {
            let _ = target;
            signal(process, false).map(|()| true)
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (target, process);
            Err(AppError::new(
                codes::UNSUPPORTED,
                "closing is not supported on this system",
            ))
        }
    }

    fn force_quit(&self, target: &Target, process: &ProcessInfo) -> Result<(), AppError> {
        #[cfg(target_os = "macos")]
        if let Target::Bundle(id) = target {
            return crate::native_macos::terminate(id, process.pid, true).map(|_| ());
        }
        #[cfg(windows)]
        {
            let _ = target;
            crate::native_windows::terminate(process)
        }
        #[cfg(unix)]
        {
            let _ = target;
            signal(process, true)
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (target, process);
            Err(AppError::new(
                codes::UNSUPPORTED,
                "closing is not supported on this system",
            ))
        }
    }

    fn running(&self, process: &ProcessInfo) -> bool {
        still_same(process)
    }

    fn elevated(&self) -> bool {
        #[cfg(unix)]
        {
            rustix::process::geteuid().is_root()
        }
        #[cfg(windows)]
        {
            crate::native_windows::elevated()
        }
        #[cfg(not(any(unix, windows)))]
        {
            false
        }
    }
}

/// The process is still the one that was matched: same id, executable
/// and owner (a process id the system gave to another program is not it).
fn still_same(process: &ProcessInfo) -> bool {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

    let pid = Pid::from_u32(process.pid);
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing()
            .with_exe(UpdateKind::OnlyIfNotSet)
            .with_user(UpdateKind::OnlyIfNotSet)
            .without_tasks(),
    );
    let Some(now) = system.process(pid) else {
        return false;
    };
    // A zombie has exited; it only waits for its parent.
    if matches!(
        now.status(),
        sysinfo::ProcessStatus::Zombie | sysinfo::ProcessStatus::Dead
    ) {
        return false;
    }
    let owner = now.user_id().map(|uid| (**uid).to_string());
    let started = Some(now.start_time()).filter(|t| *t > 0);
    owner == process.owner
        && now.exe().map(Path::to_path_buf) == process.exe
        && same_start(started, process.started)
}

/// The same start time, when both are known: a process started later
/// with the same id (and even the same executable) is another instance.
fn same_start(now: Option<u64>, matched: Option<u64>) -> bool {
    match (now, matched) {
        (Some(now), Some(matched)) => now == matched,
        _ => true,
    }
}

/// `SIGTERM`, or `SIGKILL` when forced, after checking that the process
/// is still the matched one.
///
/// On Linux the process is pinned first with a pidfd (`pidfd_open`), its
/// identity checked, and the signal sent through it (`pidfd_send_signal`),
/// so a process id the system reused in between is never signalled. Where
/// pidfds are not available (macOS, Linux before 5.3) the identity is
/// checked right before `kill`.
#[cfg(unix)]
fn signal(process: &ProcessInfo, force: bool) -> Result<(), AppError> {
    use rustix::process::{Pid, Signal, kill_process};

    let pid = i32::try_from(process.pid)
        .ok()
        .and_then(Pid::from_raw)
        .ok_or_else(|| AppError::new(codes::CLOSE_FAILED, "invalid process id"))?;
    let sig = if force { Signal::KILL } else { Signal::TERM };
    let failed = |error: rustix::io::Errno| {
        AppError::new(
            codes::CLOSE_FAILED,
            format!("process {} could not be signalled: {error}", process.pid),
        )
    };
    #[cfg(target_os = "linux")]
    {
        use rustix::process::{PidfdFlags, pidfd_open, pidfd_send_signal};
        match pidfd_open(pid, PidfdFlags::empty()) {
            Ok(pidfd) => {
                if !still_same(process) {
                    return Ok(());
                }
                return match pidfd_send_signal(&pidfd, sig) {
                    Ok(()) | Err(rustix::io::Errno::SRCH) => Ok(()),
                    Err(error) => Err(failed(error)),
                };
            }
            Err(rustix::io::Errno::SRCH) => return Ok(()),
            // No pidfds here: the check-then-kill below.
            Err(_) => {}
        }
    }
    if !still_same(process) {
        return Ok(());
    }
    match kill_process(pid, sig) {
        Ok(()) | Err(rustix::io::Errno::SRCH) => Ok(()),
        Err(error) => Err(failed(error)),
    }
}

#[cfg(test)]
mod tests;
