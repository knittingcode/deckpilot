//! What an application resource's value names, per platform.

use std::path::PathBuf;

use crate::{AppError, codes};

/// Longest application value accepted, in bytes.
pub const MAX_VALUE_LEN: usize = 1024;

/// The operating system the value is read for. Values are read the way
/// the core's launch plans read them (`deckpilot_core::launch::plan`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Platform {
    MacOs,
    Windows,
    Linux,
    Other,
}

impl Platform {
    pub const fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else {
            Self::Other
        }
    }
}

/// An application resource's value, read for one platform.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AppId {
    /// macOS: a bundle identifier such as `com.apple.Safari`.
    Bundle(String),
    /// Windows: an `AppUserModelID` (`Family_hash!App`), also written as
    /// `shell:AppsFolder\<AUMID>`.
    Aumid(String),
    /// Linux: a desktop-file id such as `org.gnome.Terminal.desktop`.
    DesktopFile(String),
    /// Windows: an App Paths registration such as `chrome.exe`.
    AppPaths(String),
    /// An absolute path: an executable, an `.app` bundle, a shortcut.
    Path(PathBuf),
    /// A bare program name, found on `PATH` (or, on macOS, an application
    /// name `open -a` finds). Kept for saved bindings.
    Program(String),
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::new(codes::INVALID_APPLICATION, message)
}

const APPS_FOLDER: &str = "shell:appsfolder\\";

impl AppId {
    /// Reads `value` for `platform`, refusing what cannot be an
    /// application: empty, too long, control characters, a leading `-`
    /// (it would read as an option), a relative path.
    pub fn parse(platform: Platform, value: &str) -> Result<Self, AppError> {
        let value = value.trim();
        if value.is_empty() {
            return Err(invalid("no application is set"));
        }
        if value.len() > MAX_VALUE_LEN {
            return Err(invalid(format!(
                "the application is longer than {MAX_VALUE_LEN} bytes"
            )));
        }
        if value.chars().any(char::is_control) {
            return Err(invalid("the application contains control characters"));
        }
        if value.starts_with('-') {
            return Err(invalid(format!("“{value}” is not an application")));
        }
        let lower = value.to_ascii_lowercase();
        if platform == Platform::Windows {
            if let Some(aumid) = lower
                .starts_with(APPS_FOLDER)
                .then(|| &value[APPS_FOLDER.len()..])
            {
                return aumid_value(aumid);
            }
            if value.contains('!') && !value.contains(['/', '\\']) {
                return aumid_value(value);
            }
        }
        if value.contains(['/', '\\']) {
            let path = PathBuf::from(value);
            let absolute = path.is_absolute()
                // A Windows path read on another system, e.g. in tests.
                || (platform == Platform::Windows && windows_absolute(value));
            if !absolute {
                return Err(invalid(format!(
                    "“{value}” must be an absolute path, an application identifier or a program \
                     name"
                )));
            }
            return Ok(Self::Path(path));
        }
        if value.contains(char::is_whitespace) && platform != Platform::MacOs {
            return Err(invalid(format!("“{value}” is not a program name")));
        }
        // `lower` is lower case: these comparisons ignore case.
        #[allow(clippy::case_sensitive_file_extension_comparisons)]
        Ok(match platform {
            Platform::MacOs
                if value.contains('.')
                    && !lower.ends_with(".app")
                    && !value.contains(char::is_whitespace) =>
            {
                Self::Bundle(value.to_owned())
            }
            Platform::Linux if lower.ends_with(".desktop") => Self::DesktopFile(value.to_owned()),
            Platform::Windows if lower.ends_with(".exe") => Self::AppPaths(value.to_owned()),
            _ => Self::Program(value.to_owned()),
        })
    }

    /// The value a binding stores for this id.
    pub fn value(&self) -> String {
        match self {
            Self::Bundle(v)
            | Self::Aumid(v)
            | Self::DesktopFile(v)
            | Self::AppPaths(v)
            | Self::Program(v) => v.clone(),
            Self::Path(path) => path.to_string_lossy().into_owned(),
        }
    }
}

fn aumid_value(aumid: &str) -> Result<AppId, AppError> {
    let valid = !aumid.is_empty()
        && aumid.len() <= 256
        && aumid
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '!'))
        && aumid.matches('!').count() <= 1;
    if valid {
        Ok(AppId::Aumid(aumid.to_owned()))
    } else {
        Err(invalid(format!("“{aumid}” is not an AppUserModelID")))
    }
}

/// `C:\…`, `C:/…` or `\\server\share\…`.
fn windows_absolute(value: &str) -> bool {
    let bytes = value.as_bytes();
    (bytes.len() > 2
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/'))
        || value.starts_with("\\\\")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(platform: Platform, value: &str) -> AppId {
        AppId::parse(platform, value).unwrap()
    }

    #[test]
    fn values_read_as_the_launch_plans_read_them() {
        use Platform::{Linux, MacOs, Windows};
        assert_eq!(
            parse(MacOs, "com.apple.Safari"),
            AppId::Bundle("com.apple.Safari".into())
        );
        assert_eq!(parse(MacOs, "Safari"), AppId::Program("Safari".into()));
        assert_eq!(
            parse(MacOs, "Visual Studio Code"),
            AppId::Program("Visual Studio Code".into())
        );
        assert_eq!(
            parse(MacOs, "Safari.app"),
            AppId::Program("Safari.app".into())
        );
        assert!(matches!(
            parse(MacOs, "/Applications/Safari.app"),
            AppId::Path(_)
        ));
        assert_eq!(
            parse(Linux, "org.gnome.Terminal.desktop"),
            AppId::DesktopFile("org.gnome.Terminal.desktop".into())
        );
        assert_eq!(parse(Linux, "code"), AppId::Program("code".into()));
        assert_eq!(
            parse(Windows, "chrome.exe"),
            AppId::AppPaths("chrome.exe".into())
        );
        assert_eq!(
            parse(Windows, "Microsoft.WindowsCalculator_8wekyb3d8bbwe!App"),
            AppId::Aumid("Microsoft.WindowsCalculator_8wekyb3d8bbwe!App".into())
        );
        assert_eq!(
            parse(
                Windows,
                r"shell:AppsFolder\Microsoft.WindowsCalculator_8wekyb3d8bbwe!App"
            ),
            AppId::Aumid("Microsoft.WindowsCalculator_8wekyb3d8bbwe!App".into())
        );
        assert_eq!(
            parse(Windows, r"C:\Program Files\App\app.exe"),
            AppId::Path(r"C:\Program Files\App\app.exe".into())
        );
    }

    #[test]
    fn untrusted_values_are_refused() {
        for (platform, value) in [
            (Platform::Linux, ""),
            (Platform::Linux, "   "),
            (Platform::Linux, "--help"),
            (Platform::Linux, "bin/tool"),
            (Platform::Linux, "a b"),
            (Platform::Linux, "app\u{0}x"),
            (Platform::Linux, "app\nrm"),
            (Platform::Windows, r"shell:AppsFolder\a b!c"),
            (Platform::Windows, "x!y!z"),
            (Platform::Windows, r"relative\app.exe"),
        ] {
            let error = AppId::parse(platform, value).unwrap_err();
            assert_eq!(error.code, codes::INVALID_APPLICATION, "{value:?}");
        }
        let long = "a".repeat(MAX_VALUE_LEN + 1);
        assert!(AppId::parse(Platform::Linux, &long).is_err());
    }

    #[test]
    fn values_round_trip() {
        for (platform, value) in [
            (Platform::MacOs, "com.apple.Safari"),
            (Platform::Linux, "firefox.desktop"),
            (Platform::Windows, "chrome.exe"),
            (Platform::Linux, "/usr/bin/code"),
        ] {
            assert_eq!(parse(platform, value).value(), value);
        }
    }
}
