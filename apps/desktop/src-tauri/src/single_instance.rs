//! One configuration app per user: a second launch brings the running app
//! to the front and ends, instead of starting a second app with its own
//! windows, agent bridge and supervisor.
//!
//! - **Windows and Linux**: `tauri-plugin-single-instance` (a named mutex
//!   and a message window in the user's session on Windows; a name on the
//!   user's session bus on Linux, `io.github.knittingcode.deckpilot.SingleInstance`).
//!   The second process hands its arguments to the first and exits with
//!   code 0; the first shows itself as for the agent's "Open DeckPilot"
//!   ([`on_second_launch`]). Without a session bus (Linux) the plugin finds
//!   no other instance and the app starts as before. A first instance that
//!   hangs is waited for: up to 10 s on Windows (the second then exits
//!   with code 0), up to D-Bus's call timeout (about 25 s) on Linux, where
//!   the second then fails to start with an error.
//! - **macOS**: `LaunchServices` already opens a running app instead of
//!   starting another (Finder, the Dock, Spotlight, `open`), and the app
//!   shows itself on that reopen (`lifecycle::on_run_event`). The plugin is
//!   not used there: it would listen on a socket at a fixed path in the
//!   shared `/tmp`. The agent's launcher starts the executable directly
//!   (`apps/agent/src/launcher.rs`), which is not guarded on macOS; it
//!   starts an app only when none is connected.
//!
//! The guard is left out for:
//!
//! - a process started for one window (`--window <id>`, the agent's
//!   password prompt while no app runs). It ends with its window, and it
//!   must not swallow the launch of the full app;
//! - a debug build, so a development build does not hand over to an
//!   installed DeckPilot;
//! - a data directory other than the default one (`DECKPILOT_DATA_DIR`:
//!   development sessions, the end-to-end tests, the release smoke test),
//!   because the plugin knows one app identity only, while the agent keeps
//!   one instance per data directory.
//!
//! The app handles no deep links (no URL scheme is registered), and the
//! arguments of the second launch are not used: a second `--window` process
//! never reaches the guard, and `--agent` runs the agent before the app
//! starts (`agent::run_agent_if_requested`).

// macOS builds use none of this outside the tests.
#![cfg_attr(target_os = "macos", allow(dead_code))]

/// Whether this process takes part in the single-instance guard: a full
/// app (`window_only` is `None`) of a release build (`debug` is false: a
/// development build must not hand over to an installed DeckPilot) on the
/// default data directory (`data_dir_override` is the value of
/// `DECKPILOT_DATA_DIR`, if any).
pub fn applies(
    window_only: Option<&str>,
    data_dir_override: Option<&std::ffi::OsStr>,
    debug: bool,
) -> bool {
    window_only.is_none() && !debug && data_dir_override.is_none_or(std::ffi::OsStr::is_empty)
}

/// [`applies`] for this process.
#[cfg(any(target_os = "windows", target_os = "linux"))]
pub fn applies_here(window_only: Option<&str>) -> bool {
    applies(
        window_only,
        std::env::var_os(deckpilot_ipc::paths::DATA_DIR_ENV).as_deref(),
        cfg!(debug_assertions),
    )
}

/// Set at the end of the app's setup, once its first window exists. The
/// plugin holds the instance before the setup runs, so a second launch
/// (a double click) can arrive while the app is still starting; it is
/// ignored then, since the setup shows the main window or the password
/// prompt anyway. Creating the main window from both places would make two.
#[derive(Debug, Default)]
pub struct Started(pub std::sync::atomic::AtomicBool);

/// Whether a second launch arriving now is acted on ([`Started`]).
pub fn acts_on_launch(started: Option<&Started>) -> bool {
    started.is_some_and(|s| s.0.load(std::sync::atomic::Ordering::SeqCst))
}

/// The plugin, with [`on_second_launch`] as its callback. Registered before
/// every other plugin, so a second process ends before the rest of the
/// setup runs.
///
/// The callback returns at once and the window work runs on the async
/// runtime, as the agent's "Open DeckPilot" does (`agent::show_window`):
/// the plugin calls it from inside a window message on Windows (the second
/// process waits for it) and from the D-Bus thread on Linux.
#[cfg(any(target_os = "windows", target_os = "linux"))]
pub fn plugin<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri_plugin_single_instance::init(|app, _args, _cwd| {
        let app = app.clone();
        tauri::async_runtime::spawn(async move { on_second_launch(&app) });
    })
}

/// The app was launched again: once it has started, it comes to the front
/// as for the agent's "Open DeckPilot": the main window, the password
/// prompt while locked, and a new main window (on every platform: the user
/// asked for the app) if the old one was closed while another window kept
/// the app running.
pub fn on_second_launch<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    use tauri::Manager;
    if !acts_on_launch(app.try_state::<Started>().as_deref()) {
        return;
    }
    crate::agent::show_window_on(app, true);
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::sync::atomic::Ordering;

    use super::*;
    use crate::lifecycle::{ShowTarget, show_target};

    #[test]
    fn only_a_full_release_app_on_the_default_data_directory_is_guarded() {
        assert!(applies(None, None, false));
        assert!(applies(None, Some(OsStr::new("")), false));
        // The password prompt the agent starts while no app runs.
        assert!(!applies(Some("security.unlock"), None, false));
        // Development sessions, end-to-end and smoke tests.
        assert!(!applies(
            None,
            Some(OsStr::new("/tmp/deckpilot-dev")),
            false
        ));
        // A development build next to an installed DeckPilot.
        assert!(!applies(None, None, true));
    }

    #[test]
    fn a_launch_during_startup_is_ignored() {
        assert!(!acts_on_launch(None));
        let started = Started::default();
        assert!(!acts_on_launch(Some(&started)));
        started.0.store(true, Ordering::SeqCst);
        assert!(acts_on_launch(Some(&started)));
    }

    #[test]
    fn a_second_launch_always_shows_something() {
        assert_eq!(show_target(true, true, true), ShowTarget::Main);
        assert_eq!(show_target(true, false, true), ShowTarget::CreateMain);
        assert_eq!(show_target(false, true, true), ShowTarget::Prompt);
        assert_eq!(show_target(false, false, true), ShowTarget::Prompt);
    }
}
