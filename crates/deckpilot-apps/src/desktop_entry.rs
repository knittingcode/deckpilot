//! Linux desktop entries (`.desktop` files), as the freedesktop.org
//! Desktop Entry Specification 1.5 defines them: the `[Desktop Entry]`
//! group, its `Exec` line split by the specification's quoting rules and
//! its field codes expanded. Nothing is handed to a shell: the result is
//! an argument list.

use std::collections::BTreeMap;

use crate::{AppError, codes};

/// Largest desktop file read, in bytes.
pub const MAX_FILE_LEN: usize = 256 * 1024;
/// Most arguments an `Exec` line may expand to.
pub const MAX_ARGUMENTS: usize = 128;

/// The keys of a `[Desktop Entry]` group DeckPilot uses.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DesktopEntry {
    /// `Type`; only `Application` entries are applications.
    pub kind: String,
    /// `Name`, and its translations by locale (`Name[pl]`).
    pub name: String,
    pub names: BTreeMap<String, String>,
    pub exec: Option<String>,
    pub try_exec: Option<String>,
    pub icon: Option<String>,
    pub no_display: bool,
    pub hidden: bool,
    pub terminal: bool,
    pub mime_types: Vec<String>,
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::new(codes::INVALID_APPLICATION, message)
}

/// A string value with the specification's escapes (`\s`, `\n`, `\t`,
/// `\r`, `\\`) resolved; other backslashes are kept for the `Exec`
/// quoting rules.
fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(other) if other != '\\' => {
                out.push('\\');
                out.push(other);
            }
            Some(_) | None => out.push('\\'),
        }
    }
    out
}

fn boolean(value: &str) -> bool {
    value.trim() == "true"
}

impl DesktopEntry {
    /// Reads the `[Desktop Entry]` group of `text`.
    pub fn parse(text: &str) -> Result<Self, AppError> {
        if text.len() > MAX_FILE_LEN {
            return Err(invalid("the desktop file is too large"));
        }
        let mut entry = Self::default();
        let mut in_group = false;
        let mut seen_group = false;
        for line in text.lines() {
            let line = line.trim_end_matches('\r').trim_start();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') {
                in_group = line.trim_end() == "[Desktop Entry]";
                seen_group |= in_group;
                continue;
            }
            if !in_group {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = value.trim();
            match key {
                "Type" => value.clone_into(&mut entry.kind),
                "Name" => entry.name = unescape(value),
                "Exec" => entry.exec = Some(unescape(value)),
                "TryExec" => entry.try_exec = Some(unescape(value)),
                "Icon" => entry.icon = Some(unescape(value)),
                "NoDisplay" => entry.no_display = boolean(value),
                "Hidden" => entry.hidden = boolean(value),
                "Terminal" => entry.terminal = boolean(value),
                "MimeType" => {
                    entry.mime_types = value
                        .split(';')
                        .map(str::trim)
                        .filter(|m| !m.is_empty())
                        .map(str::to_owned)
                        .collect();
                }
                _ => {
                    if let Some(locale) = key
                        .strip_prefix("Name[")
                        .and_then(|rest| rest.strip_suffix(']'))
                    {
                        entry.names.insert(locale.to_owned(), unescape(value));
                    }
                }
            }
        }
        if !seen_group {
            return Err(invalid("the file has no [Desktop Entry] group"));
        }
        Ok(entry)
    }

    /// An application that can be started: `Type=Application` with an
    /// `Exec` line.
    pub fn is_application(&self) -> bool {
        self.kind == "Application" && self.exec.as_deref().is_some_and(|e| !e.trim().is_empty())
    }

    /// It registers itself for web pages.
    pub fn is_browser(&self) -> bool {
        self.mime_types
            .iter()
            .any(|m| m == "x-scheme-handler/http" || m == "x-scheme-handler/https")
    }

    /// The name in `locale` (`pl_PL`, then `pl`), else `Name`.
    pub fn localized_name(&self, locale: Option<&str>) -> &str {
        if let Some(locale) = locale {
            let plain = locale.split(['.', '@']).next().unwrap_or(locale);
            let language = plain.split('_').next().unwrap_or(plain);
            for key in [plain, language] {
                if let Some(name) = self.names.get(key) {
                    return name;
                }
            }
        }
        &self.name
    }

    /// The program and its arguments, from `Exec` with its field codes
    /// expanded: `%f`/`%u` take the first of `targets` (dropped without
    /// one), `%F`/`%U` all of them, `%i` the icon, `%c` the name, `%k`
    /// `location`; deprecated codes are removed and `%%` is a `%`. A line
    /// without a file or URL code gets `targets` at its end, as `GLib`'s
    /// launcher does.
    pub fn command(&self, targets: &[String], location: &str) -> Result<Vec<String>, AppError> {
        let exec = self
            .exec
            .as_deref()
            .ok_or_else(|| invalid("the desktop entry has no Exec line"))?;
        let words = split_exec(exec)?;
        // The program is written in the line, never a field code's value.
        if words
            .first()
            .is_none_or(|(word, _)| word.is_empty() || word.contains('%'))
        {
            return Err(invalid("the desktop entry's Exec line names no program"));
        }
        let mut argv = Vec::new();
        let mut placed = false;
        for (word, quoted) in words {
            if !quoted {
                match word.as_str() {
                    "%f" | "%u" => {
                        argv.extend(targets.first().cloned());
                        placed = true;
                        continue;
                    }
                    "%F" | "%U" => {
                        argv.extend(targets.iter().cloned());
                        placed = true;
                        continue;
                    }
                    "%i" => {
                        if let Some(icon) = self.icon.as_ref().filter(|i| !i.is_empty()) {
                            argv.push("--icon".into());
                            argv.push(icon.clone());
                        }
                        continue;
                    }
                    "%d" | "%D" | "%n" | "%N" | "%v" | "%m" => continue,
                    _ => {}
                }
            }
            if !quoted
                && ["%f", "%u", "%F", "%U"]
                    .iter()
                    .any(|code| word.contains(code))
            {
                placed = true;
            }
            argv.push(if quoted {
                word
            } else {
                expand_inline(&word, targets, &self.name, location)
            });
        }
        if argv.first().is_none_or(String::is_empty) {
            return Err(invalid("the desktop entry's Exec line names no program"));
        }
        if !placed {
            argv.extend(targets.iter().cloned());
        }
        if argv.len() > MAX_ARGUMENTS {
            return Err(invalid("the desktop entry's Exec line is too long"));
        }
        Ok(argv)
    }
}

/// Field codes inside a word (`--url=%u`): single values only.
fn expand_inline(word: &str, targets: &[String], name: &str, location: &str) -> String {
    let mut out = String::with_capacity(word.len());
    let mut chars = word.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('%') => out.push('%'),
            Some('f' | 'u' | 'F' | 'U') => out.push_str(targets.first().map_or("", String::as_str)),
            Some('c') => out.push_str(name),
            Some('k') => out.push_str(location),
            // `%i` inside a word and the deprecated codes expand to nothing.
            Some(_) | None => {}
        }
    }
    out
}

/// `Exec` split into words: unquoted words are separated by spaces; a
/// double-quoted word may contain spaces, and in it a backslash escapes
/// `"`, `` ` ``, `$` and `\`. Each word says whether it was quoted (field
/// codes are not expanded in quoted words).
fn split_exec(exec: &str) -> Result<Vec<(String, bool)>, AppError> {
    let mut words = Vec::new();
    let mut chars = exec.chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| *c == ' ' || *c == '\t') {
            chars.next();
        }
        let Some(&first) = chars.peek() else {
            break;
        };
        let mut word = String::new();
        if first == '"' {
            chars.next();
            let mut closed = false;
            while let Some(c) = chars.next() {
                match c {
                    '"' => {
                        closed = true;
                        break;
                    }
                    '\\' => match chars.next() {
                        Some(escaped @ ('"' | '`' | '$' | '\\')) => word.push(escaped),
                        Some(other) => {
                            word.push('\\');
                            word.push(other);
                        }
                        None => return Err(invalid("the Exec line ends inside a quote")),
                    },
                    other => word.push(other),
                }
            }
            if !closed {
                return Err(invalid("the Exec line has an unclosed quote"));
            }
            if chars.peek().is_some_and(|c| *c != ' ' && *c != '\t') {
                return Err(invalid("the Exec line has text right after a quote"));
            }
            words.push((word, true));
        } else {
            while let Some(&c) = chars.peek() {
                if c == ' ' || c == '\t' {
                    break;
                }
                if c == '"' {
                    return Err(invalid("the Exec line has a quote inside a word"));
                }
                word.push(c);
                chars.next();
            }
            words.push((word, false));
        }
    }
    Ok(words)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIREFOX: &str = "\
[Desktop Entry]
Version=1.0
Type=Application
Name=Firefox
Name[pl]=Firefox (przeglądarka)
Exec=firefox %u
Icon=firefox
MimeType=text/html;x-scheme-handler/http;x-scheme-handler/https;
Terminal=false

[Desktop Action new-window]
Name=New Window
Exec=firefox --new-window %u
";

    fn targets(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_owned()).collect()
    }

    #[test]
    fn the_desktop_entry_group_is_read() {
        let entry = DesktopEntry::parse(FIREFOX).unwrap();
        assert!(entry.is_application());
        assert!(entry.is_browser());
        assert_eq!(entry.name, "Firefox");
        assert_eq!(
            entry.localized_name(Some("pl_PL.UTF-8")),
            "Firefox (przeglądarka)"
        );
        assert_eq!(entry.localized_name(Some("de_DE")), "Firefox");
        // The action group's Exec is not the entry's.
        assert_eq!(entry.exec.as_deref(), Some("firefox %u"));
        assert!(DesktopEntry::parse("Name=x").is_err());
    }

    #[test]
    fn exec_lines_expand_field_codes_without_a_shell() {
        let entry = DesktopEntry::parse(FIREFOX).unwrap();
        assert_eq!(entry.command(&[], "/a.desktop").unwrap(), ["firefox"]);
        assert_eq!(
            entry
                .command(&targets(&["https://example.com/?a=1;b"]), "")
                .unwrap(),
            ["firefox", "https://example.com/?a=1;b"]
        );
        let mut entry = entry;
        entry.exec = Some(r#"/opt/app/bin "my app" --name=%c %i %F %k %% "%f" $HOME `id`"#.into());
        assert_eq!(
            entry
                .command(&targets(&["/a b", "/c"]), "/x.desktop")
                .unwrap(),
            [
                "/opt/app/bin",
                "my app",
                "--name=Firefox",
                "--icon",
                "firefox",
                "/a b",
                "/c",
                "/x.desktop",
                "%",
                // A quoted field code is not expanded.
                "%f",
                // Nothing is interpreted.
                "$HOME",
                "`id`",
            ]
        );
    }

    #[test]
    fn quoting_follows_the_specification() {
        let words = split_exec(r#""C:\\x" "a \"b\" \$c \`d\`" e\f"#).unwrap();
        let plain: Vec<&str> = words.iter().map(|(w, _)| w.as_str()).collect();
        assert_eq!(plain, [r"C:\x", r#"a "b" $c `d`"#, r"e\f"]);
        for bad in [r#""open"#, r#"a"b""#, r#""a"b"#] {
            assert!(split_exec(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn string_escapes_are_resolved() {
        let entry =
            DesktopEntry::parse("[Desktop Entry]\nType=Application\nName=A\\sB\nExec=a\\\\b\n")
                .unwrap();
        assert_eq!(entry.name, "A B");
        assert_eq!(entry.exec.as_deref(), Some(r"a\b"));
        assert!(DesktopEntry::parse(&"#".repeat(MAX_FILE_LEN + 1)).is_err());
    }

    #[test]
    fn entries_without_a_program_are_not_applications() {
        let link =
            DesktopEntry::parse("[Desktop Entry]\nType=Link\nName=Docs\nURL=https://x\n").unwrap();
        assert!(!link.is_application());
        let empty =
            DesktopEntry::parse("[Desktop Entry]\nType=Application\nName=E\nExec=%u\n").unwrap();
        assert!(empty.command(&[], "").is_err());
    }
}
