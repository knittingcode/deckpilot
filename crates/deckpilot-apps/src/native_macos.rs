//! `AppKit` on macOS: where an installed bundle id is (`NSWorkspace`) and its
//! running instances (`NSRunningApplication`), which quit the way the Quit
//! menu item does (`terminate`) or at once (`forceTerminate`). These
//! bindings are safe calls; `NSRunningApplication` is thread safe.
//!
//! `NSRunningApplication` only lists the applications of the current
//! user's session. Whether an instance has exited is read from the
//! process table (`close::Control::running`), never from `isTerminated`,
//! which does not change off the main run loop.

use std::path::PathBuf;

use objc2_app_kit::{NSRunningApplication, NSWorkspace};
use objc2_foundation::NSString;

use crate::{AppError, codes};

/// Where the application with bundle id `id` is installed, if anywhere.
pub fn bundle_path(id: &str) -> Option<PathBuf> {
    let workspace = NSWorkspace::sharedWorkspace();
    let url = workspace.URLForApplicationWithBundleIdentifier(&NSString::from_str(id))?;
    url.path().map(|path| PathBuf::from(path.to_string()))
}

/// The process ids of the running applications with bundle id `id`.
pub fn bundle_pids(id: &str) -> Vec<u32> {
    let apps =
        NSRunningApplication::runningApplicationsWithBundleIdentifier(&NSString::from_str(id));
    apps.to_vec()
        .iter()
        .filter_map(|app| u32::try_from(app.processIdentifier()).ok())
        .filter(|pid| *pid > 1)
        .collect()
}

/// Asks the application with process id `pid` to quit (`force`: ends it),
/// after checking that it still is the application `id`. `Ok(false)` when
/// it is no longer running or not that application.
pub fn terminate(id: &str, pid: u32, force: bool) -> Result<bool, AppError> {
    let pid =
        i32::try_from(pid).map_err(|_| AppError::new(codes::CLOSE_FAILED, "invalid process id"))?;
    let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) else {
        return Ok(false);
    };
    let same = app
        .bundleIdentifier()
        .is_some_and(|bundle| bundle.to_string() == id);
    if !same {
        return Ok(false);
    }
    let delivered = if force {
        app.forceTerminate()
    } else {
        app.terminate()
    };
    if delivered {
        Ok(true)
    } else {
        Err(AppError::new(
            codes::CLOSE_FAILED,
            format!("{id} did not accept the request to quit"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_bundles_are_neither_installed_nor_running() {
        let id = "io.github.knittingcode.deckpilot.test-no-such-app";
        assert_eq!(bundle_path(id), None);
        assert_eq!(bundle_pids(id), Vec::<u32>::new());
        // Process 1 (launchd) is not an application with this id.
        assert_eq!(terminate(id, 1, false), Ok(false));
    }
}
