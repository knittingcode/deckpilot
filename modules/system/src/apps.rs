//! The module's view of installed and running applications
//! (`deckpilot-apps`): what an application value opens with, and closing
//! one. Behind a trait so that the actions are tested with a fake that
//! never touches a real application.

use std::fmt;
use std::sync::Mutex;
use std::time::Duration;

use deckpilot_apps::close::{self, Control, Options, Report, SystemControl, Target};
use deckpilot_apps::{AppError, Launchable, Platform};

/// Installed and running applications.
pub trait Apps: Send + Sync + fmt::Debug {
    /// What opens application `value` with `targets` (its arguments, or
    /// the page or file it opens).
    fn launchable(&self, value: &str, targets: &[String]) -> Result<Launchable, AppError>;
    /// What a Close Application action for `value` closes.
    fn close_target(&self, value: &str) -> Result<Target, AppError>;
    /// Closes `target`'s instances; `wait` sleeps between looks and returns
    /// `false` when the run is cancelled.
    fn close(
        &self,
        target: &Target,
        options: Options,
        wait: &dyn Fn(Duration) -> bool,
    ) -> Result<Report, AppError>;
}

/// This computer's applications.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemApps;

/// The real platform, waiting through the run's cancellation.
struct Cancellable<'a> {
    wait: &'a dyn Fn(Duration) -> bool,
}

impl fmt::Debug for Cancellable<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Cancellable")
    }
}

impl Control for Cancellable<'_> {
    fn snapshot(&self) -> deckpilot_apps::process::Snapshot {
        SystemControl.snapshot()
    }
    fn bundle_pids(&self, id: &str) -> Vec<u32> {
        SystemControl.bundle_pids(id)
    }
    fn aumid(&self, pid: u32) -> Option<String> {
        SystemControl.aumid(pid)
    }
    fn request_quit(
        &self,
        target: &Target,
        process: &deckpilot_apps::process::ProcessInfo,
    ) -> Result<bool, AppError> {
        SystemControl.request_quit(target, process)
    }
    fn force_quit(
        &self,
        target: &Target,
        process: &deckpilot_apps::process::ProcessInfo,
    ) -> Result<(), AppError> {
        SystemControl.force_quit(target, process)
    }
    fn running(&self, process: &deckpilot_apps::process::ProcessInfo) -> bool {
        SystemControl.running(process)
    }
    fn wait(&self, duration: Duration) -> bool {
        (self.wait)(duration)
    }
}

impl Apps for SystemApps {
    fn launchable(&self, value: &str, targets: &[String]) -> Result<Launchable, AppError> {
        deckpilot_apps::launchable(value, targets)
    }

    fn close_target(&self, value: &str) -> Result<Target, AppError> {
        close::target(
            Platform::current(),
            value,
            &deckpilot_apps::xdg::XdgDirs::from_env(),
        )
    }

    fn close(
        &self,
        target: &Target,
        options: Options,
        wait: &dyn Fn(Duration) -> bool,
    ) -> Result<Report, AppError> {
        close::close(&Cancellable { wait }, target, options)
    }
}

/// Records what it was asked and answers with fixed results; for tests.
#[derive(Debug, Default)]
pub struct RecordingApps {
    /// The report every close returns.
    pub report: Report,
    calls: Mutex<Vec<String>>,
}

impl RecordingApps {
    pub fn new(report: Report) -> Self {
        Self {
            report,
            calls: Mutex::new(Vec::new()),
        }
    }

    /// `launch <value> <targets…>` and `close <target> force=<bool>`.
    pub fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn record(&self, call: String) {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(call);
    }
}

impl Apps for RecordingApps {
    fn launchable(&self, value: &str, targets: &[String]) -> Result<Launchable, AppError> {
        self.record(
            format!("launch {value} {}", targets.join(" "))
                .trim_end()
                .to_owned(),
        );
        SystemApps.launchable(value, targets)
    }

    fn close_target(&self, value: &str) -> Result<Target, AppError> {
        // Every value names an executable or a bundle as it is; nothing is
        // looked up on this computer.
        deckpilot_apps::AppId::parse(Platform::current(), value)?;
        Ok(if value.contains(['/', '\\']) {
            Target::Executable(value.into())
        } else {
            Target::Bundle(value.into())
        })
    }

    fn close(
        &self,
        target: &Target,
        options: Options,
        _wait: &dyn Fn(Duration) -> bool,
    ) -> Result<Report, AppError> {
        self.record(format!(
            "close {} force={}",
            target.describe(),
            options.force
        ));
        Ok(self.report.clone())
    }
}
