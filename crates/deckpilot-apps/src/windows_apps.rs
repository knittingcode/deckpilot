//! Installed applications on Windows: the App Paths registrations (a
//! stable name such as `chrome.exe` that the shell resolves to the
//! executable), the browsers registered under `Clients\StartMenuInternet`,
//! and the Start menu's shortcuts (opened as files; they have no stable
//! id). The registry is read only on Windows; the parsing and the shortcut
//! scan are plain code, tested everywhere.

use std::path::{Path, PathBuf};

use crate::{IdKind, InstalledApp};

/// How deep below a Start menu folder shortcuts are looked for.
const MAX_DEPTH: usize = 3;
/// Most shortcuts read from one Start menu folder.
const MAX_SHORTCUTS: usize = 2048;

/// An App Paths name DeckPilot accepts: a file name ending in `.exe`.
pub fn valid_app_paths_name(name: &str) -> bool {
    name.len() > 4
        && name.len() <= 260
        && name.to_ascii_lowercase().ends_with(".exe")
        && !name.contains(['/', '\\', ':', '"', '*', '?', '<', '>', '|'])
        && !name.chars().any(char::is_control)
}

/// A registry path value: quotes removed and `%VARIABLE%`s expanded with
/// `variable`; `None` when it is not an absolute path afterwards.
pub fn registry_path(value: &str, variable: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let value = value.trim().trim_matches('"').trim();
    let mut out = String::new();
    let mut rest = value;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let end = after.find('%')?;
        out.push_str(&variable(&after[..end])?);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    let bytes = out.as_bytes();
    let absolute = bytes.len() > 2
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/');
    absolute.then(|| PathBuf::from(out))
}

/// The program of a `shell\open\command` value: the quoted first word, or
/// everything before the first argument of an unquoted one.
pub fn command_program(command: &str) -> Option<&str> {
    let command = command.trim();
    if let Some(rest) = command.strip_prefix('"') {
        return rest.split('"').next().filter(|p| !p.is_empty());
    }
    let lower = command.to_ascii_lowercase();
    lower
        .find(".exe")
        .map(|end| &command[..end + 4])
        .filter(|p| !p.is_empty())
}

/// The Start menu shortcuts (`.lnk`) below `roots`, by path; uninstallers
/// are left out.
pub fn shortcuts(roots: &[PathBuf]) -> Vec<InstalledApp> {
    let mut found = Vec::new();
    for root in roots {
        let mut files = Vec::new();
        collect_shortcuts(root, 0, &mut files);
        for path in files {
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            if name.is_empty() || name.to_lowercase().contains("uninstall") {
                continue;
            }
            found.push(InstalledApp {
                name,
                id: path.to_string_lossy().into_owned(),
                kind: IdKind::Path,
                path: Some(path.to_string_lossy().into_owned()),
                browser: false,
            });
        }
    }
    found
}

fn collect_shortcuts(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > MAX_DEPTH || out.len() >= MAX_SHORTCUTS {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if out.len() >= MAX_SHORTCUTS {
            return;
        }
        if path.is_dir() {
            collect_shortcuts(&path, depth + 1, out);
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("lnk"))
        {
            out.push(path);
        }
    }
}

/// The Start menu folders of the user and of all users.
pub fn start_menu_roots() -> Vec<PathBuf> {
    ["APPDATA", "ProgramData"]
        .iter()
        .filter_map(std::env::var_os)
        .map(|base| PathBuf::from(base).join(r"Microsoft\Windows\Start Menu\Programs"))
        .collect()
}

#[cfg(windows)]
fn environment(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// The executable registered for App Paths name `name` (current user
/// first, then the machine).
#[cfg(windows)]
pub fn app_path(name: &str) -> Option<PathBuf> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};

    if !valid_app_paths_name(name) {
        return None;
    }
    let key = format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\{name}");
    [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE]
        .into_iter()
        .find_map(|hive| {
            let value: String = RegKey::predef(hive)
                .open_subkey_with_flags(&key, KEY_READ)
                .ok()?
                .get_value("")
                .ok()?;
            registry_path(&value, &environment).filter(|p| p.is_file())
        })
}

#[cfg(not(windows))]
pub fn app_path(_name: &str) -> Option<PathBuf> {
    None
}

/// Every App Paths registration whose executable exists, and the
/// registered browsers.
#[cfg(windows)]
pub fn registered() -> Vec<InstalledApp> {
    use std::collections::BTreeMap;

    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};

    let mut apps: BTreeMap<String, InstalledApp> = BTreeMap::new();
    for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let Ok(root) = RegKey::predef(hive).open_subkey_with_flags(
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths",
            KEY_READ,
        ) else {
            continue;
        };
        for name in root.enum_keys().flatten().take(4096) {
            if !valid_app_paths_name(&name) || apps.contains_key(&name.to_ascii_lowercase()) {
                continue;
            }
            let Some(path) = root
                .open_subkey_with_flags(&name, KEY_READ)
                .ok()
                .and_then(|key| key.get_value::<String, _>("").ok())
                .and_then(|value| registry_path(&value, &environment))
                .filter(|p| p.is_file())
            else {
                continue;
            };
            let display = Path::new(&name)
                .file_stem()
                .map_or_else(|| name.clone(), |s| s.to_string_lossy().into_owned());
            apps.insert(
                name.to_ascii_lowercase(),
                InstalledApp {
                    name: display,
                    id: name.clone(),
                    kind: IdKind::AppPaths,
                    path: Some(path.to_string_lossy().into_owned()),
                    browser: false,
                },
            );
        }
    }
    // Browsers: by their registered name; an App Paths entry of the same
    // executable becomes the browser, otherwise its path is listed.
    for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let Ok(clients) = RegKey::predef(hive)
            .open_subkey_with_flags(r"SOFTWARE\Clients\StartMenuInternet", KEY_READ)
        else {
            continue;
        };
        for client in clients.enum_keys().flatten().take(64) {
            let Ok(key) = clients.open_subkey_with_flags(&client, KEY_READ) else {
                continue;
            };
            let name: String = key.get_value("").unwrap_or_else(|_| client.clone());
            let Some(program) = key
                .open_subkey_with_flags(r"shell\open\command", KEY_READ)
                .ok()
                .and_then(|command| command.get_value::<String, _>("").ok())
                .and_then(|command| command_program(&command).map(str::to_owned))
                .and_then(|program| registry_path(&program, &environment))
                .filter(|p| p.is_file())
            else {
                continue;
            };
            let file = program
                .file_name()
                .map(|f| f.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            if let Some(app) = apps.get_mut(&file) {
                app.browser = true;
                app.name.clone_from(&name);
            } else {
                let path = program.to_string_lossy().into_owned();
                apps.insert(
                    path.to_ascii_lowercase(),
                    InstalledApp {
                        name,
                        id: path.clone(),
                        kind: IdKind::Path,
                        path: Some(path),
                        browser: true,
                    },
                );
            }
        }
    }
    apps.into_values().collect()
}

#[cfg(not(windows))]
pub fn registered() -> Vec<InstalledApp> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_paths_are_unquoted_and_expanded() {
        let vars = |name: &str| (name == "ProgramFiles").then(|| r"C:\Program Files".to_owned());
        assert_eq!(
            registry_path(r#""%ProgramFiles%\App\app.exe""#, &vars),
            Some(PathBuf::from(r"C:\Program Files\App\app.exe"))
        );
        assert_eq!(
            registry_path(r"C:\Tools\t.exe", &vars),
            Some(PathBuf::from(r"C:\Tools\t.exe"))
        );
        // Unknown variables, relative paths and stray percent signs.
        assert_eq!(registry_path(r"%Nope%\a.exe", &vars), None);
        assert_eq!(registry_path(r"app.exe", &vars), None);
        assert_eq!(registry_path(r"C:\100%\a.exe", &vars), None);
    }

    #[test]
    fn commands_name_their_program() {
        assert_eq!(
            command_program(r#""C:\Program Files\Mozilla Firefox\firefox.exe" -osint -url "%1""#),
            Some(r"C:\Program Files\Mozilla Firefox\firefox.exe")
        );
        assert_eq!(
            command_program(r"C:\Tools\browser.exe %1"),
            Some(r"C:\Tools\browser.exe")
        );
        assert_eq!(command_program(""), None);
    }

    #[test]
    fn app_paths_names_are_file_names() {
        assert!(valid_app_paths_name("chrome.exe"));
        assert!(valid_app_paths_name("Code.EXE"));
        for bad in [".exe", "chrome", r"C:\chrome.exe", "a|b.exe", "a\nb.exe"] {
            assert!(!valid_app_paths_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn start_menu_shortcuts_are_listed_by_path() {
        let root = tempfile::tempdir().unwrap();
        let tools = root.path().join("Tools");
        std::fs::create_dir_all(&tools).unwrap();
        for file in [
            "Notepad++.lnk",
            "readme.txt",
            "Tools/Uninstall Tools.lnk",
            "Tools/Tool.LNK",
        ] {
            std::fs::write(root.path().join(file), "").unwrap();
        }
        let listed = shortcuts(&[root.path().to_path_buf()]);
        let names: Vec<&str> = listed.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Notepad++", "Tool"]);
        assert!(listed.iter().all(|a| a.kind == IdKind::Path));
    }
}
