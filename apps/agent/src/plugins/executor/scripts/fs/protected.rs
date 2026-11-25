//! Protected roots (design §5.3 item 6), whatever the grants:
//!
//! * **Protected** ([`Protected::guards`]): what no change may remove,
//!   empty or replace as a whole. A target that **is or contains** one is
//!   refused for every change: the file system root and drive roots, the
//!   user's home and the folders of users, the system's folders.
//! * **Private** ([`Protected::forbids`]): what no operation may touch at
//!   all, read or write: DeckPilot's own data (its database, its IPC
//!   token), logs, configuration and caches, and the app. A target that is
//!   inside one, is one or contains one is refused (security review of
//!   #469, H1).
//!
//! Compared on canonical paths (and their lexical form), in one form on
//! Windows ([`simplify`]: no verbatim prefix), so a link to the home folder
//! is caught.

use std::path::{Component, Path, PathBuf};

/// The protected roots of this computer.
#[derive(Debug, Clone, Default)]
pub(crate) struct Protected {
    roots: Vec<PathBuf>,
    private: Vec<PathBuf>,
    /// The file identities of the private folders, and of every folder
    /// holding one: a folder reached under another name (a firmlink such
    /// as `/System/Volumes/Data`, a bind mount, a share of this computer)
    /// is still recognized (re-check of #469).
    private_ids: Vec<FileId>,
    holder_ids: Vec<FileId>,
}

/// A folder's identity: its volume (`dev`, the volume serial on Windows)
/// and its file number (`ino`, the file index on Windows), from an open
/// handle.
pub(crate) type FileId = (u64, u64);

/// The identity of the folder at `path` (following links), if it is one.
pub(crate) fn folder_id(path: &Path) -> Option<FileId> {
    let dir = cap_std::fs::Dir::open_ambient_dir(path, cap_std::ambient_authority()).ok()?;
    let meta = dir.dir_metadata().ok()?;
    let id = (
        cap_fs_ext::MetadataExt::dev(&meta),
        cap_fs_ext::MetadataExt::ino(&meta),
    );
    // A file system that has no file numbers says nothing.
    (id != (0, 0)).then_some(id)
}

/// `path` in the form every comparison here uses: on Windows without the
/// verbatim prefix `canonicalize` adds (`\\?\C:\x` is `C:\x`,
/// `\\?\UNC\s\h` is `\\s\h`); unchanged elsewhere.
pub(crate) fn simplify(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        let text = path.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = text.strip_prefix(r"\\?\")
            && rest.as_bytes().get(1) == Some(&b':')
        {
            return PathBuf::from(rest);
        }
    }
    path
}

/// `std::fs::canonicalize` in the form of [`simplify`].
pub(crate) fn canonical(path: &Path) -> std::io::Result<PathBuf> {
    std::fs::canonicalize(path).map(simplify)
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
pub(crate) fn starts_with(path: &Path, prefix: &Path) -> bool {
    let mut path = path.components();
    for p in prefix.components() {
        match path.next() {
            Some(c) if same_component(c, p) => {}
            _ => return false,
        }
    }
    true
}

pub(crate) fn same_path(a: &Path, b: &Path) -> bool {
    starts_with(a, b) && starts_with(b, a)
}

impl Protected {
    /// This computer's: fixed system folders, the home folder, DeckPilot's
    /// data and the running app.
    #[expect(
        clippy::too_many_lines,
        reason = "one list of folders per platform, read top to bottom"
    )]
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
            "/run",
            "/snap",
            "/nix",
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
        #[cfg(windows)]
        if let Some(drive) = std::env::var_os("SystemDrive") {
            let drive = PathBuf::from(drive);
            roots.push(drive.join("\\$Recycle.Bin"));
            roots.push(drive.join("\\System Volume Information"));
        }
        let home = std::env::home_dir();
        roots.extend(home.clone());
        let mut private: Vec<PathBuf> = Vec::new();
        for paths in [
            deckpilot_ipc::paths::AgentPaths::resolve(),
            deckpilot_ipc::paths::AgentPaths::resolve_default(),
        ]
        .into_iter()
        .flatten()
        {
            private.push(paths.data_dir);
            private.push(paths.log_dir);
        }
        let id = deckpilot_ipc::paths::APP_IDENTIFIER;
        if let Some(home) = &home {
            for base in [
                "Library/Application Support",
                "Library/Caches",
                "Library/Preferences",
                "Library/Logs",
                "Library/WebKit",
                ".config",
                ".cache",
                ".local/share",
                ".local/state",
            ] {
                private.push(home.join(base).join(id));
            }
        }
        for var in [
            "APPDATA",
            "LOCALAPPDATA",
            "XDG_CONFIG_HOME",
            "XDG_CACHE_HOME",
            "XDG_DATA_HOME",
        ] {
            if let Some(base) = std::env::var_os(var) {
                private.push(PathBuf::from(base).join(id));
            }
        }
        if let Ok(exe) = std::env::current_exe() {
            // The app bundle (`…/DeckPilot.app`), or the folder of the
            // executable.
            let bundle = exe
                .ancestors()
                .find(|a| a.extension().is_some_and(|e| e.eq_ignore_ascii_case("app")))
                .map(Path::to_path_buf)
                .or_else(|| exe.parent().map(Path::to_path_buf));
            private.extend(bundle);
        }
        Self::with_roots(roots).with_private(private)
    }

    /// The same, with `private` folders no operation touches.
    #[must_use]
    pub fn with_private(mut self, private: impl IntoIterator<Item = PathBuf>) -> Self {
        for root in private {
            if let Ok(real) = canonical(&root) {
                if let Some(id) = folder_id(&real) {
                    self.private_ids.push(id);
                    self.holder_ids
                        .extend(real.ancestors().skip(1).filter_map(folder_id));
                }
                self.private.push(real);
            }
            self.private.push(simplify(root));
        }
        self
    }

    /// [`Protected::forbids`] by identity: `target` (or, when it does not
    /// exist, the nearest folder above it that does), or a folder above
    /// it, is a private folder; or `target` itself holds one.
    pub fn forbids_identity(&self, target: &Path) -> bool {
        if self.private_ids.is_empty() {
            return false;
        }
        let mut path = target.to_path_buf();
        let mut exists = true;
        let real = loop {
            if let Ok(real) = canonical(&path) {
                break real;
            }
            exists = false;
            if !path.pop() {
                return false;
            }
        };
        if exists && folder_id(&real).is_some_and(|id| self.holder_ids.contains(&id)) {
            return true;
        }
        real.ancestors()
            .filter_map(folder_id)
            .any(|id| self.private_ids.contains(&id))
    }

    /// Whether any operation on `target` (or what it holds) would touch a
    /// private folder: `target` is inside one, is one or contains one.
    pub fn forbids(&self, target: &Path) -> bool {
        self.private
            .iter()
            .any(|p| starts_with(target, p) || starts_with(p, target))
    }

    /// The given roots (and their canonical forms).
    pub fn with_roots(roots: impl IntoIterator<Item = PathBuf>) -> Self {
        let mut all = Vec::new();
        for root in roots {
            if let Ok(real) = canonical(&root) {
                all.push(real);
            }
            all.push(simplify(root));
        }
        Self {
            roots: all,
            ..Self::default()
        }
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

    #[test]
    fn a_private_folder_is_untouchable_inside_and_out() {
        let p = Protected::default().with_private([PathBuf::from("/srv/data/deckpilot")]);
        assert!(p.forbids(Path::new("/srv/data/deckpilot")));
        assert!(p.forbids(Path::new("/srv/data/deckpilot/agent.token")));
        assert!(p.forbids(Path::new("/srv/data")), "holds it");
        assert!(!p.forbids(Path::new("/srv/data/other")));
        let system = Protected::system();
        if let Ok(paths) = deckpilot_ipc::paths::AgentPaths::resolve() {
            assert!(system.forbids(&paths.token_file));
            assert!(system.forbids(&paths.database));
        }
    }
}
