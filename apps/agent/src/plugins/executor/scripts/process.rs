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
    FsAccess, FsRoot, FsScope, ProcessCwd, ProcessEnv, ProcessGrant, ScriptGrants,
};
use deckpilot_plugin_abi::privileged::Tool;
use deckpilot_plugin_abi::scripts::{
    PathArg, ProcessResult, ProcessRun, ScriptFault, ShellRun, ShellsAvailable, codes, env_denied,
    is_batch, is_env_name,
};
use deckpilot_plugin_broker::PluginId;
use serde_json::Value;

use super::fs::protected::{same_path, starts_with};
use super::fs::resolve::resolve;
use super::{Call, Notes, RunProject, fault, now_us, to_value};
use crate::plugins::executor::Executor;
use crate::plugins::tools::{self as runner, Denial, Plan, RunSlot, Stop, codes as runner_codes};

/// Output kept per stream, at most.
const MAX_STREAM: usize = 512 << 10;
/// Room the JSON of a process result may take (the 1 MiB reply cap, less
/// the envelope).
const REPLY_ROOM: usize = (1 << 20) - 4096;

/// git's global options (before the subcommand) a script may not pass
/// without `unrestrictedArgs`: they set configuration or point git at
/// another repository, work tree or set of programs. `-c` and `-C` are
/// refused in every spelling there.
const GIT_GLOBAL_DENIED: &[&str] = &[
    "--config-env",
    "--exec-path",
    "--git-dir",
    "--work-tree",
    "--namespace",
    "--super-prefix",
    "--list-cmds",
];

/// Options of any subcommand that choose a program git runs or the
/// configuration of a new repository.
const GIT_DENIED: &[&str] = &[
    "--upload-pack",
    "--receive-pack",
    "--exec",
    "--template",
    "--separate-git-dir",
    "--config",
];

/// Whether `args` (of git) hold a denied option, or name the `ext::` or
/// `fd::` transport.
fn git_denied(args: &[String]) -> bool {
    let subcommand = args.iter().position(|a| !a.starts_with('-'));
    let sub = subcommand.map(|i| args[i].to_ascii_lowercase());
    args.iter().enumerate().any(|(i, arg)| {
        let lower = arg.to_ascii_lowercase();
        let is = |opt: &str| lower == opt || lower.starts_with(&format!("{opt}="));
        let short = |c: &str| arg.starts_with(c) && !arg.starts_with("--");
        let global = subcommand.is_none_or(|s| i < s);
        lower.contains("ext::")
            || lower.starts_with("fd::")
            || (global && (short("-c") || short("-C") || GIT_GLOBAL_DENIED.iter().any(|o| is(o))))
            || GIT_DENIED.iter().any(|o| is(o))
            // `-u <program>`: the upload-pack of these subcommands.
            || (matches!(sub.as_deref(), Some("clone" | "fetch" | "pull" | "ls-remote" | "archive"))
                && short("-u"))
            // `clone -c key=value` sets the new repository's configuration.
            || (sub.as_deref() == Some("clone") && i > subcommand.unwrap_or(0) && short("-c"))
    })
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
        if cfg!(windows) && Path::new(name).extension().is_none() {
            names = [".exe", ".com"]
                .iter()
                .map(|e| format!("{name}{e}"))
                .collect();
        }
        self.tools
            .path_entries()
            .into_iter()
            .flat_map(|dir| names.iter().map(move |n| dir.join(n)))
            .find(|candidate| candidate.is_file() && runnable(candidate))
            .and_then(|found| std::fs::canonicalize(found).ok())
    }

    /// The program a granted command runs.
    fn program(&self, grant: &ProcessGrant) -> Result<PathBuf, ScriptFault> {
        let not_found = |code: &str, what: &str| ScriptFault {
            command: Some(grant.command.clone()),
            ..fault(code, format!("{what} was not found on this computer"))
        };
        let path = match grant.command.as_str() {
            "git" | "docker" => {
                let (tool, code) = if grant.command == "git" {
                    (Tool::Git, codes::GIT_UNAVAILABLE)
                } else {
                    (Tool::Docker, codes::DOCKER_UNAVAILABLE)
                };
                let path = self
                    .tools
                    .executable(tool)
                    .map_err(|_| not_found(code, &grant.command))?;
                std::fs::canonicalize(path).map_err(|_| not_found(code, &grant.command))?
            }
            command if Path::new(command).is_absolute() => {
                let real = std::fs::canonicalize(command)
                    .map_err(|_| not_found(codes::COMMAND_NOT_FOUND, command))?;
                if !real.is_file() || !runnable(&real) {
                    return Err(not_found(codes::COMMAND_NOT_FOUND, command));
                }
                real
            }
            command => self
                .lookup(command)
                .ok_or_else(|| not_found(codes::COMMAND_NOT_FOUND, command))?,
        };
        if is_batch(&path.to_string_lossy()) {
            return Err(ScriptFault {
                command: Some(grant.command.clone()),
                ..fault(
                    codes::NOT_GRANTED,
                    "a batch file runs only through shell.run with cmd",
                )
            });
        }
        Ok(path)
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
        let program = self.program(&grant)?;
        notes.add("command", stem(&program));
        notes.add("args", request.args.len().to_string());
        let git = stem(&program) == "git";
        if git && !grant.unrestricted_args && git_denied(&request.args) {
            return Err(ScriptFault {
                command: Some(request.command.clone()),
                ..fault(
                    codes::NOT_GRANTED,
                    "this git option changes git's configuration, programs or repository; \
                     it needs unrestrictedArgs",
                )
            });
        }
        let project = call.run.host.project(None)?;
        let (cwd, cwd_check) = cwd(call, project.as_ref(), grant.cwd, request.cwd.as_ref())?;
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
        if git {
            for (name, value) in [
                ("GIT_TERMINAL_PROMPT", "0"),
                ("GCM_INTERACTIVE", "never"),
                ("GIT_ASKPASS", ""),
            ] {
                env.push((name.into(), value.into()));
            }
        }
        script_env(&mut env, &request.env, grant.env == ProcessEnv::Allowed)?;
        let prepared = Prepared {
            program,
            args,
            env,
            cwd,
            cwd_check,
        };
        let stdin = request.stdin.as_ref().map(|s| s.as_bytes().to_vec());
        let script = call.script.to_owned();
        let command = grant.command.clone();
        let host = call.run.host.clone();
        let still_granted = move || {
            host.grants(&script)
                .filter(|g| g.validate().is_ok())
                .is_some_and(|g| g.process.iter().any(|p| p.command == command))
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
        let project = call.run.host.project(None)?;
        // A shell runs in a repository of the project or a scope of the
        // script, never anywhere else.
        let rule = if request.cwd.as_ref().is_some_and(|c| c.repository.is_some()) {
            ProcessCwd::Repositories
        } else {
            ProcessCwd::FsScopes
        };
        let (cwd, cwd_check) = cwd(call, project.as_ref(), rule, request.cwd.as_ref())?;
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
        let _slot =
            RunSlot::acquire(plugin.as_str(), false, &give_up).ok_or_else(|| match stop() {
                Some(Stop::Revoked) => fault(
                    codes::NOT_GRANTED,
                    "the grant was revoked while the run waited",
                ),
                Some(Stop::Cancelled) => fault(codes::CANCELLED, "the run was stopped"),
                None => fault(codes::TIMED_OUT, "no process slot became free in time"),
            })?;
        // The folder is checked again right before the start.
        if let Some(expected) = &prepared.cwd_check
            && !std::fs::canonicalize(&prepared.cwd).is_ok_and(|real| same_path(&real, expected))
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

/// Shortens the streams until the result's JSON fits its reply.
fn fit(result: &mut ProcessResult) {
    let size = |r: &ProcessResult| serde_json::to_vec(r).map_or(usize::MAX, |j| j.len());
    while size(result) > REPLY_ROOM {
        let longer = if result.stdout.len() >= result.stderr.len() {
            &mut result.stdout
        } else {
            &mut result.stderr
        };
        let mut end = longer.len() / 2;
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
        if !is_env_name(name) || env_denied(name) || value.contains('\0') {
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
/// start. Without `cwd`: an empty folder of the agent's.
fn cwd(
    call: &Call<'_>,
    project: Option<&RunProject>,
    rule: ProcessCwd,
    cwd: Option<&PathArg>,
) -> Result<(PathBuf, Option<PathBuf>), ScriptFault> {
    let Some(path) = cwd else {
        return runner::neutral_directory()
            .map(|dir| (dir, None))
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
        ProcessCwd::Repositories => resolve(path, &repositories, project, FsAccess::Read)?,
        ProcessCwd::FsScopes => resolve(path, &call.grants, project, FsAccess::Read)?,
        ProcessCwd::Any => match &path.path {
            Some(absolute) => {
                let real = std::fs::canonicalize(absolute)
                    .map_err(|_| fault(codes::NOT_FOUND, "the working folder does not exist"))?;
                if !real.is_dir() {
                    return Err(fault(
                        codes::INVALID_ARGUMENT,
                        "the working folder is not a folder",
                    ));
                }
                return Ok((real.clone(), Some(real)));
            }
            None => resolve(path, &repositories, project, FsAccess::Read)?,
        },
    };
    let real = std::fs::canonicalize(target.absolute()).map_err(|_| {
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
    Ok((real.clone(), Some(real)))
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
