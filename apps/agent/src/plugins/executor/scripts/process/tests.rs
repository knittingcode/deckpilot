//! `process.run`, `shell.run` and `shell.available` with harmless
//! commands only: shell-script stand-ins for `git` and other tools in a
//! temporary `bin`, `sh` for the shell. Never docker compose, never git
//! pull, never a real repository. Unix only (the stand-ins are scripts).
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use deckpilot_core::script_grants::{
    FsAccess, FsRoot, FsScope, ProcessCwd, ProcessEnv, ProcessGrant, ScriptGrants, ScriptLimits,
    ShellGrant,
};
use deckpilot_plugin_abi::capability::{RunContext, RunTrigger};
use deckpilot_plugin_supervisor::{AbortSignal, CallInterrupt, CapabilityExecutor as _};
use serde_json::{Value, json};

use super::super::tests::support::*;
use super::git_denied;
use crate::plugins::executor::Executor;
use crate::plugins::tools::ToolHost;

/// A temporary `bin` with stand-ins, first on the `PATH` the executor uses.
fn bin(f: &Fixture) -> PathBuf {
    let bin = f.path("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let tool = |name: &str, body: &str| {
        let path = bin.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    // Prints its arguments, one per line, and its folder.
    tool("echoargs", r#"for a in "$@"; do echo "$a"; done; pwd"#);
    tool("git", r#"for a in "$@"; do echo "$a"; done"#);
    tool("fail", "echo oops >&2; exit 3");
    tool("readin", "cat");
    tool("showenv", r#"echo "CI=${CI:-} TOKEN=${SECRET_TOKEN:-}""#);
    tool("slow", "sleep 30");
    tool("flood", "yes aaaaaaaaaaaaaaa | head -c 2000000");
    bin
}

fn executor(f: &Fixture, bin: &std::path::Path) -> Executor {
    let path = format!("{}:/usr/bin:/bin", bin.display());
    f.executor(RunContext::direct(RunTrigger::Key))
        .with_tools(ToolHost::for_test().with_environment(&[("PATH", &path)]))
}

fn loaded(f: &Fixture, bin: &std::path::Path) -> Executor {
    let mut e = executor(f, bin);
    ok(&call(
        &mut e,
        &json!({"op":"script.load","args":{"script": SCRIPT}}),
    ));
    e
}

fn grant(command: &str, cwd: ProcessCwd, env: ProcessEnv) -> ProcessGrant {
    ProcessGrant {
        command: command.into(),
        cwd,
        env,
        unrestricted_args: false,
    }
}

fn grants(process: Vec<ProcessGrant>) -> ScriptGrants {
    ScriptGrants {
        process,
        fs: vec![FsScope {
            root: FsRoot::Repository("cms".into()),
            access: FsAccess::Read,
            root_destructive: false,
        }],
        ..ScriptGrants::default()
    }
}

fn run(command: &str, args: &[&str]) -> Value {
    json!({"op":"process.run","args":{"command": command, "args": args, "env": {}}})
}

fn setup(process: Vec<ProcessGrant>) -> (Fixture, PathBuf) {
    let f = Fixture::new();
    f.project(&[("cms", "w/cms")]);
    let bin = bin(&f);
    f.grant(grants(process));
    (f, bin)
}

#[test]
fn only_a_granted_command_runs_and_gets_its_arguments_unchanged() {
    let (f, bin) = setup(vec![grant(
        "echoargs",
        ProcessCwd::Repositories,
        ProcessEnv::None,
    )]);
    let mut e = loaded(&f, &bin);
    assert_eq!(code(&call(&mut e, &run("fail", &[]))), "notGranted");
    let r = ok(&call(
        &mut e,
        &run("echoargs", &["a b", "$HOME", "*", "; rm -rf x"]),
    ));
    assert_eq!(r["ok"], true);
    assert_eq!(r["exitCode"], 0);
    let lines: Vec<&str> = r["stdout"].as_str().unwrap().lines().collect();
    assert_eq!(
        &lines[..4],
        ["a b", "$HOME", "*", "; rm -rf x"],
        "no shell, no expansion"
    );
    assert!(r["nowUs"].as_u64().unwrap() > 0);
}

#[test]
fn a_non_zero_exit_is_a_result_and_a_missing_command_a_fault() {
    let (f, bin) = setup(vec![
        grant("fail", ProcessCwd::Repositories, ProcessEnv::None),
        grant(
            "nonexistent-tool",
            ProcessCwd::Repositories,
            ProcessEnv::None,
        ),
    ]);
    let mut e = loaded(&f, &bin);
    let r = ok(&call(&mut e, &run("fail", &[])));
    assert_eq!(
        (r["ok"].as_bool(), r["exitCode"].as_i64()),
        (Some(false), Some(3))
    );
    assert_eq!(r["stderr"], "oops\n");
    assert_eq!(
        code(&call(&mut e, &run("nonexistent-tool", &[]))),
        "commandNotFound"
    );
}

#[test]
fn an_absolute_grant_runs_exactly_that_file() {
    let f = Fixture::new();
    f.project(&[("cms", "w/cms")]);
    let bin = bin(&f);
    let absolute = bin.join("echoargs").to_string_lossy().into_owned();
    f.grant(grants(vec![grant(
        &absolute,
        ProcessCwd::Any,
        ProcessEnv::None,
    )]));
    let mut e = loaded(&f, &bin);
    assert_eq!(ok(&call(&mut e, &run(&absolute, &["x"])))["ok"], true);
    // The bare name is another grant.
    assert_eq!(code(&call(&mut e, &run("echoargs", &[]))), "notGranted");
}

#[test]
fn stdin_reaches_the_program() {
    let (f, bin) = setup(vec![grant(
        "readin",
        ProcessCwd::Repositories,
        ProcessEnv::None,
    )]);
    let mut e = loaded(&f, &bin);
    let r = ok(&call(
        &mut e,
        &json!({"op":"process.run","args":{"command":"readin","args":[],"env":{},"stdin":"hello\n"}}),
    ));
    assert_eq!(r["stdout"], "hello\n");
}

#[test]
fn the_environment_needs_the_grant_and_never_takes_denied_names() {
    let (f, bin) = setup(vec![grant(
        "showenv",
        ProcessCwd::Repositories,
        ProcessEnv::None,
    )]);
    let mut e = loaded(&f, &bin);
    let with =
        |env: Value| json!({"op":"process.run","args":{"command":"showenv","args":[],"env": env}});
    assert_eq!(code(&call(&mut e, &with(json!({"CI":"1"})))), "notGranted");
    f.grant(grants(vec![grant(
        "showenv",
        ProcessCwd::Repositories,
        ProcessEnv::Allowed,
    )]));
    let r = ok(&call(&mut e, &with(json!({"CI":"1"}))));
    assert_eq!(
        r["stdout"], "CI=1 TOKEN=\n",
        "nothing of the agent's environment but the base"
    );
    // Refused already by the broker (the ABI's validation).
    assert_eq!(
        code(&call(&mut e, &with(json!({"LD_PRELOAD":"/x.so"})))),
        "call:BadRequest"
    );
    assert_eq!(
        code(&call(&mut e, &with(json!({"PATH":"/tmp"})))),
        "call:BadRequest"
    );
}

#[test]
fn the_working_folder_follows_the_grant() {
    let (f, bin) = setup(vec![grant(
        "echoargs",
        ProcessCwd::Repositories,
        ProcessEnv::None,
    )]);
    std::fs::create_dir_all(f.path("w/cms/src")).unwrap();
    std::fs::create_dir_all(f.path("w/elsewhere")).unwrap();
    let mut e = loaded(&f, &bin);
    let at = |cwd: Value| json!({"op":"process.run","args":{"command":"echoargs","args":[],"env":{},"cwd": cwd}});
    let r = ok(&call(
        &mut e,
        &at(json!({"repository":"cms","relative":"src"})),
    ));
    let pwd = r["stdout"].as_str().unwrap().trim();
    assert_eq!(
        PathBuf::from(pwd),
        std::fs::canonicalize(f.path("w/cms/src")).unwrap()
    );
    let outside = json!({"path": f.path("w/elsewhere").to_string_lossy()});
    assert_eq!(code(&call(&mut e, &at(outside))), "outsideScope");
    // A link inside the repository that leads out is refused.
    std::os::unix::fs::symlink(f.path("w/elsewhere"), f.path("w/cms/out")).unwrap();
    assert_eq!(
        code(&call(
            &mut e,
            &at(json!({"repository":"cms","relative":"out"}))
        )),
        "symlinkEscape"
    );
    // Without cwd: an empty folder of the agent's, never the repository.
    let r = ok(&call(&mut e, &run("echoargs", &[])));
    assert!(!r["stdout"].as_str().unwrap().contains("/w/cms"));
}

#[test]
fn a_timeout_stops_the_program() {
    let (f, bin) = setup(vec![grant(
        "slow",
        ProcessCwd::Repositories,
        ProcessEnv::None,
    )]);
    let mut e = loaded(&f, &bin);
    let started = Instant::now();
    let r = ok(&call(
        &mut e,
        &json!({"op":"process.run","args":{"command":"slow","args":[],"env":{},"timeoutMs":300}}),
    ));
    assert_eq!(
        (r["timedOut"].as_bool(), r["ok"].as_bool()),
        (Some(true), Some(false))
    );
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    // The script's own limit bounds a run without timeoutMs.
    f.grant(ScriptGrants {
        limits: ScriptLimits {
            timeout_ms: 300,
            ..ScriptLimits::default()
        },
        ..grants(vec![grant(
            "slow",
            ProcessCwd::Repositories,
            ProcessEnv::None,
        )])
    });
    let started = Instant::now();
    let r = ok(&call(&mut e, &run("slow", &[])));
    assert_eq!(r["timedOut"], true);
    assert!(started.elapsed() < Duration::from_secs(10));
}

/// One call as the supervisor makes it, with an interrupt.
fn call_with(
    e: &mut Executor,
    request: &Value,
    interrupt: &CallInterrupt,
) -> deckpilot_plugin_abi::CallResult {
    let plugin = deckpilot_plugin_broker::PluginId::new(PLUGIN).unwrap();
    let request: deckpilot_plugin_abi::CapabilityRequest =
        serde_json::from_value(request.clone()).unwrap();
    let session = e.broker.session(plugin.clone());
    match session.authorize_in(&request, e.run_context()) {
        deckpilot_plugin_broker::Decision::Allow(allowed) => {
            e.execute_interruptible(&plugin, &allowed, interrupt)
        }
        deckpilot_plugin_broker::Decision::Deny(r) => {
            deckpilot_plugin_abi::CallResult::error(r.call_error_code(), r.to_string())
        }
    }
}

#[test]
fn stop_cancels_the_program() {
    let (f, bin) = setup(vec![grant(
        "slow",
        ProcessCwd::Repositories,
        ProcessEnv::None,
    )]);
    let mut e = loaded(&f, &bin);
    let abort = AbortSignal::new();
    let interrupt = CallInterrupt::new(
        Some(abort.clone()),
        Instant::now() + Duration::from_secs(60),
    );
    let stopper = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        abort.abort();
    });
    let started = Instant::now();
    let r = call_with(&mut e, &run("slow", &[]), &interrupt);
    stopper.join().unwrap();
    assert_eq!(code(&r), "cancelled");
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[test]
fn revoking_the_grant_stops_the_program() {
    let (f, bin) = setup(vec![grant(
        "slow",
        ProcessCwd::Repositories,
        ProcessEnv::None,
    )]);
    let mut e = loaded(&f, &bin);
    let host = f.host.clone();
    let revoker = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        *host.grants.lock().unwrap() = Some(ScriptGrants::default());
    });
    let started = Instant::now();
    let r = call(&mut e, &run("slow", &[]));
    revoker.join().unwrap();
    assert_eq!(code(&r), "notGranted");
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[test]
fn output_is_capped_and_the_reply_fits() {
    let (f, bin) = setup(vec![grant(
        "flood",
        ProcessCwd::Repositories,
        ProcessEnv::None,
    )]);
    let mut e = loaded(&f, &bin);
    let r = ok(&call(&mut e, &run("flood", &[])));
    assert_eq!(r["truncated"], true);
    let stdout = r["stdout"].as_str().unwrap().len();
    assert!(stdout <= 256 << 10, "{stdout}");
    assert!(serde_json::to_vec(&r).unwrap().len() < 1 << 20);
}

#[test]
fn git_options_that_change_its_programs_are_refused() {
    let (f, bin) = setup(vec![grant(
        "git",
        ProcessCwd::Repositories,
        ProcessEnv::None,
    )]);
    let mut e = f.executor(RunContext::direct(RunTrigger::Key)).with_tools(
        ToolHost::for_test()
            .with_executable(deckpilot_plugin_abi::privileged::Tool::Git, bin.join("git"))
            .with_environment(&[("PATH", "/usr/bin:/bin")]),
    );
    ok(&call(
        &mut e,
        &json!({"op":"script.load","args":{"script": SCRIPT}}),
    ));
    for args in [
        vec!["-c", "core.sshCommand=touch /tmp/x", "fetch"],
        vec!["-ccore.pager=x", "log"],
        vec!["--git-dir=/elsewhere", "status"],
        vec!["-C", "/elsewhere", "status"],
        vec!["fetch", "--upload-pack=evil", "origin"],
        vec!["clone", "-u", "evil", "x"],
        vec!["clone", "-c", "core.hooksPath=/x", "x"],
        vec!["clone", "ext::sh -c touch% /tmp/x", "y"],
        vec!["archive", "--exec=evil", "--remote=x"],
        // The security review's bypasses of #472.
        vec![
            "clone",
            "--conf",
            "core.sshCommand=touch x; false",
            "ssh://h/x",
            "d",
        ],
        vec!["fetch", "--upload-p=evil", "origin"],
        vec![
            "--attr-source",
            "HEAD",
            "clone",
            "-c",
            "core.sshCommand=x",
            "a",
            "b",
        ],
        vec!["rebase", "-x", "touch x", "HEAD~1"],
        vec!["rebase", "--exec=touch x", "HEAD~1"],
        vec!["config", "alias.x", "!touch x"],
        vec!["x"],
        vec!["diff", "--output=/tmp/out"],
        vec!["diff", "--no-index", "/etc/passwd", "/dev/null"],
        vec!["submodule", "foreach", "touch x"],
        vec!["bisect", "run", "touch x"],
        vec!["difftool", "-x", "touch"],
        vec!["merge", "-s", "evil", "x"],
        // The re-review's.
        vec!["rebase", "-qx", "touch x", "HEAD~1"],
        vec!["grep", "-Otouch x", "hello"],
        vec!["grep", "--open-files-in-pager=touch", "hello"],
        vec!["ls-remote", "--u=touch x", "ssh://h/x"],
        vec!["blame", "--contents", "/outside/secret", "--", "f"],
        vec!["commit", "--allow-empty", "-F", "/outside/secret"],
        vec!["commit", "--file=/outside/secret"],
        vec!["tag", "-F", "/outside/secret", "v1"],
        vec!["add", "--pathspec-from-file=/outside/x"],
        vec!["grep", "-f", "/outside/x"],
        vec!["ls-files", "--exclude-from=/outside/x"],
        vec!["pull", "-Xours"],
        vec!["blame", "--ignore-revs-file=/outside/secret", "f"],
        vec!["diff", "-O/outside/order"],
    ] {
        assert_eq!(
            code(&call(&mut e, &run("git", &args))),
            "notGranted",
            "{args:?}"
        );
    }
    // Ordinary use passes, after the host's hardening.
    let r = ok(&call(&mut e, &run("git", &["status", "-uno"])));
    let out = r["stdout"].as_str().unwrap();
    assert!(
        out.contains("core.fsmonitor=false") && out.ends_with("status\n-uno\n"),
        "{out}"
    );
    let r = ok(&call(&mut e, &run("git", &["grep", "-c", "x"])));
    assert!(r["stdout"].as_str().unwrap().ends_with("grep\n-c\nx\n"));
    // `unrestrictedArgs` lifts it.
    f.grant(grants(vec![ProcessGrant {
        unrestricted_args: true,
        ..grant("git", ProcessCwd::Repositories, ProcessEnv::None)
    }]));
    assert_eq!(
        ok(&call(&mut e, &run("git", &["-c", "a.b=c", "status"])))["ok"],
        true
    );
}

#[test]
fn the_git_denylist_reads_positions() {
    let v = |a: &[&str]| a.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    assert!(git_denied(&v(&["-c", "x=y", "status"])));
    assert!(
        !git_denied(&v(&["commit", "-c", "HEAD"])),
        "reuse a message, after the subcommand"
    );
    assert!(!git_denied(&v(&["status", "-uall"])));
    assert!(git_denied(&v(&["fetch", "-u", "x"])));
    assert!(git_denied(&v(&["push", "--receive-pack=x"])));
    assert!(
        git_denied(&v(&["push", "--receive-p", "x"])),
        "a prefix git accepts"
    );
    assert!(!git_denied(&v(&[
        "log",
        "--oneline",
        "--stat",
        "--topo-order"
    ])));
    assert!(!git_denied(&v(&["branch", "--contains", "HEAD"])));
    assert!(!git_denied(&v(&["rebase", "--continue"])));
    assert!(!git_denied(&v(&["diff", "--exit-code"])));
    assert!(!git_denied(&v(&["pull", "--ff-only"])));
    assert!(!git_denied(&v(&["grep", "--files-with-matches", "x"])));
    assert!(!git_denied(&v(&["log", "--exclude=refs/x", "--all"])));
    assert!(!git_denied(&v(&["diff", "--output-indicator-new=+"])));
}

#[test]
fn a_shell_needs_its_own_grant_and_takes_the_script_as_one_argument() {
    let (f, bin) = setup(vec![]);
    let mut e = loaded(&f, &bin);
    let shell =
        |script: &str| json!({"op":"shell.run","args":{"shell":"sh","script": script,"env":{}}});
    assert_eq!(code(&call(&mut e, &shell("echo hi"))), "notGranted");
    assert_eq!(
        ok(&call(&mut e, &json!({"op":"shell.available","args":{}})))["shells"],
        json!([])
    );
    f.grant(ScriptGrants {
        shell: Some(ShellGrant {
            shells: vec!["sh".into(), "powershell".into()],
        }),
        ..grants(vec![])
    });
    let r = ok(&call(&mut e, &shell("echo \"$0 says hi\"")));
    assert_eq!(r["ok"], true);
    assert!(r["stdout"].as_str().unwrap().ends_with("says hi\n"));
    assert_eq!(
        ok(&call(&mut e, &json!({"op":"shell.available","args":{}})))["shells"],
        json!(["sh"])
    );
    // A process grant is not a shell grant, nor the reverse.
    assert_eq!(
        code(&call(&mut e, &run("sh", &["-c", "echo x"]))),
        "notGranted"
    );
}

#[test]
fn processes_are_refused_without_the_ceiling() {
    let (f, bin) = setup(vec![grant(
        "echoargs",
        ProcessCwd::Repositories,
        ProcessEnv::None,
    )]);
    f.broker
        .set_grants(
            &deckpilot_plugin_broker::PluginId::new(PLUGIN).unwrap(),
            &deckpilot_plugin_broker::ApprovedPermissions::from_json(
                r#"[{"kind":"scripts","fs":true}]"#,
            )
            .unwrap(),
        )
        .unwrap();
    let mut e = loaded(&f, &bin);
    assert_eq!(
        code(&call(&mut e, &run("echoargs", &[]))),
        "call:NotPermitted"
    );
}

#[test]
fn the_host_s_git_environment_wins_and_stops_at_the_scope() {
    let f = Fixture::new();
    f.project(&[("cms", "w/cms")]);
    let bin = bin(&f);
    // A git that prints the variables the host sets.
    let git = bin.join("git");
    std::fs::write(
        &git,
        "#!/bin/sh\necho \"GCM=$GCM_INTERACTIVE CEIL=$GIT_CEILING_DIRECTORIES PROMPT=$GIT_TERMINAL_PROMPT\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&git, std::fs::Permissions::from_mode(0o755)).unwrap();
    f.grant(grants(vec![grant(
        "git",
        ProcessCwd::Repositories,
        ProcessEnv::Allowed,
    )]));
    let mut e = f.executor(RunContext::direct(RunTrigger::Key)).with_tools(
        ToolHost::for_test()
            .with_executable(deckpilot_plugin_abi::privileged::Tool::Git, git)
            .with_environment(&[("PATH", "/usr/bin:/bin")]),
    );
    ok(&call(
        &mut e,
        &json!({"op":"script.load","args":{"script": SCRIPT}}),
    ));
    let r = call(
        &mut e,
        &json!({"op":"process.run","args":{"command":"git","args":["status"],"env":{"GCM_INTERACTIVE":"always"},"cwd":{"repository":"cms","relative":""}}}),
    );
    assert_eq!(code(&r), "notGranted", "GCM_* is refused");
    let r = ok(&call(
        &mut e,
        &json!({"op":"process.run","args":{"command":"git","args":["status"],"env":{"CI":"1"},"cwd":{"repository":"cms","relative":""}}}),
    ));
    let out = r["stdout"].as_str().unwrap();
    let parent = super::super::fs::protected::canonical(&f.path("w")).unwrap();
    assert!(
        out.contains("GCM=never") && out.contains("PROMPT=0"),
        "{out}"
    );
    assert!(out.contains(&format!("CEIL={}", parent.display())), "{out}");
}

#[test]
fn credentials_in_urls_never_reach_the_script() {
    let (f, bin) = setup(vec![grant(
        "echoargs",
        ProcessCwd::Repositories,
        ProcessEnv::None,
    )]);
    let mut e = loaded(&f, &bin);
    let r = ok(&call(
        &mut e,
        &run("echoargs", &["https://me:s3cret@example.com/x.git"]),
    ));
    let out = r["stdout"].as_str().unwrap();
    assert!(!out.contains("s3cret"), "{out}");
}

#[test]
fn narrowing_the_grant_mid_run_stops_the_program() {
    let (f, bin) = setup(vec![grant(
        "slow",
        ProcessCwd::Repositories,
        ProcessEnv::None,
    )]);
    let mut e = loaded(&f, &bin);
    let host = f.host.clone();
    let narrower = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        let mut grants = host.grants.lock().unwrap();
        grants.as_mut().unwrap().process[0].env = ProcessEnv::Allowed;
    });
    let started = Instant::now();
    let r = call(&mut e, &run("slow", &[]));
    narrower.join().unwrap();
    assert_eq!(code(&r), "notGranted");
    assert!(started.elapsed() < Duration::from_secs(10));
}
