//! What the core's launch service opens for an application value. This
//! module never starts anything: the System module hands the result to
//! `deckpilot_core::launch` (one service for Open, Open Application and
//! Website), which runs it without a shell.

use std::path::{Path, PathBuf};

use crate::xdg::{self, XdgDirs};
use crate::{AppError, AppId, Platform, codes, windows_apps};

/// How an application is opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launchable {
    /// The launch plan's own way for this value (macOS `open -b`/`-a`, an
    /// executable started directly); `targets` are handed to the plan as
    /// its arguments or file.
    Application(String),
    /// This program with these arguments (the targets already in place):
    /// a Linux desktop entry's `Exec`, an App Paths executable, a Windows
    /// `AppUserModelID` through `explorer.exe shell:AppsFolder\…`.
    Program {
        program: PathBuf,
        arguments: Vec<String>,
    },
    /// A file the system opens as if double-clicked (a Windows shortcut).
    File(PathBuf),
}

/// A program of the Windows folder by its full path: the core's rule
/// (`deckpilot_core::launch::windows_program`), so both open the same.
fn windows_program(relative: &str) -> PathBuf {
    PathBuf::from(deckpilot_core::launch::windows_program(relative))
}

fn not_found(message: impl Into<String>) -> AppError {
    AppError::new(codes::NOT_FOUND, message)
}

/// What opens application `value` on this computer, given `targets` (the
/// arguments, URLs or files it is to open).
pub fn launchable(value: &str, targets: &[String]) -> Result<Launchable, AppError> {
    launchable_in(Platform::current(), value, targets, &XdgDirs::from_env())
}

/// As [`launchable`], for `platform` with `xdg` folders (tests).
pub fn launchable_in(
    platform: Platform,
    value: &str,
    targets: &[String],
    xdg: &XdgDirs,
) -> Result<Launchable, AppError> {
    let id = AppId::parse(platform, value)?;
    match id {
        AppId::Bundle(bundle) => {
            check_bundle(platform, &bundle)?;
            Ok(Launchable::Application(bundle))
        }
        AppId::DesktopFile(desktop) => {
            let found = xdg::find(&desktop, xdg)?;
            if found.entry.terminal {
                return Err(AppError::new(
                    codes::INVALID_APPLICATION,
                    format!(
                        "{desktop} runs in a terminal; bind its program (or a terminal) instead"
                    ),
                ));
            }
            let mut argv = found
                .entry
                .command(targets, &found.path.to_string_lossy())?
                .into_iter();
            let name = argv.next().unwrap_or_default();
            let program = xdg::find_program(&name, xdg.path.as_deref()).ok_or_else(|| {
                not_found(format!(
                    "{desktop} starts “{name}”, which is not installed on this computer"
                ))
            })?;
            Ok(Launchable::Program {
                program,
                arguments: argv.collect(),
            })
        }
        AppId::AppPaths(name) => {
            let program = windows_apps::app_path(&name)
                .or_else(|| xdg::find_program(&name, std::env::var_os("PATH").as_deref()))
                .ok_or_else(|| {
                    not_found(format!(
                        "{name} is not installed on this computer (no App Paths registration \
                         and not on PATH); choose it again"
                    ))
                })?;
            Ok(Launchable::Program {
                program,
                arguments: targets.to_vec(),
            })
        }
        AppId::Aumid(aumid) => {
            if !targets.is_empty() {
                return Err(AppError::new(
                    codes::INVALID_APPLICATION,
                    "an AppUserModelID application cannot be given arguments or a file",
                ));
            }
            Ok(Launchable::Program {
                program: windows_program("explorer.exe"),
                arguments: vec![format!("shell:AppsFolder\\{aumid}")],
            })
        }
        AppId::Path(path) => {
            if !path_exists(platform, &path) {
                return Err(not_found(format!(
                    "the application {} does not exist; it may have been moved or uninstalled",
                    path.display()
                )));
            }
            let shortcut = path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("lnk"));
            if shortcut {
                if !targets.is_empty() {
                    return Err(AppError::new(
                        codes::INVALID_APPLICATION,
                        "a shortcut cannot be given arguments or a file; bind its program instead",
                    ));
                }
                return Ok(Launchable::File(path));
            }
            Ok(Launchable::Application(path.to_string_lossy().into_owned()))
        }
        AppId::Program(name) => Ok(Launchable::Application(name)),
    }
}

/// Whether application `value` is installed on this computer, when that
/// can be checked: a bundle id, a desktop-file id, an App Paths name or a
/// path. `None` for what cannot be checked without starting it (an
/// `AppUserModelID`, a bare program or application name).
pub fn presence(value: &str) -> Option<bool> {
    presence_in(Platform::current(), value, &XdgDirs::from_env())
}

/// As [`presence`], for `platform` with `xdg` folders (tests).
pub fn presence_in(platform: Platform, value: &str, xdg: &XdgDirs) -> Option<bool> {
    let id = AppId::parse(platform, value).ok()?;
    match id {
        AppId::Aumid(_) | AppId::Program(_) => None,
        AppId::Path(ref path) if path.to_string_lossy().starts_with("\\\\") => None,
        AppId::Bundle(_) | AppId::DesktopFile(_) | AppId::AppPaths(_) | AppId::Path(_) => {
            Some(launchable_in(platform, value, &[], xdg).is_ok())
        }
    }
}

/// A Windows path read on another system cannot be checked there (tests).
fn path_exists(platform: Platform, path: &Path) -> bool {
    path.exists() || (platform != Platform::current() && !path.is_absolute())
}

#[cfg(target_os = "macos")]
fn check_bundle(platform: Platform, bundle: &str) -> Result<(), AppError> {
    if platform != Platform::MacOs || crate::native_macos::bundle_path(bundle).is_some() {
        return Ok(());
    }
    Err(not_found(format!(
        "no application with the bundle identifier {bundle} is installed on this computer; \
         choose it again"
    )))
}

#[cfg(not(target_os = "macos"))]
#[allow(clippy::unnecessary_wraps)] // The macOS version can fail.
fn check_bundle(_platform: Platform, _bundle: &str) -> Result<(), AppError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_owned()).collect()
    }

    #[test]
    fn desktop_entries_start_their_exec_program() {
        let data = tempfile::tempdir().unwrap();
        let bin = tempfile::tempdir().unwrap();
        std::fs::write(bin.path().join("browser"), "").unwrap();
        let apps = data.path().join("applications");
        std::fs::create_dir_all(&apps).unwrap();
        std::fs::write(
            apps.join("browser.desktop"),
            "[Desktop Entry]\nType=Application\nName=B\nExec=browser --new %U\n",
        )
        .unwrap();
        std::fs::write(
            apps.join("gone.desktop"),
            "[Desktop Entry]\nType=Application\nName=G\nExec=deckpilot-gone %u\n",
        )
        .unwrap();
        std::fs::write(
            apps.join("top.desktop"),
            "[Desktop Entry]\nType=Application\nName=T\nExec=top\nTerminal=true\n",
        )
        .unwrap();
        let dirs = XdgDirs {
            path: Some(std::env::join_paths([bin.path()]).unwrap()),
            ..XdgDirs::only(&[data.path()])
        };
        let url = strings(&["https://example.com"]);
        assert_eq!(
            launchable_in(Platform::Linux, "browser.desktop", &url, &dirs).unwrap(),
            Launchable::Program {
                program: bin.path().join("browser"),
                arguments: strings(&["--new", "https://example.com"]),
            }
        );
        let gone = launchable_in(Platform::Linux, "gone.desktop", &url, &dirs).unwrap_err();
        assert_eq!(gone.code, codes::NOT_FOUND);
        let missing = launchable_in(Platform::Linux, "none.desktop", &[], &dirs).unwrap_err();
        assert_eq!(missing.code, codes::NOT_FOUND);
        let terminal = launchable_in(Platform::Linux, "top.desktop", &[], &dirs).unwrap_err();
        assert_eq!(terminal.code, codes::INVALID_APPLICATION);
    }

    #[test]
    fn presence_is_unknown_where_it_cannot_be_checked() {
        let dirs = XdgDirs::only(&[]);
        assert_eq!(
            presence_in(
                Platform::Windows,
                "Microsoft.WindowsCalculator_8wekyb3d8bbwe!App",
                &dirs
            ),
            None
        );
        assert_eq!(presence_in(Platform::Linux, "code", &dirs), None);
        assert_eq!(presence_in(Platform::MacOs, "Safari", &dirs), None);
        assert_eq!(
            presence_in(Platform::Linux, "gone.desktop", &dirs),
            Some(false)
        );
        assert_eq!(
            presence_in(Platform::Windows, "deckpilot-none.exe", &dirs),
            Some(false)
        );
        assert_eq!(presence_in(Platform::Linux, "--bad", &dirs), None);
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("tool");
        std::fs::write(&exe, "").unwrap();
        assert_eq!(
            presence_in(Platform::current(), &exe.to_string_lossy(), &dirs),
            Some(true)
        );
        assert_eq!(
            presence_in(
                Platform::current(),
                &dir.path().join("gone").to_string_lossy(),
                &dirs
            ),
            Some(false)
        );
    }

    #[test]
    fn windows_ids_open_natively() {
        let dirs = XdgDirs::only(&[]);
        assert_eq!(
            launchable_in(
                Platform::Windows,
                "Microsoft.WindowsCalculator_8wekyb3d8bbwe!App",
                &[],
                &dirs
            )
            .unwrap(),
            Launchable::Program {
                program: windows_program("explorer.exe"),
                arguments: strings(&[
                    r"shell:AppsFolder\Microsoft.WindowsCalculator_8wekyb3d8bbwe!App"
                ]),
            }
        );
        assert!(launchable_in(Platform::Windows, "A_b!App", &strings(&["x"]), &dirs).is_err());
        let unknown = launchable_in(Platform::Windows, "deckpilot-none.exe", &[], &dirs);
        assert_eq!(unknown.unwrap_err().code, codes::NOT_FOUND);
    }

    #[test]
    fn paths_must_exist_and_names_pass_through() {
        let dirs = XdgDirs::only(&[]);
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("tool");
        std::fs::write(&exe, "").unwrap();
        let platform = Platform::current();
        let value = exe.to_string_lossy().into_owned();
        assert_eq!(
            launchable_in(platform, &value, &[], &dirs).unwrap(),
            Launchable::Application(value.clone())
        );
        let missing = dir.path().join("gone");
        let error = launchable_in(platform, &missing.to_string_lossy(), &[], &dirs).unwrap_err();
        assert_eq!(error.code, codes::NOT_FOUND);
        assert!(error.message.contains("moved"));
        let shortcut = dir.path().join("App.lnk");
        std::fs::write(&shortcut, "").unwrap();
        assert_eq!(
            launchable_in(platform, &shortcut.to_string_lossy(), &[], &dirs).unwrap(),
            Launchable::File(shortcut)
        );
        assert_eq!(
            launchable_in(Platform::Linux, "code", &[], &dirs).unwrap(),
            Launchable::Application("code".into())
        );
    }
}
