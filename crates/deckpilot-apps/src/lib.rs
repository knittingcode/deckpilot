//! Installed applications and the running ones, for the System module's
//! Open Application and Close Application actions
//! (`docs/system-actions.md`).
//!
//! - [`discover`] lists the installed applications with the **stable
//!   identifier** a resource binding stores: a macOS bundle id, a Windows
//!   App Paths name, a Linux desktop-file id, or the path where there is
//!   none.
//! - [`AppId`] reads such a binding value back, and [`launchable`] turns it
//!   into what the core's single launch service opens
//!   (`deckpilot_core::launch`): this crate never starts anything itself.
//! - [`close::Target`] identifies a running application by its stable id
//!   or exact executable only, and [`close::close`] closes the instances
//!   the current user owns, gracefully first, forced only when asked,
//!   never a protected process.
//!
//! Everything a binding holds is untrusted: values are checked before use
//! ([`AppId::parse`]), nothing goes through a shell, and process names are
//! never compared.
//!
//! The platform code is small: `AppKit`'s `NSRunningApplication` on macOS
//! (safe bindings), signals through `rustix` on Unix, and the Win32 calls
//! in [`native_windows`], the one module that allows `unsafe`.

pub mod bundle;
pub mod close;
pub mod desktop_entry;
mod discovery;
mod id;
mod launch;
#[cfg(target_os = "macos")]
mod native_macos;
#[cfg(windows)]
pub mod native_windows;
pub mod process;
pub mod windows_apps;
pub mod xdg;

pub use discovery::{Roots, discover, discover_in};
pub use id::{AppId, MAX_VALUE_LEN, Platform};
pub use launch::{Launchable, launchable, launchable_in};

use serde::Serialize;

/// Stable error codes, as `ActionResult::code`.
pub mod codes {
    /// The binding's value is not an application identifier or path.
    pub const INVALID_APPLICATION: &str = "invalid-application";
    /// The application is not installed (or was moved).
    pub const NOT_FOUND: &str = "not-found";
    /// The application cannot be told apart from other programs (a script,
    /// an interpreter, a sandbox launcher, a shortcut).
    pub const NOT_IDENTIFIABLE: &str = "not-identifiable";
    /// The application is part of the system or of DeckPilot itself.
    pub const PROTECTED_PROCESS: &str = "protected-process";
    /// The operating system refused (no permission, the process is gone).
    pub const CLOSE_FAILED: &str = "close-failed";
    /// Closing is not available on this operating system.
    pub const UNSUPPORTED: &str = "unsupported-target";
    /// The run was cancelled or ran out of time during the grace period:
    /// nothing was forced.
    pub const INTERRUPTED: &str = "interrupted";
    /// DeckPilot runs as root or elevated, so it does not close anything.
    pub const ELEVATED: &str = "elevated";
}

/// A refusal with its code and a message for the user.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct AppError {
    pub code: &'static str,
    pub message: String,
}

impl AppError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// What kind of identifier [`InstalledApp::id`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum IdKind {
    /// A macOS bundle identifier, e.g. `com.apple.Safari`.
    Bundle,
    /// A Windows App Paths registration, e.g. `chrome.exe`.
    AppPaths,
    /// A Windows `AppUserModelID`.
    Aumid,
    /// An XDG desktop-file id, e.g. `org.gnome.Terminal.desktop`.
    DesktopFile,
    /// An absolute path: there is no stable identifier.
    Path,
}

/// An installed application, as the picker offers it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledApp {
    /// Its name as the system shows it.
    pub name: String,
    /// The value a resource binding stores: the stable identifier, or the
    /// path when there is none.
    pub id: String,
    pub kind: IdKind,
    /// Where it is, when known (shown to the user, never stored as the id
    /// when a stable one exists).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// It registers itself for `http` and `https` (a web browser).
    pub browser: bool,
}
