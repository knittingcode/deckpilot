//! Real discovery on the computer the tests run on (CI: macOS, Windows and
//! Linux runners): the installed applications, the browsers and icons, as
//! the pickers receive them. Each test asserts only what the runner images
//! are known to have installed, and prints what it found.

use deckpilot_apps::{IdKind, InstalledApp, browsers, discover, icons, xdg::XdgDirs};

fn summary(apps: &[InstalledApp]) -> String {
    apps.iter()
        .take(40)
        .map(|a| format!("{} [{:?}] {}", a.name, a.kind, a.id))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn installed_applications_are_found() {
    let apps = discover();
    println!("{} applications:\n{}", apps.len(), summary(&apps));
    assert!(!apps.is_empty(), "no application was found");
    let has = |id: &str| apps.iter().any(|a| a.id.eq_ignore_ascii_case(id));
    if cfg!(target_os = "macos") {
        assert!(has("com.apple.Safari"), "Safari");
        assert!(has("com.apple.finder"), "Finder");
        assert!(
            apps.iter()
                .all(|a| a.kind != IdKind::Path || a.path.is_some())
        );
    } else if cfg!(windows) {
        // Edge is registered under App Paths on every Windows image.
        assert!(has("msedge.exe"), "Microsoft Edge (App Paths)");
    } else {
        assert!(
            apps.iter().any(|a| a.kind == IdKind::DesktopFile),
            "a desktop entry"
        );
    }
}

#[test]
fn browsers_and_the_default_are_found() {
    let found = browsers::browsers();
    println!(
        "{} browsers (default {:?}):\n{}",
        found.browsers.len(),
        found.default,
        summary(&found.browsers)
    );
    assert!(found.browsers.iter().all(|b| b.browser));
    if let Some(default) = &found.default {
        assert!(found.browsers.iter().any(|b| &b.id == default));
    }
    if cfg!(target_os = "macos") {
        assert!(found.browsers.iter().any(|b| b.id == "com.apple.Safari"));
        // LaunchServices always has a default https handler.
        assert!(found.default.is_some());
    } else if cfg!(windows) {
        assert!(!found.browsers.is_empty(), "Edge is a registered browser");
    } else if std::path::Path::new(CHROME_DESKTOP).is_file() {
        // The Ubuntu runner image installs Google Chrome's desktop entry.
        assert!(
            found
                .browsers
                .iter()
                .any(|b| b.id == "google-chrome.desktop"),
            "Google Chrome is a browser"
        );
    } else {
        println!("no browser desktop entry on this runner: Linux browsers are fixture-tested only");
    }
}

/// Google Chrome's desktop entry, where the runner has it.
const CHROME_DESKTOP: &str = "/usr/share/applications/google-chrome.desktop";

#[test]
fn icons_are_drawn_by_the_system() {
    let apps = discover();
    let xdg = XdgDirs::from_env();
    let with_icons: Vec<&InstalledApp> = apps
        .iter()
        .filter(|a| icons::icon(a, &xdg).is_some())
        .take(5)
        .collect();
    println!(
        "icons for {} of the first applications: {:?}",
        with_icons.len(),
        with_icons.iter().map(|a| &a.name).collect::<Vec<_>>()
    );
    if cfg!(target_os = "macos") || cfg!(windows) {
        let first = apps.first().expect("an application");
        let icon = icons::icon(first, &xdg).expect("the system draws an icon");
        assert!(icon.len() <= icons::MAX_ICON_BYTES);
        assert!(icon.starts_with(b"\x89PNG\r\n\x1a\n"));
    } else {
        // Every desktop entry whose icon the theme has as a PNG: each must
        // be drawn. None at all: Linux icons are fixture-tested only here.
        let themed: Vec<&InstalledApp> = apps
            .iter()
            .filter(|a| a.kind == IdKind::DesktopFile)
            .filter(|a| {
                a.path
                    .as_deref()
                    .and_then(|p| std::fs::read_to_string(p).ok())
                    .and_then(|text| deckpilot_apps::desktop_entry::DesktopEntry::parse(&text).ok())
                    .and_then(|entry| entry.icon)
                    .is_some_and(|name| icons::theme_icon(&name, &xdg).is_some())
            })
            .collect();
        if themed.is_empty() {
            println!(
                "no desktop entry here has a themed PNG icon: Linux icons are fixture-tested only"
            );
        } else {
            for app in &themed {
                let icon = icons::icon(app, &xdg)
                    .unwrap_or_else(|| panic!("{} has a themed icon that was not drawn", app.name));
                assert!(icon.starts_with(b"\x89PNG\r\n\x1a\n"));
            }
            println!("drew the themed icons of {} desktop entries", themed.len());
        }
    }
}
