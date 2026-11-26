//! One configuration app per user: a second launch brings the running app
//! to the front and ends, instead of starting a second app with its own
//! windows, agent bridge and supervisor.
//!
//! - **Windows and Linux**: `tauri-plugin-single-instance` (a named mutex
//!   and a message window in the user's session on Windows; a name on the
//!   user's session bus on Linux, `io.github.knittingcode.deckpilot.SingleInstance`).
//!   The second process hands its arguments to the first and exits with
//!   code 0; the first shows itself as for the agent's "Open DeckPilot"
//!   ([`launch_target`]). Without a session bus (Linux) the plugin finds no
//!   other instance and the app starts as before.
//! - **macOS**: `LaunchServices` already opens a running app instead of
//!   starting another (Finder, the Dock, Spotlight, `open`), and the app
//!   shows itself on that reopen (`lifecycle::on_run_event`). The plugin is
//!   not used there: it would listen on a socket at a fixed path in the
//!   shared `/tmp`.
//!
//! The guard is left out for:
//!
//! - a process started for one window (`--window <id>`, the agent's
//!   password prompt while no app runs). It ends with its window, and it
//!   must not swallow the launch of the full app;
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
/// app (`window_only` is `None`) on the default data directory
/// (`data_dir_override` is the value of `DECKPILOT_DATA_DIR`, if any).
pub fn applies(window_only: Option<&str>, data_dir_override: Option<&std::ffi::OsStr>) -> bool {
    window_only.is_none() && data_dir_override.is_none_or(std::ffi::OsStr::is_empty)
}

/// [`applies`] for this process.
#[cfg(any(target_os = "windows", target_os = "linux"))]
pub fn applies_here(window_only: Option<&str>) -> bool {
    applies(
        window_only,
        std::env::var_os(deckpilot_ipc::paths::DATA_DIR_ENV).as_deref(),
    )
}

/// What the running app shows when it is launched again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchTarget {
    /// Bring the main window to the front.
    Main,
    /// No main window any more (closed while another window kept the app
    /// running): create it. The user asked for the app, so the launch is
    /// never lost, unlike the agent's "Open DeckPilot", which an app
    /// without a main window ignores on Windows and Linux.
    CreateMain,
    /// The configuration is locked: the password prompt.
    Prompt,
}

/// [`LaunchTarget`] for an app whose configuration is `unlocked` and which
/// `has_main` window.
pub fn launch_target(unlocked: bool, has_main: bool) -> LaunchTarget {
    match (unlocked, has_main) {
        (false, _) => LaunchTarget::Prompt,
        (true, true) => LaunchTarget::Main,
        (true, false) => LaunchTarget::CreateMain,
    }
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

/// The app was launched again: it comes to the front ([`launch_target`]).
pub fn on_second_launch<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    use tauri::Manager;
    let state = app.try_state::<crate::state::AppState>();
    let unlocked = state.as_ref().is_none_or(|state| state.is_unlocked());
    let has_main = app.get_webview_window(crate::windows::MAIN).is_some();
    match launch_target(unlocked, has_main) {
        LaunchTarget::Prompt => {
            if let Some(state) = state {
                let id = crate::commands::security::prompt_window(&state);
                if let Err(error) = crate::commands::windows::open_by_id(app, &state, id, None) {
                    eprintln!("cannot open the password prompt: {error}");
                }
            }
        }
        LaunchTarget::Main => {
            crate::windows::show_main(app);
        }
        LaunchTarget::CreateMain => {
            if let Err(error) = crate::create_main_window(app) {
                eprintln!("cannot open the main window: {error}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;

    #[test]
    fn only_a_full_app_on_the_default_data_directory_is_guarded() {
        assert!(applies(None, None));
        assert!(applies(None, Some(OsStr::new(""))));
        // The password prompt the agent starts while no app runs.
        assert!(!applies(Some("security.unlock"), None));
        // Development sessions, end-to-end and smoke tests.
        assert!(!applies(None, Some(OsStr::new("/tmp/deckpilot-dev"))));
    }

    #[test]
    fn a_second_launch_always_shows_something() {
        assert_eq!(launch_target(true, true), LaunchTarget::Main);
        assert_eq!(launch_target(true, false), LaunchTarget::CreateMain);
        assert_eq!(launch_target(false, true), LaunchTarget::Prompt);
        assert_eq!(launch_target(false, false), LaunchTarget::Prompt);
    }
}
