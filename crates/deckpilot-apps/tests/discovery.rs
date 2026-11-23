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
    }
}

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
    }
}
