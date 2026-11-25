//! Path resolution (design §5.3 "Path resolution"), before every file
//! system operation:
//!
//! 1. The forms: a repository-relative path (`{repository, relative}`)
//!    or an absolute path, already in their canonical forms (the ABI
//!    refuses anything else), normalized again here: a `..` that would
//!    leave its root is refused, there is no working directory.
//! 2. The **scope**: the longest granted root that is a prefix of the path
//!    (component by component, without case where names compare without
//!    case). An absolute path counts inside a repository's scope only with
//!    `repositories.paths`. No match: `outsideScope`.
//! 3. The root is opened, then checked: its canonical path must still be
//!    the recorded one, and (on Unix) the opened directory must be the one
//!    at that path, so a root swapped for a link is refused ("scope
//!    moved").
//! 4. Every operation then works **relative to that open directory** with
//!    `cap-std`, which never resolves beneath it to anything outside it: a
//!    link inside the scope that leads out fails (`symlinkEscape`).

use std::path::{Component, Path, PathBuf};

use cap_std::ambient_authority;
use cap_std::fs::Dir;
use deckpilot_core::script_grants::{FsAccess, FsRoot, ScriptGrants};
use deckpilot_plugin_abi::scripts::{PathArg, ScriptFault, codes};

use super::super::{RunProject, fault};
use super::protected::{Protected, canonical, same_path, simplify, starts_with};

/// A granted scope, for this call.
#[derive(Debug, Clone)]
pub(crate) struct Scope {
    /// Canonical.
    pub root: PathBuf,
    /// What messages and plans name instead of the path: the repository's
    /// key or the directory's label.
    pub label: String,
    pub access: FsAccess,
    pub root_destructive: bool,
    /// A repository's scope (an absolute path inside it needs
    /// `repositories.paths`).
    pub repository: bool,
}

/// The scopes of the script's grants in the run's project: a repository
/// that is not bound here (or missing) gives none. One root granted twice
/// keeps the larger access.
pub(crate) fn scopes(grants: &ScriptGrants, project: Option<&RunProject>) -> Vec<Scope> {
    let mut out: Vec<Scope> = Vec::new();
    let mut add = |scope: Scope| {
        if let Some(same) = out.iter_mut().find(|s| same_path(&s.root, &scope.root)) {
            same.access = same.access.max(scope.access);
            same.root_destructive |= scope.root_destructive;
            same.repository |= scope.repository;
        } else {
            out.push(scope);
        }
    };
    for grant in &grants.fs {
        match &grant.root {
            FsRoot::Repository(key) => {
                for repo in project.iter().flat_map(|p| &p.repositories) {
                    let Some(folder) = &repo.folder else { continue };
                    if key == "*" || *key == repo.key {
                        add(Scope {
                            root: simplify(folder.clone()),
                            label: repo.key.clone(),
                            access: grant.access,
                            root_destructive: grant.root_destructive,
                            repository: true,
                        });
                    }
                }
            }
            FsRoot::Directory { path, label } => add(Scope {
                root: simplify(path.clone()),
                label: label.clone(),
                access: grant.access,
                root_destructive: grant.root_destructive,
                repository: false,
            }),
        }
    }
    out
}

/// A path resolved into a scope, with its root open.
#[derive(Debug)]
pub(crate) struct Target {
    pub scope: Scope,
    /// The names below the root (empty: the root itself).
    pub rel: Vec<String>,
    pub root: Dir,
}

impl Target {
    /// The target's absolute path (the root's canonical path and the
    /// names); real for an operation that follows no link below the root.
    pub fn absolute(&self) -> PathBuf {
        let mut path = self.scope.root.clone();
        path.extend(&self.rel);
        path
    }

    /// How messages and plans name it: `label` or `label/a/b`.
    pub fn shown(&self) -> String {
        if self.rel.is_empty() {
            self.scope.label.clone()
        } else {
            format!("{}/{}", self.scope.label, self.rel.join("/"))
        }
    }
}

/// A UNC share of this computer: `localhost`, a loopback address, its own
/// name, or an administrative share (`C$`, `ADMIN$`).
pub(crate) fn is_local_share(server: &std::ffi::OsStr, share: &std::ffi::OsStr) -> bool {
    let server = server.to_string_lossy().to_ascii_lowercase();
    let server = server.trim_end_matches('.');
    let share = share.to_string_lossy();
    // This computer's own names: its NetBIOS name, its DNS host name with
    // or without its domain, its `.local` name.
    let own_names: Vec<String> = ["COMPUTERNAME", "HOSTNAME"]
        .iter()
        .filter_map(|var| std::env::var(var).ok())
        .map(|name| name.to_ascii_lowercase())
        .collect();
    let domain = std::env::var("USERDNSDOMAIN")
        .ok()
        .map(|d| d.to_ascii_lowercase());
    let own = own_names.iter().any(|name| {
        server == name
            || server == format!("{name}.local")
            || domain
                .as_ref()
                .is_some_and(|d| server == format!("{name}.{d}"))
    });
    // An address of any kind is never a share's server: this computer's
    // own (loopback, `0.0.0.0`, its LAN addresses, a number like
    // `2130706433`) cannot be told from another's by its text.
    let address = server.parse::<std::net::IpAddr>().is_ok()
        || (server.bytes().next().is_some_and(|b| b.is_ascii_digit())
            && server
                .bytes()
                .all(|b| b.is_ascii_hexdigit() || b == b'.' || b == b'x'))
        || server.ends_with(".ipv6-literal.net")
        || server.contains(':');
    own || address
        || share.ends_with('$')
        || matches!(server, "localhost" | "." | "?" | "")
        || server.ends_with(".localhost")
}

/// Windows names that are devices whatever the folder.
fn is_device_name(name: &str) -> bool {
    let stem = name
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end()
        .to_ascii_uppercase();
    matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ((stem.starts_with("COM") || stem.starts_with("LPT"))
        && stem.len() == 4
        && stem.as_bytes()[3].is_ascii_digit())
}

/// A Windows 8.3 short name (`GIT~1`), which reaches a folder under
/// another name than the one the checks compare (security review of
/// #469, H2).
fn is_short_name(name: &str) -> bool {
    name.as_bytes()
        .windows(2)
        .any(|w| w[0] == b'~' && w[1].is_ascii_digit())
}

/// One name below a root: never empty, `.`, `..`, with a separator, NUL
/// or `:`; on Windows never a device name, a short name or ending in a dot
/// or a space.
fn checked_name(name: &str) -> Result<(), ScriptFault> {
    let bad = name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\\', '\0', ':'])
        // A short name reaches a folder under another name on Windows and
        // on its shares mounted elsewhere.
        || is_short_name(name)
        || (cfg!(windows) && (is_device_name(name) || name.ends_with(['.', ' '])));
    if bad {
        Err(fault(
            codes::INVALID_ARGUMENT,
            format!(
                "“{}” is not a file or folder name",
                super::super::run::loggable(name)
            ),
        ))
    } else {
        Ok(())
    }
}

/// The lexical names of a repository-relative part.
fn relative_names(relative: &str) -> Result<Vec<String>, ScriptFault> {
    let mut names: Vec<String> = Vec::new();
    for name in relative.split('/') {
        match name {
            "" | "." => {}
            ".." => {
                if names.pop().is_none() {
                    return Err(fault(
                        codes::OUTSIDE_SCOPE,
                        "the path leaves its repository",
                    ));
                }
            }
            name => {
                checked_name(name)?;
                names.push(name.to_owned());
            }
        }
    }
    Ok(names)
}

/// An absolute path, lexically normalized: fully qualified for this
/// computer (on Windows a drive or a UNC share, never verbatim or a
/// device), a `..` never above its root.
fn absolute(path: &str) -> Result<PathBuf, ScriptFault> {
    let invalid = || {
        fault(
            codes::INVALID_ARGUMENT,
            "the path is not an absolute path of this computer",
        )
    };
    let p = Path::new(path);
    if !p.is_absolute() {
        return Err(invalid());
    }
    let mut out = PathBuf::new();
    let mut depth = 0usize;
    for component in p.components() {
        match component {
            Component::Prefix(prefix) => {
                use std::path::Prefix;
                match prefix.kind() {
                    Prefix::Disk(_) => {}
                    // A share needs its server and its name, and is never
                    // one of this computer's own (an administrative `C$`
                    // share or `localhost`): the same folders under
                    // another name.
                    Prefix::UNC(server, share)
                        if !server.is_empty()
                            && !share.is_empty()
                            && !is_local_share(server, share) => {}
                    _ => return Err(invalid()),
                }
                out.push(component);
            }
            Component::RootDir => out.push(component),
            Component::CurDir => {}
            Component::ParentDir => {
                if depth == 0 {
                    return Err(invalid());
                }
                out.pop();
                depth -= 1;
            }
            Component::Normal(name) => {
                let name = name.to_str().ok_or_else(invalid)?;
                checked_name(name)?;
                out.push(name);
                depth += 1;
            }
        }
    }
    Ok(out)
}

fn io_fault(e: &std::io::Error, what: &str) -> ScriptFault {
    match e.kind() {
        std::io::ErrorKind::NotFound => fault(codes::NOT_FOUND, format!("{what} does not exist")),
        _ if is_escape(e) => fault(
            codes::SYMLINK_ESCAPE,
            format!("{what} leads outside its scope through a link"),
        ),
        _ => fault(codes::HOST_ERROR, format!("{what}: {}", e.kind())),
    }
}

/// `cap-std`'s refusal of a path that resolves outside its directory.
pub(crate) fn is_escape(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::PermissionDenied
        && e.to_string().contains("outside of the filesystem")
}

/// An I/O failure as the script sees it, naming `target`.
pub(crate) fn failure(e: &std::io::Error, target: &str) -> ScriptFault {
    io_fault(e, &format!("“{target}”"))
}

/// Resolves `path` to a scope with at least `need` access; never into,
/// onto or around one of DeckPilot's private folders.
pub(crate) fn resolve(
    path: &PathArg,
    grants: &ScriptGrants,
    project: Option<&RunProject>,
    need: FsAccess,
    protected: &Protected,
) -> Result<Target, ScriptFault> {
    let scopes = scopes(grants, project);
    let (candidate, absolute_form) = match (&path.path, &path.repository) {
        (Some(path), None) => (absolute(path)?, true),
        (None, Some(key)) => {
            let repo = project
                .and_then(|p| p.repositories.iter().find(|r| r.key == *key))
                .ok_or_else(|| ScriptFault {
                    repository: Some(key.clone()),
                    ..fault(
                        codes::NOT_FOUND,
                        format!("the project has no repository “{key}”"),
                    )
                })?;
            let folder = repo.folder.as_ref().ok_or_else(|| ScriptFault {
                repository: Some(key.clone()),
                ..fault(
                    codes::NOT_FOUND,
                    format!("repository “{key}” has no folder on this computer"),
                )
            })?;
            let mut candidate = simplify(folder.clone());
            candidate.extend(relative_names(path.relative.as_deref().unwrap_or(""))?);
            (candidate, false)
        }
        _ => return Err(fault(codes::INVALID_ARGUMENT, "a path or a repository")),
    };
    let scope = scopes
        .iter()
        .filter(|s| !(absolute_form && s.repository && !grants.repositories_paths))
        .filter(|s| starts_with(&candidate, &s.root))
        .max_by_key(|s| s.root.components().count())
        .ok_or_else(|| {
            let hint = if absolute_form && !grants.repositories_paths {
                " (an absolute path inside a repository needs repositories.paths; use repo.join(...))"
            } else {
                ""
            };
            fault(
                codes::OUTSIDE_SCOPE,
                format!("the path is outside the script's file system scopes{hint}"),
            )
        })?
        .clone();
    if scope.access < need {
        let what = match need {
            FsAccess::Read => "read",
            FsAccess::Write => "write",
            FsAccess::Delete => "delete",
        };
        return Err(fault(
            codes::NOT_GRANTED,
            format!("the script may not {what} in “{}”", scope.label),
        ));
    }
    let private = || {
        fault(
            codes::PROTECTED_PATH,
            "DeckPilot's own folders are never read or changed by scripts",
        )
    };
    if protected.forbids(&candidate) || protected.forbids_identity(&candidate) {
        return Err(private());
    }
    // Where a link inside the scope leads (a read may follow one).
    if canonical(&candidate).is_ok_and(|real| protected.forbids(&real)) {
        return Err(private());
    }
    let rel: Vec<String> = candidate
        .components()
        .skip(scope.root.components().count())
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let root = open_root(&scope)?;
    // The open root, and every folder above it, by identity.
    if protected.forbids_above(&root, &scope.root) {
        return Err(private());
    }
    Ok(Target { scope, rel, root })
}

/// Opens a scope's root and checks it is still the recorded folder.
pub(crate) fn open_root(scope: &Scope) -> Result<Dir, ScriptFault> {
    let label = &scope.label;
    let dir = Dir::open_ambient_dir(&scope.root, ambient_authority())
        .map_err(|e| io_fault(&e, &format!("the folder of “{label}”")))?;
    let moved = || {
        fault(
            codes::OUTSIDE_SCOPE,
            format!("the folder of “{label}” moved or was replaced since it was granted"),
        )
    };
    let canonical = canonical(&scope.root).map_err(|_| moved())?;
    if !same_path(&canonical, &scope.root) {
        return Err(moved());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as Std;
        let opened = dir.dir_metadata().map_err(|_| moved())?;
        let there = std::fs::metadata(&canonical).map_err(|_| moved())?;
        if cap_fs_ext::MetadataExt::dev(&opened) != Std::dev(&there)
            || cap_fs_ext::MetadataExt::ino(&opened) != Std::ino(&there)
        {
            return Err(moved());
        }
    }
    Ok(dir)
}

#[cfg(test)]
pub(crate) mod lexical_tests {
    use super::*;

    #[test]
    fn relative_parts_never_leave_their_root() {
        assert_eq!(relative_names("a/./b/../c").unwrap(), ["a", "c"]);
        assert_eq!(relative_names("").unwrap(), Vec::<String>::new());
        assert!(relative_names("..").is_err());
        assert!(relative_names("a/../..").is_err());
        assert!(relative_names("a:b").is_err());
        assert!(relative_names("a\\b").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn absolute_paths_are_normalized_and_never_rise_above_the_root() {
        assert_eq!(absolute("/a/./b/../c").unwrap(), Path::new("/a/c"));
        assert!(absolute("/..").is_err());
        assert!(absolute("/a/../..").is_err());
        assert!(absolute("relative").is_err());
        assert!(absolute("C:\\x").is_err(), "not a path of this computer");
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths_are_drives_or_shares_only() {
        assert!(absolute("C:\\work\\cms").is_ok());
        assert!(absolute("\\\\server\\share\\x").is_ok());
        assert!(absolute("\\x").is_err(), "drive-relative");
        assert!(absolute("\\\\?\\C:\\x").is_err());
        assert!(absolute("\\\\.\\pipe\\x").is_err());
        assert!(absolute("C:\\work\\CON").is_err());
        assert!(absolute("C:\\work\\a.").is_err());
    }

    #[test]
    fn short_names_are_known() {
        for name in ["GIT~1", "LONGRE~2", "a~9b"] {
            assert!(is_short_name(name), "{name}");
        }
        for name in [".git", "a~b", "~", "x~"] {
            assert!(!is_short_name(name), "{name}");
        }
    }

    #[test]
    fn device_names_are_known() {
        for name in ["CON", "con.txt", "NUL", "COM1", "lpt9.log", "AUX "] {
            assert!(is_device_name(name), "{name}");
        }
        for name in ["CONSOLE", "COM", "COM10", "a"] {
            assert!(!is_device_name(name), "{name}");
        }
    }
}
