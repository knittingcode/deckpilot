//! This computer's resource bindings. They belong to the core: any module
//! may resolve them if it holds the `machine-resources` capability.

use tauri::{Runtime, State, WebviewWindow};

use deckpilot_apps::{IdKind, InstalledApp, Platform};
use deckpilot_core::gesture::GestureTimings;
use deckpilot_core::{MachineConfig, MachineResourceBinding, ResourceKey, ResourceKind};
use deckpilot_storage::ConfigStore;
use deckpilot_storage::gestures;

use crate::error::{CommandError, CommandResult};
use crate::state::{AppState, machine_config};

#[tauri::command]
pub fn get_machine_config(state: State<'_, AppState>) -> CommandResult<MachineConfig> {
    Ok(machine_config(&*state.store()?)?)
}

#[tauri::command]
pub fn set_resource_binding(
    state: State<'_, AppState>,
    key: String,
    kind: ResourceKind,
    value: String,
) -> CommandResult<MachineConfig> {
    let value = value.trim();
    if value.is_empty() {
        return Err(CommandError::invalid(
            "resourceValueEmpty",
            "resource value must not be empty",
        ));
    }
    let store = state.store()?;
    let mut config = machine_config(&store)?;
    config.set_binding(MachineResourceBinding {
        key: ResourceKey::new(key.trim())?,
        kind,
        value: value.to_owned(),
    });
    store.save_machine_config(&config)?;
    crate::agent::notify_reload(&state.paths);
    Ok(config)
}

#[tauri::command]
pub fn remove_resource_binding(
    state: State<'_, AppState>,
    key: ResourceKey,
) -> CommandResult<MachineConfig> {
    let store = state.store()?;
    let mut config = machine_config(&store)?;
    config
        .remove_binding(&key)
        .ok_or(CommandError::NotFound("resource binding"))?;
    store.save_machine_config(&config)?;
    crate::agent::notify_reload(&state.paths);
    Ok(config)
}

/// The double-click window and long-press time of every key whose page sets
/// none (the `gestures.defaults` setting of this computer).
#[tauri::command]
pub fn get_gesture_defaults(state: State<'_, AppState>) -> CommandResult<GestureTimings> {
    Ok(gestures::gesture_defaults(&*state.store()?)?)
}

/// Stores the default gesture timings; values outside the allowed ranges
/// are refused. The agent reads them on its next configuration reload.
#[tauri::command]
pub fn set_gesture_defaults(
    state: State<'_, AppState>,
    timings: GestureTimings,
) -> CommandResult<GestureTimings> {
    let store = state.store()?;
    gestures::set_gesture_defaults(&*store, timings)?;
    let stored = gestures::gesture_defaults(&*store)?;
    crate::agent::notify_reload(&state.paths);
    Ok(stored)
}

/// The installed applications of this computer, for the application and
/// browser pickers of Open Application, Close Application, Website and
/// Open (`deckpilot_apps::discover`): each with the stable id a binding
/// stores (a bundle id, an App Paths name, a desktop-file id) or its path.
/// Read off the main thread; nothing is started.
#[tauri::command]
pub async fn list_applications<R: Runtime>(
    window: WebviewWindow<R>,
) -> CommandResult<Vec<InstalledApp>> {
    main_only(&window)?;
    tauri::async_runtime::spawn_blocking(|| remember(deckpilot_apps::discover()))
        .await
        .map_err(|e| CommandError::invalid("applicationsUnreadable", e.to_string()))
}

/// The installed web browsers and the system's default, for Website's
/// "Open with" (`deckpilot_apps::browsers`: `LaunchServices` on macOS, the
/// registered browsers and the `https` association on Windows, desktop
/// entries and `mimeapps.list` on Linux).
#[tauri::command]
pub async fn list_browsers<R: Runtime>(
    window: WebviewWindow<R>,
) -> CommandResult<deckpilot_apps::browsers::Browsers> {
    main_only(&window)?;
    tauri::async_runtime::spawn_blocking(|| {
        let found = deckpilot_apps::browsers::browsers();
        remember(found.browsers.clone());
        found
    })
    .await
    .map_err(|e| CommandError::invalid("applicationsUnreadable", e.to_string()))
}

/// The pickers' commands answer the main window only (the app's editor),
/// never a module's or a plugin's window.
fn main_only<R: Runtime>(window: &WebviewWindow<R>) -> CommandResult<()> {
    if window.label() == crate::windows::MAIN {
        Ok(())
    } else {
        Err(CommandError::invalid(
            "mainWindowOnly",
            "only the main window lists applications",
        ))
    }
}

/// The applications an icon may be drawn for: those `discover()` and
/// `browsers()` reported, and those a picked file was described as when it
/// is an application the system knows (a bundle with an identifier, an
/// installed desktop entry, an App Paths registration). Never a plain path
/// a caller described, so no file the app names is ever read for an icon.
static LISTED: std::sync::Mutex<Vec<InstalledApp>> = std::sync::Mutex::new(Vec::new());
/// Most applications remembered.
const MAX_LISTED: usize = 8192;
/// Icons drawn so far, as data URLs (`None`: the system has none), and
/// the order they were drawn in, oldest first.
#[derive(Debug)]
struct IconCache {
    drawn: std::collections::BTreeMap<String, Option<String>>,
    order: std::collections::VecDeque<String>,
}

static ICONS: std::sync::Mutex<IconCache> = std::sync::Mutex::new(IconCache {
    drawn: std::collections::BTreeMap::new(),
    order: std::collections::VecDeque::new(),
});
/// Most icons kept; the oldest is dropped first.
const MAX_CACHED_ICONS: usize = 512;

fn remember(apps: Vec<InstalledApp>) -> Vec<InstalledApp> {
    let mut listed = LISTED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for app in &apps {
        if listed.len() >= MAX_LISTED {
            break;
        }
        if !listed.iter().any(|known| known.id == app.id) {
            listed.push(app.clone());
        }
    }
    apps
}

/// The icon of application `id`, one the pickers listed, as a
/// `data:image/png;base64,…` URL of at most 64×64 pixels, drawn by the
/// system and encoded again by `deckpilot_apps::icons`; `None` when there
/// is none or the id was not listed. Cached.
#[tauri::command]
pub async fn application_icon<R: Runtime>(
    window: WebviewWindow<R>,
    id: String,
) -> CommandResult<Option<String>> {
    main_only(&window)?;
    tauri::async_runtime::spawn_blocking(move || icon_for(&id))
        .await
        .map_err(|e| CommandError::invalid("applicationsUnreadable", e.to_string()))
}

fn icon_for(id: &str) -> Option<String> {
    use base64::Engine as _;
    if let Some(cached) = ICONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .drawn
        .get(id)
    {
        return cached.clone();
    }
    let app = LISTED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .find(|a| a.id == id)
        .cloned()?;
    let icon =
        deckpilot_apps::icons::icon(&app, &deckpilot_apps::xdg::XdgDirs::from_env()).map(|png| {
            format!(
                "data:image/png;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(png)
            )
        });
    let mut icons = ICONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let IconCache { drawn, order } = &mut *icons;
    while drawn.len() >= MAX_CACHED_ICONS {
        let Some(oldest) = order.pop_front() else {
            break;
        };
        drawn.remove(&oldest);
    }
    if drawn.insert(id.to_owned(), icon.clone()).is_none() {
        order.push_back(id.to_owned());
    }
    icon
}

/// Whether application `id` (a binding's value) is installed on this
/// computer: what Open Application would open exists (a bundle id
/// `LaunchServices` knows, an installed desktop entry, an App Paths
/// registration, an existing path). Nothing is read or started.
#[tauri::command]
pub async fn application_present<R: Runtime>(
    window: WebviewWindow<R>,
    id: String,
) -> CommandResult<bool> {
    main_only(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let trimmed = id.trim();
        !(trimmed.starts_with("\\\\") || trimmed.starts_with("//"))
            && deckpilot_apps::launchable(trimmed, &[]).is_ok()
    })
    .await
    .map_err(|e| CommandError::invalid("applicationsUnreadable", e.to_string()))
}

/// An application the user picked with the file picker, as a binding
/// stores it: an `.app` by its bundle id, an installed `.desktop` file by
/// its desktop-file id, a Windows executable registered under App Paths by
/// that name; anything else by its path.
#[tauri::command]
pub fn describe_application<R: Runtime>(
    window: WebviewWindow<R>,
    path: String,
) -> CommandResult<InstalledApp> {
    main_only(&window)?;
    describe_and_remember(Platform::current(), &path)
}

/// [`describe`], and an application the system knows (not a plain path)
/// is remembered for its icon.
fn describe_and_remember(platform: Platform, path: &str) -> CommandResult<InstalledApp> {
    let app = describe(platform, path)?;
    if app.kind != IdKind::Path {
        remember(vec![app.clone()]);
    }
    Ok(app)
}

fn describe(platform: Platform, path: &str) -> CommandResult<InstalledApp> {
    let trimmed = path.trim();
    // A UNC or `\\?\` path could reach another computer (and its icon
    // handlers): refused.
    if trimmed.starts_with("\\\\") || trimmed.starts_with("//") {
        return Err(CommandError::invalid(
            "applicationNotFound",
            "applications on network paths are not offered",
        ));
    }
    let file = std::path::Path::new(trimmed);
    if !file.is_absolute() || !file.exists() {
        return Err(CommandError::invalid(
            "applicationNotFound",
            format!("{} does not exist", file.display()),
        ));
    }
    let extension = file
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let name = file
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let by_path = InstalledApp {
        name: name.clone(),
        id: file.to_string_lossy().into_owned(),
        kind: IdKind::Path,
        path: Some(file.to_string_lossy().into_owned()),
        browser: false,
    };
    if extension == "app" {
        return deckpilot_apps::bundle::read(file)
            .map(|bundle| bundle.installed())
            .ok_or_else(|| {
                CommandError::invalid(
                    "applicationUnreadable",
                    format!("{} is not an application bundle", file.display()),
                )
            });
    }
    if extension == "desktop" {
        // Only an installed entry has an id that launches.
        let dirs = deckpilot_apps::xdg::XdgDirs::from_env();
        let installed = deckpilot_apps::xdg::applications(&dirs)
            .into_iter()
            .find(|app| {
                app.path
                    .as_deref()
                    .is_some_and(|p| deckpilot_apps::process::same_file(p.as_ref(), file))
            });
        return installed.ok_or_else(|| {
            CommandError::invalid(
                "applicationNotInstalled",
                format!(
                    "{} is not an installed desktop entry (it must be in an applications folder \
                     of XDG_DATA_DIRS)",
                    file.display()
                ),
            )
        });
    }
    if platform == Platform::Windows && extension == "exe" {
        let registered = deckpilot_apps::windows_apps::registered()
            .into_iter()
            .find(|app| {
                app.kind == IdKind::AppPaths
                    && app
                        .path
                        .as_deref()
                        .is_some_and(|p| deckpilot_apps::process::same_file(p.as_ref(), file))
            });
        if let Some(app) = registered {
            return Ok(app);
        }
    }
    Ok(by_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_described_file_never_gets_an_icon() {
        // A plain file picked by hand is described by its path, and no icon
        // is ever drawn for it (no read of a file the app names).
        let dir = std::env::temp_dir().join(format!("deckpilot-icon-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("hosts");
        std::fs::write(&file, "127.0.0.1 localhost").unwrap();
        let described =
            describe_and_remember(Platform::current(), &file.to_string_lossy()).unwrap();
        assert_eq!(described.kind, IdKind::Path);
        assert_eq!(icon_for(&described.id), None);
        assert_eq!(icon_for(&file.to_string_lossy()), None);
        for unc in [
            r"\\server\share\app.exe",
            r"\\?\C:\Windows\explorer.exe",
            "//server/share",
        ] {
            assert!(describe(Platform::Windows, unc).is_err(), "{unc}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn icons_are_drawn_only_for_listed_applications() {
        // Never listed: no file is read for it.
        assert_eq!(icon_for("/etc/passwd"), None);
        assert_eq!(icon_for("com.example.never-listed"), None);
        let apps = remember(deckpilot_apps::discover());
        if cfg!(target_os = "macos") || cfg!(windows) {
            let first = apps.first().expect("an installed application");
            let icon = icon_for(&first.id).expect("the system draws its icon");
            assert!(icon.starts_with("data:image/png;base64,"));
            // Cached: the same answer without drawing again.
            assert_eq!(icon_for(&first.id), Some(icon));
        }
    }

    #[test]
    fn picked_files_are_described_by_their_stable_id() {
        let dir = std::env::temp_dir().join(format!("deckpilot-describe-{}", std::process::id()));
        let app = dir.join("Editor.app/Contents");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(
            app.join("Info.plist"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>com.example.Editor</string>
<key>CFBundleName</key><string>Editor</string>
</dict></plist>"#,
        )
        .unwrap();
        let tool = dir.join("tool");
        std::fs::write(&tool, "").unwrap();
        let described =
            describe(Platform::MacOs, &dir.join("Editor.app").to_string_lossy()).unwrap();
        assert_eq!(
            (described.id.as_str(), described.kind),
            ("com.example.Editor", IdKind::Bundle)
        );
        let described = describe(Platform::Linux, &tool.to_string_lossy()).unwrap();
        assert_eq!(described.kind, IdKind::Path);
        assert!(describe(Platform::Linux, &dir.join("gone").to_string_lossy()).is_err());
        assert!(describe(Platform::Linux, "relative/tool").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
