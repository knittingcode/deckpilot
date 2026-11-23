//! Installed applications on Linux, per the XDG Base Directory and Desktop
//! Entry specifications: the `applications` folder of `$XDG_DATA_HOME`
//! (`~/.local/share`), then of each `$XDG_DATA_DIRS` folder
//! (`/usr/local/share:/usr/share`), in that order. A desktop-file id is
//! the file's path below `applications` with `/` replaced by `-`
//! (`kde/konsole.desktop` → `kde-konsole.desktop`); the first folder that
//! has an id wins, and a `Hidden` entry there removes the id.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::desktop_entry::{DesktopEntry, MAX_FILE_LEN};
use crate::{AppError, IdKind, InstalledApp, codes};

/// How deep below an `applications` folder entries are looked for.
const MAX_DEPTH: usize = 4;
/// Most entries read from one folder tree.
const MAX_ENTRIES: usize = 4096;

/// Where desktop entries are and where programs are found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XdgDirs {
    /// `$XDG_DATA_HOME`, else `~/.local/share`.
    pub data_home: Option<PathBuf>,
    /// `$XDG_DATA_DIRS`, else `/usr/local/share` and `/usr/share`.
    pub data_dirs: Vec<PathBuf>,
    /// `$PATH`, for `TryExec` and programs named without a path.
    pub path: Option<OsString>,
    /// `$LANG`-style locale for translated names.
    pub locale: Option<String>,
}

impl XdgDirs {
    /// From the environment, with the specification's defaults. Relative
    /// folders are ignored, as the specification requires.
    pub fn from_env() -> Self {
        let absolute = |p: PathBuf| p.is_absolute().then_some(p);
        let data_home = std::env::var_os("XDG_DATA_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .and_then(absolute)
            .or_else(|| std::env::home_dir().map(|home| home.join(".local/share")));
        let data_dirs = std::env::var_os("XDG_DATA_DIRS")
            .filter(|v| !v.is_empty())
            .map_or_else(
                || {
                    vec![
                        PathBuf::from("/usr/local/share"),
                        PathBuf::from("/usr/share"),
                    ]
                },
                |dirs| std::env::split_paths(&dirs).filter_map(absolute).collect(),
            );
        let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .find_map(|key| std::env::var(key).ok().filter(|v| !v.is_empty()));
        Self {
            data_home,
            data_dirs,
            path: std::env::var_os("PATH"),
            locale,
        }
    }

    /// Only `roots`' `applications` folders, in that order; no `PATH`.
    pub fn only(roots: &[&Path]) -> Self {
        Self {
            data_home: None,
            data_dirs: roots.iter().map(|r| r.to_path_buf()).collect(),
            path: None,
            locale: None,
        }
    }

    fn application_dirs(&self) -> Vec<PathBuf> {
        self.data_home
            .iter()
            .chain(&self.data_dirs)
            .map(|base| base.join("applications"))
            .collect()
    }
}

/// An entry found for an id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub id: String,
    pub path: PathBuf,
    pub entry: DesktopEntry,
}

/// Every application entry by id, after precedence: `None` for an id a
/// `Hidden` entry removed.
fn index(dirs: &XdgDirs) -> BTreeMap<String, Option<Found>> {
    let mut found: BTreeMap<String, Option<Found>> = BTreeMap::new();
    for root in dirs.application_dirs() {
        let mut files = Vec::new();
        collect(&root, &root, 0, &mut files);
        for (id, path) in files {
            if found.contains_key(&id) {
                continue;
            }
            let Some(entry) = read_entry(&path) else {
                continue;
            };
            if entry.hidden {
                found.insert(id, None);
                continue;
            }
            if !entry.is_application() || !try_exec_found(&entry, dirs) {
                continue;
            }
            found.insert(id.clone(), Some(Found { id, path, entry }));
        }
    }
    found
}

fn collect(root: &Path, dir: &Path, depth: usize, out: &mut Vec<(String, PathBuf)>) {
    if depth > MAX_DEPTH || out.len() >= MAX_ENTRIES {
        return;
    }
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = read.flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        if out.len() >= MAX_ENTRIES {
            return;
        }
        if path.is_dir() {
            collect(root, &path, depth + 1, out);
        } else if path.extension().is_some_and(|e| e == "desktop") {
            let Ok(relative) = path.strip_prefix(root) else {
                continue;
            };
            let id = relative
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("-");
            out.push((id, path));
        }
    }
}

fn read_entry(path: &Path) -> Option<DesktopEntry> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_FILE_LEN as u64 {
        return None;
    }
    DesktopEntry::parse(&std::fs::read_to_string(path).ok()?).ok()
}

fn try_exec_found(entry: &DesktopEntry, dirs: &XdgDirs) -> bool {
    match entry.try_exec.as_deref().map(str::trim) {
        None | Some("") => true,
        Some(program) => find_program(program, dirs.path.as_deref()).is_some(),
    }
}

/// An absolute path to an existing file, or a bare name found in `path`.
pub fn find_program(program: &str, path: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    let candidate = Path::new(program);
    if candidate.is_absolute() {
        return candidate.is_file().then(|| candidate.to_path_buf());
    }
    if program.is_empty() || program.contains(['/', '\\']) {
        return None;
    }
    std::env::split_paths(path?)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(program))
        .find(|file| file.is_file())
}

/// The installed application with desktop-file id `id`.
pub fn find(id: &str, dirs: &XdgDirs) -> Result<Found, AppError> {
    if !id.ends_with(".desktop") || id.contains(['/', '\\']) || id.starts_with('.') {
        return Err(AppError::new(
            codes::INVALID_APPLICATION,
            format!("“{id}” is not a desktop-file id"),
        ));
    }
    index(dirs).remove(id).flatten().ok_or_else(|| {
        AppError::new(
            codes::NOT_FOUND,
            format!(
                "the application {id} is not installed on this computer (no desktop entry with \
                 this id); choose it again"
            ),
        )
    })
}

/// The applications a menu shows: `NoDisplay` entries are left out.
pub fn applications(dirs: &XdgDirs) -> Vec<InstalledApp> {
    index(dirs)
        .into_values()
        .flatten()
        .filter(|found| !found.entry.no_display)
        .map(|found| InstalledApp {
            name: found
                .entry
                .localized_name(dirs.locale.as_deref())
                .to_owned(),
            browser: found.entry.is_browser(),
            path: Some(found.path.to_string_lossy().into_owned()),
            id: found.id,
            kind: IdKind::DesktopFile,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, relative: &str, text: &str) {
        let path = dir.join("applications").join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn app(name: &str, exec: &str, extra: &str) -> String {
        format!("[Desktop Entry]\nType=Application\nName={name}\nExec={exec}\n{extra}")
    }

    #[test]
    fn ids_follow_precedence_and_hidden_entries_remove_them() {
        let home = tempfile::tempdir().unwrap();
        let system = tempfile::tempdir().unwrap();
        write(
            home.path(),
            "editor.desktop",
            &app("Home Editor", "/usr/bin/vi", ""),
        );
        write(
            system.path(),
            "editor.desktop",
            &app("System Editor", "/usr/bin/ed", ""),
        );
        write(
            system.path(),
            "kde/konsole.desktop",
            &app("Konsole", "konsole", ""),
        );
        write(
            home.path(),
            "gone.desktop",
            "[Desktop Entry]\nHidden=true\n",
        );
        write(system.path(), "gone.desktop", &app("Gone", "gone", ""));
        write(
            system.path(),
            "firefox.desktop",
            &app(
                "Firefox",
                "firefox %u",
                "MimeType=x-scheme-handler/https;\n",
            ),
        );
        write(
            system.path(),
            "helper.desktop",
            &app("Helper", "helper", "NoDisplay=true\n"),
        );
        write(
            system.path(),
            "link.desktop",
            "[Desktop Entry]\nType=Link\nName=L\n",
        );
        write(system.path(), "broken.desktop", "not a desktop file");
        write(
            system.path(),
            "missing.desktop",
            &app("Missing", "missing", "TryExec=/nonexistent/deckpilot\n"),
        );
        let dirs = XdgDirs {
            data_home: Some(home.path().into()),
            ..XdgDirs::only(&[system.path()])
        };

        assert_eq!(
            find("editor.desktop", &dirs).unwrap().entry.name,
            "Home Editor"
        );
        assert_eq!(
            find("kde-konsole.desktop", &dirs).unwrap().entry.name,
            "Konsole"
        );
        // NoDisplay: not listed, but found.
        assert!(find("helper.desktop", &dirs).is_ok());
        for missing in [
            "gone.desktop",
            "link.desktop",
            "broken.desktop",
            "missing.desktop",
        ] {
            assert_eq!(
                find(missing, &dirs).unwrap_err().code,
                codes::NOT_FOUND,
                "{missing}"
            );
        }
        for bad in ["../x.desktop", "editor", "a/b.desktop"] {
            assert_eq!(
                find(bad, &dirs).unwrap_err().code,
                codes::INVALID_APPLICATION
            );
        }

        let listed = applications(&dirs);
        let ids: Vec<&str> = listed.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(
            ids,
            ["editor.desktop", "firefox.desktop", "kde-konsole.desktop"]
        );
        assert!(listed.iter().all(|a| a.kind == IdKind::DesktopFile));
        assert!(
            listed
                .iter()
                .find(|a| a.id == "firefox.desktop")
                .unwrap()
                .browser
        );
    }

    #[test]
    fn programs_are_found_by_absolute_path_or_on_path() {
        let bin = tempfile::tempdir().unwrap();
        std::fs::write(bin.path().join("tool"), "").unwrap();
        let path = std::env::join_paths([bin.path()]).unwrap();
        assert_eq!(
            find_program("tool", Some(&path)),
            Some(bin.path().join("tool"))
        );
        assert!(find_program("bin/tool", Some(&path)).is_none());
        assert!(find_program("none", Some(&path)).is_none());
        assert!(find_program("tool", None).is_none());
        let absolute = bin.path().join("tool");
        assert_eq!(
            find_program(absolute.to_str().unwrap(), None),
            Some(absolute)
        );
    }
}
