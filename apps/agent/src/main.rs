//! `deckpilot-agent` command-line entry point.

use std::process::ExitCode;

use deckpilot_agent::autostart::Autostart;
use deckpilot_agent::{Agent, InstanceLock, LockError, STARTUP_LOCK_WAIT, default_options};
use deckpilot_device::{DeviceModel, models};
use deckpilot_ipc::AgentPaths;
use tracing::{error, info};
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

const USAGE: &str = "\
DeckPilot background agent

Usage:
  deckpilot-agent [--headless] [--mock-devices [<models>]] [--no-usb] [--log-level <level>]
                  [--disable-plugins] [--plugin-host <path>] [--bundled-plugins <dir>]
                  [--plugin-registry <dir> | --plugin-registry-url <https url>]
                  [--plugin-registry-releases-url <https url>]
                  [--plugin-registry-roots <roots.json>]
  deckpilot-agent autostart <enable|disable|status>
  deckpilot-agent diagnose [--json]

Options:
  --headless          Run without a tray icon (servers, CI, debugging)
  --mock-devices [<models>]
                      Add simulated devices next to USB devices. <models> is a
                      comma-separated list of mk2, xl and plus (default: mk2)
  --no-usb            Do not open USB devices, e.g. for tests with simulated
                      devices while a Stream Deck is plugged in
  --log-level <level> error, warn, info (default), debug or trace
  --disable-plugins   Do not answer plugin requests and leave the plugins/
                      directory alone. Builds with the `plugins` feature
                      answer them by default; builds without it never do
  --enable-plugins    Answer plugin requests (the default of builds with the
                      `plugins` feature; overrides DECKPILOT_ENABLE_PLUGINS=0)
  --plugin-host <path>
                      The plugin host executable (default: next to the agent,
                      where the installers put it)
  --bundled-plugins <dir>
                      A bundled/ directory of first-party plugins (debug and
                      test builds only; default: the application's resources)
  --plugin-registry <dir>
                      A local plugin registry (site/v1 and releases/)
  --plugin-registry-url <https url>
                      A plugin registry served over https: the URL of its
                      site (documents at <url>v1/...). Default, once there
                      are root keys (built in or --plugin-registry-roots):
                      https://knittingcode.github.io/deckpilot-plugins/
  --plugin-registry-releases-url <https url>
                      Download packages from here instead of the release URL
                      the registry pins (they must still match the pins)
  --plugin-registry-roots <roots.json>
                      The root keys the registry must be signed with, for
                      builds without built-in ones (self-hosted registries)
                      and development: they override built-in root keys in
                      debug and test builds only. Without any root keys no
                      registry is used
  --version           Print the version
  --help              Print this help

diagnose: a read-only report for support (versions, installation, whether
the agent runs, the Elgato devices the system sees, e.g. a Stream Deck
whose USB device is present but whose HID interface is missing). It starts
and opens nothing, works while the configuration is locked, and hides
serial numbers, the home directory and the user name. --json prints the
versioned JSON report.

Environment:
  DECKPILOT_DATA_DIR    Use a different data directory
  DECKPILOT_DESKTOP_BIN Path of the configuration app executable
  DECKPILOT_ENABLE_PLUGINS
                        0 like --disable-plugins, 1 like --enable-plugins
                        (the command line wins)
  DECKPILOT_PLUGIN_HOST, DECKPILOT_BUNDLED_PLUGINS, DECKPILOT_PLUGIN_REGISTRY,
  DECKPILOT_PLUGIN_REGISTRY_URL, DECKPILOT_PLUGIN_REGISTRY_RELEASES_URL,
  DECKPILOT_PLUGIN_REGISTRY_ROOTS
                        Like the plugin options";

struct Args {
    headless: bool,
    mock_models: Vec<DeviceModel>,
    /// Open USB devices through the HID driver (off with `--no-usb`).
    usb: bool,
    level: LevelFilter,
    plugins: deckpilot_agent::plugins::PluginSettings,
    /// Test only (feature `test-hooks`): Calls with the mock provider in a
    /// meeting from the start, and "Back to work" with these parameters.
    #[cfg(feature = "test-hooks")]
    calls_mock: Option<deckpilot_core::ActionParameters>,
}

/// Test only (feature `test-hooks`): `--test-calls-mock <mode>` installs
/// Calls with the mock provider (`deckpilot-calls::testing`), already in a
/// meeting, so that an end-to-end test of a spawned agent can go "back to
/// work" and follow a real plugin action to its end. `<mode>` is the "Back
/// to work" mode: `completion` (mode B) or `default` (mode C). A build
/// without `test-hooks` has neither the option nor the mock
/// (`tests/plugins_feature.rs`). Headless only.
#[cfg(feature = "test-hooks")]
const TEST_CALLS_MOCK: &str = "--test-calls-mock";

#[cfg(feature = "test-hooks")]
fn calls_mock_back(mode: Option<&str>) -> Result<deckpilot_core::ActionParameters, String> {
    use deckpilot_core::{ActionParameters, ParameterValue};
    match mode {
        Some("completion") => {
            Ok(ActionParameters::new().with("mode", ParameterValue::Choice("completion".into())))
        }
        Some("default") => Ok(ActionParameters::new()),
        _ => Err(format!("{TEST_CALLS_MOCK} needs completion or default")),
    }
}

fn mock_models(list: &str) -> Result<Vec<DeviceModel>, String> {
    list.split(',')
        .map(|name| match name.trim() {
            "mk2" => Ok(models::stream_deck_mk2()),
            "xl" => Ok(models::stream_deck_xl()),
            "plus" => Ok(models::stream_deck_plus()),
            other => Err(format!(
                "unknown mock device model `{other}` (use mk2, xl or plus)"
            )),
        })
        .collect()
}

/// Test only (feature `test-hooks`): arguments taken from this variable,
/// whitespace-separated, before those of the command line. The
/// configuration app starts the agent without arguments; the desktop
/// end-to-end test uses this to have it run headless with a simulated
/// Stream Deck (`--headless --no-usb --mock-devices mk2`). Never read by
/// a release build.
#[cfg(feature = "test-hooks")]
const TEST_ARGS_ENV: &str = "DECKPILOT_TEST_AGENT_ARGS";

type ArgIter = std::iter::Peekable<std::vec::IntoIter<String>>;

fn arguments() -> Vec<String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    #[cfg(feature = "test-hooks")]
    if let Some(extra) = std::env::var_os(TEST_ARGS_ENV) {
        let mut all: Vec<String> = extra
            .to_string_lossy()
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        all.extend(args);
        return all;
    }
    args
}

fn parse_args() -> Result<Result<Args, String>, ExitCode> {
    let mut args: ArgIter = arguments().into_iter().peekable();
    let mut parsed = Args {
        headless: false,
        mock_models: Vec::new(),
        usb: true,
        level: LevelFilter::INFO,
        plugins: deckpilot_agent::plugins::PluginSettings::from_env(),
        #[cfg(feature = "test-hooks")]
        calls_mock: None,
    };
    let path = |args: &mut ArgIter, name: &str| {
        args.next()
            .filter(|v| !v.is_empty())
            .map(std::path::PathBuf::from)
            .ok_or_else(|| format!("{name} needs a path"))
    };
    let text = |args: &mut ArgIter, name: &str| {
        args.next()
            .filter(|v| !v.is_empty())
            .ok_or_else(|| format!("{name} needs a URL"))
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--headless" => parsed.headless = true,
            "--no-usb" => parsed.usb = false,
            switch if parsed.plugins.apply_switch(switch) => {}
            "--plugin-host" => match path(&mut args, "--plugin-host") {
                Ok(p) => parsed.plugins.host = Some(p),
                Err(message) => return Ok(Err(message)),
            },
            "--bundled-plugins" => match path(&mut args, "--bundled-plugins") {
                Ok(p) => parsed.plugins.bundled = Some(p),
                Err(message) => return Ok(Err(message)),
            },
            // Hidden: for the release check (check-bundled-plugins.sh).
            "--print-bundled-pins" => {
                use std::io::Write as _;
                let mut out = std::io::stdout();
                let _ = out.write_all(deckpilot_agent::plugins::embedded_pins());
                let _ = out.flush();
                return Err(ExitCode::SUCCESS);
            }
            "--plugin-registry" => match path(&mut args, "--plugin-registry") {
                Ok(p) => parsed.plugins.registry = Some(p),
                Err(message) => return Ok(Err(message)),
            },
            "--plugin-registry-url" => match text(&mut args, "--plugin-registry-url") {
                Ok(url) => parsed.plugins.registry_url = Some(url),
                Err(message) => return Ok(Err(message)),
            },
            "--plugin-registry-releases-url" => {
                match text(&mut args, "--plugin-registry-releases-url") {
                    Ok(url) => parsed.plugins.registry_releases_url = Some(url),
                    Err(message) => return Ok(Err(message)),
                }
            }
            "--plugin-registry-roots" => match path(&mut args, "--plugin-registry-roots") {
                Ok(p) => parsed.plugins.registry_roots = Some(p),
                Err(message) => return Ok(Err(message)),
            },
            "--mock-devices" => {
                let list = match args.peek() {
                    Some(next) if !next.starts_with("--") => args.next().unwrap_or_default(),
                    _ => "mk2".to_owned(),
                };
                match mock_models(&list) {
                    Ok(models) => parsed.mock_models = models,
                    Err(message) => return Ok(Err(message)),
                }
            }
            "--log-level" => {
                let value = args.next().unwrap_or_default();
                match value.parse() {
                    Ok(level) => parsed.level = level,
                    Err(_) => return Ok(Err(format!("invalid log level `{value}`"))),
                }
            }
            "--version" => {
                println!("deckpilot-agent {}", deckpilot_agent::service::VERSION);
                return Err(ExitCode::SUCCESS);
            }
            "--help" | "-h" => {
                println!("{USAGE}");
                return Err(ExitCode::SUCCESS);
            }
            #[cfg(feature = "test-hooks")]
            TEST_CALLS_MOCK => match calls_mock_back(args.next().as_deref()) {
                Ok(back) => parsed.calls_mock = Some(back),
                Err(message) => return Ok(Err(message)),
            },
            "autostart" => return Err(autostart_command(args.next().as_deref())),
            "diagnose" => return Err(diagnose_command(&args.by_ref().collect::<Vec<_>>())),
            other => return Ok(Err(format!("unknown argument `{other}`"))),
        }
    }
    Ok(Ok(parsed))
}

/// `deckpilot-agent diagnose [--json]` (`deckpilot_agent::diagnose`).
fn diagnose_command(options: &[String]) -> ExitCode {
    let json = match options {
        [] => false,
        [flag] if flag == "--json" => true,
        _ => {
            eprintln!("usage: deckpilot-agent diagnose [--json]");
            return ExitCode::from(2);
        }
    };
    let environment = match deckpilot_agent::diagnose::Environment::current() {
        Ok(environment) => environment,
        Err(error) => {
            eprintln!("cannot find the data directory: {error}");
            return ExitCode::FAILURE;
        }
    };
    let report = deckpilot_agent::diagnose::collect(&environment);
    if json {
        match serde_json::to_string_pretty(&report) {
            Ok(text) => println!("{text}"),
            Err(error) => {
                eprintln!("{error}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        println!("{}", deckpilot_agent::diagnose::summary(&report));
    }
    ExitCode::SUCCESS
}

fn autostart_command(action: Option<&str>) -> ExitCode {
    let autostart = match Autostart::for_current_exe(deckpilot_distribution::current()) {
        Ok(autostart) => autostart,
        Err(error) => {
            eprintln!("autostart is unavailable: {error}");
            return ExitCode::FAILURE;
        }
    };
    let result = match action {
        Some("enable") => autostart.set_enabled(true).map(|()| "enabled".to_owned()),
        Some("disable") => autostart.set_enabled(false).map(|()| "disabled".to_owned()),
        Some("status") => autostart
            .is_enabled()
            .map(|on| if on { "enabled" } else { "disabled" }.to_owned()),
        _ => Err("expected enable, disable or status".into()),
    };
    match result {
        Ok(message) => {
            println!("autostart {message}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

/// Logs to stderr and to daily-rotated files in the data directory.
fn init_logging(
    paths: &AgentPaths,
    level: LevelFilter,
) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let file = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("agent")
        .filename_suffix("log")
        .max_log_files(7)
        .build(&paths.log_dir);
    let (file_layer, guard) = match file {
        Ok(appender) => {
            let (writer, guard) = tracing_appender::non_blocking(appender);
            let layer = tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(writer);
            (Some(layer), Some(guard))
        }
        Err(error) => {
            eprintln!("file logging disabled: {error}");
            (None, None)
        }
    };
    tracing_subscriber::registry()
        .with(level)
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
                .with_writer(std::io::stderr),
        )
        .with(file_layer)
        .init();
    guard
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(Ok(args)) => args,
        Ok(Err(message)) => {
            eprintln!("{message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
        Err(code) => return code,
    };
    let paths = match AgentPaths::resolve().and_then(|paths| paths.create_dirs().map(|()| paths)) {
        Ok(paths) => paths,
        Err(error) => {
            eprintln!("cannot prepare the data directory: {error}");
            return ExitCode::FAILURE;
        }
    };
    let _guard = init_logging(&paths, args.level);

    // A previous agent may still be finishing its shutdown (a restart or an
    // update starts the next one as soon as the endpoint closes).
    let lock = match InstanceLock::acquire_waiting(&paths.lock_file, STARTUP_LOCK_WAIT) {
        Ok(lock) => lock,
        Err(LockError::AlreadyRunning) => {
            info!("the agent is already running");
            return ExitCode::SUCCESS;
        }
        Err(error) => {
            error!(%error, "cannot start");
            return ExitCode::FAILURE;
        }
    };
    let options = default_options(paths, &args.mock_models, args.usb);
    #[cfg(feature = "test-hooks")]
    if args.calls_mock.is_some() && !args.headless {
        eprintln!("{TEST_CALLS_MOCK} needs --headless");
        return ExitCode::from(2);
    }
    #[cfg(feature = "test-hooks")]
    let calls_mock = args.calls_mock;

    if args.headless {
        let runtime = match tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                error!(%error, "cannot start the async runtime");
                return ExitCode::FAILURE;
            }
        };
        let plugins = args.plugins;
        let result = runtime.block_on(async {
            let agent = Agent::start(options, lock)?.with_plugins(plugins);
            #[cfg(feature = "test-hooks")]
            if let Some(back) = calls_mock {
                // Queued before the loop runs; answered once it does.
                let handle = agent.handle();
                tokio::spawn(async move {
                    let mut mock = deckpilot_calls::testing::MockProvider::new();
                    mock.join(deckpilot_calls::clock::Instant::ZERO, "meeting");
                    handle.install_calls_mock(mock, back).await;
                    info!("Calls installed with the mock provider (test hooks)");
                });
            }
            // Boxed: with Calls the loop's future is large.
            Box::pin(agent.run(deckpilot_agent::service::shutdown_signal())).await
        });
        if let Err(error) = result {
            error!(%error, "agent stopped with an error");
            return ExitCode::FAILURE;
        }
    } else {
        deckpilot_agent::tray::run_with_plugins(options, lock, args.plugins);
    }
    ExitCode::SUCCESS
}
