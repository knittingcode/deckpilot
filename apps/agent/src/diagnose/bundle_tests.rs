//! The support bundle's redaction on fixture logs with fake secrets.
//!
//! Failure messages name the case with a fixed label and never print a
//! secret, a log line or the bundle.

use super::*;
use crate::diagnose::{Environment, HidSource};
use deckpilot_distribution::{Architecture, Distribution, DistributionChannel, Platform};
use deckpilot_ipc::AgentPaths;

static GITHUB_MACOS: Distribution = match Distribution::new(
    DistributionChannel::Github,
    None,
    "0.15.0",
    Platform::Macos,
    Architecture::Aarch64,
) {
    Ok(distribution) => distribution,
    Err(_) => panic!("valid record"),
};

// Fake values: none of them is a real secret.
const SERIAL: &str = "CL19K2B04711";
const SERIAL_GONE: &str = "AL31H1A99999";
const MACHINE_ID: &str = "5e1f0c2a9b8d7e6f5e1f0c2a9b8d7e6f";
const HOST: &str = "Alices-MacBook-Pro.local";
const USER: &str = "alice";
const OTHER_USER: &str = "bobsmith";
/// Token-shaped fakes are put together at run time, so that secret
/// scanners do not take the source for a leak.
fn github_token() -> String {
    ["gh", "p_FAKEfake0123456789abcdefABCDEF012345"].concat()
}
fn gitlab_token() -> String {
    ["gl", "pat-FAKE0123456789abcdef"].concat()
}
const PASSWORD: &str = "hunter2-fixture";
const API_KEY: &str = "fk_live_51Fixture";
const BEARER: &str = "fixtureBearerValue77";
const URL_PASSWORD: &str = "urlpass-fixture";
const COOKIE: &str = "sid=fixture-cookie-42";
fn jwt() -> String {
    [
        "ey",
        "JhbGciOiJIUzI1NiJ9.eyJzdWIiOiJmaXh0dXJlIn0.c2lnbmF0dXJlZml4dHVyZQ",
    ]
    .concat()
}
const OPAQUE_KEY: &str = "Q2xhdWRlRml4dHVyZUtleTAxMjM0NTY3ODk";
const EMAIL_ADDRESS: &str = "alice.example@mail.example.org";
const PRIVATE_HOST: &str = "gitlab.corp.example.net";
const PRIVATE_IP: &str = "192.168.17.42";

fn fixture(root: &Path) -> Environment {
    let home = root.join("Users").join(USER);
    let data = home.join("Library/Application Support/io.github.knittingcode.deckpilot");
    Environment {
        root: root.to_path_buf(),
        platform: "macos",
        arch: "aarch64",
        agent_version: "0.15.0",
        distribution: &GITHUB_MACOS,
        paths: AgentPaths::in_dir(data),
        data_dir_overridden: false,
        exe: Some(PathBuf::from(
            "/Applications/DeckPilot.app/Contents/MacOS/deckpilot-agent",
        )),
        appimage: None,
        home: Some(home),
        user: Some(USER.into()),
        machine_id: Some(MACHINE_ID.into()),
        hostname: Some(HOST.into()),
        hid: HidSource::Listed(Ok(vec![deckpilot_hid::HidInterface {
            vendor_id: 0x0fd9,
            product_id: 0x0080,
            serial: Some(SERIAL.into()),
        }])),
    }
}

/// Agent and update logs with every kind of value the bundle must remove.
fn write_logs(env: &Environment) {
    let home = env.home.as_ref().unwrap().display().to_string();
    let dir = &env.paths.log_dir;
    fs::create_dir_all(dir).unwrap();
    let (github_token, gitlab_token, jwt) = (github_token(), gitlab_token(), jwt());
    let lines = [
        format!(
            "2026-11-26T10:00:00Z  INFO deckpilot_agent::service: device connected device=elgato.streamdeck.mk2#{SERIAL}"
        ),
        format!(
            "2026-11-26T10:00:01Z  WARN deckpilot_agent::service: cannot open device device=elgato.streamdeck.xl#{SERIAL_GONE} error=\"permission denied\""
        ),
        format!(
            "2026-11-26T10:00:02Z DEBUG plugin: request url=https://{USER}:{URL_PASSWORD}@{PRIVATE_HOST}/api/v4?access_token={gitlab_token}&page=2"
        ),
        format!("2026-11-26T10:00:03Z  WARN plugin: header Authorization: Bearer {BEARER}"),
        format!(
            "2026-11-26T10:00:04Z  WARN plugin: config {{ password: \"{PASSWORD}\", api_key: {API_KEY} }}"
        ),
        format!("2026-11-26T10:00:05Z  INFO plugin: github token {github_token} accepted"),
        format!("2026-11-26T10:00:06Z  INFO plugin: \"cookie\": \"{COOKIE}\""),
        format!("2026-11-26T10:00:07Z  INFO plugin: jwt {jwt}"),
        format!("2026-11-26T10:00:08Z  INFO plugin: key material {OPAQUE_KEY}"),
        format!(
            "2026-11-26T10:00:09Z  INFO data directory {home}/Library/Application Support/io.github.knittingcode.deckpilot"
        ),
        format!(
            "2026-11-26T10:00:10Z  INFO opened /Users/{OTHER_USER}/Projects/app and C:\\Users\\{OTHER_USER}\\Desktop"
        ),
        format!("2026-11-26T10:00:11Z  INFO running on {HOST} as {USER}; mail {EMAIL_ADDRESS}"),
        format!("2026-11-26T10:00:12Z  INFO obs at ws://{PRIVATE_IP}:4455 and 127.0.0.1:4455"),
        format!("2026-11-26T10:00:13Z  INFO machine {MACHINE_ID} serial={SERIAL_GONE}"),
        format!("2026-11-26T10:00:14Z  INFO hidapi path {SERIAL} on short host Alices-MacBook-Pro"),
    ];
    fs::write(dir.join("agent.2026-11-26.log"), lines.join("\n") + "\n").unwrap();
    fs::write(
        dir.join("agent.2026-11-25.log"),
        "2026-11-25T09:00:00Z  INFO an older day\n",
    )
    .unwrap();
    fs::write(
        dir.join("updates.log"),
        format!("1764150000 version 0.15.0: download from https://github.com/knittingcode/deckpilot/releases failed: {home}/Downloads token={github_token}\n"),
    )
    .unwrap();
}

/// Every fake value of the fixture, with a label for failure messages.
fn secrets() -> Vec<(&'static str, String)> {
    vec![
        ("serial of a listed device", SERIAL.into()),
        ("serial of a device only in the log", SERIAL_GONE.into()),
        ("machine ID", MACHINE_ID.into()),
        ("host name", HOST.into()),
        ("short host name", "Alices-MacBook-Pro".into()),
        ("user name", USER.into()),
        ("other user's name", OTHER_USER.into()),
        ("GitHub token", github_token()),
        ("GitLab token", gitlab_token()),
        ("password field", PASSWORD.into()),
        ("API key field", API_KEY.into()),
        ("bearer token", BEARER.into()),
        ("password in a URL", URL_PASSWORD.into()),
        ("cookie", "fixture-cookie-42".into()),
        ("JSON web token", jwt()),
        ("JSON web token payload", "eyJzdWIiOiJmaXh0dXJlIn0".into()),
        ("opaque key", OPAQUE_KEY.into()),
        ("e-mail address", EMAIL_ADDRESS.into()),
        ("private host", PRIVATE_HOST.into()),
        ("private IP address", PRIVATE_IP.into()),
        ("home directory", "/Users/alice".into()),
    ]
}

/// The labels of the fake values `text` contains (case-insensitive).
fn leaks(text: &str) -> Vec<&'static str> {
    let lower = text.to_lowercase();
    secrets()
        .into_iter()
        .filter(|(_, secret)| lower.contains(&secret.to_lowercase()))
        .map(|(label, _)| label)
        .collect()
}

fn root() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

#[test]
fn the_bundle_contains_no_fixture_secret() {
    let root = root();
    let env = fixture(root.path());
    write_logs(&env);
    let value = bundle(&env, "2026-11-26T10:00:00Z", Limits::default());
    let text = serde_json::to_string(&value).unwrap();
    let found = leaks(&text);
    // Labels only: never the values or the bundle.
    assert!(found.is_empty(), "the bundle contains: {found:?}");
}

#[test]
fn the_bundle_keeps_what_support_needs() {
    let root = root();
    let env = fixture(root.path());
    write_logs(&env);
    let value = bundle(&env, "2026-11-26T10:00:00Z", Limits::default());
    assert!(value["schema"] == SCHEMA, "schema name");
    assert!(value["schemaVersion"] == SCHEMA_VERSION, "schema version");
    assert!(
        value["report"]["schema"] == "deckpilot.diagnose",
        "the report is included"
    );
    assert!(
        value["report"]["os"]["platform"] == "macos",
        "the platform is kept"
    );
    assert!(
        value["report"]["install"]["distributionChannel"].is_string(),
        "the distribution channel is kept"
    );
    let logs = value["logs"].as_array().unwrap();
    let files: Vec<&str> = logs.iter().filter_map(|log| log["file"].as_str()).collect();
    assert!(
        files
            == [
                "agent.2026-11-26.log",
                "agent.2026-11-25.log",
                "updates.log"
            ],
        "the logs are the newest agent logs first, then the update log"
    );
    let lines: Vec<&str> = logs[0]["lines"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let has = |needle: &str| lines.iter().any(|line| line.contains(needle));
    // A serial becomes the report's pseudonym, so log and report match.
    let pseudonym = serial_pseudonym(SERIAL, Some(MACHINE_ID));
    assert!(
        has(&format!("elgato.streamdeck.mk2#{pseudonym}")),
        "the device ID keeps its model and gets the pseudonym"
    );
    assert!(
        value["report"]["devices"]["hid"][0]["serial"] == pseudonym.as_str(),
        "the report has the same pseudonym"
    );
    assert!(
        has(&format!(
            "serial={}",
            serial_pseudonym(SERIAL_GONE, Some(MACHINE_ID))
        )),
        "a serial field gets the pseudonym"
    );
    assert!(
        has("~/Library/Application Support/io.github.knittingcode.deckpilot"),
        "the home directory is ~"
    );
    assert!(
        has("/Users/<user>/Projects/app"),
        "another user's home directory"
    );
    assert!(
        has("C:\\Users\\<user>\\Desktop"),
        "a Windows home directory"
    );
    assert!(
        has("running on <host> as <user>; mail <email>"),
        "host, user and e-mail"
    );
    assert!(
        has("ws://<host>:4455") || has("ws://<ip>:4455"),
        "a private address"
    );
    assert!(has("127.0.0.1:4455"), "loopback is kept");
    assert!(has("machine <machine-id>"), "the machine ID");
    assert!(has("permission denied"), "ordinary messages are kept");
    assert!(has("deckpilot_agent::service"), "module paths are kept");
    assert!(
        has("access_token=<redacted>&page=<redacted>"),
        "a private URL keeps its query names, not its values"
    );
    let updates = logs[2]["lines"][0].as_str().unwrap();
    assert!(
        updates.contains("https://github.com/knittingcode/deckpilot/releases"),
        "DeckPilot's own URLs are kept"
    );
    assert!(
        value["redaction"]["rules"]
            .as_array()
            .is_some_and(|rules| !rules.is_empty()),
        "the bundle says what was redacted"
    );
}

#[test]
fn the_report_passes_the_log_redaction_unchanged() {
    // The report's own redaction (#515) already removes everything: the
    // second pass is a safety net and changes nothing in a clean report.
    let root = root();
    let env = fixture(root.path());
    let mut report = collect(&env);
    let before = report.clone();
    LogRedactor::new(&env).value(&mut report);
    assert!(report == before, "the second pass changed the report");
}

#[test]
fn logs_are_bounded() {
    let root = root();
    let env = fixture(root.path());
    let dir = &env.paths.log_dir;
    fs::create_dir_all(dir).unwrap();
    for day in 1..=9 {
        let line = format!("2026-11-0{day} INFO {}\n", "x".repeat(200));
        fs::write(
            dir.join(format!("agent.2026-11-0{day}.log")),
            line.repeat(2000),
        )
        .unwrap();
    }
    fs::write(
        dir.join("agent.2026-11-10.log"),
        format!("{}\n", "y".repeat(10_000)),
    )
    .unwrap();
    let limits = Limits {
        agent_files: 3,
        agent_bytes: 64 * 1024,
        updates_bytes: 1024,
        lines: 300,
        line_chars: 100,
    };
    let value = bundle(&env, "2026-11-26T10:00:00Z", limits);
    let logs = value["logs"].as_array().unwrap();
    assert!(logs.len() <= 3, "at most three agent logs");
    assert!(
        logs[0]["file"] == "agent.2026-11-10.log",
        "the newest log first"
    );
    let line_count: usize = logs
        .iter()
        .map(|log| log["lines"].as_array().map_or(0, Vec::len))
        .sum();
    assert!(line_count <= 300, "at most the line limit");
    for log in logs {
        for line in log["lines"].as_array().unwrap() {
            assert!(
                line.as_str().unwrap().chars().count() <= 100 + " [cut]".len(),
                "a long line is cut"
            );
        }
    }
    assert!(
        logs[1]["truncated"] == true,
        "a log read from its end says so"
    );
    let text = serde_json::to_string(&value).unwrap();
    assert!(text.len() < 128 * 1024, "the bundle stays small");
}

#[cfg(unix)]
#[test]
fn a_linked_log_is_not_followed() {
    let root = root();
    let env = fixture(root.path());
    let dir = &env.paths.log_dir;
    fs::create_dir_all(dir).unwrap();
    let outside = root.path().join("outside.txt");
    fs::write(&outside, "OUTSIDE-FIXTURE-CONTENT\n").unwrap();
    std::os::unix::fs::symlink(&outside, dir.join("agent.2026-11-26.log")).unwrap();
    std::os::unix::fs::symlink(&outside, dir.join("updates.log")).unwrap();
    let value = bundle(&env, "2026-11-26T10:00:00Z", Limits::default());
    let text = serde_json::to_string(&value).unwrap();
    assert!(
        !text.contains("OUTSIDE-FIXTURE-CONTENT"),
        "a linked file was read"
    );
}

#[test]
fn rules_one_by_one() {
    let root = root();
    let env = fixture(root.path());
    let redactor = LogRedactor::new(&env);
    for (label, input, expected) in [
        ("tracing field", "token=abc123", "token=<redacted>"),
        (
            "quoted field",
            r#"password="a b \"c\"" next"#,
            "password=<redacted> next",
        ),
        (
            "json field",
            r#""client_secret": "s3""#,
            r#""client_secret": <redacted>"#,
        ),
        (
            "basic",
            "Authorization: Basic dXNlcjpwYXNz",
            "Authorization: <redacted> <redacted>",
        ),
        ("token word stays", "the token expired", "the token expired"),
        (
            "url without credentials",
            "https://github.com/x",
            "https://github.com/x",
        ),
        (
            "private url",
            "https://intranet.example/x",
            "https://<host>/x",
        ),
        (
            "version stays",
            "version 0.15.0 on 6.8.0-49-generic",
            "version 0.15.0 on 6.8.0-49-generic",
        ),
        ("shared stays", "/Users/Shared/x", "/Users/Shared/x"),
        ("linux home", "/home/carol/.config", "/home/<user>/.config"),
        ("broadcast stays", "255.255.255.255", "255.255.255.255"),
    ] {
        assert!(redactor.line(input) == expected, "rule: {label}");
    }
}

/// Forms a log line may give a secret: each fake value must be gone.
#[test]
fn secrets_in_other_forms_are_removed() {
    let root = root();
    let env = fixture(root.path());
    let redactor = LogRedactor::new(&env);
    for (label, input, secret) in [
        (
            "escaped json",
            r#"body="{\"password\":\"FAKEescaped\"}""#,
            "FAKEescaped",
        ),
        (
            "escaped json in debug",
            r#"error=Json("{\"access_token\":\"FAKEat\"}")"#,
            "FAKEat",
        ),
        (
            "option with a space",
            r#"token=Some("FAKE with space")"#,
            "with space",
        ),
        ("tuple", r#"auth: ("bob", Some("FAKEtuple"))"#, "FAKEtuple"),
        (
            "struct",
            r#"credentials: Credentials { user: "bob", pass: "FAKEpass" }"#,
            "FAKEpass",
        ),
        ("unclosed quote", r#"password="abc FAKEtail"#, "FAKEtail"),
    ] {
        let output = redactor.line(input).to_lowercase();
        assert!(
            !output.contains(&secret.to_lowercase()),
            "a secret is left: {label}"
        );
    }
}

/// URLs, addresses, keys and serials in other forms.
#[test]
fn secrets_in_urls_addresses_and_serials_are_removed() {
    let root = root();
    let env = fixture(root.path());
    let redactor = LogRedactor::new(&env);
    for (label, input, secret) in [
        (
            "query key",
            "https://api.example/x?key=FAKEKEY123&sig=FAKESIG",
            "FAKEKEY123",
        ),
        (
            "query sig",
            "https://api.example/x?key=FAKEKEY123&sig=FAKESIG",
            "FAKESIG",
        ),
        ("passphrase", "passphrase=FAKEphrase", "FAKEphrase"),
        ("pw", "pw=FAKEpw", "FAKEpw"),
        ("pin", "pin=135799", "135799"),
        ("option", "running tool --password FAKEcli now", "FAKEcli"),
        ("arrow", "password -> FAKEarrow", "FAKEarrow"),
        ("fat arrow", "password => FAKEfat", "FAKEfat"),
        (
            "slack webhook",
            "https://hooks.slack.com/services/T000/B000/FAKEhook",
            "FAKEhook",
        ),
        (
            "discord webhook",
            "https://discord.com/api/webhooks/123/FAKE-wh-token",
            "FAKE-wh-token",
        ),
        (
            "url-encoded",
            "data=client_secret%3DFAKEenc%26x%3D1",
            "FAKEenc",
        ),
        ("ipv6", "ip 2001:db8:85a3::8a2e:370:7334 up", "2001:db8"),
        ("ipv6 link-local", "via fe80::1c2b:3d4e%en0", "fe80::1c2b"),
        (
            "base64 with slashes",
            "key material wJalrXUtnFEMI/K7MDENG/bPxRfiCYFAKEKEY9",
            "wJalrXUtnFEMI",
        ),
        (
            "serial in another case",
            "serial cl19k2b04711 seen",
            "cl19k2b04711",
        ),
        ("serial short", "ID_SERIAL_SHORT=ZZ99XX88", "ZZ99XX88"),
        (
            "serial option",
            r#"serial_number: Some("ZZ99XX77")"#,
            "ZZ99XX77",
        ),
        ("unc server", r"opened \\fileserver\share\x", "fileserver"),
        (
            "base64 after a bracket",
            "msg=<aB3/dE5fG7hI9/jK1lM3nO5pQ7/rS9tU1vW3xY5zA7bC9=>",
            "dE5fG7hI9",
        ),
        (
            "base64 in braces",
            "signed {MIIBIjAN/BgkqhkiG9w0/BAQEFAAOCAQ8AMIIB/CgKCAQEAu1SU1L7VLPHCgcBz}",
            "BgkqhkiG9w0",
        ),
        (
            "after one colon",
            "env:GITHUB_TOKEN=FAKEenvtok",
            "FAKEenvtok",
        ),
        ("after a level", "ERROR:password=FAKEerrpw", "FAKEerrpw"),
        (
            "bracket in a quoted value",
            r#"auth: Some(("bob", "ab(FAKEparen"))"#,
            "FAKEparen",
        ),
        (
            "closing bracket in a value",
            r#"password: Some("ab)FAKEclose")"#,
            "FAKEclose",
        ),
        (
            "option-wrapped tuple",
            r#"auth: Some(("bob", Some("FAKEnested")))"#,
            "FAKEnested",
        ),
        (
            "nested newtypes",
            r#"token: Some(Secret(Inner("a FAKEinner")))"#,
            "FAKEinner",
        ),
        (
            "unclosed with an escape",
            r#"password="abc \"FAKEesc"#,
            "FAKEesc",
        ),
        (
            "deep struct",
            r#"c: Credentials { u: "b", i: I { d: D { pass: "FAKEdeep" } } }"#,
            "FAKEdeep",
        ),
    ] {
        let output = redactor.line(input).to_lowercase();
        assert!(
            !output.contains(&secret.to_lowercase()),
            "a secret is left: {label}"
        );
    }
}

/// What support needs stays readable.
#[test]
fn ordinary_text_stays_readable() {
    let root = root();
    let env = fixture(root.path());
    let redactor = LogRedactor::new(&env);
    for (label, input) in [
        ("timestamps", "2026-11-26T10:00:00.123Z"),
        ("module paths", "deckpilot_agent::service::device_retry"),
        (
            "library paths",
            "/usr/lib/x86_64-linux-gnu/libayatana-appindicator3.so.1",
        ),
        ("loopback ipv6", "listening on ::1"),
        (
            "windows home of another user stays a path",
            r"C:\\Users\\<user>\\x",
        ),
        ("exit codes", "exited with code=1"),
        (
            "release urls",
            "https://github.com/knittingcode/DeckPilot/releases/download/v0.15.0/DeckPilot_0.15.0_aarch64.dmg",
        ),
        (
            "relative paths",
            "io.github.knittingcode.deckpilot/Plugins/Com9Example/Data",
        ),
        ("key bindings", "ControlBinding { key: Key { index: 3 } }"),
        ("translation keys", "missing key=label.title"),
        (
            "log targets",
            "INFO deckpilot_agent::service::work_session: work session started",
        ),
        ("a pseudonym stays", "serial=sha256:0123456789abcdef"),
    ] {
        assert!(redactor.line(input) == input, "changed: {label}");
    }
}
