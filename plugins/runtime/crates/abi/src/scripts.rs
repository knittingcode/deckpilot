//! The capability operations of the Scripts host bridge (plugin API 1.13,
//! `docs/scripts-bridge.md`): the requests the `deckpilot.scripts` guest
//! sends for a script it runs, and the values the agent answers with.
//!
//! Only `deckpilot.scripts`, a first-party plugin shipped inside the app,
//! is ever answered: the broker refuses these operations to every other
//! plugin, and the agent checks the plugin, its `scripts` permission (the
//! ceiling), the script's own machine-local grants (read at every call),
//! the run's origin and the limits again before anything happens. The
//! guest is never trusted: it validates every argument itself, and this
//! crate and the agent validate them again.
//!
//! The request structs mirror the guest's marshallers
//! (`deckpilot-plugins:official/scripts/src/api/*.rs`) field for field,
//! with unknown fields refused. The shared fixture
//! `tests/fixtures/script-requests.json` holds one request of every
//! operation, and both repositories' tests parse it, so the two stay in
//! lockstep.
//!
//! **Chunked I/O (decision D1).** No frame cap of ABI 1 is raised for this
//! plugin. A request is at most
//! [`MAX_HOST_CALL_REQUEST`](crate::limits::MAX_HOST_CALL_REQUEST) (64 KiB)
//! and a reply at most
//! [`MAX_HOST_CALL_REPLY`](crate::limits::MAX_HOST_CALL_REPLY) (1 MiB), so
//! a file is read [`MAX_READ_CHUNK`] bytes at a time (`fs.readFile` with
//! `offset` and `length`) and written [`MAX_WRITE_CHUNK`] bytes at a time
//! (`fs.write.open`, `fs.write.chunk`, `fs.write.commit`), and a process's
//! arguments, its stdin and a shell script are each at most
//! [`MAX_PROCESS_ARGS_TOTAL`], [`MAX_PROCESS_STDIN`] and
//! [`MAX_SHELL_SCRIPT`] bytes.
//!
//! **The continuous-CPU deadline.** The host resets its epoch deadline after
//! a host call that waits on the agent, so a long `process.run` never trips
//! it; it does **not** reset it after `script.poll` and `script.log`
//! ([`resets_deadline`]), which a busy loop may send as often as it likes.
//!
//! Added compatibly within ABI 1: a guest built before them never sends
//! them, and a host or agent that predates them refuses the unknown op.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::capability::{CapabilityRequest, LogLevel, RequestError};

/// Longest path, in bytes (an absolute path or a repository-relative part).
pub const MAX_PATH: usize = 4096;
/// Longest script id (the core's script ids are UUIDs).
pub const MAX_SCRIPT_ID: usize = 64;
/// Longest repository key (`^[a-z][a-z0-9_]{0,31}$`).
pub const MAX_REPOSITORY_KEY: usize = 32;
/// Longest project id.
pub const MAX_PROJECT_ID: usize = 64;
/// Longest log line of a script, in bytes.
pub const MAX_SCRIPT_LOG_LINE: usize = 8 << 10;
/// Longest single `script.sleep`, in milliseconds.
pub const MAX_SLEEP_MS: u64 = 600_000;
/// Longest `timeoutMs` of a process or shell run.
pub const MAX_TIMEOUT_MS: u64 = 600_000;
/// Longest confirmation token, in bytes (the agent's are 32 hex digits).
pub const MAX_TOKEN: usize = 128;
/// Longest write handle, in bytes (the agent's are 32 hex digits).
pub const MAX_WRITE_HANDLE: usize = 64;
/// Largest file a script reads or writes (`readFile`'s `maxBytes` and a
/// chunked write's total).
pub const MAX_SCRIPT_FILE: u64 = 4 << 20;
/// Bytes one `fs.readFile` returns at most (D1): their base64 is about
/// 683 KiB, within the 1 MiB reply cap.
pub const MAX_READ_CHUNK: u32 = 512 << 10;
/// Bytes of content one `fs.write.chunk` or `fs.writeFile` carries at
/// most (D1): their base64 is about 43 KiB, within the 64 KiB request cap.
pub const MAX_WRITE_CHUNK: usize = 32 << 10;
/// Most `keep` names of `fs.removeContents`.
pub const MAX_KEEP: usize = 256;
/// Longest `keep` name.
pub const MAX_KEEP_NAME: usize = 255;
/// Most arguments of `process.run`.
pub const MAX_PROCESS_ARGS: usize = 256;
/// Longest single argument of `process.run`.
pub const MAX_PROCESS_ARG: usize = 32 << 10;
/// All arguments of one `process.run`, in bytes (D1).
pub const MAX_PROCESS_ARGS_TOTAL: usize = 48 << 10;
/// stdin of one `process.run`, in bytes (D1; a larger input goes through a
/// file the script writes).
pub const MAX_PROCESS_STDIN: usize = 48 << 10;
/// A `shell.run` script, in bytes (D1).
pub const MAX_SHELL_SCRIPT: usize = 48 << 10;
/// Longest command name or absolute command path.
pub const MAX_COMMAND: usize = 4096;
/// Most environment entries of one run.
pub const MAX_ENV: usize = 64;
/// Longest environment name.
pub const MAX_ENV_NAME: usize = 128;
/// Longest environment value.
pub const MAX_ENV_VALUE: usize = 32 << 10;
/// The shells `shell.run` may name.
pub const SHELLS: &[&str] = &["sh", "bash", "zsh", "pwsh", "powershell", "cmd"];

/// Every operation of the bridge, as the guest names it. `project.list` is
/// shared with plugin API 1.9's project context.
pub const OPS: &[&str] = &[
    "script.load",
    "script.poll",
    "script.log",
    "script.sleep",
    "script.confirm",
    "project.list",
    "project.repositories",
    "fs.exists",
    "fs.stat",
    "fs.list",
    "fs.mkdir",
    "fs.readFile",
    "fs.writeFile",
    "fs.write.open",
    "fs.write.chunk",
    "fs.write.commit",
    "fs.write.abort",
    "fs.copy",
    "fs.move",
    "fs.remove",
    "fs.removeContents",
    "process.run",
    "shell.run",
    "shell.available",
];

/// Whether the host resets the guest's continuous-CPU (epoch) deadline
/// after a reply to `op`: true for every operation that waits on the
/// agent, false for `script.poll` and `script.log`, which a busy loop may
/// send at will and which must not keep it alive past `continuousMs`.
pub fn resets_deadline(op: &str) -> bool {
    !matches!(op, "script.poll" | "script.log")
}

/// Whether a reply to `op` may carry the script detail
/// ([`ScriptFault`]): the bridge's own operations only. Every other
/// guest's `CallError` refuses the unknown field.
pub fn carries_script_detail(op: &str) -> bool {
    op != "project.list" && OPS.contains(&op)
}

/// What a bridge operation needs from the `scripts` permission (the
/// ceiling) and the script's grants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScriptScope {
    /// The run itself: `script.load`, `script.poll`, `script.log`,
    /// `script.sleep`, `script.confirm`.
    Run,
    /// The active profile's projects and repositories (`projects.read`).
    Projects,
    /// The file system, in the script's granted scopes (`fs`).
    Fs,
    /// `process.run` (`process`).
    Process,
    /// `shell.run` and `shell.available` (`shell`).
    Shell,
}

impl CapabilityRequest {
    /// The bridge scope of a Scripts operation; `None` for every other
    /// operation. `project.list` is `None` here: it is also plugin API
    /// 1.9's, and the broker decides by the plugin's grants which it is.
    pub fn script_scope(&self) -> Option<ScriptScope> {
        Some(match self {
            Self::ScriptLoad(_)
            | Self::ScriptPoll(_)
            | Self::ScriptLog(_)
            | Self::ScriptSleep(_)
            | Self::ScriptConfirm(_) => ScriptScope::Run,
            Self::ProjectRepositories(_) => ScriptScope::Projects,
            Self::FsExists(_)
            | Self::FsStat(_)
            | Self::FsList(_)
            | Self::FsMkdir(_)
            | Self::FsReadFile(_)
            | Self::FsWriteFile(_)
            | Self::FsWriteOpen(_)
            | Self::FsWriteChunk(_)
            | Self::FsWriteCommit(_)
            | Self::FsWriteAbort(_)
            | Self::FsCopy(_)
            | Self::FsMove(_)
            | Self::FsRemove(_)
            | Self::FsRemoveContents(_) => ScriptScope::Fs,
            Self::ProcessRun(_) => ScriptScope::Process,
            Self::ShellRun(_) | Self::ShellAvailable(_) => ScriptScope::Shell,
            _ => return None,
        })
    }
}

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

/// A path as the bridge carries it: `{"path": "<absolute>"}` (fully
/// qualified on Windows), or `{"repository": "<key>", "relative": "a/b"}`
/// (`relative` absent or `""` is the repository's root). The agent resolves
/// both against the script's granted scopes.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathArg {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relative: Option<String>,
}

impl PathArg {
    /// An absolute path.
    pub fn absolute(path: impl Into<String>) -> Self {
        Self {
            path: Some(path.into()),
            ..Self::default()
        }
    }

    /// A path inside a repository.
    pub fn in_repository(key: impl Into<String>, relative: impl Into<String>) -> Self {
        Self {
            repository: Some(key.into()),
            relative: Some(relative.into()),
            ..Self::default()
        }
    }

    /// The canonical forms only, as the guest sends them: exactly one of
    /// an absolute path and a repository; an absolute path that is fully
    /// qualified ([`is_absolute_path`]); a repository key; and a relative
    /// part of `/`-separated names, never absolute, never `.`, `..` or
    /// empty, without `\\` or `:`. No part holds NUL, another control
    /// character or a bidirectional override. Anything else is a hostile
    /// guest. Whether a path stays inside a scope is the agent's to decide.
    pub fn validate(&self) -> Result<(), RequestError> {
        match (&self.path, &self.repository) {
            (Some(path), None) => {
                if self.relative.is_some() {
                    return Err(RequestError::Invalid("path"));
                }
                if path.len() > MAX_PATH {
                    return Err(RequestError::TooLarge("path"));
                }
                if is_absolute_path(path) {
                    Ok(())
                } else {
                    Err(RequestError::Invalid("path"))
                }
            }
            (None, Some(key)) => {
                if !is_repository_key(key) {
                    return Err(RequestError::Invalid("repository"));
                }
                let relative = self.relative.as_deref().unwrap_or("");
                if relative.len() > MAX_PATH {
                    return Err(RequestError::TooLarge("relative"));
                }
                if relative.is_empty() || is_relative_part(relative) {
                    Ok(())
                } else {
                    Err(RequestError::Invalid("relative"))
                }
            }
            _ => Err(RequestError::Invalid("path")),
        }
    }
}

/// A character no path or command may hold: a control character or a
/// bidirectional formatting character (which could make a path in a
/// confirmation read as another).
fn is_hidden_char(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{061c}'
                | '\u{200b}'..='\u{200f}'
                | '\u{2028}'..='\u{202e}'
                | '\u{2066}'..='\u{2069}'
                | '\u{feff}'
        )
}

/// One name of a path: not empty, `.` or `..`, no hidden character.
fn is_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.chars().any(is_hidden_char)
}

/// A repository-relative part as the guest normalizes it: `a/b/c`.
fn is_relative_part(relative: &str) -> bool {
    !relative.contains(['\\', ':']) && relative.split('/').all(is_name)
}

/// A fully qualified path in a canonical form: `/a/b` (Unix), `C:\\a\\b`
/// or `C:/a/b` (a drive), or `\\\\server\\share\\a` (UNC, which the agent
/// accepts only inside a UNC scope); never a verbatim (`\\\\?\\`) or device
/// (`\\\\.\\`) path, a `.` or `..` name, an empty name (but a trailing
/// separator), a stream (`:` after the drive) or a hidden character.
pub fn is_absolute_path(path: &str) -> bool {
    let b = path.as_bytes();
    let rest = if let Some(rest) = path.strip_prefix('/') {
        rest
    } else if b.len() >= 3
        && b[0].is_ascii_alphabetic()
        && b[1] == b':'
        && matches!(b[2], b'\\' | b'/')
    {
        &path[3..]
    } else if let Some(unc) = path.strip_prefix("\\\\") {
        if unc.starts_with(['?', '.']) {
            return false;
        }
        unc
    } else {
        return false;
    };
    if rest.contains(':') {
        return false;
    }
    let rest = rest.strip_suffix(['/', '\\']).unwrap_or(rest);
    rest.is_empty() || rest.split(['/', '\\']).all(is_name)
}

/// The repository key grammar of the core (`^[a-z][a-z0-9_]{0,31}$`).
pub fn is_repository_key(key: &str) -> bool {
    let bytes = key.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= MAX_REPOSITORY_KEY
        && bytes[0].is_ascii_lowercase()
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_')
}

/// A script or project id: letters, digits, `-` and `_`.
fn is_id(id: &str, max: usize) -> bool {
    !id.is_empty()
        && id.len() <= max
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// An opaque token or handle of the agent: printable ASCII, bounded.
fn is_opaque(token: &str, max: usize) -> bool {
    !token.is_empty() && token.len() <= max && token.bytes().all(|b| b.is_ascii_graphic())
}

/// `script.load`: the invocation's own `script` parameter, nothing else.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptLoad {
    pub script: String,
}

/// `script.log`: one line of the script's log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptLog {
    pub level: LogLevel,
    pub message: String,
}

/// `script.sleep`: waits in the agent, cancellable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptSleep {
    pub ms: u64,
}

/// `script.confirm`: shows the agent's own stored plan for `token`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptConfirm {
    pub token: String,
}

/// `project.repositories`: a project of the active profile (`None`: the
/// run's current one).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectRepositories {
    #[serde(default)]
    pub project: Option<String>,
}

/// `fs.exists`, `fs.stat` and `fs.list`: one path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FsPath {
    pub path: PathArg,
}

/// `fs.mkdir`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FsMkdir {
    pub path: PathArg,
    #[serde(default)]
    pub recursive: bool,
}

/// How file content travels: UTF-8 text, or base64 of the bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Encoding {
    #[default]
    Utf8,
    Base64,
}

/// `fs.readFile`: up to [`MAX_READ_CHUNK`] bytes from `offset` (D1).
/// `maxBytes` bounds the whole file: a larger file is `tooLarge`. A
/// request without `length` is a whole-file read and is refused with
/// `tooLarge` when the file is larger than one chunk, so a guest built
/// before D1 never receives a silently cut file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct FsReadFile {
    pub path: PathArg,
    #[serde(default)]
    pub encoding: Encoding,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<u32>,
    /// The `version` the first chunk's reply gave: every later chunk of
    /// the same read names it, and the agent serves it from the snapshot
    /// the first chunk took, refusing (`fileChanged`) when the file at the
    /// path was replaced or changed meanwhile. A chunk at an offset other
    /// than 0 needs it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub if_version: Option<String>,
}

/// Longest read `version` token, in bytes (the agent's are 32 hex digits).
pub const MAX_READ_VERSION: usize = 64;

/// `fs.writeFile`: a whole file of at most [`MAX_WRITE_CHUNK`] bytes in
/// one request. A larger one is written with `fs.write.*`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct FsWriteFile {
    pub path: PathArg,
    pub content: String,
    #[serde(default)]
    pub encoding: Encoding,
    #[serde(default)]
    pub create_only: bool,
}

/// `fs.write.open`: starts a chunked write (D1). The agent writes into a
/// temporary file next to the target and renames it over the target on
/// `fs.write.commit`; an abort, or the end of the run, removes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct FsWriteOpen {
    pub path: PathArg,
    #[serde(default)]
    pub create_only: bool,
}

/// `fs.write.chunk`: the next part of an open write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FsWriteChunk {
    pub handle: String,
    pub content: String,
    #[serde(default)]
    pub encoding: Encoding,
}

/// `fs.write.commit` and `fs.write.abort`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FsWriteHandle {
    pub handle: String,
}

/// `fs.copy`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FsCopy {
    pub src: PathArg,
    pub dst: PathArg,
    #[serde(default)]
    pub recursive: bool,
    #[serde(default)]
    pub overwrite: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmation: Option<String>,
}

/// `fs.move`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FsMove {
    pub src: PathArg,
    pub dst: PathArg,
    #[serde(default)]
    pub overwrite: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmation: Option<String>,
}

/// `fs.remove`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct FsRemove {
    pub path: PathArg,
    #[serde(default)]
    pub recursive: bool,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmation: Option<String>,
}

/// `fs.removeContents`: everything inside a directory but the `keep`
/// names directly in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct FsRemoveContents {
    pub path: PathArg,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub keep: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmation: Option<String>,
}

/// `process.run`: a command name or the absolute path the user granted,
/// and an argument **array**. Never a shell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProcessRun {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathArg>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdin: Option<String>,
}

/// `shell.run`: a granted shell with the script as one argument.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ShellRun {
    pub shell: String,
    pub script: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathArg>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

/// Environment names a script may never set, whatever its grant (design
/// §5.3, OD-7, widened by the security review), compared without case. The
/// agent applies the same list. Denied: what changes which program runs or
/// where it finds its configuration (`PATH`, `HOME`, `XDG_*`, every
/// `GIT_*` but the author and committer, `DOCKER_*` but BuildKit), what
/// loads code into a program (`LD_*`, `DYLD_*`, interpreter options and
/// paths, shell start-up files), what redirects the network or its trust
/// (proxies, certificate bundles, `SSH_*`), and DeckPilot's own.
pub fn env_denied(name: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "LD_",
        "DYLD_",
        "DECKPILOT_",
        "GIT_",
        "SSH_",
        "XDG_",
        "DOCKER_",
        "PYTHON",
        "PERL",
        "RUBY",
        "NPM_CONFIG_",
        "JAVA_",
        "_JAVA_",
        "JDK_JAVA_",
        "RUSTC",
        "RUSTDOC",
        "CARGO_",
        "SSL_CERT_",
        "LUA_",
        "TCL",
        "DOTNET_",
        "COR_",
        "CORECLR_",
        "PHP",
        "GEM_",
        "BUNDLE_",
        "PIP_",
        "AWS_",
        "ELECTRON_",
    ];
    const EXACT: &[&str] = &[
        "PATH",
        "PATHEXT",
        "HOME",
        "USERPROFILE",
        "HOMEDRIVE",
        "HOMEPATH",
        "APPDATA",
        "LOCALAPPDATA",
        "SYSTEMROOT",
        "WINDIR",
        "COMSPEC",
        "PSMODULEPATH",
        "SHELL",
        "BASH_ENV",
        "ENV",
        "ZDOTDIR",
        "IFS",
        "PS4",
        "PROMPT_COMMAND",
        "CDPATH",
        "TMPDIR",
        "TEMP",
        "TMP",
        "EDITOR",
        "VISUAL",
        "PAGER",
        "BROWSER",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
        "FTP_PROXY",
        "CURL_CA_BUNDLE",
        "REQUESTS_CA_BUNDLE",
        "NODE_OPTIONS",
        "NODE_PATH",
        "NODE_EXTRA_CA_CERTS",
        "NODE_TLS_REJECT_UNAUTHORIZED",
        "NODE_REPL_EXTERNAL_MODULE",
        "GCONV_PATH",
        "LOCPATH",
        "NLSPATH",
        "MALLOC_CONF",
        "HOSTALIASES",
        "RES_OPTIONS",
        "LOCALDOMAIN",
        "OPENSSL_CONF",
        "CLASSPATH",
        "GOFLAGS",
        "MAKEFLAGS",
        "MFLAGS",
        "LESSOPEN",
        "LESSCLOSE",
        "SHELLOPTS",
        "BASHOPTS",
        "KUBECONFIG",
    ];
    /// `GIT_*` names that only label a commit.
    const GIT_ALLOWED: &[&str] = &[
        "GIT_AUTHOR_NAME",
        "GIT_AUTHOR_EMAIL",
        "GIT_AUTHOR_DATE",
        "GIT_COMMITTER_NAME",
        "GIT_COMMITTER_EMAIL",
        "GIT_COMMITTER_DATE",
    ];
    let upper = name.to_ascii_uppercase();
    if GIT_ALLOWED.contains(&upper.as_str()) || upper == "DOCKER_BUILDKIT" {
        return false;
    }
    PREFIXES.iter().any(|p| upper.starts_with(p)) || EXACT.contains(&upper.as_str())
}

/// An environment name: `[A-Za-z_][A-Za-z0-9_]*`, at most
/// [`MAX_ENV_NAME`] bytes.
pub fn is_env_name(name: &str) -> bool {
    let b = name.as_bytes();
    !b.is_empty()
        && b.len() <= MAX_ENV_NAME
        && (b[0].is_ascii_alphabetic() || b[0] == b'_')
        && b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'_')
}

fn validate_env(env: &BTreeMap<String, String>) -> Result<(), RequestError> {
    if env.len() > MAX_ENV {
        return Err(RequestError::TooLarge("env"));
    }
    for (name, value) in env {
        if !is_env_name(name) || env_denied(name) || value.contains('\0') {
            return Err(RequestError::Invalid("env"));
        }
        if value.len() > MAX_ENV_VALUE {
            return Err(RequestError::TooLarge("env"));
        }
    }
    Ok(())
}

fn validate_timeout(timeout: Option<u64>) -> Result<(), RequestError> {
    match timeout {
        Some(0) => Err(RequestError::Invalid("timeoutMs")),
        Some(t) if t > MAX_TIMEOUT_MS => Err(RequestError::TooLarge("timeoutMs")),
        _ => Ok(()),
    }
}

fn validate_confirmation(token: Option<&str>) -> Result<(), RequestError> {
    match token {
        Some(t) if !is_opaque(t, MAX_TOKEN) => Err(RequestError::Invalid("confirmation")),
        _ => Ok(()),
    }
}

/// The bytes `content` stands for in `encoding`, or `None` when it is not
/// valid base64 (padded, standard alphabet).
pub fn content_len(content: &str, encoding: Encoding) -> Option<usize> {
    match encoding {
        Encoding::Utf8 => Some(content.len()),
        Encoding::Base64 => {
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD
                .decode(content.as_bytes())
                .ok()
                .map(|bytes| bytes.len())
        }
    }
}

fn validate_chunk(content: &str, encoding: Encoding) -> Result<(), RequestError> {
    match content_len(content, encoding) {
        None => Err(RequestError::Invalid("content")),
        Some(n) if n > MAX_WRITE_CHUNK => Err(RequestError::TooLarge("content")),
        Some(_) => Ok(()),
    }
}

/// A process's command: a bare name ([`is_command_name`]) or an absolute
/// path of a drive or the Unix root, never UNC ([`is_absolute_path`]).
/// Which one it may be is the grant's to say.
fn validate_command(command: &str) -> Result<(), RequestError> {
    if command.len() > MAX_COMMAND {
        return Err(RequestError::TooLarge("command"));
    }
    if is_command_name(command)
        || (is_absolute_path(command) && !command.starts_with("\\\\") && !is_batch(command))
    {
        Ok(())
    } else {
        Err(RequestError::Invalid("command"))
    }
}

/// A bare command name: `^[A-Za-z0-9][A-Za-z0-9._+-]*$`, at most 255
/// bytes, not ending in `.`, and never a batch file (`.bat`, `.cmd`), which
/// only `shell.run` with `cmd` may start.
pub fn is_command_name(name: &str) -> bool {
    let b = name.as_bytes();
    !b.is_empty()
        && b.len() <= 255
        && b[0].is_ascii_alphanumeric()
        && b.iter()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'+' | b'-'))
        && !name.ends_with('.')
        && !is_batch(name)
}

/// A Windows batch file, by its extension (CVE-2024-24576).
pub fn is_batch(command: &str) -> bool {
    let lower = command.trim_end_matches([' ', '.']).to_ascii_lowercase();
    lower.ends_with(".bat") || lower.ends_with(".cmd")
}

impl ScriptLoad {
    pub(crate) fn validate(&self) -> Result<(), RequestError> {
        if is_id(&self.script, MAX_SCRIPT_ID) {
            Ok(())
        } else {
            Err(RequestError::Invalid("script"))
        }
    }
}

impl ScriptLog {
    pub(crate) fn validate(&self) -> Result<(), RequestError> {
        if self.message.len() > MAX_SCRIPT_LOG_LINE {
            Err(RequestError::TooLarge("message"))
        } else {
            Ok(())
        }
    }
}

impl ScriptSleep {
    pub(crate) fn validate(self) -> Result<(), RequestError> {
        if self.ms > MAX_SLEEP_MS {
            Err(RequestError::TooLarge("ms"))
        } else {
            Ok(())
        }
    }
}

impl ScriptConfirm {
    pub(crate) fn validate(&self) -> Result<(), RequestError> {
        validate_confirmation(Some(&self.token)).map_err(|_| RequestError::Invalid("token"))
    }
}

impl ProjectRepositories {
    pub(crate) fn validate(&self) -> Result<(), RequestError> {
        match &self.project {
            Some(id) if !is_id(id, MAX_PROJECT_ID) => Err(RequestError::Invalid("project")),
            _ => Ok(()),
        }
    }
}

impl FsReadFile {
    pub(crate) fn validate(&self) -> Result<(), RequestError> {
        self.path.validate()?;
        if self
            .max_bytes
            .is_some_and(|m| m == 0 || m > MAX_SCRIPT_FILE)
        {
            return Err(RequestError::TooLarge("maxBytes"));
        }
        if self.offset.is_some_and(|o| o > MAX_SCRIPT_FILE) {
            return Err(RequestError::TooLarge("offset"));
        }
        // A chunked read names both; `offset` alone is ambiguous.
        if self.offset.is_some() && self.length.is_none() {
            return Err(RequestError::Invalid("length"));
        }
        if self
            .if_version
            .as_deref()
            .is_some_and(|v| !is_opaque(v, MAX_READ_VERSION))
        {
            return Err(RequestError::Invalid("ifVersion"));
        }
        // A later chunk belongs to a read: it names its version.
        if self.offset.is_some_and(|o| o > 0) && self.if_version.is_none() {
            return Err(RequestError::Invalid("ifVersion"));
        }
        if self.length.is_some_and(|l| l == 0 || l > MAX_READ_CHUNK) {
            return Err(RequestError::TooLarge("length"));
        }
        Ok(())
    }
}

impl FsRemoveContents {
    pub(crate) fn validate(&self) -> Result<(), RequestError> {
        self.path.validate()?;
        if self.keep.len() > MAX_KEEP {
            return Err(RequestError::TooLarge("keep"));
        }
        if self.keep.iter().any(|name| !is_entry_name(name)) {
            return Err(RequestError::Invalid("keep"));
        }
        validate_confirmation(self.confirmation.as_deref())
    }
}

/// A name directly inside a directory: never empty, `.`, `..`, a path or
/// longer than [`MAX_KEEP_NAME`] bytes.
pub fn is_entry_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name.len() <= MAX_KEEP_NAME
        && !name.contains(['/', '\\', '\0', ':'])
}

impl ProcessRun {
    pub(crate) fn validate(&self) -> Result<(), RequestError> {
        validate_command(&self.command)?;
        if self.args.len() > MAX_PROCESS_ARGS {
            return Err(RequestError::TooLarge("args"));
        }
        if self.args.iter().any(|a| a.contains('\0')) {
            return Err(RequestError::Invalid("args"));
        }
        if self.args.iter().any(|a| a.len() > MAX_PROCESS_ARG)
            || self.args.iter().map(String::len).sum::<usize>() > MAX_PROCESS_ARGS_TOTAL
        {
            return Err(RequestError::TooLarge("args"));
        }
        if self
            .stdin
            .as_ref()
            .is_some_and(|stdin| stdin.len() > MAX_PROCESS_STDIN)
        {
            return Err(RequestError::TooLarge("stdin"));
        }
        if let Some(cwd) = &self.cwd {
            cwd.validate()?;
        }
        validate_env(&self.env)?;
        validate_timeout(self.timeout_ms)
    }
}

impl ShellRun {
    pub(crate) fn validate(&self) -> Result<(), RequestError> {
        if !SHELLS.contains(&self.shell.as_str()) {
            return Err(RequestError::Invalid("shell"));
        }
        if self.script.contains('\0') {
            return Err(RequestError::Invalid("script"));
        }
        if self.script.len() > MAX_SHELL_SCRIPT {
            return Err(RequestError::TooLarge("script"));
        }
        if let Some(cwd) = &self.cwd {
            cwd.validate()?;
        }
        validate_env(&self.env)?;
        validate_timeout(self.timeout_ms)
    }
}

/// [`CapabilityRequest::validate`] for the bridge's operations.
pub(crate) fn validate(request: &CapabilityRequest) -> Result<(), RequestError> {
    use CapabilityRequest as R;
    match request {
        R::ScriptLoad(l) => l.validate(),
        R::ScriptPoll(_) | R::ShellAvailable(_) => Ok(()),
        R::ScriptLog(l) => l.validate(),
        R::ScriptSleep(s) => s.validate(),
        R::ScriptConfirm(c) => c.validate(),
        R::ProjectRepositories(p) => p.validate(),
        R::FsExists(p) | R::FsStat(p) | R::FsList(p) => p.path.validate(),
        R::FsMkdir(m) => m.path.validate(),
        R::FsReadFile(r) => r.validate(),
        R::FsWriteFile(w) => {
            w.path.validate()?;
            validate_chunk(&w.content, w.encoding)
        }
        R::FsWriteOpen(o) => o.path.validate(),
        R::FsWriteChunk(c) => {
            if !is_opaque(&c.handle, MAX_WRITE_HANDLE) {
                return Err(RequestError::Invalid("handle"));
            }
            validate_chunk(&c.content, c.encoding)
        }
        R::FsWriteCommit(h) | R::FsWriteAbort(h) => {
            if is_opaque(&h.handle, MAX_WRITE_HANDLE) {
                Ok(())
            } else {
                Err(RequestError::Invalid("handle"))
            }
        }
        R::FsCopy(c) => {
            c.src.validate()?;
            c.dst.validate()?;
            validate_confirmation(c.confirmation.as_deref())
        }
        R::FsMove(m) => {
            m.src.validate()?;
            m.dst.validate()?;
            validate_confirmation(m.confirmation.as_deref())
        }
        R::FsRemove(r) => {
            r.path.validate()?;
            validate_confirmation(r.confirmation.as_deref())
        }
        R::FsRemoveContents(r) => r.validate(),
        R::ProcessRun(p) => p.validate(),
        R::ShellRun(s) => s.validate(),
        _ => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// Replies (host to guest: unknown fields are ignored, so the agent can add
// to them within ABI 1)
// ---------------------------------------------------------------------------

/// The script-level detail of a refusal or failure, carried as
/// [`CallError::script`](crate::capability::CallError::script) on the
/// bridge's operations only. `code` is one of the `ErrorCode`s of the
/// guest's `deckpilot.d.ts` ([`codes`]).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptFault {
    pub code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// For `needsConfirmation`: the token of the plan the agent computed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmation: Option<Confirmation>,
}

/// The token of a plan waiting for `script.confirm`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Confirmation {
    pub token: String,
}

impl ScriptFault {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            message: Some(message.into()),
            ..Self::default()
        }
    }
}

/// The script `ErrorCode`s the agent sends (`deckpilot.d.ts`).
pub mod codes {
    pub const NOT_GRANTED: &str = "notGranted";
    pub const OUTSIDE_SCOPE: &str = "outsideScope";
    pub const PROTECTED_PATH: &str = "protectedPath";
    pub const SYMLINK_ESCAPE: &str = "symlinkEscape";
    pub const NOT_FOUND: &str = "notFound";
    /// A file changed (replaced or rewritten) between the chunks of a read.
    pub const FILE_CHANGED: &str = "fileChanged";
    pub const EXISTS: &str = "exists";
    pub const TOO_LARGE: &str = "tooLarge";
    pub const COMMAND_NOT_FOUND: &str = "commandNotFound";
    pub const GIT_UNAVAILABLE: &str = "gitUnavailable";
    pub const DOCKER_UNAVAILABLE: &str = "dockerUnavailable";
    pub const NON_ZERO_EXIT: &str = "nonZeroExit";
    pub const TIMED_OUT: &str = "timedOut";
    pub const CANCELLED: &str = "cancelled";
    pub const NEEDS_CONFIRMATION: &str = "needsConfirmation";
    pub const NOT_USER_INITIATED: &str = "notUserInitiated";
    pub const LIMIT: &str = "limit";
    pub const INVALID_ARGUMENT: &str = "invalidArgument";
    pub const NO_PROJECT: &str = "noProject";
    pub const SCRIPT_MISSING: &str = "scriptMissing";
    pub const SCRIPT_NEEDS_REVIEW: &str = "scriptNeedsReview";
    pub const UNAVAILABLE: &str = "unavailable";
    pub const HOST_ERROR: &str = "hostError";
}

/// The operating system, as `script.load` names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Macos,
    Windows,
    Linux,
}

/// The script's own settings, as `script.load` passes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptSettings {
    pub timeout_ms: u64,
    pub stop_on_error: bool,
    /// `memoryMib` on the wire: the spelling the guest's `Settings` reads
    /// (its README writes `memoryMiB`).
    pub memory_mib: u32,
}

/// What `deckpilot.run` shows: the run's id, its origin and whether the
/// user started it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptRunInfo {
    pub id: String,
    pub origin: String,
    pub user_initiated: bool,
}

/// The `ok` value of `script.load`. `grants` is the summary of what the
/// guest may expose (`docs/scripts-bridge.md`); it is never trusted for a
/// decision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptLoaded {
    pub source: String,
    pub platform: Platform,
    pub settings: ScriptSettings,
    pub grants: Value,
    pub run: ScriptRunInfo,
    /// The run's time left when it starts, in milliseconds.
    pub remaining_ms: u64,
    pub now_us: u64,
}

/// The `ok` value of `script.poll`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptPolled {
    pub cancelled: bool,
    pub remaining_ms: u64,
    pub now_us: u64,
}

/// The `ok` value of `script.sleep` (the guest reads only `nowUs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptSlept {
    pub now_us: u64,
}

/// The `ok` value of `script.confirm`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Confirmed {
    pub confirmed: bool,
}

/// What a file system entry is. A link is never followed to say what it
/// points to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    File,
    Directory,
    Symlink,
    Other,
}

/// `fs.stat`'s value (inside `{"stat": …}`; `null` when nothing is there).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FsStat {
    pub kind: EntryKind,
    pub size: u64,
    pub modified_ms: u64,
}

/// One entry of `fs.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsEntry {
    pub name: String,
    pub kind: EntryKind,
}

/// The `ok` value of `fs.readFile` (D1): `content` holds `bytes` bytes of
/// the file from the request's offset; `size` is the whole file's; `eof`
/// says whether they reach its end. In UTF-8 a chunk ends on a character
/// boundary, so `bytes` may be up to 3 less than the length asked for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FsRead {
    pub content: String,
    pub bytes: u64,
    pub size: u64,
    pub eof: bool,
    /// The read's version, when it did not reach the end: the later
    /// chunks name it (`ifVersion`), so all of them come from one snapshot
    /// of one file in one state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// The `ok` value of `fs.write.open`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsWriteOpened {
    pub handle: String,
}

/// What a destructive operation removes (or, with `dryRun`, would
/// remove): counts, at most 50 sample paths relative to `root`, and
/// whether it was carried out.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemovalPlan {
    pub dry_run: bool,
    /// The label of the scope root (a repository key, or a directory's
    /// label); a path only with `repositories.paths`.
    pub root: String,
    pub files: u64,
    pub directories: u64,
    pub bytes: u64,
    pub sample: Vec<String>,
    pub removed: bool,
}

/// Paths in [`RemovalPlan::sample`].
pub const MAX_PLAN_SAMPLE: usize = 50;

/// The `ok` value of `process.run` and `shell.run`. A non-zero exit is
/// `ok: false`, never an error.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessResult {
    pub ok: bool,
    pub exit_code: Option<i32>,
    pub signal: Option<String>,
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
    pub duration_ms: u64,
    pub timed_out: bool,
    pub cancelled: bool,
    pub now_us: u64,
}

/// A repository's remote, as scripts see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryRemote {
    pub url: Option<String>,
    pub default_branch: Option<String>,
}

/// A repository of a project, as scripts see it: `path` is `None` without
/// the `repositories.paths` grant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Repository {
    pub id: String,
    pub key: String,
    pub name: String,
    pub description: Option<String>,
    pub is_default: bool,
    pub path: Option<String>,
    pub exists: bool,
    pub remote: Option<RepositoryRemote>,
    /// Every remote, in the stored order (decision K4): with the link to a
    /// Git Providers connection on this computer and the provider's
    /// immutable repository id. Script objects only; text variables never
    /// carry either id. Absent from replies of agents before it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remotes: Vec<RemoteInfo>,
}

/// One remote of a [`Repository`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteInfo {
    /// The git remote name (`origin`).
    pub name: String,
    pub primary: bool,
    pub url: Option<String>,
    pub default_branch: Option<String>,
    /// The Git Providers connection it is linked to here.
    pub connection_id: Option<String>,
    /// The provider's immutable id (`gh:…`, `gl:…`, `bb:…`).
    pub remote_repository_id: Option<String>,
}

/// The `ok` value of `project.repositories`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectRepositoriesReply {
    pub project: Option<crate::privileged::ProjectInfo>,
    pub repositories: Vec<Repository>,
}

/// The `ok` value of `shell.available`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellsAvailable {
    pub shells: Vec<String>,
}

#[cfg(test)]
mod tests;
