use serde_json::{Value, json};

use super::*;
use crate::capability::{CallErrorCode, CallResult};
use crate::limits::{MAX_HOST_CALL_REPLY, MAX_HOST_CALL_REQUEST};

const FIXTURE: &str = include_str!("../../tests/fixtures/script-requests.json");

fn fixture() -> Value {
    serde_json::from_str(FIXTURE).expect("the fixture is JSON")
}

fn parse(v: &Value) -> Result<CapabilityRequest, String> {
    let request: CapabilityRequest =
        serde_json::from_value(v.clone()).map_err(|e| e.to_string())?;
    request.validate().map_err(|e| e.to_string())?;
    Ok(request)
}

#[test]
fn every_fixture_request_parses_validates_and_round_trips() {
    let fixture = fixture();
    let valid = fixture["valid"].as_array().unwrap();
    let mut seen = std::collections::BTreeSet::new();
    for case in valid {
        let op = case["op"].as_str().unwrap();
        let request = parse(&case["request"]).unwrap_or_else(|e| panic!("{op}: {e}"));
        assert_eq!(request.op(), op);
        let back = serde_json::to_value(&request).unwrap();
        assert_eq!(
            back, case["request"],
            "{op} serializes back to the guest's form"
        );
        assert!(
            serde_json::to_vec(&request).unwrap().len() <= MAX_HOST_CALL_REQUEST,
            "{op}"
        );
        seen.insert(op.to_owned());
    }
    let all: std::collections::BTreeSet<String> = OPS.iter().map(|s| (*s).to_owned()).collect();
    assert_eq!(seen, all, "the fixture holds every operation of the bridge");
}

#[test]
fn every_invalid_fixture_request_is_refused() {
    let fixture = fixture();
    for case in fixture["invalid"].as_array().unwrap() {
        let why = case["why"].as_str().unwrap();
        assert!(parse(&case["request"]).is_err(), "{why}: must be refused");
    }
}

#[test]
fn every_op_has_a_scope_but_the_shared_project_list() {
    for case in fixture()["valid"].as_array().unwrap() {
        let request = parse(&case["request"]).unwrap();
        let op = request.op();
        assert_eq!(
            request.script_scope().is_some(),
            op != "project.list",
            "{op}"
        );
        assert_eq!(carries_script_detail(op), op != "project.list", "{op}");
    }
    assert_eq!(
        parse(&json!({"op":"process.run","args":{"command":"git","args":[],"env":{}}}))
            .unwrap()
            .script_scope(),
        Some(ScriptScope::Process)
    );
    assert_eq!(
        parse(&json!({"op":"shell.available","args":{}}))
            .unwrap()
            .script_scope(),
        Some(ScriptScope::Shell)
    );
    assert_eq!(
        parse(&json!({"op":"time.now","args":{}}))
            .unwrap()
            .script_scope(),
        None
    );
}

#[test]
fn only_poll_and_log_keep_the_deadline() {
    assert!(!resets_deadline("script.poll"));
    assert!(!resets_deadline("script.log"));
    for op in OPS
        .iter()
        .filter(|op| !matches!(**op, "script.poll" | "script.log"))
    {
        assert!(resets_deadline(op), "{op}");
    }
    assert!(resets_deadline("http.fetch"));
    assert!(resets_deadline("log"));
}

#[test]
fn requests_stay_within_the_frame_caps_d1() {
    let big = |n: usize| "a".repeat(n);
    let run = |args: Vec<String>, stdin: Option<String>| {
        parse(
            &json!({"op":"process.run","args":{"command":"git","args":args,"env":{},"stdin":stdin}}),
        )
    };
    // The total of the arguments and stdin each fit 48 KiB, not more.
    assert!(run(vec![big(24 << 10), big(24 << 10)], None).is_ok());
    assert!(run(vec![big(24 << 10), big((24 << 10) + 1)], None).is_err());
    assert!(run(vec![big((32 << 10) + 1)], None).is_err());
    assert!(run(vec![], Some(big(48 << 10))).is_ok());
    assert!(run(vec![], Some(big((48 << 10) + 1))).is_err());
    assert!(run((0..=MAX_PROCESS_ARGS).map(|_| "a".into()).collect(), None).is_err());
    // The largest request each allows still fits one host call.
    let largest = CapabilityRequest::ProcessRun(ProcessRun {
        command: "git".into(),
        args: vec![
            big(MAX_PROCESS_ARGS_TOTAL / 2),
            big(MAX_PROCESS_ARGS_TOTAL / 2),
        ],
        cwd: Some(PathArg::in_repository("cms", "")),
        env: BTreeMap::new(),
        timeout_ms: Some(1),
        stdin: None,
    });
    largest.validate().unwrap();
    assert!(serde_json::to_vec(&largest).unwrap().len() < MAX_HOST_CALL_REQUEST);

    let shell = |script: String| {
        parse(&json!({"op":"shell.run","args":{"shell":"sh","script":script,"env":{}}}))
    };
    assert!(shell(big(MAX_SHELL_SCRIPT)).is_ok());
    assert!(shell(big(MAX_SHELL_SCRIPT + 1)).is_err());

    // A chunk of content is at most 32 KiB, in either encoding.
    use base64::Engine as _;
    let b64 = |n: usize| base64::engine::general_purpose::STANDARD.encode(vec![7u8; n]);
    let chunk = |content: String, encoding: &str| {
        parse(
            &json!({"op":"fs.write.chunk","args":{"handle":"h1","content":content,"encoding":encoding}}),
        )
    };
    assert!(chunk(b64(MAX_WRITE_CHUNK), "base64").is_ok());
    assert!(chunk(b64(MAX_WRITE_CHUNK + 1), "base64").is_err());
    assert!(chunk(big(MAX_WRITE_CHUNK), "utf8").is_ok());
    assert!(chunk(big(MAX_WRITE_CHUNK + 1), "utf8").is_err());
    let write = |content: String| {
        parse(
            &json!({"op":"fs.writeFile","args":{"path":{"path":"/tmp/a"},"content":content,"encoding":"base64","createOnly":false}}),
        )
    };
    assert!(write(b64(MAX_WRITE_CHUNK)).is_ok());
    assert!(write(b64(MAX_WRITE_CHUNK + 1)).is_err());
    let largest = CapabilityRequest::FsWriteChunk(FsWriteChunk {
        handle: "0123456789abcdef0123456789abcdef".into(),
        content: b64(MAX_WRITE_CHUNK),
        encoding: Encoding::Base64,
    });
    assert!(serde_json::to_vec(&largest).unwrap().len() < MAX_HOST_CALL_REQUEST);

    // A read chunk's reply fits the reply cap, whatever the encoding.
    let reply = FsRead {
        content: b64(MAX_READ_CHUNK as usize),
        bytes: u64::from(MAX_READ_CHUNK),
        size: MAX_SCRIPT_FILE,
        eof: false,
    };
    let reply = serde_json::to_vec(&CallResult::Ok(serde_json::to_value(reply).unwrap())).unwrap();
    assert!(reply.len() < MAX_HOST_CALL_REPLY, "{}", reply.len());
}

#[test]
fn paths_are_one_form_and_never_escape_lexically() {
    let ok = |p: PathArg| p.validate().is_ok();
    assert!(ok(PathArg::absolute("/Users/me/cms")));
    assert!(ok(PathArg::absolute("C:\\work\\cms")));
    assert!(ok(PathArg::in_repository("cms", "")));
    assert!(ok(PathArg::in_repository("cms_2", "src/a.rs")));
    assert!(ok(PathArg {
        repository: Some("cms".into()),
        ..PathArg::default()
    }));
    assert!(!ok(PathArg::in_repository("Cms", "")));
    assert!(!ok(PathArg::in_repository("cms", "\\\\server\\share")));
    assert!(!ok(PathArg::in_repository("cms", "a:stream")));
    assert!(!ok(PathArg::absolute("a".repeat(MAX_PATH + 1))));
    assert!(!ok(PathArg::default()));
    for hidden in ['\u{200b}', '\u{2028}', '\u{feff}', '\u{061c}', '\u{202e}'] {
        assert!(
            !ok(PathArg::absolute(format!("/tmp/a{hidden}b"))),
            "{hidden:?}"
        );
    }
}

#[test]
fn the_script_detail_reaches_bridge_replies_only() {
    let fault = ScriptFault {
        confirmation: Some(Confirmation {
            token: "0123456789abcdef0123456789abcdef".into(),
        }),
        path: Some("node_modules".into()),
        ..ScriptFault::new(codes::NEEDS_CONFIRMATION, "confirm the removal")
    };
    let reply = CallResult::script_error(CallErrorCode::NotPermitted, fault.clone());
    let json = serde_json::to_value(&reply).unwrap();
    assert_eq!(json["error"]["code"], "not-permitted");
    assert_eq!(json["error"]["script"]["code"], "needsConfirmation");
    assert_eq!(
        json["error"]["script"]["confirmation"]["token"],
        "0123456789abcdef0123456789abcdef"
    );
    // Kept on a bridge op, whatever the guest's features.
    assert_eq!(reply.clone().for_op("fs.remove", 0), reply);
    // Removed from every other op (and from the shared project.list).
    for op in ["http.fetch", "log", "tool.run", "project.list"] {
        let CallResult::Error(e) = reply.clone().for_op(op, u64::MAX) else {
            panic!()
        };
        assert_eq!(e.script, None, "{op}");
    }
    // A plain error is unchanged, and without the detail the JSON is the
    // original ABI 1 shape.
    let plain = CallResult::error(CallErrorCode::Failed, "x");
    assert_eq!(
        serde_json::to_string(&plain.for_op("fs.stat", 0)).unwrap(),
        r#"{"error":{"code":"failed","message":"x"}}"#
    );
}

#[test]
fn replies_are_what_the_guest_reads() {
    let loaded = ScriptLoaded {
        source: "deckpilot.log.info('hi')".into(),
        platform: Platform::Macos,
        settings: ScriptSettings {
            timeout_ms: 60_000,
            stop_on_error: true,
            memory_mib: 64,
        },
        grants: json!({"shell": null}),
        run: ScriptRunInfo {
            id: "r1".into(),
            origin: "key".into(),
            user_initiated: true,
        },
        now_us: 1,
    };
    let v = serde_json::to_value(&loaded).unwrap();
    assert_eq!(v["platform"], "macos");
    assert_eq!(v["settings"]["memoryMib"], 64);
    assert_eq!(v["settings"]["stopOnError"], true);
    assert_eq!(v["run"]["userInitiated"], true);
    assert_eq!(v["nowUs"], 1);
    let polled = serde_json::to_value(ScriptPolled {
        cancelled: false,
        remaining_ms: 5,
        now_us: 2,
    })
    .unwrap();
    assert_eq!(
        polled,
        json!({"cancelled": false, "remainingMs": 5, "nowUs": 2})
    );
    let result = serde_json::to_value(ProcessResult {
        exit_code: Some(1),
        ..ProcessResult::default()
    })
    .unwrap();
    for key in [
        "ok",
        "exitCode",
        "signal",
        "stdout",
        "stderr",
        "truncated",
        "durationMs",
        "timedOut",
        "cancelled",
        "nowUs",
    ] {
        assert!(result.get(key).is_some(), "{key}");
    }
    let plan = serde_json::to_value(RemovalPlan::default()).unwrap();
    for key in [
        "dryRun",
        "root",
        "files",
        "directories",
        "bytes",
        "sample",
        "removed",
    ] {
        assert!(plan.get(key).is_some(), "{key}");
    }
}

#[test]
fn the_env_denylist_matches_the_design() {
    for name in [
        "LD_PRELOAD",
        "ld_library_path",
        "DYLD_INSERT_LIBRARIES",
        "DECKPILOT_DB",
        "GIT_CONFIG_GLOBAL",
        "GIT_CONFIG",
        "GIT_SSH_COMMAND",
        "GIT_SSH",
        "GIT_EXEC_PATH",
        "DOCKER_HOST",
        "NODE_OPTIONS",
        "PYTHONPATH",
        "ComSpec",
        "PATH",
        "Path",
        "PATHEXT",
        "HOME",
        "XDG_CONFIG_HOME",
        "GIT_PROXY_COMMAND",
        "GIT_ASKPASS",
        "GIT_EXTERNAL_DIFF",
        "GIT_DIR",
        "GIT_WORK_TREE",
        "SSH_ASKPASS",
        "BASH_ENV",
        "ENV",
        "ZDOTDIR",
        "PERL5OPT",
        "RUBYOPT",
        "NODE_PATH",
        "JAVA_TOOL_OPTIONS",
        "RUSTC_WRAPPER",
        "PSModulePath",
        "HTTPS_PROXY",
        "SSL_CERT_FILE",
        "DOCKER_CONFIG",
        "DOTNET_STARTUP_HOOKS",
        "COR_PROFILER",
        "OPENSSL_CONF",
        "CLASSPATH",
        "PHPRC",
        "GOFLAGS",
        "MAKEFLAGS",
        "LESSOPEN",
        "GEM_PATH",
        "PIP_INDEX_URL",
        "KUBECONFIG",
        "AWS_PROFILE",
    ] {
        assert!(env_denied(name), "{name}");
    }
    for name in [
        "CI",
        "NODE_ENV",
        "GIT_AUTHOR_NAME",
        "GIT_COMMITTER_EMAIL",
        "PATH_EXTRA",
        "DOCKER_BUILDKIT",
        "COMPOSE_PROJECT_NAME",
        "RUST_LOG",
    ] {
        assert!(!env_denied(name), "{name}");
    }
}

#[test]
fn commands_are_names_or_plain_absolute_paths() {
    for ok in [
        "git",
        "docker",
        "make",
        "g++",
        "python3.12",
        "/usr/bin/git",
        "C:\\Program Files\\Git\\bin\\git.exe",
    ] {
        assert!(validate_command(ok).is_ok(), "{ok}");
    }
    for bad in [
        "",
        "git pull",
        "./x",
        "-c",
        "..",
        "C:foo",
        "x.bat",
        "X.CMD",
        "x.cmd.",
        "git;rm",
        "$(x)",
        "\\\\host\\share\\x.exe",
        "/usr/bin/../bin/sh",
        "/usr/bin/git\u{1b}",
        "C:\\tools\\run.bat",
    ] {
        assert!(validate_command(bad).is_err(), "{bad:?}");
    }
}
