//! The bridge's run operations, the projects, the confirmation token and
//! the checks every call passes (`support` is shared with `fs/tests.rs`).
//! Temporary directories only: never a real user folder.

use deckpilot_core::script_grants::{Destructive, FsAccess, FsRoot, FsScope, ScriptGrants};
use deckpilot_plugin_abi::capability::{RunCause, RunContext, RunTrigger};
use serde_json::json;

use support::*;

pub(crate) mod support {
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use deckpilot_core::script_grants::ScriptGrants;
    use deckpilot_plugin_abi::capability::{RunContext, RunTrigger};
    use deckpilot_plugin_abi::scripts::{ScriptFault, ScriptSettings, codes};
    use deckpilot_plugin_abi::{CallResult, CapabilityRequest};
    use deckpilot_plugin_broker::{Broker, Decision, PluginId};
    use deckpilot_plugin_supervisor::CapabilityExecutor;
    use serde_json::{Value, json};
    use tokio::sync::broadcast;

    use super::super::super::{
        Executor, Opener, OwnVariables, PrivilegedAudit, PrivilegedEntry, PublishedVariables,
        Transport,
    };
    use super::super::fs::protected::canonical;
    use super::super::{
        Protected, Refusal, RunProject, RunRepository, ScriptHost, ScriptRun, StoredScript,
    };

    pub const PLUGIN: &str = "deckpilot.scripts";
    pub const SCRIPT: &str = "6f1c2a8e-4b1d-4c2e-9a77-0d5b1e3f9c10";

    /// The store, as a test sets it: grants and the project may change
    /// between calls (a revocation).
    #[derive(Debug, Default)]
    pub struct FakeHost {
        pub grants: Mutex<Option<ScriptGrants>>,
        pub project: Mutex<Option<RunProject>>,
        pub trusted: Mutex<bool>,
    }

    impl ScriptHost for FakeHost {
        fn grants(&self, script: &str, _loaded: &str) -> Option<ScriptGrants> {
            // Trusted with none (revoked): no grants, the run goes on.
            (script == SCRIPT && *self.trusted.lock().unwrap())
                .then(|| self.grants.lock().unwrap().clone().unwrap_or_default())
        }

        fn load(&self, script: &str) -> Result<StoredScript, Refusal> {
            if script != SCRIPT {
                return Err(ScriptFault::new(codes::SCRIPT_MISSING, "no such script"));
            }
            if !*self.trusted.lock().unwrap() {
                return Err(ScriptFault::new(codes::SCRIPT_NEEDS_REVIEW, "review it"));
            }
            Ok(StoredScript {
                source: "deckpilot.log.info('hi')".into(),
                settings: ScriptSettings {
                    timeout_ms: 60_000,
                    stop_on_error: true,
                    memory_mib: 64,
                },
            })
        }

        fn projects(&self) -> Result<Vec<RunProject>, Refusal> {
            Ok(self.project.lock().unwrap().clone().into_iter().collect())
        }

        fn project(&self, id: Option<&str>) -> Result<Option<RunProject>, Refusal> {
            let project = self.project.lock().unwrap().clone();
            Ok(project.filter(|p| id.is_none_or(|id| id == p.id)))
        }
    }

    /// A temporary folder (canonical: `/var` is `/private/var` on macOS),
    /// never inside the user's home, and an executor of one run.
    pub struct Fixture {
        pub _tmp: tempfile::TempDir,
        pub root: PathBuf,
        pub host: Arc<FakeHost>,
        pub broker: Broker,
        pub audit: Arc<Mutex<Vec<PrivilegedEntry>>>,
        /// The test's protected roots: its own `home` (inside the
        /// temporary folder) and the system's fixed folders.
        pub home: PathBuf,
    }

    impl Fixture {
        pub fn new() -> Self {
            let tmp = tempfile::tempdir().unwrap();
            // One form on every platform (no `\\?\` on Windows), as the
            // agent compares and as a script would name it.
            let root = canonical(tmp.path()).unwrap();
            if let Some(real) = std::env::home_dir()
                && let Ok(real) = std::fs::canonicalize(real)
            {
                assert!(
                    !root.starts_with(&real),
                    "the tests' folder must not be inside the real home ({})",
                    root.display()
                );
            }
            let home = root.join("home");
            std::fs::create_dir(&home).unwrap();
            let broker = Broker::new(
                Arc::new(deckpilot_plugin_broker::MemoryAudit::new(10_000)),
                Arc::new(deckpilot_plugin_broker::ManualClock::default()),
                deckpilot_plugin_broker::RateLimits::default(),
            );
            broker
                .set_grants(
                    &PluginId::new(PLUGIN).unwrap(),
                    &deckpilot_plugin_broker::ApprovedPermissions::from_json(
                        r#"[{"kind":"scripts","fs":true,"process":true,"shell":true}]"#,
                    )
                    .unwrap(),
                )
                .unwrap();
            let host = Arc::new(FakeHost::default());
            *host.trusted.lock().unwrap() = true;
            Self {
                _tmp: tmp,
                root,
                host,
                broker,
                audit: Arc::new(Mutex::new(Vec::new())),
                home,
            }
        }

        /// A path inside the temporary folder (never anything else).
        pub fn path(&self, rel: &str) -> PathBuf {
            let p = self.root.join(rel);
            assert!(p.starts_with(&self.root));
            p
        }

        pub fn grant(&self, grants: ScriptGrants) {
            *self.host.grants.lock().unwrap() = Some(grants);
        }

        pub fn revoke(&self) {
            *self.host.grants.lock().unwrap() = None;
        }

        /// A project with repositories `(key, folder)`; each folder is made
        /// inside the temporary folder.
        pub fn project(&self, repos: &[(&str, &str)]) {
            let repositories = repos
                .iter()
                .enumerate()
                .map(|(i, (key, folder))| {
                    let path = self.path(folder);
                    std::fs::create_dir_all(&path).unwrap();
                    RunRepository {
                        id: format!("r{i}"),
                        key: (*key).into(),
                        name: key.to_uppercase(),
                        description: None,
                        is_default: i == 0,
                        folder: Some(canonical(&path).unwrap()),
                        remote: None,
                        remotes: Vec::new(),
                    }
                })
                .collect();
            *self.host.project.lock().unwrap() = Some(RunProject {
                id: "p1".into(),
                key: "site".into(),
                name: "Site".into(),
                repositories,
            });
        }

        pub fn protected(&self) -> Arc<Protected> {
            let mut roots = vec![self.home.clone()];
            if cfg!(unix) {
                roots.extend(["/", "/usr", "/etc", "/var", "/tmp"].map(PathBuf::from));
            }
            Arc::new(Protected::with_roots(roots).with_private([self.root.join("appdata")]))
        }

        pub fn executor(&self, run: RunContext) -> Executor {
            let (events, _) = broadcast::channel(4);
            let sink = self.audit.clone();
            let audit: PrivilegedAudit = Arc::new(move |entry: &PrivilegedEntry| {
                sink.lock().unwrap().push(entry.clone());
            });
            let script =
                ScriptRun::with_protected(self.host.clone(), Some(SCRIPT.into()), self.protected());
            Executor::new(
                self.broker.clone(),
                "Scripts".into(),
                events,
                Transport::Network,
                OwnVariables {
                    store: PublishedVariables::default(),
                    namespace: "plugins.deckpilot.scripts".into(),
                    generation: 0,
                    declared: Vec::new(),
                },
                Opener::System,
            )
            .with_audit(audit)
            .with_run(run)
            .with_script_run(script)
        }

        /// A key press's executor with its script loaded.
        pub fn loaded(&self) -> Executor {
            self.loaded_in(RunContext::direct(RunTrigger::Key))
        }

        pub fn loaded_in(&self, run: RunContext) -> Executor {
            let mut e = self.executor(run);
            ok(&call(
                &mut e,
                &json!({"op":"script.load","args":{"script": SCRIPT}}),
            ));
            e
        }

        /// Sets what the next `script.confirm` dialog answers.
        pub fn answer(e: &Executor, yes: bool) {
            *e.scripts.as_ref().unwrap().answer.lock().unwrap() = Some(yes);
        }
    }

    /// One call through the broker and the executor, as the supervisor
    /// makes it.
    pub fn call(e: &mut Executor, request: &Value) -> CallResult {
        let plugin = PluginId::new(PLUGIN).unwrap();
        let request: CapabilityRequest = serde_json::from_value(request.clone()).unwrap();
        let session = e.broker.session(plugin.clone());
        match session.authorize_in(&request, e.run_context()) {
            Decision::Allow(allowed) => e.execute(&plugin, &allowed),
            Decision::Deny(reason) => {
                CallResult::error(reason.call_error_code(), reason.to_string())
            }
        }
    }

    pub fn ok(r: &CallResult) -> Value {
        match r {
            CallResult::Ok(v) => v.clone(),
            CallResult::Error(e) => panic!("expected ok, got {e:?}"),
        }
    }

    /// The script code of a refusal.
    pub fn code(r: &CallResult) -> String {
        match r {
            CallResult::Error(e) => e
                .script
                .as_ref()
                .map_or_else(|| format!("call:{:?}", e.code), |s| s.code.clone()),
            CallResult::Ok(v) => panic!("expected a refusal, got {v}"),
        }
    }

    pub fn token(r: &CallResult) -> String {
        match r {
            CallResult::Error(e) => e
                .script
                .as_ref()
                .and_then(|s| s.confirmation.as_ref())
                .map_or_else(|| panic!("no token in {e:?}"), |c| c.token.clone()),
            CallResult::Ok(v) => panic!("expected needsConfirmation, got {v}"),
        }
    }

    pub fn exists(p: &Path) -> bool {
        p.symlink_metadata().is_ok()
    }
}

fn all_of(key: &str, access: FsAccess) -> ScriptGrants {
    ScriptGrants {
        projects_read: true,
        fs: vec![FsScope {
            root: FsRoot::Repository(key.into()),
            access,
            root_destructive: false,
        }],
        ..ScriptGrants::default()
    }
}

#[test]
fn a_run_loads_only_its_own_trusted_script_once() {
    let f = Fixture::new();
    let mut e = f.executor(RunContext::direct(RunTrigger::Key));
    // Nothing before the load.
    assert_eq!(
        code(&call(
            &mut e,
            &json!({"op":"fs.exists","args":{"path":{"repository":"cms","relative":""}}})
        )),
        "invalidArgument"
    );
    let other = call(
        &mut e,
        &json!({"op":"script.load","args":{"script":"another"}}),
    );
    assert_eq!(code(&other), "notGranted");
    let loaded = ok(&call(
        &mut e,
        &json!({"op":"script.load","args":{"script": SCRIPT}}),
    ));
    assert_eq!(loaded["source"], "deckpilot.log.info('hi')");
    assert_eq!(loaded["run"]["origin"], "key");
    assert_eq!(loaded["run"]["userInitiated"], true);
    assert_eq!(loaded["settings"]["memoryMib"], 64);
    assert!(loaded["nowUs"].as_u64().unwrap() > 0);
    let left = loaded["remainingMs"].as_u64().unwrap();
    assert!(left > 50_000 && left <= 60_000, "{left}");
    assert_eq!(
        code(&call(
            &mut e,
            &json!({"op":"script.load","args":{"script": SCRIPT}})
        )),
        "invalidArgument"
    );

    // Trust revoked mid-run: the run's next operation, a confirmation
    // included, is refused and the run ends.
    *f.host.trusted.lock().unwrap() = false;
    for request in [
        json!({"op":"script.log","args":{"level":"info","message":"x"}}),
        json!({"op":"fs.exists","args":{"path":{"repository":"cms","relative":""}}}),
    ] {
        assert_eq!(
            code(&call(&mut e, &request)),
            "scriptNeedsReview",
            "{request}"
        );
    }
    let mut e = f.executor(RunContext::direct(RunTrigger::Key));
    assert_eq!(
        code(&call(
            &mut e,
            &json!({"op":"script.load","args":{"script": SCRIPT}})
        )),
        "scriptNeedsReview"
    );
}

#[test]
fn the_grant_summary_names_no_path() {
    let f = Fixture::new();
    // An absolute program path on this platform: a Unix path such as
    // `/opt/tools/bin/make` is not absolute on Windows (no drive), so the
    // record would fail `validate` and grant nothing at all (shown as no
    // grants: the product refuses an invalid record whole, on every OS).
    let program = if cfg!(windows) {
        r"C:\opt\tools\bin\make"
    } else {
        "/opt/tools/bin/make"
    };
    let grants = ScriptGrants {
        fs: vec![FsScope {
            root: FsRoot::Directory {
                path: f.path("secret-place"),
                label: "Out".into(),
            },
            access: FsAccess::Read,
            root_destructive: false,
        }],
        process: vec![deckpilot_core::script_grants::ProcessGrant {
            command: program.into(),
            cwd: deckpilot_core::script_grants::ProcessCwd::Any,
            env: deckpilot_core::script_grants::ProcessEnv::None,
            unrestricted_args: false,
        }],
        shell: Some(deckpilot_core::script_grants::ShellGrant {
            shells: vec!["sh".into()],
        }),
        ..ScriptGrants::default()
    };
    // A valid record, so every grant below must be in the summary.
    assert_eq!(grants.validate(), Ok(()));
    f.grant(grants);
    let mut e = f.executor(RunContext::direct(RunTrigger::Key));
    let loaded = ok(&call(
        &mut e,
        &json!({"op":"script.load","args":{"script": SCRIPT}}),
    ));
    let text = loaded["grants"].to_string();
    assert!(
        !text.contains("secret-place") && !text.contains("opt") && !text.contains("tools"),
        "{text}"
    );
    assert_eq!(loaded["grants"]["shell"], json!(["sh"]));
    assert_eq!(loaded["grants"]["process"][0]["command"], "make");
    assert_eq!(loaded["grants"]["fs"][0]["root"]["directory"], "Out");
}

#[test]
fn poll_reports_time_and_log_keeps_a_bounded_buffer() {
    let f = Fixture::new();
    let mut e = f.loaded();
    let polled = ok(&call(&mut e, &json!({"op":"script.poll","args":{}})));
    assert_eq!(polled["cancelled"], false);
    let left = polled["remainingMs"].as_u64().unwrap();
    assert!(left > 50_000 && left <= 60_000, "{left}");
    for i in 0..1_005 {
        let r = call(
            &mut e,
            &json!({"op":"script.log","args":{"level":"info","message":format!("line {i}\u{1b}[31m")}}),
        );
        assert_eq!(ok(&r), json!(null));
    }
    let run = e.scripts.as_ref().unwrap();
    let state = run.state();
    assert_eq!(state.log.lines.len(), 1_001, "1,000 lines and the marker");
    assert!(state.log.truncated);
    assert_eq!(super::run::loggable("a\u{1b}[31mb\nc"), "a\\u{1b}[31mb\\nc");
}

#[test]
fn sleep_waits_and_never_outlives_the_run() {
    let f = Fixture::new();
    let mut e = f.loaded();
    let started = std::time::Instant::now();
    ok(&call(
        &mut e,
        &json!({"op":"script.sleep","args":{"ms":60}}),
    ));
    assert!(started.elapsed() >= std::time::Duration::from_millis(60));
    // The run's deadline: a sleep past it ends there with timedOut.
    e.scripts.as_ref().unwrap().state().deadline =
        Some(std::time::Instant::now() + std::time::Duration::from_millis(50));
    let started = std::time::Instant::now();
    assert_eq!(
        code(&call(
            &mut e,
            &json!({"op":"script.sleep","args":{"ms":10_000}})
        )),
        "timedOut"
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    assert_eq!(
        code(&call(&mut e, &json!({"op":"script.sleep","args":{"ms":1}}))),
        "timedOut"
    );
}

#[test]
fn projects_need_projects_read_and_paths_need_repositories_paths() {
    let f = Fixture::new();
    f.project(&[("cms", "w/cms"), ("web", "w/web")]);
    f.grant(ScriptGrants::default());
    let mut e = f.loaded();
    assert_eq!(
        code(&call(&mut e, &json!({"op":"project.list","args":{}}))),
        "notGranted"
    );
    f.grant(all_of("cms", FsAccess::Read));
    let list = ok(&call(&mut e, &json!({"op":"project.list","args":{}})));
    assert_eq!(list["projects"][0]["key"], "site");
    let repos = ok(&call(
        &mut e,
        &json!({"op":"project.repositories","args":{"project":null}}),
    ));
    assert_eq!(repos["repositories"][0]["key"], "cms");
    assert_eq!(repos["repositories"][0]["path"], json!(null));
    assert_eq!(repos["repositories"][0]["exists"], true);
    f.grant(ScriptGrants {
        repositories_paths: true,
        ..all_of("cms", FsAccess::Read)
    });
    let repos = ok(&call(
        &mut e,
        &json!({"op":"project.repositories","args":{"project":null}}),
    ));
    assert!(
        repos["repositories"][1]["path"]
            .as_str()
            .unwrap()
            .ends_with("web")
    );
}

#[test]
fn every_op_is_refused_to_another_plugin_and_without_a_run() {
    use deckpilot_plugin_supervisor::CapabilityExecutor as _;
    let f = Fixture::new();
    let mut e = f.loaded();
    // The executor itself refuses another plugin, whatever was allowed.
    let request: deckpilot_plugin_abi::CapabilityRequest =
        serde_json::from_value(json!({"op":"script.poll","args":{}})).unwrap();
    let allowed =
        deckpilot_plugin_broker::Allowed::Script(Box::new(deckpilot_plugin_broker::ScriptCall {
            request,
            scope: deckpilot_plugin_abi::scripts::ScriptScope::Run,
            ceiling: deckpilot_plugin_broker::ScriptsGrant::default(),
        }));
    let other = deckpilot_plugin_broker::PluginId::new("acme.scripts").unwrap();
    assert_eq!(code(&e.execute(&other, &allowed)), "notGranted");
    // An executor without a script run refuses too.
    let (events, _) = tokio::sync::broadcast::channel(4);
    let mut bare = super::super::Executor::new(
        f.broker.clone(),
        "Scripts".into(),
        events,
        super::super::Transport::Network,
        super::super::OwnVariables {
            store: super::super::PublishedVariables::default(),
            namespace: "plugins.deckpilot.scripts".into(),
            generation: 0,
            declared: Vec::new(),
        },
        super::super::Opener::System,
    );
    let plugin = deckpilot_plugin_broker::PluginId::new(PLUGIN).unwrap();
    assert_eq!(code(&bare.execute(&plugin, &allowed)), "unavailable");
    // Without a shell grant there is no shell.
    assert_eq!(
        ok(&call(&mut e, &json!({"op":"shell.available","args":{}}))),
        json!({"shells": []})
    );
}

#[test]
fn the_default_store_has_no_script() {
    let mut e = Fixture::new().executor(RunContext::direct(RunTrigger::Key));
    e.scripts = Some(super::ScriptRun::new(
        std::sync::Arc::new(super::NoScripts),
        Some(SCRIPT.into()),
    ));
    assert_eq!(
        code(&call(
            &mut e,
            &json!({"op":"script.load","args":{"script": SCRIPT}})
        )),
        "scriptMissing"
    );
}

#[test]
fn every_call_but_poll_and_log_is_audited_without_paths() {
    let f = Fixture::new();
    f.project(&[("cms", "w/cms")]);
    f.grant(all_of("cms", FsAccess::Read));
    let mut e = f.loaded();
    call(&mut e, &json!({"op":"script.poll","args":{}}));
    call(
        &mut e,
        &json!({"op":"script.log","args":{"level":"info","message":"x"}}),
    );
    call(
        &mut e,
        &json!({"op":"fs.exists","args":{"path":{"repository":"cms","relative":"secret.txt"}}}),
    );
    let audit = f.audit.lock().unwrap();
    let ops: Vec<_> = audit.iter().map(|a| a.record.op).collect();
    assert_eq!(ops, ["script.load", "fs.exists"]);
    let text = format!("{:?}", audit[1]);
    assert!(
        text.contains("\"cms\"") && !text.contains("secret.txt"),
        "{text}"
    );
    assert!(!text.contains(f.root.to_string_lossy().as_ref()), "{text}");
}

// --- The confirmation token (p6-plan §2) --------------------------------

fn deletable(f: &Fixture, destructive: Destructive) {
    f.project(&[("cms", "w/cms")]);
    f.grant(ScriptGrants {
        destructive,
        ..all_of("cms", FsAccess::Delete)
    });
    let dir = f.path("w/cms/build");
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("a.txt"), "aaaa").unwrap();
    std::fs::write(dir.join("sub/b.txt"), "bb").unwrap();
}

fn remove_build(token: Option<&str>) -> serde_json::Value {
    let mut args =
        json!({"path":{"repository":"cms","relative":"build"},"recursive":true,"dryRun":false});
    if let Some(t) = token {
        args["confirmation"] = json!(t);
    }
    json!({"op":"fs.remove","args":args})
}

#[test]
fn a_confirmed_token_removes_once() {
    let f = Fixture::new();
    deletable(&f, Destructive::Ask);
    let mut e = f.loaded();
    let first = call(&mut e, &remove_build(None));
    assert_eq!(code(&first), "needsConfirmation");
    let t = token(&first);
    assert!(exists(&f.path("w/cms/build/a.txt")), "nothing removed yet");
    // Using it before the confirmation fails, and uses it up.
    assert_eq!(
        code(&call(&mut e, &remove_build(Some(&t)))),
        "needsConfirmation"
    );
    assert_eq!(
        code(&call(
            &mut e,
            &json!({"op":"script.confirm","args":{"token":t}})
        )),
        "invalidArgument"
    );

    let t = token(&call(&mut e, &remove_build(None)));
    Fixture::answer(&e, true);
    assert_eq!(
        ok(&call(
            &mut e,
            &json!({"op":"script.confirm","args":{"token":t}})
        )),
        json!({"confirmed": true})
    );
    // A second confirm of the same token is refused.
    assert_eq!(
        code(&call(
            &mut e,
            &json!({"op":"script.confirm","args":{"token":t}})
        )),
        "invalidArgument"
    );
    let done = ok(&call(&mut e, &remove_build(Some(&t))));
    assert_eq!(done["removed"], true);
    assert_eq!(
        (done["files"].as_u64(), done["directories"].as_u64()),
        (Some(2), Some(2))
    );
    assert!(!exists(&f.path("w/cms/build")));
    // Single use.
    std::fs::create_dir_all(f.path("w/cms/build/x")).unwrap();
    assert_eq!(
        code(&call(&mut e, &remove_build(Some(&t)))),
        "invalidArgument"
    );
    assert!(exists(&f.path("w/cms/build/x")));
}

#[test]
fn a_token_is_bound_to_its_op_args_and_run() {
    let f = Fixture::new();
    deletable(&f, Destructive::Ask);
    std::fs::create_dir_all(f.path("w/cms/other/x")).unwrap();
    let mut e = f.loaded();
    let t = token(&call(&mut e, &remove_build(None)));
    Fixture::answer(&e, true);
    ok(&call(
        &mut e,
        &json!({"op":"script.confirm","args":{"token":t}}),
    ));
    // Other args.
    let other = json!({"op":"fs.remove","args":{"path":{"repository":"cms","relative":"other"},"recursive":true,"dryRun":false,"confirmation":t}});
    assert_eq!(code(&call(&mut e, &other)), "notGranted");
    assert!(exists(&f.path("w/cms/other/x")) && exists(&f.path("w/cms/build/a.txt")));

    // Another op.
    let t = token(&call(&mut e, &remove_build(None)));
    ok(&call(
        &mut e,
        &json!({"op":"script.confirm","args":{"token":t}}),
    ));
    let contents = json!({"op":"fs.removeContents","args":{"path":{"repository":"cms","relative":"build"},"dryRun":false,"keep":[],"confirmation":t}});
    assert_eq!(code(&call(&mut e, &contents)), "notGranted");

    // Another run.
    let t = token(&call(&mut e, &remove_build(None)));
    ok(&call(
        &mut e,
        &json!({"op":"script.confirm","args":{"token":t}}),
    ));
    let mut other_run = f.loaded();
    assert_eq!(
        code(&call(&mut other_run, &remove_build(Some(&t)))),
        "invalidArgument"
    );
    assert!(exists(&f.path("w/cms/build/a.txt")));
}

#[test]
fn a_declined_or_grown_plan_removes_nothing() {
    let f = Fixture::new();
    deletable(&f, Destructive::Ask);
    let mut e = f.loaded();
    let t = token(&call(&mut e, &remove_build(None)));
    Fixture::answer(&e, false);
    assert_eq!(
        ok(&call(
            &mut e,
            &json!({"op":"script.confirm","args":{"token":t}})
        )),
        json!({"confirmed": false})
    );
    assert_eq!(code(&call(&mut e, &remove_build(Some(&t)))), "cancelled");
    assert!(exists(&f.path("w/cms/build/a.txt")));

    let t = token(&call(&mut e, &remove_build(None)));
    Fixture::answer(&e, true);
    ok(&call(
        &mut e,
        &json!({"op":"script.confirm","args":{"token":t}}),
    ));
    std::fs::write(f.path("w/cms/build/new.txt"), "more").unwrap();
    let grown = call(&mut e, &remove_build(Some(&t)));
    assert_eq!(code(&grown), "needsConfirmation", "asked again");
    assert_ne!(token(&grown), t);
    assert!(exists(&f.path("w/cms/build/a.txt")));
}

#[test]
fn an_expired_token_is_refused() {
    let f = Fixture::new();
    deletable(&f, Destructive::Ask);
    let mut e = f.loaded();
    let t = token(&call(&mut e, &remove_build(None)));
    Fixture::answer(&e, true);
    ok(&call(
        &mut e,
        &json!({"op":"script.confirm","args":{"token":t}}),
    ));
    {
        let run = e.scripts.as_ref().unwrap();
        run.state().tokens.age(&t, super::confirm::TTL);
    }
    assert_eq!(
        code(&call(&mut e, &remove_build(Some(&t)))),
        "needsConfirmation"
    );
    assert!(exists(&f.path("w/cms/build/a.txt")));
}

#[test]
fn a_run_nobody_started_is_refused_unless_destructive_is_allowed() {
    let f = Fixture::new();
    deletable(&f, Destructive::Ask);
    let schedule = RunContext::workflow(RunCause::Schedule, None);
    let mut e = f.loaded_in(schedule.clone());
    assert_eq!(code(&call(&mut e, &remove_build(None))), "notUserInitiated");
    assert_eq!(
        code(&call(
            &mut e,
            &json!({"op":"script.confirm","args":{"token":"0123456789abcdef0123456789abcdef"}})
        )),
        "notUserInitiated"
    );
    assert!(exists(&f.path("w/cms/build/a.txt")));
    // `allow`: no token, in any run.
    f.grant(ScriptGrants {
        destructive: Destructive::Allow,
        ..all_of("cms", FsAccess::Delete)
    });
    let done = ok(&call(&mut e, &remove_build(None)));
    assert_eq!(done["removed"], true);
    assert!(!exists(&f.path("w/cms/build")));
}
