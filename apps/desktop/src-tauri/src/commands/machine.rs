//! This computer's resource bindings. They belong to the core: any module
//! may resolve them if it holds the `machine-resources` capability.

use tauri::State;

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
pub async fn list_applications() -> CommandResult<Vec<InstalledApp>> {
    tauri::async_runtime::spawn_blocking(deckpilot_apps::discover)
        .await
        .map_err(|e| CommandError::invalid("applicationsUnreadable", e.to_string()))
}

/// An application the user picked with the file picker, as a binding
/// stores it: an `.app` by its bundle id, an installed `.desktop` file by
/// its desktop-file id, a Windows executable registered under App Paths by
/// that name; anything else by its path.
#[tauri::command]
pub fn describe_application(path: String) -> CommandResult<InstalledApp> {
    describe(Platform::current(), &path)
}

fn describe(platform: Platform, path: &str) -> CommandResult<InstalledApp> {
    let file = std::path::Path::new(path.trim());
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
