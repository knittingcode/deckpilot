//! The web browsers installed on this computer, and which one is the
//! default, as the system itself records them:
//!
//! - macOS: `LaunchServices`' handlers of `https` (`NSWorkspace`
//!   `URLsForApplicationsToOpenURL`, `URLForApplicationToOpenURL`); the
//!   bundles' `Info.plist` URL schemes before macOS 12.
//! - Windows: `Clients\StartMenuInternet` (the browsers), and the user's
//!   `https` association (`UrlAssociations\https\UserChoice`) for the
//!   default.
//! - Linux: desktop entries with `x-scheme-handler/https` (or `http`), and
//!   `mimeapps.list` (the XDG MIME Applications specification) for the
//!   default.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::xdg::XdgDirs;
use crate::{InstalledApp, Platform, Roots};

/// The browsers, sorted by name, and the default's id.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Browsers {
    pub browsers: Vec<InstalledApp>,
    /// The id (in [`Browsers::browsers`]) of the default browser, when the
    /// system says which it is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

/// This computer's browsers.
pub fn browsers() -> Browsers {
    let apps = crate::discover();
    match Platform::current() {
        Platform::MacOs => macos(&apps),
        Platform::Windows => windows(&apps),
        Platform::Linux => {
            let dirs = XdgDirs::from_env();
            linux(&apps, &MimeDirs::from_env(&dirs))
        }
        Platform::Other => Browsers::default(),
    }
}

/// The browsers among `apps` (their own registration), with `default`.
fn among(apps: &[InstalledApp], default: Option<String>) -> Browsers {
    let browsers: Vec<InstalledApp> = apps.iter().filter(|a| a.browser).cloned().collect();
    let default = default.filter(|id| browsers.iter().any(|b| &b.id == id));
    Browsers { browsers, default }
}

#[cfg(target_os = "macos")]
fn macos(apps: &[InstalledApp]) -> Browsers {
    use crate::bundle;
    let id_of = |path: &Path| bundle::read(path).map(|b| b.installed());
    let mut apps = apps.to_vec();
    // LaunchServices knows browsers outside the scanned folders too.
    if let Some(handlers) = crate::native_macos::browsers() {
        for app in &mut apps {
            app.browser = false;
        }
        for path in handlers {
            let Some(found) = id_of(&path) else { continue };
            match apps.iter_mut().find(|a| a.id == found.id) {
                Some(app) => app.browser = true,
                None => apps.push(InstalledApp {
                    browser: true,
                    ..found
                }),
            }
        }
    }
    let default = crate::native_macos::default_browser()
        .and_then(|path| id_of(&path))
        .map(|app| app.id);
    let mut found = among(&apps, default);
    found.browsers.sort_by_key(|a| a.name.to_lowercase());
    found
}

#[cfg(not(target_os = "macos"))]
fn macos(apps: &[InstalledApp]) -> Browsers {
    among(apps, None)
}

fn windows(apps: &[InstalledApp]) -> Browsers {
    let default = windows_default_program().and_then(|program| {
        apps.iter()
            .filter(|a| a.browser)
            .find(|a| {
                a.path
                    .as_deref()
                    .is_some_and(|p| crate::process::same_file(Path::new(p), &program))
            })
            .map(|a| a.id.clone())
    });
    among(apps, default)
}

/// The program of the user's `https` association (Windows).
#[cfg(windows)]
fn windows_default_program() -> Option<PathBuf> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, KEY_READ};

    let prog_id: String = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(
            r"Software\Microsoft\Windows\Shell\Associations\UrlAssociations\https\UserChoice",
            KEY_READ,
        )
        .ok()?
        .get_value("ProgId")
        .ok()?;
    if prog_id.is_empty() || prog_id.contains(['\\', '/']) {
        return None;
    }
    let command: String = RegKey::predef(HKEY_CLASSES_ROOT)
        .open_subkey_with_flags(format!(r"{prog_id}\shell\open\command"), KEY_READ)
        .ok()?
        .get_value("")
        .ok()?;
    let program = crate::windows_apps::command_program(&command)?;
    crate::windows_apps::registry_path(program, &|name| std::env::var(name).ok())
}

#[cfg(not(windows))]
fn windows_default_program() -> Option<PathBuf> {
    None
}

/// Where `mimeapps.list` files are, most important first (XDG MIME
/// Applications specification): `$XDG_CONFIG_HOME`, `$XDG_CONFIG_DIRS`,
/// then the `applications` folder of each XDG data folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MimeDirs {
    pub files: Vec<PathBuf>,
    pub xdg: XdgDirs,
}

impl MimeDirs {
    pub fn from_env(xdg: &XdgDirs) -> Self {
        let home = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::home_dir().map(|h| h.join(".config")));
        let config_dirs: Vec<PathBuf> = std::env::var_os("XDG_CONFIG_DIRS")
            .filter(|v| !v.is_empty())
            .map_or_else(
                || vec![PathBuf::from("/etc/xdg")],
                |v| {
                    std::env::split_paths(&v)
                        .filter(|p| p.is_absolute())
                        .collect()
                },
            );
        let mut files: Vec<PathBuf> = home
            .into_iter()
            .chain(config_dirs)
            .map(|d| d.join("mimeapps.list"))
            .collect();
        files.extend(
            xdg.data_home
                .iter()
                .chain(&xdg.data_dirs)
                .map(|d| d.join("applications/mimeapps.list")),
        );
        Self {
            files,
            xdg: xdg.clone(),
        }
    }
}

/// The desktop-file ids `text` (a `mimeapps.list`) gives as default for
/// `mime`, in order.
pub fn mimeapps_defaults(text: &str, mime: &str) -> Vec<String> {
    let mut in_defaults = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_defaults = line == "[Default Applications]";
            continue;
        }
        if !in_defaults {
            continue;
        }
        if let Some((key, value)) = line.split_once('=')
            && key.trim() == mime
        {
            return value
                .split(';')
                .map(str::trim)
                .filter(|id| id.ends_with(".desktop") && !id.contains(['/', '\\']))
                .map(str::to_owned)
                .collect();
        }
    }
    Vec::new()
}

fn linux(apps: &[InstalledApp], dirs: &MimeDirs) -> Browsers {
    let installed = |id: &String| apps.iter().any(|a| a.browser && &a.id == id);
    let default = ["x-scheme-handler/https", "x-scheme-handler/http"]
        .iter()
        .find_map(|mime| {
            dirs.files.iter().find_map(|file| {
                let metadata = std::fs::metadata(file).ok()?;
                if metadata.len() > 256 * 1024 {
                    return None;
                }
                let text = std::fs::read_to_string(file).ok()?;
                mimeapps_defaults(&text, mime).into_iter().find(installed)
            })
        });
    among(apps, default)
}

/// The browsers `roots` hold for `platform` with a given default (tests).
pub fn browsers_in(platform: Platform, roots: &Roots, mime: &MimeDirs) -> Browsers {
    let apps = crate::discover_in(platform, roots);
    match platform {
        Platform::Linux => linux(&apps, mime),
        _ => among(&apps, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = "\
[Added Associations]
x-scheme-handler/https=other.desktop;

[Default Applications]
text/html=firefox.desktop
x-scheme-handler/https=gone.desktop;org.mozilla.firefox.desktop;
x-scheme-handler/http=org.mozilla.firefox.desktop
";

    #[test]
    fn mimeapps_lists_name_the_default_in_order() {
        assert_eq!(
            mimeapps_defaults(LIST, "x-scheme-handler/https"),
            ["gone.desktop", "org.mozilla.firefox.desktop"]
        );
        assert_eq!(
            mimeapps_defaults(LIST, "x-scheme-handler/ftp"),
            Vec::<String>::new()
        );
        assert_eq!(
            mimeapps_defaults(
                "[Default Applications]\nx-scheme-handler/https=../x.desktop",
                "x-scheme-handler/https"
            ),
            Vec::<String>::new()
        );
    }

    #[test]
    fn linux_browsers_and_the_first_installed_default() {
        let data = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        let apps = data.path().join("applications");
        std::fs::create_dir_all(&apps).unwrap();
        for (id, name, mime) in [
            (
                "org.mozilla.firefox.desktop",
                "Firefox",
                "x-scheme-handler/https;",
            ),
            ("chromium.desktop", "Chromium", "x-scheme-handler/http;"),
            ("editor.desktop", "Editor", "text/plain;"),
        ] {
            std::fs::write(
                apps.join(id),
                format!("[Desktop Entry]\nType=Application\nName={name}\nExec=/bin/true\nMimeType={mime}\n"),
            )
            .unwrap();
        }
        std::fs::write(config.path().join("mimeapps.list"), LIST).unwrap();
        let xdg = XdgDirs::only(&[data.path()]);
        let mime = MimeDirs {
            files: vec![config.path().join("mimeapps.list")],
            xdg: xdg.clone(),
        };
        let roots = Roots {
            bundles: Vec::new(),
            bundle_files: Vec::new(),
            xdg,
            start_menu: Vec::new(),
            registry: false,
        };
        let found = browsers_in(Platform::Linux, &roots, &mime);
        let names: Vec<&str> = found.browsers.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, ["Chromium", "Firefox"]);
        // `gone.desktop` is not installed: the next one is the default.
        assert_eq!(
            found.default.as_deref(),
            Some("org.mozilla.firefox.desktop")
        );
        let none = MimeDirs {
            files: Vec::new(),
            xdg: XdgDirs::only(&[]),
        };
        assert_eq!(browsers_in(Platform::Linux, &roots, &none).default, None);
    }
}
