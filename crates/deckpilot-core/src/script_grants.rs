//! What one script may do on this computer (`design-projects-scripts.md`
//! §5.1 layer 2): its machine-local grants, edited on the script's
//! Permissions tab and stored beside its trust record. They are **never**
//! taken from the script's source, an import or the Marketplace, never
//! exported, and read again at every bridge call, so a revocation applies
//! to the next call (`docs/scripts-bridge.md`).
//!
//! A new script has none ([`ScriptGrants::default`]): the Scripts plugin's
//! `scripts` permission is only the ceiling.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Most file system scopes of one script.
pub const MAX_FS_SCOPES: usize = 32;
/// Most commands of one script.
pub const MAX_PROCESS_GRANTS: usize = 32;
/// Longest directory scope label.
pub const MAX_LABEL: usize = 64;
/// The run time of a script: default and maximum, in milliseconds.
pub const DEFAULT_TIMEOUT_MS: u64 = 60_000;
pub const MAX_TIMEOUT_MS: u64 = 600_000;
/// Memory of a script's runtime: default and maximum, in MiB.
pub const DEFAULT_MEMORY_MIB: u32 = 64;
pub const MAX_MEMORY_MIB: u32 = 128;
/// Output of one process, per stream: default and maximum, in KiB.
pub const DEFAULT_MAX_OUTPUT_KIB: u32 = 256;
pub const MAX_MAX_OUTPUT_KIB: u32 = 4_096;

/// One script's grants.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScriptGrants {
    /// `projects.read` (low): projects, repository keys and names.
    #[serde(default)]
    pub projects_read: bool,
    /// `repositories.paths` (medium): repositories' folders as text, and
    /// absolute paths inside repository scopes.
    #[serde(default)]
    pub repositories_paths: bool,
    /// File system scopes.
    #[serde(default)]
    pub fs: Vec<FsScope>,
    /// Commands `process.run` may start.
    #[serde(default)]
    pub process: Vec<ProcessGrant>,
    /// Shells `shell.run` may start (high, off by default).
    #[serde(default)]
    pub shell: Option<ShellGrant>,
    /// What a destructive operation needs.
    #[serde(default)]
    pub destructive: Destructive,
    /// The script's limits.
    #[serde(default)]
    pub limits: ScriptLimits,
}

/// A file system scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FsScope {
    pub root: FsRoot,
    pub access: FsAccess,
    /// Whether the root itself may be removed, emptied or overwritten.
    #[serde(default)]
    pub root_destructive: bool,
}

/// Where a scope starts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum FsRoot {
    /// A repository of the run's current project by key, or `"*"` for
    /// every repository of it.
    Repository(String),
    /// A folder the user picked with the native folder picker: its
    /// canonical path when it was granted, and a label shown instead of
    /// the path (to scripts without `repositories.paths`, in logs).
    #[serde(rename_all = "camelCase")]
    Directory { path: PathBuf, label: String },
}

/// What a scope allows, each level including the ones before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FsAccess {
    Read,
    Write,
    Delete,
}

/// A command `process.run` may start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProcessGrant {
    /// `git`, another bare name, or an absolute path picked with the native
    /// file picker. Never a container runtime ([`CONTAINER_RUNTIMES`]).
    pub command: String,
    /// Where it may run.
    #[serde(default)]
    pub cwd: ProcessCwd,
    /// Whether the script may set environment variables (never the
    /// denied ones).
    #[serde(default)]
    pub env: ProcessEnv,
    /// For `git`: whether the options the tools runner refuses
    /// (`-c`, `--upload-pack`, …) are allowed (OD-8, advanced).
    #[serde(default)]
    pub unrestricted_args: bool,
}

/// Where a granted command may run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProcessCwd {
    /// Inside a repository of the run's project.
    #[default]
    Repositories,
    /// Inside one of the script's file system scopes.
    FsScopes,
    /// Anywhere the script can name (still a directory that exists).
    Any,
}

/// Whether a granted command takes the script's environment entries.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProcessEnv {
    #[default]
    None,
    Allowed,
}

/// The shells `shell.run` may start.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShellGrant {
    pub shells: Vec<String>,
}

/// What a destructive file system operation needs (OD-9).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Destructive {
    /// The user confirms the agent's plan, in a run the user started; a
    /// run nobody started is refused.
    #[default]
    Ask,
    /// No confirmation, in any run.
    Allow,
}

/// A script's limits, each within its maximum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScriptLimits {
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
    #[serde(default = "default_memory", rename = "memoryMiB")]
    pub memory_mib: u32,
    #[serde(default = "default_output", rename = "maxOutputKiB")]
    pub max_output_kib: u32,
}

fn default_timeout() -> u64 {
    DEFAULT_TIMEOUT_MS
}
fn default_memory() -> u32 {
    DEFAULT_MEMORY_MIB
}
fn default_output() -> u32 {
    DEFAULT_MAX_OUTPUT_KIB
}

impl Default for ScriptLimits {
    fn default() -> Self {
        Self {
            timeout_ms: DEFAULT_TIMEOUT_MS,
            memory_mib: DEFAULT_MEMORY_MIB,
            max_output_kib: DEFAULT_MAX_OUTPUT_KIB,
        }
    }
}

impl ScriptLimits {
    /// The same limits within their bounds (a stored value out of range
    /// can only make them stricter, never looser than the maximum).
    #[must_use]
    pub fn clamped(self) -> Self {
        Self {
            timeout_ms: self.timeout_ms.clamp(1, MAX_TIMEOUT_MS),
            memory_mib: self.memory_mib.clamp(16, MAX_MEMORY_MIB),
            max_output_kib: self.max_output_kib.clamp(1, MAX_MAX_OUTPUT_KIB),
        }
    }
}

/// Why a stored grant record is refused as a whole.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GrantsError {
    #[error("too many file system scopes")]
    TooManyScopes,
    #[error("too many commands")]
    TooManyCommands,
    #[error("a repository scope names no repository key")]
    RepositoryKey,
    #[error("a directory scope is not an absolute path")]
    DirectoryPath,
    #[error("a directory scope's label is empty or too long")]
    Label,
    #[error("a command is neither a bare name nor an absolute path")]
    Command,
    #[error("an unknown shell")]
    Shell,
    /// Container runtimes are not grantable until the owner decides how
    /// (they are effectively root on most machines).
    #[error("container runtimes are not available to scripts yet")]
    ContainerRuntime,
    #[error("a script cannot run a program as another user")]
    Elevation,
    #[error("a shell runs only through the shell grant")]
    ShellCommand,
}

/// Container runtimes, virtual machines and namespace tools: never
/// grantable to a script in this version (the owner's open decision; the
/// safest default). They are effectively root on most machines.
pub const CONTAINER_RUNTIMES: &[&str] = &[
    "docker",
    "dockerd",
    "docker-compose",
    "podman",
    "podman-remote",
    "nerdctl",
    "colima",
    "lima",
    "limactl",
    "containerd",
    "ctr",
    "crictl",
    "runc",
    "crun",
    "buildah",
    "finch",
    "rancher-desktop",
    "rdctl",
    "orb",
    "orbctl",
    "container",
    "wsl",
    "lxc",
    "lxd",
    "incus",
    "multipass",
    "nsenter",
    "unshare",
    "chroot",
    "systemd-nspawn",
    "lxc-attach",
    "lxc-execute",
    "machinectl",
    // Virtual machines.
    "virsh",
    "virt-install",
    "vboxmanage",
    "prlctl",
    "vmrun",
    "utmctl",
    "vagrant",
    "tart",
    "hyperkit",
    "krunvm",
    "vboxheadless",
    "virtualboxvm",
    "prlsrvctl",
    "kvm",
    "apptainer",
    "singularity",
    "distrobox",
    "toolbox",
    "kind",
    "minikube",
    "k3d",
    "microk8s",
    "bwrap",
    "firejail",
    "flatpak-spawn",
];

/// Programs that act as another user: never grantable.
pub const ELEVATION: &[&str] = &[
    "sudo",
    "sudoedit",
    "doas",
    "pkexec",
    "su",
    "runuser",
    "run0",
    "systemd-run",
    "runas",
    "gsudo",
    "osascript",
    "launchctl",
    "sudo-rs",
    "pfexec",
];

/// Shells: only through `shell.run` and its own grant, never as a
/// process command.
pub const PROCESS_SHELLS: &[&str] = &[
    "sh",
    "bash",
    "zsh",
    "fish",
    "dash",
    "ksh",
    "mksh",
    "csh",
    "tcsh",
    "nu",
    "elvish",
    "xonsh",
    "pwsh",
    "powershell",
    "cmd",
    // A shell among their applets.
    "busybox",
    "toybox",
    "ash",
    "rbash",
    "yash",
];

/// Programs that run any other program (interpreters, launchers, build
/// tools, package runners): grantable, but each such grant is as broad as
/// a shell, a container runtime included, and is shown as such.
pub const RUNS_ANY_PROGRAM: &[&str] = &[
    "env",
    "xargs",
    "nohup",
    "timeout",
    "nice",
    "time",
    "watch",
    "find",
    "parallel",
    "python",
    "python3",
    "node",
    "deno",
    "bun",
    "ruby",
    "perl",
    "php",
    "lua",
    "tclsh",
    "java",
    "dotnet",
    "make",
    "cmake",
    "ninja",
    "npm",
    "npx",
    "pnpm",
    "yarn",
    "pip",
    "pip3",
    "uv",
    "pipx",
    "cargo",
    "go",
    "gradle",
    "mvn",
    "open",
    "start",
    "explorer",
    "rundll32",
    "mshta",
    "wscript",
    "cscript",
    "ssh",
    "curl",
    "wget",
    "arch",
    "xcrun",
    "setsid",
    "stdbuf",
    "taskset",
    "chrt",
    "ionice",
    "caffeinate",
    "nix-shell",
    "just",
];

/// What a granted command is, by its file name (without case and a
/// program extension).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CommandClass {
    /// [`CONTAINER_RUNTIMES`]: not grantable.
    ContainerRuntime,
    /// [`ELEVATION`]: not grantable.
    Elevation,
    /// [`PROCESS_SHELLS`]: only through the shell grant.
    Shell,
    /// [`RUNS_ANY_PROGRAM`]: grantable, as broad as a shell.
    RunsAnyProgram,
    /// Anything else.
    Plain,
}

impl CommandClass {
    /// Whether a process grant may name it.
    pub fn grantable(self) -> bool {
        matches!(self, Self::RunsAnyProgram | Self::Plain)
    }
}

/// The file name `command` (a bare name or a path) stands for, lowercase,
/// without a program extension.
fn program_name(command: &str) -> String {
    let name = command
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(command)
        .to_lowercase();
    for ext in [".exe", ".com", ".cmd", ".bat", ".ps1", ".sh", ".app"] {
        if let Some(stem) = name.strip_suffix(ext) {
            return stem.to_owned();
        }
    }
    name
}

/// The [`CommandClass`] of `command`. `python3.12`, `node18` and the like
/// count as their interpreter; `docker-*` and `podman-*` as theirs.
pub fn command_class(command: &str) -> CommandClass {
    let name = program_name(command);
    let base = name.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.' || c == '-');
    let is = |list: &[&str]| list.contains(&name.as_str()) || list.contains(&base);
    // Their helpers (`docker-compose`, `dockerd-rootless.sh`,
    // `containerd-shim-runc-v2`, `qemu-img`, `lxc-attach`) and Lima's
    // wrappers (`nerdctl.lima`).
    let helper = [
        "docker-",
        "dockerd-",
        "containerd-",
        "podman-",
        "lxc-",
        "qemu-",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix))
        || std::path::Path::new(&name)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("lima"));
    if is(CONTAINER_RUNTIMES) || helper || name == "qemu" {
        CommandClass::ContainerRuntime
    } else if is(ELEVATION) {
        CommandClass::Elevation
    } else if is(PROCESS_SHELLS) {
        CommandClass::Shell
    } else if is(RUNS_ANY_PROGRAM) {
        CommandClass::RunsAnyProgram
    } else {
        CommandClass::Plain
    }
}

/// Whether `command` names a container runtime ([`CONTAINER_RUNTIMES`]).
pub fn is_container_runtime(command: &str) -> bool {
    command_class(command) == CommandClass::ContainerRuntime
}

/// The shells a grant may name.
pub const SHELLS: &[&str] = &["sh", "bash", "zsh", "pwsh", "powershell", "cmd"];

impl ScriptGrants {
    /// Checks a stored record; a record that fails is treated as no
    /// grants at all (never a partial grant).
    pub fn validate(&self) -> Result<(), GrantsError> {
        if self.fs.len() > MAX_FS_SCOPES {
            return Err(GrantsError::TooManyScopes);
        }
        if self.process.len() > MAX_PROCESS_GRANTS {
            return Err(GrantsError::TooManyCommands);
        }
        for scope in &self.fs {
            match &scope.root {
                FsRoot::Repository(key) => {
                    if key != "*" && !is_repository_key(key) {
                        return Err(GrantsError::RepositoryKey);
                    }
                }
                FsRoot::Directory { path, label } => {
                    // Absolute and canonical in form: no `.` or `..`.
                    if !path.is_absolute()
                        || path.as_os_str().is_empty()
                        // By its text: `components` hides a `.` inside.
                        || path
                            .to_string_lossy()
                            .split(['/', '\\'])
                            .any(|part| part == "." || part == "..")
                    {
                        return Err(GrantsError::DirectoryPath);
                    }
                    if label.trim().is_empty()
                        || label.chars().count() > MAX_LABEL
                        || label
                            .chars()
                            .any(|c| c.is_control() || crate::script::is_hidden(c))
                        || path.to_string_lossy().chars().any(crate::script::is_hidden)
                    {
                        return Err(GrantsError::Label);
                    }
                }
            }
        }
        for grant in &self.process {
            let c = &grant.command;
            let bare = !c.is_empty()
                && c.len() <= 255
                && !c.starts_with('-')
                && !c.contains(['/', '\\', '\0'])
                && !c.chars().any(|ch| ch.is_whitespace() || ch.is_control());
            match command_class(c) {
                CommandClass::ContainerRuntime => return Err(GrantsError::ContainerRuntime),
                CommandClass::Elevation => return Err(GrantsError::Elevation),
                CommandClass::Shell => return Err(GrantsError::ShellCommand),
                CommandClass::RunsAnyProgram | CommandClass::Plain => {}
            }
            // A bare name is ASCII: no look-alike of a refused one; no
            // command hides characters from the review.
            if (bare && !c.is_ascii())
                || c.chars()
                    .any(|ch| ch.is_control() || crate::script::is_hidden(ch))
            {
                return Err(GrantsError::Command);
            }
            if !bare && !std::path::Path::new(c).is_absolute() {
                return Err(GrantsError::Command);
            }
            // `unrestrictedArgs` lifts git's limits, so it is for git only.
            if grant.unrestricted_args && program_name(c) != "git" {
                return Err(GrantsError::Command);
            }
        }
        // Without case: `make` and `Make` are one program where names
        // compare so, and never meant as two grants elsewhere.
        let mut commands: Vec<String> = self
            .process
            .iter()
            .map(|g| {
                if cfg!(any(target_os = "macos", windows)) {
                    g.command.to_lowercase()
                } else {
                    g.command.clone()
                }
            })
            .collect();
        commands.sort_unstable();
        if commands.windows(2).any(|w| w[0] == w[1]) {
            return Err(GrantsError::Command);
        }
        if let Some(shell) = &self.shell
            && shell.shells.iter().any(|s| !SHELLS.contains(&s.as_str()))
        {
            return Err(GrantsError::Shell);
        }
        Ok(())
    }
}

/// The repository key grammar (`^[a-z][a-z0-9_]{0,31}$`).
fn is_repository_key(key: &str) -> bool {
    let b = key.as_bytes();
    !b.is_empty()
        && b.len() <= 32
        && b[0].is_ascii_lowercase()
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_script_has_no_grants() {
        let g = ScriptGrants::default();
        assert!(!g.projects_read && !g.repositories_paths);
        assert!(g.fs.is_empty() && g.process.is_empty() && g.shell.is_none());
        assert_eq!(g.destructive, Destructive::Ask);
        assert_eq!(serde_json::from_str::<ScriptGrants>("{}").unwrap(), g);
    }

    #[test]
    fn the_record_round_trips_and_is_strict() {
        // An absolute path of this platform.
        let dir = if cfg!(windows) {
            r"C:\\work\\out"
        } else {
            "/work/out"
        };
        let json = r#"{"projectsRead":true,"fs":[{"root":{"repository":"cms"},"access":"delete"},{"root":{"directory":{"path":"DIR","label":"out"}},"access":"read","rootDestructive":true}],"process":[{"command":"git","cwd":"repositories","env":"none"}],"shell":{"shells":["sh"]},"destructive":"allow","limits":{"timeoutMs":1000,"memoryMiB":32,"maxOutputKiB":64}}"#
            .replace("DIR", dir);
        let g: ScriptGrants = serde_json::from_str(&json).unwrap();
        g.validate().unwrap();
        assert_eq!(g.fs[0].access, FsAccess::Delete);
        assert!(FsAccess::Delete > FsAccess::Write && FsAccess::Write > FsAccess::Read);
        let back: ScriptGrants = serde_json::from_str(&serde_json::to_string(&g).unwrap()).unwrap();
        assert_eq!(back, g);
        assert!(serde_json::from_str::<ScriptGrants>(r#"{"everything":true}"#).is_err());
    }

    #[test]
    fn bad_records_are_refused_whole() {
        let bad = |g: ScriptGrants| g.validate().is_err();
        let scope = |root| FsScope {
            root,
            access: FsAccess::Read,
            root_destructive: false,
        };
        assert!(bad(ScriptGrants {
            fs: vec![scope(FsRoot::Repository("../x".into()))],
            ..ScriptGrants::default()
        }));
        assert!(bad(ScriptGrants {
            fs: vec![scope(FsRoot::Directory {
                path: "relative/dir".into(),
                label: "x".into()
            })],
            ..ScriptGrants::default()
        }));
        for command in ["git pull", "./x", "-c", ""] {
            assert!(
                bad(ScriptGrants {
                    process: vec![ProcessGrant {
                        command: command.into(),
                        cwd: ProcessCwd::Any,
                        env: ProcessEnv::None,
                        unrestricted_args: false,
                    }],
                    ..ScriptGrants::default()
                }),
                "{command}"
            );
        }
        assert!(bad(ScriptGrants {
            shell: Some(ShellGrant {
                shells: vec!["fish".into()]
            }),
            ..ScriptGrants::default()
        }));
    }

    #[test]
    fn container_runtimes_are_never_grantable() {
        for command in [
            "docker",
            "Docker.exe",
            "podman",
            "nerdctl",
            "colima",
            "lima",
            "limactl",
            "docker-compose",
            "docker-buildx",
            "/usr/local/bin/docker",
            "C:\\Program Files\\Docker\\docker.exe",
            "/opt/homebrew/bin/podman-remote",
        ] {
            assert!(is_container_runtime(command), "{command}");
            let grants = ScriptGrants {
                process: vec![ProcessGrant {
                    command: command.into(),
                    cwd: ProcessCwd::Repositories,
                    env: ProcessEnv::None,
                    unrestricted_args: false,
                }],
                ..ScriptGrants::default()
            };
            assert_eq!(
                grants.validate(),
                Err(GrantsError::ContainerRuntime),
                "{command}"
            );
        }
        for command in ["git", "npm", "dock", "/usr/bin/make", "mydocker"] {
            assert!(!is_container_runtime(command), "{command}");
        }
        for command in [
            "docker.com",
            "orbctl",
            "wsl.exe",
            "nsenter",
            "limactl",
            "incus",
            "lxc-attach",
            "lxc-execute",
            "lxc-create",
            "machinectl",
            "qemu-system-x86_64",
            "qemu-img",
            "VBoxManage",
            "virsh",
            "vagrant",
            "utmctl",
            "prlctl",
            "nerdctl.lima",
            "docker.lima",
            "dockerd-rootless.sh",
            "containerd-shim-runc-v2",
            "kvm",
            "apptainer",
            "distrobox",
            "minikube",
            "kind",
            "bwrap",
            "flatpak-spawn",
        ] {
            assert_eq!(
                command_class(command),
                CommandClass::ContainerRuntime,
                "{command}"
            );
        }
    }

    #[test]
    fn elevation_and_shells_are_never_process_grants_and_launchers_are_flagged() {
        let refused = |command: &str| {
            ScriptGrants {
                process: vec![ProcessGrant {
                    command: command.into(),
                    cwd: ProcessCwd::Repositories,
                    env: ProcessEnv::None,
                    unrestricted_args: false,
                }],
                ..ScriptGrants::default()
            }
            .validate()
        };
        for command in [
            "sudo",
            "sudoedit",
            "/usr/bin/doas",
            "pkexec",
            "su",
            "runuser",
            "run0",
            "systemd-run",
            "runas.exe",
            "osascript",
            "sudo-rs",
            "pfexec",
        ] {
            assert_eq!(command_class(command), CommandClass::Elevation, "{command}");
            assert_eq!(refused(command), Err(GrantsError::Elevation), "{command}");
        }
        for command in [
            "sh",
            "/bin/bash",
            "zsh",
            "pwsh.exe",
            "cmd.exe",
            "fish",
            "busybox",
            "ash",
            "yash",
        ] {
            assert_eq!(command_class(command), CommandClass::Shell, "{command}");
            assert_eq!(
                refused(command),
                Err(GrantsError::ShellCommand),
                "{command}"
            );
        }
        for command in ["python3.12", "node", "npx", "env", "xargs", "make", "curl"] {
            assert_eq!(
                command_class(command),
                CommandClass::RunsAnyProgram,
                "{command}"
            );
            assert_eq!(refused(command), Ok(()), "{command}");
        }
        assert_eq!(command_class("git"), CommandClass::Plain);
        // A look-alike folds to the refused name; any other bare name is
        // ASCII; no command or label hides characters.
        assert_eq!(
            refused("/usr/local/bin/ma\u{202E}ke"),
            Err(GrantsError::Command)
        );
        assert_eq!(refused("doc\u{212A}er"), Err(GrantsError::ContainerRuntime));
        assert_eq!(refused("g\u{00EF}t"), Err(GrantsError::Command));
        // A folder scope is canonical in form: no `.` or `..`.
        for path in ["/srv/work/../etc", "/srv/./work"] {
            let scope = ScriptGrants {
                fs: vec![FsScope {
                    root: FsRoot::Directory {
                        path: path.into(),
                        label: "Work".into(),
                    },
                    access: FsAccess::Read,
                    root_destructive: false,
                }],
                ..ScriptGrants::default()
            };
            assert_eq!(scope.validate(), Err(GrantsError::DirectoryPath), "{path}");
        }
        // git's unrestricted arguments are git's only; a command once.
        let with = |command: &str, unrestricted: bool| ProcessGrant {
            command: command.into(),
            cwd: ProcessCwd::Repositories,
            env: ProcessEnv::None,
            unrestricted_args: unrestricted,
        };
        let set = |process: Vec<ProcessGrant>| {
            ScriptGrants {
                process,
                ..ScriptGrants::default()
            }
            .validate()
        };
        assert_eq!(set(vec![with("/usr/bin/git", true)]), Ok(()));
        assert_eq!(set(vec![with("make", true)]), Err(GrantsError::Command));
        assert_eq!(
            set(vec![with("make", false), with("make", false)]),
            Err(GrantsError::Command)
        );
    }

    #[test]
    fn limits_are_clamped() {
        let l = ScriptLimits {
            timeout_ms: u64::MAX,
            memory_mib: 0,
            max_output_kib: u32::MAX,
        }
        .clamped();
        assert_eq!(l.timeout_ms, MAX_TIMEOUT_MS);
        assert_eq!(l.memory_mib, 16);
        assert_eq!(l.max_output_kib, MAX_MAX_OUTPUT_KIB);
    }
}
