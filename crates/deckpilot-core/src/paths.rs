//! Paths and URLs inside resource bindings: the containment and
//! validation the built-in launchers (`system`, `workspace`) and the plugin
//! host (resource handles, plugin API 1.9) share. A relative path never
//! leaves its resource, by name (`..`, absolute paths, drive prefixes) or
//! through a link; a URL to open is `http`, `https` or `mailto` only.

use std::path::{Component, Path, PathBuf};

use crate::{ActionResult, ExecutionContext, ResourceKind, ResourceRef};

/// Error codes of these checks, as [`ActionResult::code`].
pub mod codes {
    /// The URL is not a valid `http`, `https` or `mailto` address.
    pub const INVALID_URL: &str = "invalid-url";
    /// A relative path leaves its resource (`..`) or is absolute.
    pub const INVALID_PATH: &str = "invalid-path";
    /// The resolved file, folder or program does not exist.
    pub const NOT_FOUND: &str = "not-found";
}

/// A failure, as the action's result.
pub type Failure = ActionResult;

fn fail(code: &str, message: impl Into<String>) -> Failure {
    ActionResult::error(code, message)
}

/// Resolves a resource parameter to its value on this computer.
pub fn resolve(
    context: &ExecutionContext<'_>,
    reference: Option<&ResourceRef>,
    kind: Option<ResourceKind>,
    what: &str,
) -> Result<String, Failure> {
    let reference = reference.ok_or_else(|| fail("invalid-parameters", format!("no {what}")))?;
    context
        .resolve(reference, kind)
        .map(|binding| binding.value.clone())
        .map_err(|error| fail(error.code(), error.to_string()))
}

/// `relative` joined to `base`, refusing absolute paths and `..`, so the
/// result stays inside the resource.
pub fn join_inside(base: &str, relative: Option<&str>) -> Result<PathBuf, Failure> {
    let base = PathBuf::from(base);
    let Some(relative) = relative.map(str::trim).filter(|r| !r.is_empty()) else {
        return Ok(base);
    };
    let path = Path::new(relative);
    let inside = path
        .components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
    // `C:foo` or `\\server` parse as prefixes on Windows only; reject the
    // characters everywhere so a workflow means the same on every machine.
    if !inside || relative.contains(':') || relative.starts_with(['/', '\\']) {
        return Err(fail(
            codes::INVALID_PATH,
            format!("“{relative}” must be a relative path inside the resource"),
        ));
    }
    Ok(base.join(path))
}

/// An existing file or folder.
pub fn existing(path: PathBuf, directory: bool) -> Result<PathBuf, Failure> {
    let found = if directory {
        path.is_dir()
    } else {
        path.exists()
    };
    if found {
        Ok(path)
    } else {
        Err(fail(
            codes::NOT_FOUND,
            format!(
                "{} {} does not exist",
                if directory { "the folder" } else { "the file" },
                path.display()
            ),
        ))
    }
}

/// An existing file or folder at `relative` inside the resource folder
/// `base`, checked by name ([`join_inside`]) and by where it really is:
/// links are followed, and the target must still be inside `base`. Both
/// sides are canonicalized the same way, so `/var` and `/private/var` on
/// macOS and `\\?\` prefixes on Windows compare alike, and a resource
/// folder that is itself a link is fine. Without `relative`, the folder
/// itself is returned (it only has to exist).
///
/// The returned path is the joined one, as the user wrote it; it has been
/// checked to resolve inside the resource.
pub fn existing_inside(
    base: &str,
    relative: Option<&str>,
    directory: bool,
) -> Result<PathBuf, Failure> {
    let path = join_inside(base, relative)?;
    let path = existing(path, directory)?;
    if relative.map(str::trim).is_none_or(str::is_empty) {
        return Ok(path);
    }
    let unresolved = |what: &Path| {
        fail(
            codes::NOT_FOUND,
            format!("{} could not be resolved", what.display()),
        )
    };
    let root = Path::new(base)
        .canonicalize()
        .map_err(|_| unresolved(Path::new(base)))?;
    let target = path.canonicalize().map_err(|_| unresolved(&path))?;
    if target.starts_with(&root) {
        Ok(path)
    } else {
        Err(fail(
            codes::INVALID_PATH,
            format!(
                "“{}” leads outside the resource through a link",
                relative.unwrap_or_default().trim()
            ),
        ))
    }
}

/// A validated `http`, `https` or `mailto` URL.
pub fn web_url(text: &str) -> Result<String, Failure> {
    let text = text.trim();
    let invalid = |reason: &str| fail(codes::INVALID_URL, format!("“{text}” {reason}"));
    if text.len() > 2048 {
        return Err(invalid("is too long"));
    }
    let url = url::Url::parse(text).map_err(|_| invalid("is not a valid URL"))?;
    match url.scheme() {
        "http" | "https" if url.host().is_some() => Ok(url.to_string()),
        "mailto" => Ok(url.to_string()),
        _ => Err(invalid(
            "must be an http, https or mailto address; other schemes can start programs",
        )),
    }
}

/// Longest URL that is opened, in bytes (the plugin host's
/// `MAX_OPEN_URL_LEN` too).
pub const MAX_URL_LEN: usize = 2048;

/// Why a parsed URL may not be opened as a web page, in words that never
/// repeat it: the rules the built-in Website action ([`website_url`]) and
/// the plugin host's `url.openParam` share. `None` when it may.
pub fn http_url_problem(url: &url::Url) -> Option<&'static str> {
    if !matches!(url.scheme(), "http" | "https") {
        return Some("is not an http or https address");
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Some("carries a user name or password");
    }
    if url.as_str().len() > MAX_URL_LEN {
        return Some("is longer than 2048 bytes");
    }
    None
}

/// Whether `lower` (lower case) starts with a URL scheme: letters, digits,
/// `+`, `-` or `.` after a letter, then `:` not followed by a digit (so
/// `localhost:3000` is a host and a port).
fn has_scheme(lower: &str) -> bool {
    let Some((scheme, rest)) = lower.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '+' | '-' | '.'))
        && !rest.starts_with(|c: char| c.is_ascii_digit())
}

/// A web page address for the Website action, validated and normalised:
/// trimmed, `https://` added when no scheme is written (`example.com` →
/// `https://example.com/`), then only `http` and `https` with a host and
/// the shared rules ([`http_url_problem`]: no user name or password, at
/// most [`MAX_URL_LEN`] bytes), no backslash and no control characters.
/// The URL parser normalises the rest (lower-case scheme and host, an
/// internationalised host in its `xn--` form, spaces percent-encoded).
/// Every other scheme is refused, because it can start a program.
pub fn website_url(text: &str) -> Result<String, Failure> {
    let text = text.trim();
    let invalid = |reason: &str| fail(codes::INVALID_URL, format!("the address {reason}"));
    if text.is_empty() {
        return Err(invalid("is empty"));
    }
    if text.len() > MAX_URL_LEN {
        return Err(invalid("is longer than 2048 bytes"));
    }
    if text.chars().any(char::is_control) {
        return Err(invalid("contains control characters"));
    }
    if text.contains('\\') {
        return Err(invalid("contains a backslash"));
    }
    let lower = text.to_ascii_lowercase();
    let full = if has_scheme(&lower) {
        if !(lower.starts_with("http://") || lower.starts_with("https://")) {
            return Err(invalid(
                "must be an http or https address; other schemes can start programs",
            ));
        }
        text.to_owned()
    } else {
        format!("https://{text}")
    };
    let url = url::Url::parse(&full).map_err(|_| invalid("is not a valid web address"))?;
    if let Some(problem) = http_url_problem(&url) {
        return Err(invalid(problem));
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err(invalid("has no host"));
    }
    Ok(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn websites_are_validated_and_normalised() {
        for (text, normal) in [
            (" https://Example.COM/a?b=c ", "https://example.com/a?b=c"),
            ("example.com", "https://example.com/"),
            ("example.com/docs#intro", "https://example.com/docs#intro"),
            ("localhost:3000/x", "https://localhost:3000/x"),
            ("HTTP://example.com", "http://example.com/"),
            ("https://example.com/a b", "https://example.com/a%20b"),
            ("https://zażółć.pl", "https://xn--za-6ja4f8n1l.pl/"),
            ("http://[::1]:8080/", "http://[::1]:8080/"),
        ] {
            assert_eq!(website_url(text).unwrap(), normal, "{text}");
        }
        for bad in [
            "",
            "   ",
            "mailto:team@example.com",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "data:text/html,x",
            "vscode://open",
            "ftp://example.com",
            "https://",
            "https://user:pass@example.com",
            "https://bank.example@evil.example",
            "https://example.com\\@evil.example",
            "https://example.com/\u{7}",
            "exa mple.com",
        ] {
            assert_eq!(
                website_url(bad).unwrap_err().code.as_deref(),
                Some(codes::INVALID_URL),
                "{bad:?}"
            );
        }
        let long = format!("https://example.com/{}", "a".repeat(MAX_URL_LEN));
        assert!(website_url(&long).is_err());
    }

    #[test]
    fn relative_paths_stay_inside_their_resource() {
        assert_eq!(
            join_inside("/src/shop", Some("docs/README.md")).unwrap(),
            PathBuf::from("/src/shop/docs/README.md")
        );
        assert_eq!(
            join_inside("/src/shop", Some("  ")).unwrap(),
            PathBuf::from("/src/shop")
        );
        for bad in [
            "../etc",
            "/etc/passwd",
            "a/../../b",
            r"C:\Windows",
            r"\\server\x",
        ] {
            assert_eq!(
                join_inside("/src/shop", Some(bad))
                    .unwrap_err()
                    .code
                    .as_deref(),
                Some(codes::INVALID_PATH),
                "{bad}"
            );
        }
    }

    /// A link at `link` to `target`; false where links cannot be made
    /// (Windows without the privilege).
    fn link(target: &Path, link: &Path) -> bool {
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
    #[allow(clippy::too_many_lines)] // One tree of links and its cases.
    fn links_must_resolve_inside_their_resource() {
        let dir = std::env::temp_dir().join(format!(
            "deckpilot-links-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let base = dir.join("repo");
        let outside = dir.join("outside");
        std::fs::create_dir_all(base.join("sub")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(base.join("sub/inside.txt"), "in").unwrap();
        std::fs::write(outside.join("secret.txt"), "out").unwrap();
        // A sibling whose name starts like the resource's.
        std::fs::create_dir_all(dir.join("repo-other")).unwrap();
        std::fs::write(dir.join("repo-other/x.txt"), "x").unwrap();
        let made = [
            link(&outside.join("secret.txt"), &base.join("out-file")),
            link(&outside, &base.join("out-dir")),
            link(&dir.join("repo-other"), &base.join("sibling")),
            link(&base.join("sub/inside.txt"), &base.join("in-file")),
            link(&base.join("sub"), &base.join("in-dir")),
            link(&base.join("missing.txt"), &base.join("dangling")),
            link(&base, &dir.join("linked-repo")),
        ];
        if made.contains(&false) {
            eprintln!("links cannot be made here; skipped");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        let root = base.display().to_string();
        let code = |base: &str, relative: &str, directory: bool| {
            existing_inside(base, Some(relative), directory)
                .err()
                .and_then(|f| f.code)
                .unwrap_or_default()
        };

        // Plain paths and links that stay inside.
        assert_eq!(
            existing_inside(&root, Some("sub/inside.txt"), false).unwrap(),
            base.join("sub/inside.txt")
        );
        assert_eq!(code(&root, "in-file", false), "");
        assert_eq!(code(&root, "in-dir", true), "");
        assert_eq!(code(&root, "in-dir/inside.txt", false), "");
        assert_eq!(code(&root, "in-dir/../in-file", false), codes::INVALID_PATH);

        // Links that lead outside, to a file, a folder or through one.
        for (relative, directory) in [
            ("out-file", false),
            ("out-dir", true),
            ("out-dir", false),
            ("out-dir/secret.txt", false),
            ("sibling/x.txt", false),
        ] {
            assert_eq!(
                code(&root, relative, directory),
                codes::INVALID_PATH,
                "{relative}"
            );
        }
        let refused = existing_inside(&root, Some("out-file"), false).unwrap_err();
        assert!(refused.message.contains("outside"), "{}", refused.message);
        assert!(
            !refused.message.contains("secret"),
            "the target is not shown: {}",
            refused.message
        );

        // A link to nothing is not found.
        assert_eq!(code(&root, "dangling", false), codes::NOT_FOUND);

        // The resource folder may itself be a link: compared resolved
        // against resolved.
        let linked = dir.join("linked-repo").display().to_string();
        assert_eq!(code(&linked, "sub/inside.txt", false), "");
        assert_eq!(code(&linked, "in-file", false), "");
        assert_eq!(code(&linked, "out-file", false), codes::INVALID_PATH);
        assert_eq!(
            existing_inside(&linked, None, true).unwrap(),
            dir.join("linked-repo")
        );
        // The temporary folder is itself under a link on macOS
        // (/var -> /private/var); both spellings work.
        let resolved = base.canonicalize().unwrap().display().to_string();
        assert_eq!(code(&resolved, "in-file", false), "");
        assert_eq!(code(&resolved, "out-file", false), codes::INVALID_PATH);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_web_and_mail_urls_open() {
        assert_eq!(
            web_url(" https://example.com/a?b=c ").unwrap(),
            "https://example.com/a?b=c"
        );
        assert!(web_url("mailto:team@example.com").is_ok());
        for bad in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "vscode://open",
            "http://",
            "example.com",
        ] {
            assert_eq!(
                web_url(bad).unwrap_err().code.as_deref(),
                Some(codes::INVALID_URL),
                "{bad}"
            );
        }
    }
}
