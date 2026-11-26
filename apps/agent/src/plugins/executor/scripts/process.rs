//! `process.run`, `shell.run` and `shell.available` (design §5.3
//! "Process", T5, T18, T19), through the tools runner
//! (`crate::plugins::tools::execute_with`): the program runs in its own
//! process group with stdin written and closed, its output captured up to
//! a cap, and it is stopped (`SIGTERM`, `SIGKILL` after 2 s; `taskkill /T
//! /F` on Windows) when its time is up, the run is stopped, or the grant it
//! used is revoked; at most 4 run at once for the plugin.
//!
//! * **The command** must be one the script was granted, by the same name
//!   or path. `git` and `docker` are the user's configured tools; another
//!   bare name is looked up on the agent's `PATH` (with `PATHEXT` on
//!   Windows); an absolute path must exist. The file found is
//!   canonicalized and never a batch file (`.bat`, `.cmd`: only
//!   `shell.run` with `cmd`).
//! * **Arguments** are an array, never a shell. For `git`, the options that
//!   change its configuration, its programs or its repository (`-c`,
//!   `--config-env`, `--exec-path`, `--git-dir`, `--work-tree`, `-C`,
//!   `--upload-pack`, `--receive-pack`, `--exec`, `-u`, `--template`, …)
//!   and the `ext::` transport are refused unless the grant says
//!   `unrestrictedArgs` (OD-8); the hardening of `tool.run` is prepended
//!   and its prompts are off.
//! * **`cwd`** follows the grant's rule (`repositories`, `fsScopes`, `any`),
//!   resolved like a file system path, canonicalized, a directory inside
//!   its scope, and checked again right before the start. The window
//!   between that check and the start is documented: closing it needs
//!   `fchdir` in the child, which this crate's `forbid(unsafe_code)` does
//!   not allow; using it needs write access to a folder inside the scope,
//!   which the user granted. Without `cwd`, an empty folder of the agent's.
//! * **The environment** is the tools runner's base (no secrets) plus the
//!   script's entries when the grant says `env: allowed`, each a valid name
//!   the denylist does not refuse (`scripts::env_denied`).
//! * **Time**: `min(timeoutMs or the script's limit, the run's time
//!   left)`. **Output**: the script's `maxOutputKiB` per stream (at most
//!   512 KiB), then shortened until the reply fits its cap.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use deckpilot_core::script_grants::{
    CommandClass, FsAccess, FsRoot, FsScope, ProcessCwd, ProcessEnv, ProcessGrant, ScriptGrants,
    command_class,
};
use deckpilot_plugin_abi::privileged::Tool;
use deckpilot_plugin_abi::scripts::{
    PathArg, ProcessResult, ProcessRun, ScriptFault, ShellRun, ShellsAvailable, codes, env_denied,
    is_batch, is_env_name,
};
use deckpilot_plugin_broker::PluginId;
use serde_json::Value;

use super::fs::protected::{canonical, same_path, starts_with};
use super::fs::resolve::resolve;
use super::{Call, Notes, RunProject, fault, now_us, to_value};
use crate::plugins::executor::Executor;
use crate::plugins::tools::{self as runner, Denial, Plan, RunSlot, Stop, codes as runner_codes};

/// Output kept per stream, at most; both together are cut to
/// [`MAX_OUTPUT_TOTAL`] (the Scripts guest review of plugins #100).
const MAX_STREAM: usize = 1 << 20;
/// Output kept of both streams together: under 1 MiB, with room for the
/// rest of the reply.
const MAX_OUTPUT_TOTAL: usize = 960 << 10;
/// Room the JSON of a process result may take (the 1 MiB reply cap, less
/// the envelope).
const REPLY_ROOM: usize = (1 << 20) - 4096;

/// The git subcommands a script may run without `unrestrictedArgs`
/// (OD-8, security review of #472): working in the repository the run is
/// in. Never `config` (aliases, `core.sshCommand`), `clone`, `init` or
/// `worktree` (they write where a path says), `submodule`, `bisect`,
/// `difftool`, `mergetool`, `filter-branch`, `instaweb`, `archive`,
/// `format-patch`, `bundle` or `help`, which run programs or write files
/// outside it.
const GIT_SUBCOMMANDS: &[&str] = &[
    "add",
    "blame",
    "branch",
    "checkout",
    "cherry-pick",
    "clean",
    "commit",
    "describe",
    "diff",
    "fetch",
    "gc",
    "grep",
    "log",
    "ls-files",
    "ls-remote",
    "merge",
    "mv",
    "pull",
    "push",
    "rebase",
    "remote",
    "reset",
    "restore",
    "rev-parse",
    "revert",
    "rm",
    "shortlog",
    "show",
    "stash",
    "status",
    "switch",
    "tag",
];

/// Long options of any subcommand a script may not pass without
/// `unrestrictedArgs`, nor any prefix of them git would accept as one
/// (`--upload-p`): they run a program, read or write files elsewhere, or
/// change git's configuration.
const GIT_DENIED_LONG: &[&str] = &[
    "--upload-pack",
    "--receive-pack",
    "--exec",
    "--template",
    "--separate-git-dir",
    "--config",
    "--config-env",
    "--output",
    "--output-directory",
    "--ext-diff",
    "--textconv",
    "--extcmd",
    "--tool",
    "--no-index",
    "--git-dir",
    "--work-tree",
    "--exec-path",
    "--namespace",
    "--super-prefix",
    "--attr-source",
    "--strategy",
    "--strategy-option",
    "--gpg-sign",
    "--signoff-command",
    "--open-files-in-pager",
    "--contents",
    "--file",
    "--pathspec-from-file",
    "--exclude-from",
    "--exclude-per-directory",
    "--ignore-revs-file",
    "--orderfile",
];

/// Exact options git knows that are also the start of a denied one: an
/// exact name is that option, never an abbreviation.
const GIT_EXACT_ALLOWED: &[&str] = &["exclude"];

/// Short options, by subcommand, that run a program or read or write a
/// file outside the repository (`rebase -x`, `grep -O`, `commit -F`, ...).
fn git_denied_letters(sub: &str) -> &'static [u8] {
    match sub {
        "rebase" => b"xsX",
        "fetch" | "ls-remote" => b"u",
        "pull" => b"usX",
        "merge" => b"sXF",
        "cherry-pick" | "revert" => b"sX",
        "grep" => b"Of",
        "commit" => b"Ft",
        "tag" => b"F",
        "blame" => b"S",
        "diff" | "log" | "show" => b"O",
        "ls-files" => b"X",
        _ => b"",
    }
}

/// Whether a single-dash argument holds a denied letter anywhere in its
/// cluster (`-qx` is `-q -x`); over-refusing a value is fine.
fn git_denied_short(sub: &str, arg: &str) -> bool {
    match arg.strip_prefix('-') {
        Some(cluster) if !cluster.starts_with('-') => cluster
            .bytes()
            .any(|b| git_denied_letters(sub).contains(&b)),
        _ => false,
    }
}

/// Whether git with `args` steps outside what a script may ask without
/// `unrestrictedArgs`: the first argument must be one of
/// [`GIT_SUBCOMMANDS`] (no global option at all: the host sets its own),
/// no [`GIT_DENIED_LONG`] option or a prefix of one, no denied short
/// option, and no `ext::`/`fd::` transport.
fn git_denied(args: &[String]) -> bool {
    let Some(sub) = args.first() else {
        return false;
    };
    if !GIT_SUBCOMMANDS.contains(&sub.as_str()) {
        return true;
    }
    args.iter().skip(1).any(|arg| {
        let lower = arg.to_ascii_lowercase();
        if lower.contains("ext::") || lower.contains("fd::") {
            return true;
        }
        if git_denied_short(sub, arg) {
            return true;
        }
        if let Some(long) = lower.strip_prefix("--") {
            let name = long.split('=').next().unwrap_or("");
            // git accepts any unique prefix of a long option, down to one
            // letter; a longer name is another option.
            return !name.is_empty()
                && !GIT_EXACT_ALLOWED.contains(&name)
                && GIT_DENIED_LONG
                    .iter()
                    .any(|denied| denied[2..].starts_with(name));
        }
        false
    })
}

/// Environment names the agent refuses on top of the ABI's denylist
/// (security review of #472): credential managers' traces, TLS key logs,
/// compilers and build tools' options, editors' start-up commands, and
/// tools' own configuration files.
fn env_denied_here(name: &str) -> bool {
    const PREFIXES: &[&str] = &["GCM_", "GO", "COMPOSE_", "MAVEN_", "GRADLE_", "CURL_", "HG"];
    const EXACT: &[&str] = &[
        "SSLKEYLOGFILE",
        "CC",
        "CXX",
        "CPP",
        "LD",
        "AR",
        "CFLAGS",
        "CXXFLAGS",
        "CPPFLAGS",
        "LDFLAGS",
        "RUSTFLAGS",
        "VIMINIT",
        "EXINIT",
        "GLIBC_TUNABLES",
        "WGETRC",
        "NPMRC",
        "YARNRC",
        "BUNDLE_GEMFILE",
        "KUBECTL_EXTERNAL_DIFF",
    ];
    let upper = name.to_ascii_uppercase();
    PREFIXES.iter().any(|p| upper.starts_with(p)) || EXACT.contains(&upper.as_str())
}

/// The file name of a command, without `.exe`.
fn stem(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    name.strip_suffix(".exe").map(str::to_owned).unwrap_or(name)
}

fn same_command(granted: &str, asked: &str) -> bool {
    let (g, a) = (Path::new(granted), Path::new(asked));
    if g.is_absolute() || a.is_absolute() {
        g.is_absolute() && a.is_absolute() && same_path(g, a)
    } else if cfg!(windows) {
        granted.eq_ignore_ascii_case(asked)
    } else {
        granted == asked
    }
}

fn denial(d: &Denial) -> ScriptFault {
    fault(codes::HOST_ERROR, d.message.clone())
}

/// The plan's environment and its program, before the script's entries.
struct Prepared {
    program: PathBuf,
    args: Vec<OsString>,
    env: Vec<(String, OsString)>,
    cwd: PathBuf,
    /// The scope the cwd was checked in (its root and the relative path),
    /// to check again right before the start.
    cwd_check: Option<PathBuf>,
}

impl Executor {
    /// A bare name on the agent's `PATH` (with `PATHEXT` on Windows),
    /// canonicalized.
    fn lookup(&self, name: &str) -> Option<PathBuf> {
        let mut names = vec![name.to_owned()];
        if cfg!(windows) {
            let runnable_ext = Path::new(name)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("exe") || e.eq_ignore_ascii_case("com"));
            if !runnable_ext {
                // `python3.12` is a name, not an extension.
                names = [".exe", ".com"]
                    .iter()
                    .map(|e| format!("{name}{e}"))
                    .collect();
            }
        }
        self.tools
            .path_entries()
            .into_iter()
            .flat_map(|dir| names.iter().map(move |n| dir.join(n)))
            .find(|candidate| candidate.is_file() && runnable(candidate))
    }

    /// The program a granted command runs: the path to start (as found,
    /// so a program that reads its own name sees the granted one) and the
    /// file it is (canonical, checked).
    fn program(&self, grant: &ProcessGrant) -> Result<(PathBuf, PathBuf), ScriptFault> {
        let not_found = |code: &str, what: &str| ScriptFault {
            command: Some(grant.command.clone()),
            ..fault(code, format!("{what} was not found on this computer"))
        };
        // Not grantable, whatever a stored record says, by name or by the
        // file it resolves to: container runtimes (an open owner decision),
        // programs that act as another user, and shells (their own grant).
        let refused = |what: &str| {
            let message = match command_class(what) {
                CommandClass::ContainerRuntime => {
                    "container runtimes are not available to scripts yet"
                }
                CommandClass::Elevation => "a script cannot run a program as another user",
                CommandClass::Shell => "a shell runs only through shell.run and its grant",
                CommandClass::RunsAnyProgram | CommandClass::Plain => return None,
            };
            Some(ScriptFault {
                command: Some(grant.command.clone()),
                ..fault(codes::NOT_GRANTED, message)
            })
        };
        if let Some(fault) = refused(&grant.command) {
            return Err(fault);
        }
        let (code, path) = match grant.command.as_str() {
            "git" => {
                let path = self
                    .tools
                    .executable(Tool::Git)
                    .map_err(|_| not_found(codes::GIT_UNAVAILABLE, &grant.command))?;
                (codes::GIT_UNAVAILABLE, path)
            }
            command if Path::new(command).is_absolute() => {
                (codes::COMMAND_NOT_FOUND, PathBuf::from(command))
            }
            command => (
                codes::COMMAND_NOT_FOUND,
                self.lookup(command)
                    .ok_or_else(|| not_found(codes::COMMAND_NOT_FOUND, command))?,
            ),
        };
        let real = canonical(&path).map_err(|_| not_found(code, &grant.command))?;
        if let Some(fault) =
            refused(&path.to_string_lossy()).or_else(|| refused(&real.to_string_lossy()))
        {
            return Err(fault);
        }
        if !real.is_file() || !runnable(&real) {
            return Err(not_found(code, &grant.command));
        }
        if is_batch(&path.to_string_lossy()) || is_batch(&real.to_string_lossy()) {
            return Err(ScriptFault {
                command: Some(grant.command.clone()),
                ..fault(
                    codes::NOT_GRANTED,
                    "a batch file runs only through shell.run with cmd",
                )
            });
        }
        Ok((path, real))
    }

    /// `process.run`.
    pub(super) fn script_process(
        &self,
        plugin: &PluginId,
        call: &Call<'_>,
        request: &ProcessRun,
        notes: &mut Notes,
    ) -> Result<Value, ScriptFault> {
        let grant = call
            .grants
            .process
            .iter()
            .find(|g| same_command(&g.command, &request.command))
            .cloned()
            .ok_or_else(|| ScriptFault {
                command: Some(request.command.clone()),
                ..fault(
                    codes::NOT_GRANTED,
                    format!("the script may not run `{}`", request.command),
                )
            })?;
        let (program, real) = self.program(&grant)?;
        notes.add("command", stem(&real));
        notes.add("args", request.args.len().to_string());
        // git by its grant, its name or the file it is: a link named
        // otherwise is still git.
        let git = grant.command == "git" || stem(&program) == "git" || stem(&real) == "git";
        if git && !grant.unrestricted_args && git_denied(&request.args) {
            return Err(ScriptFault {
                command: Some(request.command.clone()),
                ..fault(
                    codes::NOT_GRANTED,
                    "this git subcommand or option runs other programs, reaches beyond the \
                     repository or changes git's configuration; it needs unrestrictedArgs",
                )
            });
        }
        let project = call.project()?;
        let (cwd, cwd_check, ceiling) =
            cwd(call, project.as_ref(), grant.cwd, request.cwd.as_ref())?;
        let mut args: Vec<OsString> = Vec::new();
        let mut extra = vec!["LANG", "SSH_AUTH_SOCK"];
        if git {
            for setting in runner::git_hardening() {
                args.push("-c".into());
                args.push((*setting).into());
            }
            extra.extend(["GIT_EXEC_PATH", "XDG_CONFIG_HOME"]);
        }
        args.extend(request.args.iter().map(OsString::from));
        let mut env = self.tools.environment(&extra, &program);
        script_env(&mut env, &request.env, grant.env == ProcessEnv::Allowed)?;
        if git {
            // The host's, after the script's, so none is overridden.
            let mut forced: Vec<(&str, OsString)> = vec![
                ("GIT_TERMINAL_PROMPT", "0".into()),
                ("GCM_INTERACTIVE", "never".into()),
                ("GIT_ASKPASS", "".into()),
                ("GIT_OPTIONAL_LOCKS", "0".into()),
                ("GIT_PROTOCOL_FROM_USER", "0".into()),
            ];
            // git never looks for a repository above the scope the run is
            // in (a dotfiles repository in the home folder).
            if let Some(ceiling) = cwd_check.as_ref().and(ceiling.as_ref()) {
                forced.push(("GIT_CEILING_DIRECTORIES", ceiling.as_os_str().to_owned()));
            }
            for (name, value) in forced {
                env.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
                env.push((name.into(), value));
            }
        }
        let prepared = Prepared {
            program,
            args,
            env,
            cwd,
            cwd_check,
        };
        let stdin = request.stdin.as_ref().map(|s| s.as_bytes().to_vec());
        let script = call.script.to_owned();
        let granted = grant.clone();
        let host = call.run.host.clone();
        // The very grant the run started under: narrowing it (its cwd
        // rule, env, unrestrictedArgs) stops the program too.
        let still_granted = move || {
            host.grants(&script)
                .filter(|g| g.validate().is_ok())
                .is_some_and(|g| g.process.contains(&granted))
        };
        self.start(
            plugin,
            call,
            &prepared,
            request.timeout_ms,
            stdin,
            notes,
            ScopeNeed::Process,
            &still_granted,
        )
    }

    /// `shell.run`.
    pub(super) fn script_shell(
        &self,
        plugin: &PluginId,
        call: &Call<'_>,
        request: &ShellRun,
        notes: &mut Notes,
    ) -> Result<Value, ScriptFault> {
        if !granted_shell(&call.grants, &request.shell) {
            return Err(fault(
                codes::NOT_GRANTED,
                format!("the script may not run the shell `{}`", request.shell),
            ));
        }
        let program = self
            .shell_program(&request.shell)
            .ok_or_else(|| ScriptFault {
                command: Some(request.shell.clone()),
                ..fault(
                    codes::COMMAND_NOT_FOUND,
                    format!("{} was not found on this computer", request.shell),
                )
            })?;
        notes.add("command", stem(&program));
        let project = call.project()?;
        // A shell runs in a repository of the project or a scope of the
        // script, never anywhere else.
        let rule = if request.cwd.as_ref().is_some_and(|c| c.repository.is_some()) {
            ProcessCwd::Repositories
        } else {
            ProcessCwd::FsScopes
        };
        let (cwd, cwd_check, _) = cwd(call, project.as_ref(), rule, request.cwd.as_ref())?;
        let mut args: Vec<OsString> = match request.shell.as_str() {
            "pwsh" | "powershell" => vec![
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-Command".into(),
            ],
            "cmd" => vec!["/d".into(), "/c".into()],
            _ => vec!["-c".into()],
        };
        // The script is one argument, never joined with anything.
        args.push(request.script.clone().into());
        let mut env = self.tools.environment(&["LANG"], &program);
        script_env(&mut env, &request.env, true)?;
        let prepared = Prepared {
            program,
            args,
            env,
            cwd,
            cwd_check,
        };
        let script = call.script.to_owned();
        let shell = request.shell.clone();
        let host = call.run.host.clone();
        let still_granted = move || {
            host.grants(&script)
                .filter(|g| g.validate().is_ok())
                .is_some_and(|g| granted_shell(&g, &shell))
        };
        self.start(
            plugin,
            call,
            &prepared,
            request.timeout_ms,
            None,
            notes,
            ScopeNeed::Shell,
            &still_granted,
        )
    }

    /// `shell.available`: the granted shells this computer has.
    pub(super) fn shells_available(&self, call: &Call<'_>) -> Result<Value, ScriptFault> {
        let shells = call
            .grants
            .shell
            .iter()
            .flat_map(|s| &s.shells)
            .filter(|s| self.shell_program(s).is_some())
            .cloned()
            .collect();
        to_value(&ShellsAvailable { shells })
    }

    fn shell_program(&self, shell: &str) -> Option<PathBuf> {
        if !deckpilot_plugin_abi::scripts::SHELLS.contains(&shell) {
            return None;
        }
        #[cfg(windows)]
        if shell == "cmd" {
            let root = std::env::var_os("SystemRoot")?;
            return std::fs::canonicalize(PathBuf::from(root).join("System32").join("cmd.exe"))
                .ok();
        }
        if cfg!(not(windows)) && matches!(shell, "cmd" | "powershell") {
            return None;
        }
        self.lookup(shell)
    }

    /// Runs `prepared`, watched.
    #[expect(
        clippy::too_many_arguments,
        reason = "one run, handed over field by field"
    )]
    fn start(
        &self,
        plugin: &PluginId,
        call: &Call<'_>,
        prepared: &Prepared,
        timeout_ms: Option<u64>,
        stdin: Option<Vec<u8>>,
        notes: &mut Notes,
        need: ScopeNeed,
        still_granted: &(dyn Fn() -> bool + Sync),
    ) -> Result<Value, ScriptFault> {
        let left = call.alive()?;
        let limits = call.grants.limits.clamped();
        let wanted = Duration::from_millis(timeout_ms.unwrap_or(limits.timeout_ms));
        let timeout = wanted.min(left);
        let per_stream = (limits.max_output_kib as usize * 1024).min(MAX_STREAM);
        let plan = Plan {
            program: prepared.program.clone(),
            args: prepared.args.clone(),
            env: prepared.env.clone(),
            cwd: prepared.cwd.clone(),
            timeout,
            max_stdout: per_stream,
            remote: None,
        };
        let broker = &self.broker;
        let interrupt = call.interrupt;
        let stop = || -> Option<Stop> {
            if interrupt.is_some_and(|i| i.is_aborted() || i.host_exited()) {
                return Some(Stop::Cancelled);
            }
            let ceiling = broker.scripts_grant(plugin).is_some_and(|c| match need {
                ScopeNeed::Process => c.process,
                ScopeNeed::Shell => c.shell,
            });
            (!ceiling || !still_granted()).then_some(Stop::Revoked)
        };
        let deadline = Instant::now() + timeout;
        let give_up = || stop().is_some() || Instant::now() >= deadline;
        let _slot = RunSlot::acquire(self.tools.slots(), plugin.as_str(), false, &give_up)
            .ok_or_else(|| match stop() {
                Some(Stop::Revoked) => fault(
                    codes::NOT_GRANTED,
                    "the grant was revoked while the run waited",
                ),
                Some(Stop::Cancelled) => fault(codes::CANCELLED, "the run was stopped"),
                None => fault(codes::TIMED_OUT, "no process slot became free in time"),
            })?;
        // The folder is checked again right before the start.
        if let Some(expected) = &prepared.cwd_check
            && !canonical(&prepared.cwd).is_ok_and(|real| same_path(&real, expected))
        {
            return Err(fault(
                codes::OUTSIDE_SCOPE,
                "the working folder changed before the start",
            ));
        }
        let started = Instant::now();
        let finished = runner::execute_with(&plan, Some(deadline), &stop, stdin, per_stream);
        let mut result = match finished {
            Ok(finished) => ProcessResult {
                ok: finished.output.success(),
                exit_code: finished.output.code,
                signal: finished.output.code.is_none().then(|| "signal".to_owned()),
                stdout: finished.output.stdout,
                stderr: finished.output.stderr,
                truncated: finished.output.stdout_truncated || finished.output.stderr_truncated,
                duration_ms: millis(finished.elapsed),
                timed_out: false,
                cancelled: false,
                now_us: now_us(),
            },
            Err(d) if d.code == runner_codes::TIMED_OUT => ProcessResult {
                timed_out: true,
                duration_ms: millis(started.elapsed()),
                now_us: now_us(),
                ..ProcessResult::default()
            },
            Err(d) if d.code == runner_codes::CANCELLED => {
                return Err(fault(
                    codes::CANCELLED,
                    "the run was stopped; so was the program",
                ));
            }
            Err(d) if d.code == runner_codes::REVOKED => {
                return Err(fault(
                    codes::NOT_GRANTED,
                    "the grant was revoked; the program was stopped",
                ));
            }
            Err(d) if d.code == runner_codes::NOT_FOUND => {
                return Err(fault(
                    codes::COMMAND_NOT_FOUND,
                    "the program could not be started: not found",
                ));
            }
            Err(d) => return Err(denial(&d)),
        };
        // Credentials in URLs (`https://user:token@…`) never reach the
        // script, as with tool.run.
        result.stdout = runner::redact_userinfo(&result.stdout);
        result.stderr = runner::redact_userinfo(&result.stderr);
        cap_together(&mut result);
        fit(&mut result);
        notes.add(
            "exit",
            result
                .exit_code
                .map_or_else(|| "none".into(), |c| c.to_string()),
        );
        notes.add("stdoutBytes", result.stdout.len().to_string());
        notes.add("stderrBytes", result.stderr.len().to_string());
        to_value(&result)
    }
}

/// Which sub-ceiling a run needs.
#[derive(Clone, Copy)]
enum ScopeNeed {
    Process,
    Shell,
}

fn granted_shell(grants: &ScriptGrants, shell: &str) -> bool {
    grants
        .shell
        .as_ref()
        .is_some_and(|s| s.shells.iter().any(|g| g == shell))
}

fn millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// Cuts the longer stream until both together are at most
/// [`MAX_OUTPUT_TOTAL`] bytes.
fn cap_together(result: &mut ProcessResult) {
    while result.stdout.len() + result.stderr.len() > MAX_OUTPUT_TOTAL {
        let over = result.stdout.len() + result.stderr.len() - MAX_OUTPUT_TOTAL;
        let longer = if result.stdout.len() >= result.stderr.len() {
            &mut result.stdout
        } else {
            &mut result.stderr
        };
        let mut end = longer.len().saturating_sub(over);
        while !longer.is_char_boundary(end) {
            end -= 1;
        }
        longer.truncate(end);
        result.truncated = true;
    }
}

/// Shortens the streams until the result's JSON fits its reply (text
/// that JSON escapes heavily is cut further).
fn fit(result: &mut ProcessResult) {
    let size = |r: &ProcessResult| serde_json::to_vec(r).map_or(usize::MAX, |j| j.len());
    loop {
        let now = size(result);
        if now <= REPLY_ROOM {
            return;
        }
        let longer = if result.stdout.len() >= result.stderr.len() {
            &mut result.stdout
        } else {
            &mut result.stderr
        };
        // At least the overflow, at most half, per round.
        let cut = (now - REPLY_ROOM).max(1).min(longer.len().div_ceil(2));
        let mut end = longer.len().saturating_sub(cut);
        while !longer.is_char_boundary(end) {
            end -= 1;
        }
        longer.truncate(end);
        result.truncated = true;
    }
}

/// Adds the script's environment entries: only with the grant, only valid
/// names the denylist does not refuse; each replaces the base's.
fn script_env(
    env: &mut Vec<(String, OsString)>,
    entries: &std::collections::BTreeMap<String, String>,
    allowed: bool,
) -> Result<(), ScriptFault> {
    if entries.is_empty() {
        return Ok(());
    }
    if !allowed {
        return Err(fault(
            codes::NOT_GRANTED,
            "the script may not set environment variables for this command (grant: env allowed)",
        ));
    }
    for (name, value) in entries {
        if !is_env_name(name) || env_denied(name) || env_denied_here(name) || value.contains('\0') {
            return Err(fault(
                codes::NOT_GRANTED,
                format!("the variable {name} may never be set by a script"),
            ));
        }
        env.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
        env.push((name.clone(), value.into()));
    }
    Ok(())
}

/// The working folder of a run under `rule`, canonical, a directory, and
/// (but `any`) inside a scope; with the path to check again before the
/// start, and the folder above its scope's root (git's ceiling). Without
/// `cwd`: an empty folder of the agent's.
fn cwd(
    call: &Call<'_>,
    project: Option<&RunProject>,
    rule: ProcessCwd,
    cwd: Option<&PathArg>,
) -> Result<(PathBuf, Option<PathBuf>, Option<PathBuf>), ScriptFault> {
    let Some(path) = cwd else {
        return runner::neutral_directory()
            .map(|dir| (dir, None, None))
            .map_err(|d| denial(&d));
    };
    // The scopes the rule allows, at read access: the script's own, or
    // every repository of the run's project.
    let repositories = ScriptGrants {
        repositories_paths: call.grants.repositories_paths,
        fs: vec![FsScope {
            root: FsRoot::Repository("*".into()),
            access: FsAccess::Read,
            root_destructive: false,
        }],
        ..ScriptGrants::default()
    };
    let target = match rule {
        ProcessCwd::Repositories => resolve(
            path,
            &repositories,
            project,
            FsAccess::Read,
            &call.run.protected,
        )?,
        ProcessCwd::FsScopes => resolve(
            path,
            &call.grants,
            project,
            FsAccess::Read,
            &call.run.protected,
        )?,
        ProcessCwd::Any => match &path.path {
            Some(absolute) => {
                let real = canonical(std::path::Path::new(absolute))
                    .map_err(|_| fault(codes::NOT_FOUND, "the working folder does not exist"))?;
                if call.run.protected.forbids(&real) || call.run.protected.forbids_identity(&real) {
                    return Err(fault(
                        codes::PROTECTED_PATH,
                        "DeckPilot's own folders are never a working folder",
                    ));
                }
                if !real.is_dir() {
                    return Err(fault(
                        codes::INVALID_ARGUMENT,
                        "the working folder is not a folder",
                    ));
                }
                // No scope to stop at: git finds the folder's repository.
                return Ok((real.clone(), Some(real), None));
            }
            None => resolve(
                path,
                &repositories,
                project,
                FsAccess::Read,
                &call.run.protected,
            )?,
        },
    };
    // Every folder of the way, by its identity on the handle; the path the
    // program starts in must be that very folder (the spawn takes a path:
    // `fchdir` would need `unsafe`, see the module doc).
    let _guard = super::fs::Guard::enter(&call.run.protected);
    let opened = super::fs::open_follow(&target.root, &target.rel, false, &target.shown())?;
    let real = canonical(&target.absolute()).map_err(|_| {
        fault(
            codes::NOT_FOUND,
            format!("“{}” does not exist", target.shown()),
        )
    })?;
    if !starts_with(&real, &target.scope.root) {
        return Err(fault(
            codes::SYMLINK_ESCAPE,
            format!(
                "“{}” leads outside its scope through a link",
                target.shown()
            ),
        ));
    }
    if !real.is_dir() {
        return Err(fault(
            codes::INVALID_ARGUMENT,
            format!("“{}” is not a folder", target.shown()),
        ));
    }
    if super::fs::protected::handle_id(&opened) != super::fs::protected::folder_id(&real) {
        return Err(fault(
            codes::OUTSIDE_SCOPE,
            format!("“{}” changed while it was checked", target.shown()),
        ));
    }
    let ceiling = target.scope.root.parent().map(Path::to_path_buf);
    Ok((real.clone(), Some(real), ceiling))
}

#[cfg(unix)]
fn runnable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn runnable(_path: &Path) -> bool {
    true
}

#[cfg(test)]
mod tests;
