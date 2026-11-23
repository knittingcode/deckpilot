use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use deckpilot_core::launch::RecordingLauncher;
use deckpilot_core::{
    ActionParameters, CancellationToken, Capability, ExecutionContext, ExecutionControl,
    MachineConfig, MachineResourceBinding, Os, ParameterValue, Profile, Progress, Project,
    ProjectKey, ResourceKey, ResourceKind, ResourceRef,
};

use super::*;
use crate::apps::RecordingApps;
use deckpilot_apps::close::Report as CloseReport;
use deckpilot_core::{RunOrigin, WorkflowCause};

struct Fixture {
    profile: Profile,
    machine: MachineConfig,
    dir: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn key(value: &str) -> ResourceKey {
    ResourceKey::new(value).unwrap()
}

fn fixed(value: &str) -> ParameterValue {
    ParameterValue::Resource(ResourceRef::Fixed { key: key(value) })
}

fn role(value: &str) -> ParameterValue {
    ParameterValue::Resource(ResourceRef::ProjectRole { role: value.into() })
}

fn text(value: &str) -> ParameterValue {
    ParameterValue::Text(value.into())
}

fn shell() -> String {
    if cfg!(windows) { "cmd" } else { "sh" }.into()
}

/// A script argument list for the platform's shell.
fn script(unix: &str, windows: &str) -> String {
    if cfg!(windows) {
        format!("/C\n{windows}")
    } else {
        format!("-c\n{unix}")
    }
}

fn fixture() -> Fixture {
    let dir = std::env::temp_dir().join(format!(
        "deckpilot-system-test-{}",
        deckpilot_core::ProfileId::new()
    ));
    std::fs::create_dir_all(dir.join("docs")).unwrap();
    std::fs::write(dir.join("docs/README.md"), "hi").unwrap();
    let mut profile = Profile::new("Work");
    let mut project = Project::new(ProjectKey::new("SHOP").unwrap(), "Shop");
    project
        .resources
        .insert("repository".into(), key("shop.repository"));
    profile.projects.push(project);
    let mut machine = MachineConfig::new(Os::current());
    for (k, kind, value) in [
        (
            "shop.repository",
            ResourceKind::Directory,
            dir.display().to_string(),
        ),
        ("tools.shell", ResourceKind::Other, shell()),
        (
            "tools.editor",
            ResourceKind::Application,
            "Editor".to_owned(),
        ),
        (
            "tools.missing-app",
            ResourceKind::Application,
            dir.join("Missing.app").display().to_string(),
        ),
    ] {
        machine.set_binding(MachineResourceBinding {
            key: key(k),
            kind,
            value,
        });
    }
    Fixture {
        profile,
        machine,
        dir,
    }
}

impl Fixture {
    fn context(&self) -> ExecutionContext<'_> {
        ExecutionContext::new(
            &self.profile,
            Some(&self.profile.projects[0]),
            &self.machine,
        )
    }
}

fn run(
    handler: &dyn ActionHandler,
    parameters: impl Into<ActionParameters>,
    context: &ExecutionContext<'_>,
) -> ActionResult {
    let parameters = parameters.into();
    let normalized = handler.definition().normalize(&parameters).unwrap();
    handler.execute(&normalized, context)
}

fn code(result: &ActionResult) -> Option<&str> {
    result.code.as_deref()
}

#[test]
fn every_action_type_declares_its_node_contract() {
    let launcher: Arc<dyn Launcher> = Arc::new(RecordingLauncher::new());
    let apps: Arc<dyn Apps> = Arc::new(RecordingApps::default());
    let handlers = handlers(&launcher, &apps, &Arc::new(Sampler::new()), false);
    let ids: Vec<_> = handlers
        .iter()
        .map(|h| h.definition().id.to_string())
        .collect();
    assert_eq!(
        ids,
        [
            OPEN_APP,
            OPEN_URL,
            OPEN_FILE,
            OPEN_DIRECTORY,
            CLOSE_APP,
            RUN_COMMAND,
            PROCESS_STATUS,
            CPU,
            MEMORY,
            NETWORK,
            DISK
        ]
    );
    for handler in &handlers {
        let definition = handler.definition();
        assert!(!definition.node.outputs.is_empty(), "{}", definition.id);
        assert!(definition.node.icon.is_some());
        let expected = match definition.id.as_str() {
            RUN_COMMAND | CLOSE_APP => ActionEffect::Modifies,
            PROCESS_STATUS | CPU | MEMORY | NETWORK | DISK => ActionEffect::ReadOnly,
            _ => ActionEffect::Opens,
        };
        assert_eq!(definition.node.effect, expected, "{}", definition.id);
        // Only the monitor keys show a live state.
        let monitor = [CPU, MEMORY, NETWORK, DISK].contains(&definition.id.as_str());
        assert_eq!(
            handler.state_policy(&ActionParameters::new()).is_some(),
            monitor,
            "{}",
            definition.id
        );
    }
    let command = RunCommand::new(true);
    let spec = &command.definition().node;
    assert!(spec.cancellable && !spec.retry_safe && !spec.idempotent);
    let sensitive: Vec<_> = spec
        .outputs
        .iter()
        .filter(|o| o.sensitive)
        .map(|o| o.key.as_str())
        .collect();
    assert_eq!(sensitive, ["stdout", "stderr"]);
}

#[test]
fn urls_open_through_the_platform_launcher() {
    let fixture = fixture();
    let launcher = Arc::new(RecordingLauncher::new());
    let handler = OpenUrl::new(launcher.clone());
    let result = run(
        &handler,
        ActionParameters::new().with("url", text("https://example.com/x")),
        &fixture.context(),
    );
    assert!(result.is_success(), "{}", result.message);
    assert_eq!(result.data["url"], "https://example.com/x");
    let plans = launcher.plans();
    assert_eq!(plans.len(), 1);
    assert!(plans[0].args.contains(&"https://example.com/x".to_owned()));

    let refused = run(
        &handler,
        ActionParameters::new().with("url", text("file:///etc/passwd")),
        &fixture.context(),
    );
    assert_eq!(code(&refused), Some(crate::codes::INVALID_URL));
    assert_eq!(launcher.plans().len(), 1, "nothing else was opened");
}

#[test]
fn opening_requires_the_capability() {
    let fixture = fixture();
    let launcher = Arc::new(RecordingLauncher::new());
    let granted = BTreeSet::from([Capability::MachineResources]);
    let context = fixture.context().restricted(&granted);
    let result = run(
        &OpenUrl::new(launcher.clone()),
        ActionParameters::new().with("url", text("https://example.com")),
        &context,
    );
    assert_eq!(code(&result), Some("not-permitted"));
    assert_eq!(launcher.plans(), []);
}

#[test]
fn applications_come_from_resources() {
    let fixture = fixture();
    let launcher = Arc::new(RecordingLauncher::new());
    let handler = OpenApp::new(launcher.clone());
    let result = run(
        &handler,
        ActionParameters::new()
            .with("application", fixed("tools.editor"))
            .with("arguments", text("--new-window\nmy file.txt")),
        &fixture.context(),
    );
    assert!(result.is_success(), "{}", result.message);
    let plan = &launcher.plans()[0];
    assert!(
        plan.args.ends_with(&["my file.txt".to_owned()])
            || plan.args.contains(&"my file.txt".to_owned())
    );

    let unbound = run(
        &handler,
        ActionParameters::new().with("application", fixed("tools.unbound")),
        &fixture.context(),
    );
    assert_eq!(code(&unbound), Some("resource-unbound"));
    let missing = run(
        &handler,
        ActionParameters::new().with("application", fixed("tools.missing-app")),
        &fixture.context(),
    );
    assert_eq!(code(&missing), Some(crate::codes::NOT_FOUND));
    assert_eq!(launcher.plans().len(), 1);

    let failing = OpenApp::new(Arc::new(RecordingLauncher::failing()));
    let result = run(
        &failing,
        ActionParameters::new().with("application", fixed("tools.editor")),
        &fixture.context(),
    );
    assert_eq!(code(&result), Some("program-not-found"));
}

#[test]
fn files_and_folders_stay_inside_their_resource() {
    let fixture = fixture();
    let launcher = Arc::new(RecordingLauncher::new());
    let folder = OpenDirectory::new(launcher.clone());
    let file = OpenFile::new(launcher.clone());

    let opened = run(
        &folder,
        ActionParameters::new()
            .with("location", role("repository"))
            .with("path", text("docs")),
        &fixture.context(),
    );
    assert!(opened.is_success(), "{}", opened.message);
    assert_eq!(
        PathBuf::from(&opened.data["path"]),
        fixture.dir.join("docs")
    );

    let readme = run(
        &file,
        ActionParameters::new()
            .with("location", role("repository"))
            .with("path", text("docs/README.md"))
            .with("application", fixed("tools.editor")),
        &fixture.context(),
    );
    assert!(readme.is_success(), "{}", readme.message);
    assert_eq!(launcher.plans().len(), 2);

    for (path, expected) in [
        ("../outside", crate::codes::INVALID_PATH),
        ("docs/missing.md", crate::codes::NOT_FOUND),
    ] {
        let result = run(
            &file,
            ActionParameters::new()
                .with("location", role("repository"))
                .with("path", text(path)),
            &fixture.context(),
        );
        assert_eq!(code(&result), Some(expected), "{path}");
    }
    // Without an active project, a project role cannot be resolved.
    let no_project = ExecutionContext::new(&fixture.profile, None, &fixture.machine);
    let result = run(
        &folder,
        ActionParameters::new().with("location", role("repository")),
        &no_project,
    );
    assert_eq!(code(&result), Some("no-active-project"));
    assert_eq!(launcher.plans().len(), 2);
}

fn command(arguments: &str) -> ActionParameters {
    ActionParameters::new()
        .with("program", fixed("tools.shell"))
        .with("arguments", text(arguments))
        .with("workingDirectory", role("repository"))
}

#[test]
fn commands_are_off_until_allowed() {
    let fixture = fixture();
    let result = run(
        &RunCommand::new(false),
        command(&script("echo hi", "echo hi")),
        &fixture.context(),
    );
    assert_eq!(code(&result), Some(crate::codes::COMMANDS_DISABLED));
}

#[test]
fn commands_run_without_a_shell_in_their_folder() {
    let fixture = fixture();
    let handler = RunCommand::new(true);
    // The whole line is one argument: it reaches the script unsplit.
    let result = run(
        &handler,
        command(&script("pwd; printf '%s' \"$0\"", "cd")),
        &fixture.context(),
    );
    assert!(result.is_success(), "{}", result.message);
    assert_eq!(result.data["exitCode"], "0");
    let folder = fixture.dir.canonicalize().unwrap();
    let reported = PathBuf::from(result.data["stdout"].lines().next().unwrap().trim());
    assert_eq!(reported.canonicalize().unwrap(), folder);

    let failed = run(
        &handler,
        command(&script("exit 3", "exit 3")),
        &fixture.context(),
    );
    assert_eq!(code(&failed), Some(crate::codes::EXIT_NONZERO));
    assert_eq!(failed.data["exitCode"], "3");
    let tolerated = run(
        &handler,
        command(&script("exit 3", "exit 3")).with("failOnNonZero", ParameterValue::Boolean(false)),
        &fixture.context(),
    );
    assert!(tolerated.is_success());
}

#[test]
fn commands_do_not_see_the_agents_secrets() {
    let fixture = fixture();
    // Only names outside the minimal environment are removed; use one the
    // test process has (cargo sets it when running tests).
    assert!(std::env::var_os("CARGO_PKG_NAME").is_some());
    let result = run(
        &RunCommand::new(true),
        command(&script(
            "printf '[%s]' \"${CARGO_PKG_NAME:-}\"",
            "echo [%CARGO_PKG_NAME%]",
        )),
        &fixture.context(),
    );
    assert!(result.is_success(), "{}", result.message);
    assert!(
        !result.data["stdout"].contains("deckpilot"),
        "{}",
        result.data["stdout"]
    );
}

#[test]
fn commands_stop_when_cancelled_or_out_of_time() {
    let fixture = fixture();
    let handler = RunCommand::new(true);
    let long = command(&script("sleep 30", "ping -n 30 127.0.0.1"));

    let token = CancellationToken::new();
    let progress = |_: Progress| {};
    let control = ExecutionControl::new(&token, &progress);
    let context = fixture.context().with_control(&control);
    let canceller = {
        let token = token.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            token.cancel();
        })
    };
    let begun = Instant::now();
    let result = run(&handler, long.clone(), &context);
    canceller.join().unwrap();
    assert_eq!(code(&result), Some("cancelled"), "{}", result.message);
    assert!(begun.elapsed() < Duration::from_secs(10));

    let begun = Instant::now();
    let result = run(
        &handler,
        long.with("timeoutSeconds", ParameterValue::Integer(1)),
        &fixture.context(),
    );
    assert_eq!(code(&result), Some("timed-out"), "{}", result.message);
    assert!(begun.elapsed() < Duration::from_secs(10));
}

#[test]
fn programs_must_be_bound_and_valid() {
    let mut fixture = fixture();
    fixture.machine.set_binding(MachineResourceBinding {
        key: key("tools.relative"),
        kind: ResourceKind::Other,
        value: "bin/tool".into(),
    });
    let handler = RunCommand::new(true);
    for (program, expected) in [
        ("tools.unbound", "resource-unbound"),
        ("tools.relative", crate::codes::INVALID_PATH),
    ] {
        let result = run(
            &handler,
            ActionParameters::new().with("program", fixed(program)),
            &fixture.context(),
        );
        assert_eq!(code(&result), Some(expected), "{program}");
    }
    let granted = BTreeSet::from([Capability::MachineResources]);
    let restricted = fixture.context().restricted(&granted);
    let denied = run(&handler, command(&script("true", "exit 0")), &restricted);
    assert_eq!(code(&denied), Some("not-permitted"));
}

#[test]
fn process_status_reports_by_name() {
    let fixture = fixture();
    let handler = ProcessStatus::new();
    let result = run(
        &handler,
        ActionParameters::new().with("name", text("deckpilot-no-such-program")),
        &fixture.context(),
    );
    assert!(result.is_success());
    assert_eq!(result.data["running"], "false");
    assert_eq!(result.data["count"], "0");
    assert_eq!(normalize_process_name(" Slack.EXE "), "slack");

    let granted = BTreeSet::from([Capability::MachineResources]);
    let restricted = fixture.context().restricted(&granted);
    let denied = run(
        &handler,
        ActionParameters::new().with("name", text("x")),
        &restricted,
    );
    assert_eq!(code(&denied), Some("not-permitted"));
}

#[cfg(unix)]
#[test]
fn process_status_finds_running_programs() {
    let fixture = fixture();
    let mut child = std::process::Command::new("sleep")
        .arg("10")
        .spawn()
        .unwrap();
    let result = run(
        &ProcessStatus::new(),
        ActionParameters::new().with("name", text("sleep")),
        &fixture.context(),
    );
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(result.data["running"], "true", "{}", result.message);
    assert!(
        result.data["pids"]
            .split(',')
            .any(|pid| pid == child.id().to_string())
    );
}

/// A link at `link` to `target`; false where links cannot be made
/// (Windows without the privilege).
fn make_link(target: &std::path::Path, link: &std::path::Path) -> bool {
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(target, link);
    #[cfg(windows)]
    let made = if target.is_dir() {
        std::os::windows::fs::symlink_dir(target, link)
    } else {
        std::os::windows::fs::symlink_file(target, link)
    };
    made.is_ok()
}

#[test]
fn links_out_of_the_resource_are_not_opened() {
    let fixture = fixture();
    let outside = fixture.dir.with_extension("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.txt"), "out").unwrap();
    let made = make_link(&outside.join("secret.txt"), &fixture.dir.join("leak.txt"))
        && make_link(&outside, &fixture.dir.join("leak-dir"))
        && make_link(&fixture.dir.join("docs"), &fixture.dir.join("manual"));
    if !made {
        eprintln!("links cannot be made here; skipped");
        let _ = std::fs::remove_dir_all(&outside);
        return;
    }
    let launcher = Arc::new(RecordingLauncher::new());
    let folder = OpenDirectory::new(launcher.clone());
    let file = OpenFile::new(launcher.clone());
    let at = |path: &str| {
        ActionParameters::new()
            .with("location", role("repository"))
            .with("path", text(path))
    };
    for (handler, path) in [
        (&file as &dyn ActionHandler, "leak.txt"),
        (&file, "leak-dir/secret.txt"),
        (&folder, "leak-dir"),
    ] {
        let result = run(handler, at(path), &fixture.context());
        assert_eq!(
            code(&result),
            Some(crate::codes::INVALID_PATH),
            "{path}: {}",
            result.message
        );
    }
    assert_eq!(launcher.plans().len(), 0);
    // A link that stays inside opens.
    let inside = run(&file, at("manual/README.md"), &fixture.context());
    assert!(inside.is_success(), "{}", inside.message);
    let inside = run(&folder, at("manual"), &fixture.context());
    assert!(inside.is_success(), "{}", inside.message);
    assert_eq!(launcher.plans().len(), 2);
    let _ = std::fs::remove_dir_all(&outside);
}

fn monitor(metric: Metric) -> Monitor {
    Monitor::new(metric, Arc::new(Sampler::new()))
}

fn percent_of(result: &ActionResult, key: &str) -> u64 {
    let value: u64 = result.data[key].parse().unwrap();
    assert!(value <= 100, "{key} = {value}");
    value
}

/// The monitor actions read this computer: only the shape of the values
/// is checked, never the load.
#[test]
fn monitors_report_this_computers_values() {
    let fixture = fixture();
    let context = fixture.context();
    let cpu = run(&monitor(Metric::Cpu), ActionParameters::new(), &context);
    assert!(cpu.is_success(), "{}", cpu.message);
    percent_of(&cpu, "usagePercent");
    assert!(["ok", "warning", "critical"].contains(&cpu.data["level"].as_str()));

    let memory = run(&monitor(Metric::Memory), ActionParameters::new(), &context);
    assert!(memory.is_success(), "{}", memory.message);
    percent_of(&memory, "usedPercent");
    let total: u64 = memory.data["totalBytes"].parse().unwrap();
    assert!(total > 0);

    let network = run(&monitor(Metric::Network), ActionParameters::new(), &context);
    assert!(network.is_success(), "{}", network.message);
    for key in ["receivedPerSecond", "transmittedPerSecond"] {
        network.data[key].parse::<u64>().unwrap();
    }

    let disk = run(&monitor(Metric::Disk), ActionParameters::new(), &context);
    assert!(disk.is_success(), "{}", disk.message);
    percent_of(&disk, "freePercent");
    assert_ne!(disk.data["mountPoint"], "");

    // The key shows the same reading.
    let handler = monitor(Metric::Memory);
    let parameters = handler
        .definition()
        .normalize(&ActionParameters::new())
        .unwrap();
    let state = handler.button_state(&parameters, &context).unwrap();
    assert!(state.detail.unwrap().ends_with(" GiB"));
    assert!(state.indicator.is_some() && state.progress.is_some());
}

#[test]
fn disk_space_of_a_folder_resource() {
    let fixture = fixture();
    let handler = monitor(Metric::Disk);
    let at = |reference: ParameterValue| ActionParameters::new().with("location", reference);
    let result = run(&handler, at(role("repository")), &fixture.context());
    assert!(result.is_success(), "{}", result.message);
    assert!(
        result.data["freeBytes"].parse::<u64>().unwrap()
            <= result.data["totalBytes"].parse::<u64>().unwrap()
    );
    let unbound = run(&handler, at(fixed("tools.nowhere")), &fixture.context());
    assert!(!unbound.is_success());
    assert!(code(&unbound).is_some());

    std::fs::remove_dir_all(&fixture.dir).unwrap();
    let gone = run(&handler, at(role("repository")), &fixture.context());
    assert_eq!(code(&gone), Some(codes::NOT_FOUND), "{}", gone.message);
}

#[test]
fn an_unknown_network_interface_is_not_found() {
    let fixture = fixture();
    let handler = monitor(Metric::Network);
    let parameters = ActionParameters::new().with("interface", text("deckpilot-no-such-if"));
    let result = run(&handler, parameters.clone(), &fixture.context());
    assert_eq!(code(&result), Some(codes::NOT_FOUND), "{}", result.message);
    let state = handler.button_state(&parameters, &fixture.context());
    assert_eq!(state.unwrap_err().code, codes::NOT_FOUND);
}

#[test]
fn thresholds_must_be_in_order() {
    let with = |warning: i64, critical: i64| {
        ActionParameters::new()
            .with("warningPercent", ParameterValue::Integer(warning))
            .with("criticalPercent", ParameterValue::Integer(critical))
    };
    let cpu = monitor(Metric::Cpu);
    assert_eq!(cpu.validate(&with(70, 90)), Vec::new());
    assert_eq!(cpu.validate(&with(80, 80)), Vec::new());
    assert_eq!(cpu.validate(&with(90, 70)).len(), 1);
    let disk = monitor(Metric::Disk);
    assert_eq!(disk.validate(&with(20, 10)), Vec::new());
    assert_eq!(disk.validate(&with(10, 20)).len(), 1);
    // The defaults are in order.
    for metric in [Metric::Cpu, Metric::Memory, Metric::Disk] {
        let handler = monitor(metric);
        let defaults = handler
            .definition()
            .normalize(&ActionParameters::new())
            .unwrap();
        assert_eq!(handler.validate(&defaults), Vec::new(), "{metric:?}");
    }
}

#[test]
fn keys_share_one_reading() {
    let fixture = fixture();
    let sampler = Arc::new(Sampler::new());
    let first = sampler.cpu(&fixture.context()).unwrap();
    let again = sampler.cpu(&fixture.context()).unwrap();
    // Read once: the second call shares the first reading.
    assert_eq!(first, again);
    assert_eq!(first.1.len(), 1);
    let cpu = Monitor::new(Metric::Cpu, sampler.clone());
    let memory = Monitor::new(Metric::Memory, sampler);
    let parameters = ActionParameters::new();
    assert!(cpu.button_state(&parameters, &fixture.context()).is_ok());
    assert!(memory.button_state(&parameters, &fixture.context()).is_ok());
    assert_eq!(
        cpu.state_policy(&parameters).unwrap().min_interval(),
        crate::monitor::STATE_INTERVAL
    );
    assert_eq!(
        monitor(Metric::Disk)
            .state_policy(&parameters)
            .unwrap()
            .min_interval(),
        crate::monitor::DISK_STATE_INTERVAL
    );
}

#[test]
fn a_first_reading_stops_when_cancelled() {
    let fixture = fixture();
    let token = CancellationToken::new();
    token.cancel();
    let progress = |_: Progress| {};
    let control = ExecutionControl::new(&token, &progress);
    let context = fixture.context().with_control(&control);
    for metric in [Metric::Cpu, Metric::Network] {
        let result = run(&monitor(metric), ActionParameters::new(), &context);
        assert_eq!(code(&result), Some("cancelled"), "{metric:?}");
    }
}

// ---------------------------------------------------------------------------
// Website, Open, Open Application and Close Application
// (docs/system-actions.md).

fn opener(launcher: &Arc<RecordingLauncher>, apps: &Arc<RecordingApps>) -> Opener {
    Opener {
        launcher: launcher.clone(),
        apps: apps.clone(),
    }
}

fn bind(fixture: &mut Fixture, k: &str, kind: ResourceKind, value: &str) {
    fixture.machine.set_binding(MachineResourceBinding {
        key: key(k),
        kind,
        value: value.to_owned(),
    });
}

#[test]
fn websites_are_normalised_and_other_schemes_refused() {
    let fixture = fixture();
    let launcher = Arc::new(RecordingLauncher::new());
    let handler = OpenUrl::new(launcher.clone());
    let result = run(
        &handler,
        ActionParameters::new().with("url", text("Example.com/docs")),
        &fixture.context(),
    );
    assert!(result.is_success(), "{}", result.message);
    assert_eq!(result.data["url"], "https://example.com/docs");
    for bad in [
        "javascript:alert(1)",
        "file:///etc/passwd",
        "data:text/html,x",
        "ms-settings:privacy",
        "https://user:secret@example.com",
        "https://example.com\\@evil.example",
    ] {
        let refused = run(
            &handler,
            ActionParameters::new().with("url", text(bad)),
            &fixture.context(),
        );
        assert_eq!(code(&refused), Some(crate::codes::INVALID_URL), "{bad}");
        let problems = handler.validate(&ActionParameters::new().with("url", text(bad)));
        assert_eq!(problems.len(), 1, "{bad}");
    }
    assert_eq!(launcher.plans().len(), 1, "nothing else was opened");
    // A saved mail address still opens, in the mail program only.
    let mail = ActionParameters::new().with("url", text("mailto:team@example.com"));
    assert_eq!(handler.validate(&mail), []);
    assert!(run(&handler, mail.clone(), &fixture.context()).is_success());
    let with_browser = mail.with("browser", fixed("tools.editor"));
    assert_eq!(handler.validate(&with_browser).len(), 1);
    // A template is checked when it runs, not when it is saved.
    let template = ActionParameters::new().with(
        "url",
        ParameterValue::Template("https://{{ shared.host }}/".into()),
    );
    assert_eq!(handler.validate(&template), []);
}

#[test]
fn a_website_opens_in_the_chosen_browser() {
    let mut fixture = fixture();
    let launcher = Arc::new(RecordingLauncher::new());
    let apps = Arc::new(RecordingApps::default());
    let browser_dir = fixture.dir.join("Browser");
    std::fs::create_dir_all(&browser_dir).unwrap();
    let browser = browser_dir.join(if cfg!(windows) {
        "browser.exe"
    } else {
        "browser"
    });
    std::fs::write(&browser, "").unwrap();
    bind(
        &mut fixture,
        "apps.browser",
        ResourceKind::Application,
        &browser.to_string_lossy(),
    );
    let gone = fixture.dir.join("Gone").join("browser");
    bind(
        &mut fixture,
        "apps.gone-browser",
        ResourceKind::Application,
        &gone.to_string_lossy(),
    );
    let handler = OpenUrl::with(opener(&launcher, &apps));
    let result = run(
        &handler,
        ActionParameters::new()
            .with("url", text("https://example.com"))
            .with("browser", fixed("apps.browser")),
        &fixture.context(),
    );
    assert!(result.is_success(), "{}", result.message);
    let plan = &launcher.plans()[0];
    assert!(
        plan.args.contains(&"https://example.com/".to_owned()),
        "{plan:?}"
    );
    assert!(
        plan.program == browser.to_string_lossy()
            || plan.args.contains(&browser.to_string_lossy().into_owned()),
        "{plan:?}"
    );
    assert_eq!(
        apps.calls(),
        [format!("launch {} https://example.com/", browser.display())]
    );
    // A browser that is not installed any more: the page opens in the
    // default browser, and the result says so.
    let missing = run(
        &handler,
        ActionParameters::new()
            .with("url", text("https://example.com"))
            .with("browser", fixed("apps.gone-browser")),
        &fixture.context(),
    );
    assert!(missing.is_success(), "{}", missing.message);
    assert_eq!(missing.data["fallback"], "default-browser");
    assert!(
        missing.message.contains("default browser"),
        "{}",
        missing.message
    );
    let fallback = launcher.plans().last().cloned().unwrap();
    assert!(
        !fallback.args.iter().any(|a| a.contains("Gone"))
            && fallback.program != gone.to_string_lossy(),
        "{fallback:?}"
    );
    let unbound = run(
        &handler,
        ActionParameters::new()
            .with("url", text("https://example.com"))
            .with("browser", fixed("apps.unbound")),
        &fixture.context(),
    );
    assert_eq!(code(&unbound), Some("resource-unbound"));
    assert_eq!(launcher.plans().len(), 2);
    // A launcher that cannot start says so.
    let failing = OpenUrl::new(Arc::new(RecordingLauncher::failing()));
    let result = run(
        &failing,
        ActionParameters::new().with("url", text("https://example.com")),
        &fixture.context(),
    );
    assert_eq!(code(&result), Some("program-not-found"));
}

#[test]
fn open_opens_a_bound_file_folder_or_application_itself() {
    let mut fixture = fixture();
    let launcher = Arc::new(RecordingLauncher::new());
    let file = fixture.dir.join("docs/README.md");
    bind(
        &mut fixture,
        "files.readme",
        ResourceKind::Other,
        &file.to_string_lossy(),
    );
    let moved_file = fixture.dir.join("moved.pdf");
    bind(
        &mut fixture,
        "files.moved",
        ResourceKind::Other,
        &moved_file.to_string_lossy(),
    );
    let handler = OpenFile::new(launcher.clone());
    for location in ["files.readme", "shop.repository"] {
        let result = run(
            &handler,
            ActionParameters::new().with("location", fixed(location)),
            &fixture.context(),
        );
        assert!(result.is_success(), "{location}: {}", result.message);
    }
    let plans = launcher.plans();
    assert_eq!(plans.len(), 2);
    assert!(plans[0].args.contains(&file.to_string_lossy().into_owned()));
    let moved = run(
        &handler,
        ActionParameters::new().with("location", fixed("files.moved")),
        &fixture.context(),
    );
    assert_eq!(code(&moved), Some(crate::codes::NOT_FOUND));
    assert!(
        moved.message.contains("Settings › Resources"),
        "{}",
        moved.message
    );
    assert_eq!(launcher.plans().len(), 2);
}

#[test]
fn saved_open_actions_keep_their_parameters() {
    // As saved before Website, Open and Open Application were extended.
    let saved = [
        (
            OPEN_URL,
            ActionParameters::new().with("url", text("https://example.com/x")),
        ),
        (
            OPEN_FILE,
            ActionParameters::new()
                .with("location", role("repository"))
                .with("path", text("docs/README.md"))
                .with("application", fixed("tools.editor")),
        ),
        (
            OPEN_APP,
            ActionParameters::new()
                .with("application", fixed("tools.editor"))
                .with("arguments", text("--new-window")),
        ),
        (
            OPEN_DIRECTORY,
            ActionParameters::new()
                .with("location", role("repository"))
                .with("path", text("docs")),
        ),
    ];
    let fixture = fixture();
    let launcher: Arc<dyn Launcher> = Arc::new(RecordingLauncher::new());
    let apps: Arc<dyn Apps> = Arc::new(RecordingApps::default());
    let all = handlers(&launcher, &apps, &Arc::new(Sampler::new()), false);
    for (id, parameters) in saved {
        let handler = all
            .iter()
            .find(|h| h.definition().id.as_str() == id)
            .unwrap();
        // The stored JSON reads back the same, normalizes and validates.
        let json = serde_json::to_string(&parameters).unwrap();
        let read: ActionParameters = serde_json::from_str(&json).unwrap();
        assert_eq!(read, parameters, "{id}");
        let normalized = handler.definition().normalize(&read).unwrap();
        assert!(handler.validate(&normalized).is_empty(), "{id}");
        let result = handler.execute(&normalized, &fixture.context());
        assert!(result.is_success(), "{id}: {}", result.message);
    }
}

fn close_parameters(force: bool) -> ActionParameters {
    ActionParameters::new()
        .with("application", fixed("apps.editor"))
        .with("force", ParameterValue::Boolean(force))
}

fn close_fixture() -> Fixture {
    let mut fixture = fixture();
    bind(
        &mut fixture,
        "apps.editor",
        ResourceKind::Application,
        "com.example.Editor",
    );
    fixture
}

#[test]
fn close_application_runs_only_when_the_user_started_it() {
    let fixture = close_fixture();
    let apps = Arc::new(RecordingApps::new(CloseReport {
        instances: 1,
        asked: 1,
        ..CloseReport::default()
    }));
    let handler = CloseApp::new(apps.clone());
    for origin in [
        None,
        Some(RunOrigin::Simulated),
        Some(RunOrigin::Workflow {
            cause: WorkflowCause::Schedule,
        }),
        Some(RunOrigin::Workflow {
            cause: WorkflowCause::Event,
        }),
        Some(RunOrigin::Workflow {
            cause: WorkflowCause::Startup,
        }),
        Some(RunOrigin::Workflow {
            cause: WorkflowCause::WorkSession,
        }),
    ] {
        let context = fixture.context().with_origin(origin);
        let result = run(&handler, close_parameters(false), &context);
        assert_eq!(
            code(&result),
            Some(crate::codes::NOT_USER_INITIATED),
            "{origin:?}"
        );
    }
    assert_eq!(apps.calls(), Vec::<String>::new());
    for origin in [
        RunOrigin::Key,
        RunOrigin::Gesture,
        RunOrigin::App,
        RunOrigin::Workflow {
            cause: WorkflowCause::User,
        },
    ] {
        let context = fixture.context().with_origin(Some(origin));
        let result = run(&handler, close_parameters(false), &context);
        assert!(result.is_success(), "{origin:?}: {}", result.message);
        assert_eq!(result.data["closed"], "true");
    }
    assert_eq!(apps.calls().len(), 4);
}

#[test]
fn close_application_needs_its_capabilities() {
    let fixture = close_fixture();
    let apps = Arc::new(RecordingApps::default());
    let handler = CloseApp::new(apps.clone());
    for granted in [
        BTreeSet::from([Capability::MachineResources, Capability::ProcessInspect]),
        BTreeSet::from([Capability::MachineResources, Capability::ProcessControl]),
    ] {
        let context = fixture
            .context()
            .with_origin(Some(RunOrigin::Key))
            .restricted(&granted);
        let result = run(&handler, close_parameters(true), &context);
        assert_eq!(code(&result), Some("not-permitted"));
    }
    assert_eq!(apps.calls(), Vec::<String>::new());
}

#[test]
fn close_application_is_graceful_unless_force_is_on() {
    let fixture = close_fixture();
    let context = fixture.context().with_origin(Some(RunOrigin::Key));
    // Still running after the grace period: reported, never killed.
    let stays = Arc::new(RecordingApps::new(CloseReport {
        instances: 2,
        asked: 2,
        remaining: 1,
        ..CloseReport::default()
    }));
    let result = run(
        &CloseApp::new(stays.clone()),
        close_parameters(false),
        &context,
    );
    assert!(result.is_success(), "{}", result.message);
    assert_eq!(result.data["closed"], "false");
    assert!(result.message.contains("Force quit"), "{}", result.message);
    assert_eq!(stays.calls(), ["close com.example.Editor force=false"]);
    // With force, what remains is a failure.
    let result = run(
        &CloseApp::new(stays.clone()),
        close_parameters(true),
        &context,
    );
    assert_eq!(code(&result), Some(deckpilot_apps::codes::CLOSE_FAILED));
    assert_eq!(stays.calls()[1], "close com.example.Editor force=true");
    // Forced and gone.
    let forced = Arc::new(RecordingApps::new(CloseReport {
        instances: 1,
        asked: 1,
        forced: 1,
        ..CloseReport::default()
    }));
    let result = run(&CloseApp::new(forced), close_parameters(true), &context);
    assert!(result.is_success());
    assert_eq!(result.data["forced"], "1");
    // Not running: nothing to do.
    let idle = Arc::new(RecordingApps::default());
    let result = run(&CloseApp::new(idle), close_parameters(false), &context);
    assert!(result.message.contains("not running"), "{}", result.message);
    assert_eq!(result.data["instances"], "0");
}

#[test]
fn a_cancelled_close_application_says_nothing_was_forced() {
    let fixture = close_fixture();
    let context = fixture.context().with_origin(Some(RunOrigin::Key));
    let apps = Arc::new(RecordingApps::new(CloseReport {
        instances: 1,
        asked: 1,
        remaining: 1,
        interrupted: true,
        ..CloseReport::default()
    }));
    let result = run(&CloseApp::new(apps), close_parameters(true), &context);
    assert_eq!(code(&result), Some(deckpilot_apps::codes::INTERRUPTED));
    assert!(
        result.message.contains("nothing was forced"),
        "{}",
        result.message
    );
}

#[test]
fn close_application_refuses_what_it_cannot_identify() {
    let mut fixture = close_fixture();
    bind(
        &mut fixture,
        "apps.bad",
        ResourceKind::Application,
        "--help",
    );
    let context = fixture.context().with_origin(Some(RunOrigin::Key));
    let apps = Arc::new(RecordingApps::default());
    let result = run(
        &CloseApp::new(apps.clone()),
        ActionParameters::new().with("application", fixed("apps.bad")),
        &context,
    );
    assert_eq!(code(&result), Some(crate::codes::INVALID_APPLICATION));
    let result = run(
        &CloseApp::new(apps.clone()),
        ActionParameters::new().with("application", fixed("apps.unbound")),
        &context,
    );
    assert_eq!(code(&result), Some("resource-unbound"));
    // The real platform: the deny-list is applied before anything is done.
    let real = CloseApp::new(Arc::new(crate::apps::SystemApps));
    bind(
        &mut fixture,
        "apps.finder",
        ResourceKind::Application,
        "com.apple.finder",
    );
    let context = fixture.context().with_origin(Some(RunOrigin::Key));
    let result = run(
        &real,
        ActionParameters::new().with("application", fixed("apps.finder")),
        &context,
    );
    let expected = if cfg!(target_os = "macos") {
        deckpilot_apps::codes::PROTECTED_PROCESS
    } else {
        // Elsewhere a dotted name is a program that is not on PATH.
        deckpilot_apps::codes::NOT_FOUND
    };
    assert_eq!(code(&result), Some(expected), "{}", result.message);
    assert_eq!(apps.calls(), Vec::<String>::new());
}
