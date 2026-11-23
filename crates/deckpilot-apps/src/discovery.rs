//! The installed applications of this computer, for the application and
//! browser pickers.

use std::path::PathBuf;

use crate::xdg::XdgDirs;
use crate::{InstalledApp, Platform, bundle, windows_apps, xdg};

/// Most applications listed.
const MAX_LISTED: usize = 4096;

/// Where applications are looked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Roots {
    /// macOS: folders holding `.app` bundles.
    pub bundles: Vec<PathBuf>,
    /// Linux: the XDG data folders.
    pub xdg: XdgDirs,
    /// Windows: Start menu folders.
    pub start_menu: Vec<PathBuf>,
    /// Windows: read the registry (App Paths, browsers).
    pub registry: bool,
}

impl Roots {
    /// This computer's standard locations.
    pub fn standard() -> Self {
        let mut bundles = vec![
            PathBuf::from("/Applications"),
            PathBuf::from("/System/Applications"),
        ];
        if let Some(home) = std::env::home_dir() {
            bundles.push(home.join("Applications"));
        }
        Self {
            bundles,
            xdg: XdgDirs::from_env(),
            start_menu: windows_apps::start_menu_roots(),
            registry: true,
        }
    }
}

/// The installed applications of this computer, sorted by name.
pub fn discover() -> Vec<InstalledApp> {
    discover_in(Platform::current(), &Roots::standard())
}

/// The applications `roots` hold, as `platform` lists them, sorted by name
/// (case ignored) and without repeated ids.
pub fn discover_in(platform: Platform, roots: &Roots) -> Vec<InstalledApp> {
    let mut apps = match platform {
        Platform::MacOs => roots
            .bundles
            .iter()
            .flat_map(|dir| bundle::scan(dir))
            .map(|b| b.installed())
            .collect(),
        Platform::Linux => xdg::applications(&roots.xdg),
        Platform::Windows => {
            let mut apps = if roots.registry {
                windows_apps::registered()
            } else {
                Vec::new()
            };
            apps.extend(windows_apps::shortcuts(&roots.start_menu));
            apps
        }
        Platform::Other => Vec::new(),
    };
    let mut seen = std::collections::BTreeSet::new();
    apps.retain(|app| seen.insert(app.id.to_lowercase()));
    apps.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.id.cmp(&b.id))
    });
    apps.truncate(MAX_LISTED);
    apps
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IdKind;
    use crate::bundle::tests::fake_bundle;

    fn empty() -> Roots {
        Roots {
            bundles: Vec::new(),
            xdg: XdgDirs::only(&[]),
            start_menu: Vec::new(),
            registry: false,
        }
    }

    #[test]
    fn macos_lists_bundles_by_identifier_and_drops_repeats() {
        let system = tempfile::tempdir().unwrap();
        let user = tempfile::tempdir().unwrap();
        fake_bundle(system.path(), "Safari", Some("com.apple.Safari"), true);
        fake_bundle(
            system.path(),
            "calculator",
            Some("com.apple.calculator"),
            false,
        );
        // The same id twice (a copy in ~/Applications): listed once.
        fake_bundle(user.path(), "Safari", Some("com.apple.Safari"), true);
        let roots = Roots {
            bundles: vec![system.path().into(), user.path().into()],
            ..empty()
        };
        let apps = discover_in(Platform::MacOs, &roots);
        let ids: Vec<&str> = apps.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["com.apple.calculator", "com.apple.Safari"]);
        assert!(apps.iter().all(|a| a.kind == IdKind::Bundle));
    }

    #[test]
    fn linux_lists_desktop_entries_and_windows_shortcuts() {
        let data = tempfile::tempdir().unwrap();
        let apps_dir = data.path().join("applications");
        std::fs::create_dir_all(&apps_dir).unwrap();
        std::fs::write(
            apps_dir.join("org.gnome.Terminal.desktop"),
            "[Desktop Entry]\nType=Application\nName=Terminal\nExec=gnome-terminal\n",
        )
        .unwrap();
        let start = tempfile::tempdir().unwrap();
        std::fs::write(start.path().join("Paint.lnk"), "").unwrap();
        let roots = Roots {
            xdg: XdgDirs::only(&[data.path()]),
            start_menu: vec![start.path().into()],
            ..empty()
        };
        let linux = discover_in(Platform::Linux, &roots);
        assert_eq!(linux.len(), 1);
        assert_eq!(linux[0].id, "org.gnome.Terminal.desktop");
        let windows = discover_in(Platform::Windows, &roots);
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].name, "Paint");
        assert_eq!(discover_in(Platform::Other, &roots), []);
    }
}
