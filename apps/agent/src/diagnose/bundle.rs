//! `deckpilot-agent diagnose --bundle`: a support bundle.
//!
//! One JSON document ([`SCHEMA`], [`SCHEMA_VERSION`]) with the diagnose
//! report and the end of the recent logs, redacted. The app's "Export
//! diagnostics" shows it and writes it to a file the user picks; nothing is
//! uploaded anywhere. Like the report it starts, opens and writes nothing:
//! it prints the bundle.
//!
//! The logs are bounded ([`Limits`]) and every line goes through
//! [`LogRedactor`], which removes what a log line may contain about the
//! user even though logs never contain configuration values, keys or
//! passwords by policy (docs/security.md):
//!
//! - values of fields whose names say they are secret (`token`, `password`,
//!   `secret`, `api_key`, `authorization`, `cookie`, `session`, …), the
//!   value after `Bearer`/`Basic`, user and password in URLs, and strings
//!   that look like access tokens (known prefixes such as `ghp_` or
//!   `glpat-`, JSON web tokens, and runs of 32 or more letters and digits);
//! - serial numbers, as the report's pseudonyms: after a device ID
//!   (`model#serial`), after `serial=`, and every serial of a device the
//!   report sees;
//! - the machine ID;
//! - the home directory (`~`), the home directories of other users
//!   (`/home/<user>`), the user name (`<user>`), this computer's name
//!   (`<host>`), e-mail addresses (`<email>`), IPv4 addresses other than
//!   loopback (`<ip>`) and the host of URLs, except DeckPilot's own
//!   (`<host>`).
//!
//! The report's strings get the same pass, after the report's own
//! redaction. Redaction is a filter over text, so it can miss a secret
//! written in a form it does not know: the app shows the whole bundle
//! before it is saved.

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::{Captures, Regex};
use serde_json::{Value, json};

use super::{Environment, collect, replace_word, serial_pseudonym};

/// The bundle's schema name.
pub const SCHEMA: &str = "deckpilot.support-bundle";
/// The bundle's schema version; fields are only added within a version.
pub const SCHEMA_VERSION: u32 = 1;

/// How much of the logs a bundle takes.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// The newest agent log files (one per day).
    pub agent_files: usize,
    /// Read from the end of the agent logs, newest first, all files together.
    pub agent_bytes: u64,
    /// Read from the end of the app's update log.
    pub updates_bytes: u64,
    /// Lines of all logs together.
    pub lines: usize,
    /// A longer line is cut (after redaction).
    pub line_chars: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            agent_files: 3,
            agent_bytes: 512 * 1024,
            updates_bytes: 64 * 1024,
            lines: 5000,
            line_chars: 2000,
        }
    }
}

/// The app's update log (`apps/desktop/src-tauri/src/updates.rs`).
const UPDATES_LOG: &str = "updates.log";
const REDACTED: &str = "<redacted>";

/// The bundle: the report and the logs of `env`, redacted. `created_at` is
/// an RFC 3339 time.
pub fn bundle(env: &Environment, created_at: &str, limits: Limits) -> Value {
    let redactor = LogRedactor::new(env);
    let mut report = collect(env);
    redactor.value(&mut report);
    let mut lines_left = limits.lines;
    let mut logs = Vec::new();
    let mut bytes_left = limits.agent_bytes;
    for path in agent_logs(&env.paths.log_dir, limits.agent_files) {
        if bytes_left == 0 || lines_left == 0 {
            break;
        }
        if let Some(log) = read_log(&path, bytes_left, &mut lines_left, &redactor, limits) {
            bytes_left = bytes_left.saturating_sub(log.read);
            logs.push(log.value);
        }
    }
    let updates = env.paths.log_dir.join(UPDATES_LOG);
    if lines_left > 0
        && let Some(log) = read_log(
            &updates,
            limits.updates_bytes,
            &mut lines_left,
            &redactor,
            limits,
        )
    {
        logs.push(log.value);
    }
    json!({
        "schema": SCHEMA,
        "schemaVersion": SCHEMA_VERSION,
        "createdAt": created_at,
        "report": report,
        "logs": logs,
        "redaction": {
            "rules": RULES,
            "note": "Redaction is a filter over text: review the bundle before you share it.",
        },
    })
}

/// What [`LogRedactor`] replaces, for the bundle's `redaction.rules`.
const RULES: [&str; 12] = [
    "secret-fields",
    "authorization-values",
    "url-credentials",
    "access-tokens",
    "serial-numbers",
    "machine-id",
    "home-directory",
    "user-directories",
    "user-name",
    "host-name",
    "email-addresses",
    "ip-addresses",
];

struct Log {
    value: Value,
    /// Bytes read from the file.
    read: u64,
}

/// The newest `count` `agent*.log` files of `dir`, newest first (daily
/// files sort by date). Only regular files: a link is not followed.
fn agent_logs(dir: &Path, count: usize) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .filter(|entry| {
            entry.file_name().to_str().is_some_and(|name| {
                name.starts_with("agent")
                    && Path::new(name)
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("log"))
            })
        })
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .map(|entry| entry.path())
        .collect();
    files.sort();
    files.reverse();
    files.truncate(count);
    files
}

/// The end of the log at `path` (at most `max_bytes`, at most `lines_left`
/// lines), redacted. `None` when there is no such regular file.
fn read_log(
    path: &Path,
    max_bytes: u64,
    lines_left: &mut usize,
    redactor: &LogRedactor,
    limits: Limits,
) -> Option<Log> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    let name = path.file_name()?.to_string_lossy().into_owned();
    let mut file = fs::File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    let start = size.saturating_sub(max_bytes);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.take(max_bytes).read_to_end(&mut bytes).ok()?;
    let read = bytes.len() as u64;
    let text = String::from_utf8_lossy(&bytes);
    let mut all: Vec<&str> = text.lines().collect();
    if start > 0 && !all.is_empty() {
        // The first line was probably cut.
        all.remove(0);
    }
    let skip = all.len().saturating_sub(*lines_left);
    let truncated = start > 0 || skip > 0;
    let lines: Vec<String> = all[skip..]
        .iter()
        .map(|line| cut(&redactor.line(line), limits.line_chars))
        .collect();
    *lines_left -= lines.len();
    Some(Log {
        value: json!({
            "file": redactor.line(&name),
            "size": size,
            "truncated": truncated,
            "lines": lines,
        }),
        read,
    })
}

/// `line` cut to `max` characters.
fn cut(line: &str, max: usize) -> String {
    if line.chars().count() <= max {
        return line.to_owned();
    }
    let mut text: String = line.chars().take(max).collect();
    text.push_str(" [cut]");
    text
}

/// What a field's value may look like in a log line: `\"…\"` (JSON inside
/// a quoted string), `"…"`, `'…'`, a quote not closed before the end of the
/// line, `Type { … }`, `Type(…)` (`Some("…")`), `{ … }`, `( … )`, `[ … ]`
/// (nested up to four levels), or a bare word.
static FIELD_VALUE: LazyLock<String> = LazyLock::new(|| {
    let parens = nested(r"\(", r"\)", "()");
    let braces = nested(r"\{", r"\}", "{}");
    let brackets = nested(r"\[", r"\]", r"\[\]");
    [
        r#"\\"(?:[^\\]|\\[^"])*\\""#.to_owned(),
        r#""(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'"#.to_owned(),
        r#""(?:[^"\\]|\\.)*$|'(?:[^'\\]|\\.)*$"#.to_owned(),
        format!(r"[A-Za-z_][A-Za-z0-9_:]*\s*{braces}"),
        format!(r"[A-Za-z_][A-Za-z0-9_:]*{parens}"),
        braces,
        parens,
        brackets,
        r"[^\s&,;)}\]:][^\s&,;)}\]]*".to_owned(),
    ]
    .join("|")
});

/// A bracketed group (`open`, `close`, both in `class` form) with up to
/// four levels of nesting.
fn nested(open: &str, close: &str, class: &str) -> String {
    // A quoted string inside is one unit, so a bracket in it does not end
    // the group; a lone quote is an ordinary character.
    let quoted = r#""(?:[^"\\]|\\.)*""#;
    let mut pattern = format!(r#"{open}(?:{quoted}|[^{class}"]|")*{close}"#);
    for _ in 0..3 {
        pattern = format!(r#"{open}(?:{quoted}|[^{class}"]|"|{pattern})*{close}"#);
    }
    pattern
}

/// Between a field's name and its value: `=`, `:`, `=>`, `->`, `%3D`,
/// after an optional closing quote (also `\"`).
const FIELD_SEPARATOR: &str = r#"(\\?["']?\s*(?:=>|->|%3[Dd]|[=:])\s*)"#;

/// Secret fields: a field (`name=value`, `name: value`, `"name": "value"`,
/// `name => value`) whose name contains one of the words, is or ends in one
/// of the short ones, or ends in `_key`, has its value redacted. A name
/// right after `::` is a module path (a log target), not a field; after a
/// single `:` it is one.
static SECRET_FIELD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?i)(^:?|[^:A-Za-z0-9_]|[^:]:)((?:[A-Za-z0-9_.-]*(?:token|passw(?:or)?d|passphrase|pwd|secret|credential|api[_-]?key|private[_-]?key|access[_-]?key|auth|cookie|session|signature|bearer)[A-Za-z0-9_.-]*)|(?:[A-Za-z0-9.-]*[_.-])?(?:sig|pass|pw|pin|otp)|[A-Za-z0-9.-]*[_.-]key){FIELD_SEPARATOR}({value})",
        value = *FIELD_VALUE,
    ))
    .expect("valid regex")
});

/// A secret command-line option and its value: `--password X`.
static SECRET_OPTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(^|\s)(--?[A-Za-z0-9_-]*(?:passw(?:or)?d|passphrase|token|secret|api-?key|key|pass|pw)[A-Za-z0-9_-]*)(\s+)([^\s-]\S*)")
        .expect("valid regex")
});

/// The credential after an HTTP authorization scheme.
static AUTH_SCHEME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(bearer|basic|digest|token)(\s+)([A-Za-z0-9._~+/=-]+)").expect("valid regex")
});

/// A URL: scheme, user information, host, and the rest (port, path, query).
static URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)\b([a-z][a-z0-9+.-]*://)([^\s/@?#"'<>]*@)?(\[[0-9a-f:.]+\]|[^\s/:?#"'<>\[\]@]+)([^\s"'<>]*)"#,
    )
    .expect("valid regex")
});

/// A Windows UNC path's server: `\\server\share`.
static UNC_SERVER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(^|[\s"'=(\[])(\\{2,})([A-Za-z0-9._-]+)(\\+)"#).expect("valid regex")
});

/// A verbatim UNC path's server: `\\?\UNC\server\share`.
static VERBATIM_UNC: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(\\{2,}\?\\+UNC\\+)([A-Za-z0-9._-]+)(\\+)").expect("valid regex")
});

/// The agent's own Windows pipe (`\\.\pipe\deckpilot-agent-<hash of the
/// data directory>`): the same in every report, not a secret.
static AGENT_PIPE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^deckpilot-agent-[0-9a-f]{16}$").expect("valid regex"));

/// Tokens of well-known services and JSON web tokens.
static KNOWN_TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"\b(?:gh[pousr]_[A-Za-z0-9]{16,}|github_pat_[A-Za-z0-9_]{16,}|glpat-[A-Za-z0-9_-]{16,}",
        r"|xox[abprs]-[A-Za-z0-9-]{10,}|sk-[A-Za-z0-9_-]{16,}|AKIA[0-9A-Z]{16}",
        r"|eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]*)",
    ))
    .expect("valid regex")
});

/// A long run of letters, digits and `+=_-` (a token, a key, a hash).
static OPAQUE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Za-z0-9+=_-]{32,}").expect("valid regex"));

/// The same with `/` (base64, a PEM line): redacted only when it mixes
/// upper and lower case and digits, which paths rarely do.
/// It must start a word (not right after a letter, a digit or `.`), so a
/// path or URL that goes on after a `.` is not taken for one.
static OPAQUE_SLASHED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(^|[^A-Za-z0-9.+/=_-])([A-Za-z0-9+/=_-]{32,})").expect("valid regex")
});

/// A device ID as the agent logs it: `<model>#<serial>`, the model a dotted
/// ID (`elgato.streamdeck.mk2`).
static DEVICE_ID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b([a-z0-9_-]+(?:\.[a-z0-9_-]+)+)#([A-Za-z0-9_-]+(?::[A-Za-z0-9_-]+)?)")
        .expect("valid regex")
});

/// A serial number field (also `ID_SERIAL_SHORT=`, `serial_number:
/// Some("…")`, `serial ABC123`).
static SERIAL_FIELD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r#"(?i)(serial(?:[_-]?(?:number|no|short))?)(\\?["']?\s*(?:=>|->|[=:])\s*|\s+)"#,
        r#"(\\"(?:[^\\]|\\[^"])*\\"|"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'"#,
        r#"|Some\((?:"(?:[^"\\]|\\.)*"|[^)]*)\)|[A-Za-z0-9_-]+)"#,
    ))
    .expect("valid regex")
});

/// Another user's home directory (the user's own is `~` already).
static USER_DIRECTORY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)(/home/|/Users/|/var/home/|/media/|/run/media/|[a-z]:[\\/]+Users[\\/]+|\\\\[^\\\s]+\\+Users\\+)([^/\\\s"']+)"#)
        .expect("valid regex")
});

static EMAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}\b")
        .expect("valid regex")
});

static IPV4: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})\b").expect("valid regex")
});

/// An IPv6 address (four or more groups, or a `::`), with a zone.
static IPV6: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?i)(^|[^0-9a-z:.])(",
        r"(?:[0-9a-f]{1,4}:){3,7}[0-9a-f]{1,4}",
        r"|(?:[0-9a-f]{1,4}:){1,6}(?::[0-9a-f]{1,4}){1,6}",
        r"|(?:[0-9a-f]{1,4}:){1,7}:",
        r"|::(?:[0-9a-f]{1,4}(?::[0-9a-f]{1,4}){0,6})?",
        r")(%[0-9a-z]+)?",
    ))
    .expect("valid regex")
});

/// A webhook URL's secret path: everything after these segments.
const WEBHOOK_SEGMENTS: [&str; 3] = ["/webhooks/", "/services/", "/hooks/"];

/// A URL without what may identify the user: user information goes, and a
/// host other than GitHub's or loopback becomes `<host>`, its query values
/// `<redacted>` and a webhook path's secret part too.
fn url(c: &Captures<'_>) -> String {
    let scheme = &c[1];
    let userinfo = if c.get(2).is_some() {
        "<redacted>@"
    } else {
        ""
    };
    let host = &c[3];
    let rest = &c[4];
    if public_host(host) {
        return format!("{scheme}{userinfo}{host}{rest}");
    }
    let (path, query) = match rest.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (rest, None),
    };
    let mut path = path.to_owned();
    let lower = path.to_ascii_lowercase();
    if let Some(at) = WEBHOOK_SEGMENTS
        .iter()
        .filter_map(|segment| lower.find(segment).map(|at| at + segment.len()))
        .min()
    {
        path.truncate(at);
        path.push_str(REDACTED);
    }
    let query = query.map(|query| {
        query
            .split('&')
            .map(|pair| match pair.split_once('=') {
                Some((name, value)) if !value.is_empty() => format!("{name}={REDACTED}"),
                _ => pair.to_owned(),
            })
            .collect::<Vec<_>>()
            .join("&")
    });
    match query {
        Some(query) => format!("{scheme}{userinfo}<host>{path}?{query}"),
        None => format!("{scheme}{userinfo}<host>{path}"),
    }
}

/// A field value without the `Some(…)` and quotes around it.
fn bare_value(value: &str) -> &str {
    let value = value
        .strip_prefix("Some(")
        .and_then(|inner| inner.strip_suffix(')'))
        .unwrap_or(value);
    let value = value
        .strip_prefix("\\\"")
        .and_then(|inner| inner.strip_suffix("\\\""))
        .unwrap_or(value);
    value.trim_matches(['"', '\''])
}

/// Hosts that say nothing about the user: DeckPilot's own and loopback.
fn public_host(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    let host = host.trim_end_matches('.');
    host == "localhost"
        || host == "[::1]"
        || host.starts_with("127.")
        || ["github.com", "github.io", "githubusercontent.com"]
            .iter()
            .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
}

/// Removes what may identify the user from log lines (module docs).
pub struct LogRedactor {
    /// The home directory's spellings, compiled once.
    home: Vec<Regex>,
    user: Option<String>,
    hosts: Vec<String>,
    /// Each known serial (without case) and its pseudonym.
    serials: Vec<(Regex, String)>,
    machine_id: Option<String>,
    machine_id_pattern: Option<Regex>,
}

impl std::fmt::Debug for LogRedactor {
    /// Without the values it removes.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogRedactor").finish_non_exhaustive()
    }
}

impl LogRedactor {
    pub fn new(env: &Environment) -> Self {
        let mut home = Vec::new();
        if let Some(path) = &env.home {
            let text = path.display().to_string();
            let text = text.trim_end_matches(['/', '\\']).to_owned();
            if text.len() > 1 {
                home.push(text.replace('\\', "/"));
                home.push(text.replace('/', "\\"));
                // JSON and Debug output escape backslashes.
                home.push(text.replace('/', "\\").replace('\\', "\\\\"));
                home.sort();
                home.dedup();
                // Longest first, so a variant never leaves part of another.
                home.sort_by_key(|variant| std::cmp::Reverse(variant.len()));
            }
        }
        let mut hosts = Vec::new();
        if let Some(name) = &env.hostname {
            hosts.push(name.clone());
            if let Some((short, _)) = name.split_once('.') {
                hosts.push(short.to_owned());
            }
            hosts.sort_by_key(|host| std::cmp::Reverse(host.len()));
        }
        let machine_id = env.machine_id.clone().filter(|id| id.len() >= 8);
        let serials = env
            .known_serials()
            .into_iter()
            .filter(|serial| serial.len() >= 4)
            .filter_map(|serial| {
                let pattern = literal_ignore_case(&serial)?;
                Some((pattern, serial_pseudonym(&serial, machine_id.as_deref())))
            })
            .collect();
        Self {
            home: home
                .iter()
                .filter_map(|prefix| path_prefix(prefix))
                .collect(),
            user: env.user.clone(),
            hosts,
            serials,
            machine_id_pattern: machine_id.as_deref().and_then(literal_ignore_case),
            machine_id,
        }
    }

    fn pseudonym(&self, serial: &str) -> String {
        serial_pseudonym(serial, self.machine_id.as_deref())
    }

    /// `line` without what may identify the user.
    pub fn line(&self, line: &str) -> String {
        let text = Self::credentials(line);
        let text = self.serials(text);
        let text = self.names(text);
        Self::opaque(&text)
    }

    /// Credentials first: they may contain anything the later rules would
    /// split.
    fn credentials(line: &str) -> String {
        let mut text = URL.replace_all(line, url).into_owned();
        text = UNC_SERVER
            .replace_all(&text, |c: &Captures<'_>| {
                // `\\.\` is the local device namespace (the agent's pipe),
                // not a server; `\\?\UNC\server` names one after `UNC`.
                if &c[3] == "." {
                    c[0].to_owned()
                } else {
                    format!("{}{}<host>{}", &c[1], &c[2], &c[4])
                }
            })
            .into_owned();
        text = VERBATIM_UNC
            .replace_all(&text, |c: &Captures<'_>| {
                format!("{}<host>{}", &c[1], &c[3])
            })
            .into_owned();
        text = KNOWN_TOKEN.replace_all(&text, REDACTED).into_owned();
        text = AUTH_SCHEME
            .replace_all(&text, |c: &Captures<'_>| {
                let value = &c[3];
                let word = c[1].to_ascii_lowercase();
                // "token" is also an ordinary word: only a value that looks
                // like one.
                let keep = word == "token"
                    && !(value.len() >= 8 && value.chars().any(|ch| ch.is_ascii_digit()));
                if keep {
                    c[0].to_owned()
                } else {
                    format!("{}{}{REDACTED}", &c[1], &c[2])
                }
            })
            .into_owned();
        text = SECRET_OPTION
            .replace_all(&text, |c: &Captures<'_>| {
                format!("{}{}{}{REDACTED}", &c[1], &c[2], &c[3])
            })
            .into_owned();
        SECRET_FIELD
            .replace_all(&text, |c: &Captures<'_>| {
                format!("{}{}{}{REDACTED}", &c[1], &c[2], &c[3])
            })
            .into_owned()
    }

    /// Serials, as the report's pseudonyms, and the machine ID.
    fn serials(&self, mut text: String) -> String {
        text = DEVICE_ID
            .replace_all(&text, |c: &Captures<'_>| {
                let serial = &c[2];
                if serial.starts_with("sha256:") {
                    c[0].to_owned()
                } else {
                    format!("{}#{}", &c[1], self.pseudonym(serial))
                }
            })
            .into_owned();
        text = SERIAL_FIELD
            .replace_all(&text, |c: &Captures<'_>| {
                let raw = bare_value(&c[3]);
                // `serial number`: only a value that looks like a serial
                // after a space.
                let spaced = c[2].trim().is_empty();
                if raw.starts_with("sha256:")
                    || raw.eq_ignore_ascii_case("sha256")
                    || raw.is_empty()
                    || (spaced && !raw.chars().any(|ch| ch.is_ascii_digit()))
                {
                    c[0].to_owned()
                } else {
                    format!("{}{}{}", &c[1], &c[2], self.pseudonym(raw))
                }
            })
            .into_owned();
        for (pattern, pseudonym) in &self.serials {
            text = pattern
                .replace_all(&text, regex::NoExpand(pseudonym))
                .into_owned();
        }
        if let Some(pattern) = &self.machine_id_pattern {
            text = pattern.replace_all(&text, "<machine-id>").into_owned();
        }
        text
    }

    /// Paths, names and addresses.
    fn names(&self, mut text: String) -> String {
        for home in &self.home {
            text = home
                .replace_all(&text, |c: &Captures<'_>| format!("~{}", &c["end"]))
                .into_owned();
        }
        text = USER_DIRECTORY
            .replace_all(&text, |c: &Captures<'_>| {
                let name = &c[2];
                let common = ["shared", "public", "default", "all users", "<user>", "~"];
                if common.contains(&name.to_lowercase().as_str()) {
                    c[0].to_owned()
                } else {
                    format!("{}<user>", &c[1])
                }
            })
            .into_owned();
        text = EMAIL.replace_all(&text, "<email>").into_owned();
        for host in &self.hosts {
            text = replace_word(&text, host, "<host>", true);
        }
        if let Some(user) = &self.user {
            text = replace_word(&text, user, "<user>", true);
        }
        text = IPV4
            .replace_all(&text, |c: &Captures<'_>| {
                let octets: Vec<u16> = (1..=4).filter_map(|i| c[i].parse().ok()).collect();
                let valid = octets.len() == 4 && octets.iter().all(|octet| *octet <= 255);
                let keep = !valid
                    || octets[0] == 127
                    || octets.iter().all(|octet| *octet == 0)
                    || octets.iter().all(|octet| *octet == 255);
                if keep {
                    c[0].to_owned()
                } else {
                    "<ip>".to_owned()
                }
            })
            .into_owned();
        IPV6.replace_all(&text, |c: &Captures<'_>| {
            let address = &c[2];
            if address == "::" || address == "::1" {
                c[0].to_owned()
            } else {
                format!("{}<ip>", &c[1])
            }
        })
        .into_owned()
    }

    /// Last: whatever long opaque string is left.
    fn opaque(text: &str) -> String {
        let text = OPAQUE_SLASHED.replace_all(text, |c: &Captures<'_>| {
            let run = &c[2];
            // A run that starts with `/` is a path (`/var/folders/…/T/…`).
            let mixed = run.contains('/')
                && !run.starts_with('/')
                && run.chars().any(|ch| ch.is_ascii_digit())
                && run.chars().any(|ch| ch.is_ascii_uppercase())
                && run.chars().any(|ch| ch.is_ascii_lowercase());
            if mixed {
                format!("{}{REDACTED}", &c[1])
            } else {
                c[0].to_owned()
            }
        });
        OPAQUE
            .replace_all(&text, |c: &Captures<'_>| {
                let run = &c[0];
                let digits = run.chars().any(|ch| ch.is_ascii_digit());
                let letters = run.chars().any(|ch| ch.is_ascii_alphabetic());
                if digits && letters && !AGENT_PIPE.is_match(run) {
                    REDACTED.to_owned()
                } else {
                    run.to_owned()
                }
            })
            .into_owned()
    }

    /// Every string in `value` (keys too) through [`Self::line`].
    pub fn value(&self, value: &mut Value) {
        match value {
            Value::String(text) => *text = self.line(text),
            Value::Array(items) => items.iter_mut().for_each(|item| self.value(item)),
            Value::Object(map) => {
                let entries = std::mem::take(map);
                for (key, mut item) in entries {
                    self.value(&mut item);
                    map.insert(self.line(&key), item);
                }
            }
            _ => {}
        }
    }
}

/// A pattern that finds `needle` without case.
fn literal_ignore_case(needle: &str) -> Option<Regex> {
    Regex::new(&format!("(?i){}", regex::escape(needle))).ok()
}

/// A pattern that finds `prefix` (a directory) where it is a whole path:
/// followed by a separator or by something that cannot be part of a file
/// name (`end`). Without case (macOS and Windows file systems are usually
/// case-insensitive).
fn path_prefix(prefix: &str) -> Option<Regex> {
    Regex::new(&format!(
        r#"(?i){}(?P<end>[/\\\s"',;:)\]}}]|$)"#,
        regex::escape(prefix)
    ))
    .ok()
}

#[cfg(test)]
#[path = "bundle_tests.rs"]
mod tests;
