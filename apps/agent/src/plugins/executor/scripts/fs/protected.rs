//! Protected roots (design §5.3 item 6): what no script operation may
//! change as a whole, whatever its grants. A target that **is or contains**
//! one is refused for every change: the file system root and drive roots,
//! the user's home and the folders of users, the system's folders,
//! DeckPilot's own data and the app bundle. Compared on canonical paths
//! (and their lexical form), so a link to the home folder is caught.

use std::path::{Component, Path, PathBuf};

/// The protected roots of this computer.
#[derive(Debug, Clone, Default)]
pub(crate) struct Protected {
    roots: Vec<PathBuf>,
}

/// Whether names compare without case here (the default volumes of macOS
/// and Windows).
pub(super) const FOLD_CASE: bool = cfg!(any(target_os = "macos", windows));

fn same_name(a: &std::ffi::OsStr, b: &std::ffi::OsStr) -> bool {
    if FOLD_CASE {
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    } else {
        a == b
    }
}

fn same_component(a: Component<'_>, b: Component<'_>) -> bool {
    match (a, b) {
        (Component::Normal(x), Component::Normal(y)) => same_name(x, y),
        (Component::Prefix(x), Component::Prefix(y)) => same_name(x.as_os_str(), y.as_os_str()),
        (x, y) => x == y,
    }
}

/// Whether `path` starts with `prefix`, component by component (without
/// case where names compare without case).
pub(super) fn starts_with(path: &Path, prefix: &Path) -> bool {
    let mut path = path.components();
    for p in prefix.components() {
        match path.next() {
            Some(c) if same_component(c, p) => {}
            _ => return false,
        }
    }
    true
}

pub(super) fn same_path(a: &Path, b: &Path) -> bool {
    starts_with(a, b) && starts_with(b, a)
}

impl Protected {
    /// This computer's: fixed system folders, the home folder, DeckPilot's
    /// data and the running app.
    pub fn system() -> Self {
        let mut roots: Vec<PathBuf> = Vec::new();
        #[cfg(unix)]
        for fixed in [
            "/",
            "/Users",
            "/home",
            "/root",
            "/System",
            "/Library",
            "/Applications",
            "/Volumes",
            "/usr",
            "/bin",
            "/sbin",
            "/etc",
            "/var",
            "/opt",
            "/tmp",
            "/private",
            "/private/etc",
            "/private/var",
            "/private/tmp",
            "/dev",
            "/proc",
            "/sys",
            "/boot",
            "/lib",
            "/lib64",
            "/mnt",
            "/media",
            "/srv",
        ] {
            roots.push(PathBuf::from(fixed));
        }
        #[cfg(windows)]
        for var in [
            "SystemRoot",
            "windir",
            "ProgramFiles",
            "ProgramFiles(x86)",
            "ProgramW6432",
            "ProgramData",
            "APPDATA",
            "LOCALAPPDATA",
            "USERPROFILE",
            "PUBLIC",
        ] {
            if let Some(value) = std::env::var_os(var) {
                roots.push(PathBuf::from(value));
            }
        }
        if let Some(home) = std::env::home_dir() {
            roots.push(home);
        }
        if let Ok(paths) = deckpilot_ipc::paths::AgentPaths::resolve() {
            roots.push(paths.data_dir);
            roots.push(paths.log_dir);
        }
        if let Ok(exe) = std::env::current_exe() {
            // The app bundle (`…/DeckPilot.app`), or the folder of the
            // executable.
            let bundle = exe
                .ancestors()
                .find(|a| a.extension().is_some_and(|e| e.eq_ignore_ascii_case("app")))
                .map(Path::to_path_buf)
                .or_else(|| exe.parent().map(Path::to_path_buf));
            roots.extend(bundle);
        }
        Self::with_roots(roots)
    }

    /// The given roots (and their canonical forms).
    pub fn with_roots(roots: impl IntoIterator<Item = PathBuf>) -> Self {
        let mut all = Vec::new();
        for root in roots {
            if let Ok(canonical) = std::fs::canonicalize(&root) {
                all.push(canonical);
            }
            all.push(root);
        }
        Self { roots: all }
    }

    /// Whether changing `target` as a whole (removing it, emptying it,
    /// replacing it) would touch a protected root: `target` is one, or
    /// contains one, or is a file system or drive root.
    pub fn guards(&self, target: &Path) -> bool {
        let is_root = target
            .components()
            .all(|c| matches!(c, Component::RootDir | Component::Prefix(_)));
        is_root || self.roots.iter().any(|root| starts_with(root, target))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_set_holds_the_home_folder_and_the_root() {
        let p = Protected::system();
        if let Some(home) = std::env::home_dir() {
            assert!(p.guards(&home));
            // Its parent contains it.
            if let Some(parent) = home.parent() {
                assert!(p.guards(parent));
            }
        }
        #[cfg(unix)]
        {
            assert!(p.guards(Path::new("/")));
            assert!(p.guards(Path::new("/usr")));
            assert!(p.guards(Path::new("/etc")));
        }
    }

    #[test]
    fn a_target_inside_a_root_is_not_the_root() {
        let p = Protected::with_roots([PathBuf::from("/srv/home/me")]);
        assert!(p.guards(Path::new("/srv/home/me")));
        assert!(p.guards(Path::new("/srv/home")));
        assert!(p.guards(Path::new("/")));
        assert!(!p.guards(Path::new("/srv/home/me/project")));
        assert!(!p.guards(Path::new("/srv/other")));
        if FOLD_CASE {
            assert!(p.guards(Path::new("/SRV/Home/ME")));
        }
    }
}
