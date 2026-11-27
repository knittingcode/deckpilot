//! Names of the user, of this computer, of the domain and of the
//! organisation in paths and messages. The report's `Redactor` and the
//! bundle's `LogRedactor` share these rules, so plain `diagnose` does not
//! rely on the bundle's second pass.

use std::sync::LazyLock;

use regex::{Captures, Regex};

use super::replace_word;

/// Names that are ordinary path words: a domain or organisation called so
/// is not replaced (it would destroy the report's paths).
const COMMON_WORDS: [&str; 23] = [
    "users",
    "user",
    "home",
    "appdata",
    "local",
    "roaming",
    "program files",
    "windows",
    "deckpilot",
    "public",
    "default",
    "shared",
    "temp",
    "workgroup",
    // The placeholders: replacing them would nest them (`<<host>>`).
    "host",
    "share",
    "domain",
    "org",
    "id",
    "ip",
    "email",
    "redacted",
    "machine-id",
];

/// Each of `names` (of two or more characters, longest first, without
/// case) replaced with `replacement` wherever it stands as a word.
pub(crate) fn words(text: &str, names: &[String], replacement: &str) -> String {
    let mut names: Vec<&String> = names
        .iter()
        .filter(|name| !COMMON_WORDS.contains(&name.to_lowercase().as_str()))
        .collect();
    names.sort_by_key(|name| std::cmp::Reverse(name.chars().count()));
    let mut text = text.to_owned();
    for name in names {
        text = replace_word(&text, name, replacement, true);
    }
    text
}

/// The domain and the organisation names worth replacing: not empty, not
/// an ordinary path word and not a built-in account domain.
pub(crate) fn useful_names(names: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = names
        .into_iter()
        .map(|name| name.trim().to_owned())
        .filter(|name| name.chars().count() >= 2)
        .filter(|name| {
            let lower = name.to_lowercase();
            !COMMON_WORDS.contains(&lower.as_str())
                && !["nt authority", "builtin", "azuread"].contains(&lower.as_str())
        })
        .collect();
    out.sort();
    out.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    out
}

/// A UNC path's server and share: `\\server\share`, `\\?\UNC\server\share`,
/// `\\.\UNC\server\share`, `\??\UNC\server`, `\Device\Mup\server`,
/// WebDAV servers (`server@SSL@8443`), `/` or mixed separators after the
/// server, `//server/share` in text, and doubled (escaped) backslashes.
/// The local device namespaces (`\\.\pipe\…`, `\\?\C:\…`) are kept, and so
/// are the administrative shares (`C$`, `ADMIN$`, `IPC$`) and `pipe`.
pub(crate) fn unc(text: &str) -> String {
    let text = VERBATIM_UNC.replace_all(text, |c: &Captures<'_>| {
        format!("{}<host>{}{}", &c["lead"], &c["dav"], share(c))
    });
    let text = SLASHED_UNC.replace_all(&text, |c: &Captures<'_>| {
        format!("{}{}<host>{}{}", &c["pre"], &c["lead"], &c["dav"], share(c))
    });
    UNC.replace_all(&text, |c: &Captures<'_>| {
        let end = c.name("end").map_or("", |end| end.as_str());
        // A bare `\\x` at the end of a string: a single letter is an
        // escape (`"\\n"`), not a server.
        if c.name("sep").is_none() && c["server"].len() < 2 {
            return c[0].to_owned();
        }
        format!(
            "{}{}<host>{}{}{end}",
            &c["pre"],
            &c["lead"],
            &c["dav"],
            share(c)
        )
    })
    .into_owned()
}

/// `//server/share` at the start of a path (Windows accepts `/`): only for
/// a path, never in free text, where `//` is a comment or a URL's.
pub(crate) fn forward_unc(path: &str) -> String {
    FORWARD_UNC
        .replace(path, |c: &Captures<'_>| {
            let share = match c.name("share") {
                Some(share) if is_admin_share(share.as_str()) => share.as_str(),
                Some(_) => "<share>",
                None => "",
            };
            format!("//<host>/{share}")
        })
        .into_owned()
}

/// The separator and share that follow a redacted server.
fn share(c: &Captures<'_>) -> String {
    let Some(sep) = c.name("sep") else {
        return String::new();
    };
    let sep = sep.as_str();
    // A share with spaces is taken only up to the next backslash, which
    // the pattern consumed.
    let (share, after) = match (c.name("spaced"), c.name("share")) {
        (Some(spaced), _) => (spaced.as_str(), c.name("after").map_or("", |a| a.as_str())),
        (None, Some(share)) => (share.as_str(), ""),
        (None, None) => return sep.to_owned(),
    };
    if is_admin_share(share) {
        format!("{sep}{share}{after}")
    } else {
        format!("{sep}<share>{after}")
    }
}

fn is_admin_share(share: &str) -> bool {
    let lower = share.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "admin$" | "ipc$" | "print$" | "pipe" | "mailslot" | "davwwwroot"
    ) || (lower.len() == 2
        && lower.ends_with('$')
        && lower.starts_with(|c: char| c.is_ascii_alphabetic()))
}

/// What a share name may be: a run with spaces that ends before a
/// separator (consumed as `after`), or a run without spaces.
const SHARE: &str = r#"(?:(?P<spaced>[^\\/"'<>|:*?\r\n]*[^\\/\s"'<>|:*?])(?P<after>[\\/])|(?P<share>[^\\/\s"'<>|:*?,;()\[\]{}]+))"#;

/// A UNC server: a name, and the WebDAV forms `server@SSL`, `server@8443`
/// and `server@SSL@8443`.
const SERVER: &str = r"(?P<server>[A-Za-z0-9_-][A-Za-z0-9._-]*)(?P<dav>(?:@(?:SSL|\d{1,5}))*)";

/// `\\server\share`. Before it: the start, a space, a quote, or one of
/// `=([<,;{|*` or a backtick (`<\\server`, `,\\server`), each also
/// followed by a `:`,
/// or a `:` after a word of two or more characters (`path:\\server`): not
/// a drive (`C:\\x`) and not a file name character, so an escaped path
/// (`C:\\Program Files (x86)\\x`, `~\\x`, `<user>\\x`) keeps its
/// directories.
static UNC: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r#"(?P<pre>^:?|[\s"'=(\[<,;{{|`*]:?|[A-Za-z0-9_]{{2,}}:)(?P<lead>\\{{2,}})(?i:{SERVER})(?:(?P<sep>[\\/]+){SHARE}?|(?P<end>$|["'>]))"#,
    ))
    .expect("valid regex")
});

/// `//server/share` in text: only after the start, a space, a quote or
/// `=([<,;{|*` (never after `scheme:`, which is a URL's), and only with a
/// share, so a `// comment` or a lone `//x` stays.
static SLASHED_UNC: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r#"(?P<pre>^|[\s"'=(\[<,;{{|`*])(?P<lead>//)(?i:{SERVER})(?P<sep>[\\/]+){SHARE}"#,
    ))
    .expect("valid regex")
});

/// `\\?\UNC\server\share` and `\\.\UNC\server\share` (also with `/`), and
/// the NT forms `\??\UNC\server` and `\Device\Mup\server`.
static VERBATIM_UNC: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?i)(?P<lead>(?:[\\/]{{2,}}[?.]|\\+\?\?)[\\/]+UNC[\\/]+|\\+Device\\+Mup\\+){SERVER}(?:(?P<sep>[\\/]+){SHARE}?)?",
    ))
    .expect("valid regex")
});

static FORWARD_UNC: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^//(?P<server>[A-Za-z0-9_-][A-Za-z0-9._-]*)(?:/(?P<share>[^/\\]+))?")
        .expect("valid regex")
});

/// An 8.3 short name (`ALEXAN~1`, `ALICE~1.COR`) made from the user name:
/// its stem is a prefix of the name as Windows shortens it. It becomes
/// `<user>`.
pub(crate) fn short_names(text: &str, user: &str) -> String {
    let bases = short_name_bases(user);
    if bases.is_empty() {
        return text.to_owned();
    }
    SHORT_NAME
        .replace_all(text, |c: &Captures<'_>| {
            let stem = c["stem"].to_uppercase();
            if bases.iter().any(|base| base.starts_with(&stem)) {
                format!("{}<user>{}", &c["pre"], &c["end"])
            } else {
                c[0].to_owned()
            }
        })
        .into_owned()
}

/// The user name as the stem of its 8.3 name would start: upper case,
/// without spaces and dots, other characters 8.3 does not allow as `_`;
/// once with the part after the last dot (an extension) left out, once
/// without dots at all.
fn short_name_bases(user: &str) -> Vec<String> {
    let clean = |text: &str| -> String {
        text.chars()
            .filter(|c| *c != ' ' && *c != '.')
            .map(|c| if "+,;=[]".contains(c) { '_' } else { c })
            .collect::<String>()
            .to_uppercase()
    };
    let mut bases = vec![clean(user)];
    if let Some((stem, _)) = user.rsplit_once('.') {
        bases.push(clean(stem));
    }
    bases.retain(|base| !base.is_empty());
    bases
}

static SHORT_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?P<pre>^|[^A-Za-z0-9_$!#%&@^~-])(?P<stem>[A-Za-z0-9_$!#%&@^-]{1,6})~[0-9]{1,6}(?:\.[A-Za-z0-9_$!#%&@^-]{1,3})?(?P<end>[^A-Za-z0-9_$!#%&@^~.-]|$)",
    )
    .expect("valid regex")
});

/// Another user's home directory: the user part becomes `<user>` (the
/// user's own is `~` already).
pub(crate) fn other_users(text: &str) -> String {
    USER_DIRECTORY
        .replace_all(text, |c: &Captures<'_>| {
            let name = &c[2];
            let common = ["shared", "public", "default", "all users", "<user>", "~"];
            if common.contains(&name.to_lowercase().as_str()) {
                c[0].to_owned()
            } else {
                format!("{}<user>", &c[1])
            }
        })
        .into_owned()
}

static USER_DIRECTORY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)(/home/|/Users/|/var/home/|/media/|/run/media/|[a-z]:[\\/]+Users[\\/]+|\\\\[^\\\s]+\\+Users\\+)([^/\\\s"']+)"#)
        .expect("valid regex")
});

/// What qualifies the user name once it is `<user>`: `CORP\<user>` becomes
/// `<domain>\<user>` and `<user>@corp.example.com` `<user>@<domain>`. A
/// path's directory (`Users\<user>`, after a separator) is not a domain.
pub(crate) fn qualified_user(text: &str) -> String {
    let text = DOMAIN_USER.replace_all(text, "${pre}<domain>${sep}<user>");
    USER_AT_DOMAIN
        .replace_all(&text, "<user>@<domain>")
        .into_owned()
}

static DOMAIN_USER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?P<pre>^|[\s"'=(\[<,;:])[A-Za-z0-9][A-Za-z0-9._-]*(?P<sep>\\{1,2})<user>"#)
        .expect("valid regex")
});

static USER_AT_DOMAIN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<user>@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+").expect("valid regex"));

/// The organisation of a `OneDrive` for work or school folder
/// (`OneDrive - Contoso`): `OneDrive - <org>`.
pub(crate) fn onedrive(text: &str) -> String {
    ONEDRIVE.replace_all(text, "${1}<org>").into_owned()
}

static ONEDRIVE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)(OneDrive\s+-\s+)[^<\\/"'>|:*?\r\n][^\\/"'<>|:*?\r\n]*"#)
        .expect("valid regex")
});

/// The organisation names of this user's `OneDrive` for work or school
/// folders (Windows: `OneDriveCommercial`, `OneDrive`), for [`words`].
pub(crate) fn organisations_from(folders: impl IntoIterator<Item = String>) -> Vec<String> {
    useful_names(folders.into_iter().filter_map(|folder| {
        let name = folder
            .trim_end_matches(['\\', '/'])
            .rsplit(['\\', '/'])
            .next()?;
        let rest = name
            .get(..8)
            .filter(|prefix| prefix.eq_ignore_ascii_case("onedrive"))
            .map(|_| &name[8..])?;
        let org = rest.trim_start().strip_prefix('-')?.trim();
        Some(org.to_owned())
    }))
}
