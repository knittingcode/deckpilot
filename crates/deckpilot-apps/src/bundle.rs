//! macOS application bundles: `Contents/Info.plist` (XML or binary) gives
//! the bundle identifier, the name, the executable and whether the
//! application opens web pages. Read with the `plist` crate on every
//! platform, so fake bundles are tested everywhere; no `mdfind`, no
//! `LaunchServices` database.

use std::path::{Path, PathBuf};

use plist::Value;

use crate::{IdKind, InstalledApp};

/// Largest `Info.plist` read, in bytes.
const MAX_PLIST_LEN: u64 = 1024 * 1024;
/// Most bundles read from one folder.
const MAX_BUNDLES: usize = 2048;

/// What DeckPilot reads of a bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bundle {
    pub path: PathBuf,
    pub identifier: Option<String>,
    pub name: String,
    pub executable: Option<PathBuf>,
    pub browser: bool,
}

fn text<'a>(dict: &'a plist::Dictionary, key: &str) -> Option<&'a str> {
    dict.get(key)
        .and_then(Value::as_string)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// A bundle identifier DeckPilot accepts: reverse-DNS characters only.
pub fn valid_identifier(identifier: &str) -> bool {
    !identifier.is_empty()
        && identifier.len() <= 255
        && identifier.contains('.')
        && identifier
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// Reads the bundle at `path` (an `.app` folder).
pub fn read(path: &Path) -> Option<Bundle> {
    let plist = path.join("Contents/Info.plist");
    let metadata = std::fs::metadata(&plist).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_PLIST_LEN {
        return None;
    }
    let value = Value::from_file(&plist).ok()?;
    let dict = value.as_dictionary()?;
    let identifier = text(dict, "CFBundleIdentifier")
        .filter(|id| valid_identifier(id))
        .map(str::to_owned);
    let fallback = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = text(dict, "CFBundleDisplayName")
        .or_else(|| text(dict, "CFBundleName"))
        .map_or(fallback, str::to_owned);
    let executable = text(dict, "CFBundleExecutable")
        .filter(|e| !e.contains(['/', '\\']) && *e != ".." && *e != ".")
        .map(|e| path.join("Contents/MacOS").join(e));
    let browser = dict
        .get("CFBundleURLTypes")
        .and_then(Value::as_array)
        .is_some_and(|types| {
            types.iter().any(|t| {
                t.as_dictionary()
                    .and_then(|d| d.get("CFBundleURLSchemes"))
                    .and_then(Value::as_array)
                    .is_some_and(|schemes| {
                        schemes.iter().filter_map(Value::as_string).any(|s| {
                            s.eq_ignore_ascii_case("http") || s.eq_ignore_ascii_case("https")
                        })
                    })
            })
        });
    Some(Bundle {
        path: path.to_path_buf(),
        identifier,
        name,
        executable,
        browser,
    })
}

/// The `.app` bundles directly in `dir` (and one level of plain folders
/// below it, e.g. `/Applications/Utilities`), sorted by path.
pub fn scan(dir: &Path) -> Vec<Bundle> {
    let mut found = Vec::new();
    scan_into(dir, 0, &mut found);
    found.sort_by(|a, b| a.path.cmp(&b.path));
    found
}

fn scan_into(dir: &Path, depth: usize, out: &mut Vec<Bundle>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if out.len() >= MAX_BUNDLES {
            return;
        }
        let is_app = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("app"));
        if is_app {
            out.extend(read(&path));
        } else if depth == 0 && path.is_dir() {
            scan_into(&path, depth + 1, out);
        }
    }
}

impl Bundle {
    /// As the picker lists it: by bundle id, by path without one.
    pub fn installed(&self) -> InstalledApp {
        let path = self.path.to_string_lossy().into_owned();
        match &self.identifier {
            Some(id) => InstalledApp {
                name: self.name.clone(),
                id: id.clone(),
                kind: IdKind::Bundle,
                path: Some(path),
                browser: self.browser,
            },
            None => InstalledApp {
                name: self.name.clone(),
                id: path.clone(),
                kind: IdKind::Path,
                path: Some(path),
                browser: self.browser,
            },
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A fake bundle at `dir/<name>.app` with an XML `Info.plist`.
    pub fn fake_bundle(dir: &Path, name: &str, identifier: Option<&str>, browser: bool) -> PathBuf {
        let app = dir.join(format!("{name}.app"));
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        let mut dict = plist::Dictionary::new();
        if let Some(id) = identifier {
            dict.insert("CFBundleIdentifier".into(), id.into());
        }
        dict.insert("CFBundleName".into(), name.into());
        dict.insert("CFBundleExecutable".into(), name.into());
        if browser {
            let mut scheme = plist::Dictionary::new();
            scheme.insert(
                "CFBundleURLSchemes".into(),
                Value::Array(vec!["http".into(), "https".into()]),
            );
            dict.insert(
                "CFBundleURLTypes".into(),
                Value::Array(vec![Value::Dictionary(scheme)]),
            );
        }
        Value::Dictionary(dict)
            .to_file_xml(app.join("Contents/Info.plist"))
            .unwrap();
        std::fs::write(app.join("Contents/MacOS").join(name), "").unwrap();
        app
    }

    #[test]
    fn bundles_are_read_from_their_info_plist() {
        let dir = tempfile::tempdir().unwrap();
        fake_bundle(dir.path(), "Safari", Some("com.apple.Safari"), true);
        fake_bundle(dir.path(), "NoId", None, false);
        let utilities = dir.path().join("Utilities");
        std::fs::create_dir_all(&utilities).unwrap();
        fake_bundle(&utilities, "Terminal", Some("com.apple.Terminal"), false);
        fake_bundle(dir.path(), "Bad", Some("bad id; rm"), false);
        std::fs::create_dir_all(dir.path().join("Empty.app")).unwrap();

        let bundles = scan(dir.path());
        let names: Vec<&str> = bundles.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, ["Bad", "NoId", "Safari", "Terminal"]);
        let safari = bundles.iter().find(|b| b.name == "Safari").unwrap();
        assert_eq!(safari.identifier.as_deref(), Some("com.apple.Safari"));
        assert!(safari.browser);
        assert!(
            safari
                .executable
                .as_ref()
                .unwrap()
                .ends_with("Contents/MacOS/Safari")
        );
        let listed = safari.installed();
        assert_eq!(
            (listed.id.as_str(), listed.kind),
            ("com.apple.Safari", IdKind::Bundle)
        );
        // No (valid) identifier: listed by path.
        for name in ["NoId", "Bad"] {
            let bundle = bundles.iter().find(|b| b.name == name).unwrap();
            assert_eq!(bundle.identifier, None);
            assert_eq!(bundle.installed().kind, IdKind::Path);
        }
    }

    #[test]
    fn identifiers_are_reverse_dns() {
        assert!(valid_identifier("com.apple.Safari"));
        assert!(valid_identifier("org.mozilla.firefox"));
        for bad in [
            "",
            "Safari",
            "com.apple/Safari",
            "com.apple Safari",
            "a.b\0",
        ] {
            assert!(!valid_identifier(bad), "{bad:?}");
        }
    }
}
