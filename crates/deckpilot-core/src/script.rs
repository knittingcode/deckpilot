//! Scripts of a profile (the Scripts plugin, `docs/scripts-bridge.md`).
//!
//! A [`Script`] is portable: its name, its source and its settings travel
//! with the profile. Whether it may run, and with which grants, is a
//! machine-local [`ScriptTrust`] that never leaves this computer (never
//! exported, never synced, never taken from an import):
//!
//! - **New in the editor:** trusted, bound to the hash of its source, with
//!   no grants. Saving never grants anything.
//! - **Saved in the editor while trusted:** bound to the new hash, its
//!   grants kept (the author is the user).
//! - **Any other change** (an import, an overwrite, a duplicate, a source
//!   that does not match the recorded hash): not trusted. Its grants are
//!   kept dormant and the script never runs until the user reviews the
//!   source and trusts it again.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::ScriptId;
use crate::script_grants::ScriptGrants;

/// Longest name, in characters.
pub const MAX_NAME: usize = 64;
/// Largest source, in bytes of UTF-8.
pub const MAX_SOURCE: usize = 256 * 1024;
/// Longest description, in characters.
pub const MAX_DESCRIPTION: usize = 500;
/// Timeout bounds and default, in milliseconds.
pub const MIN_TIMEOUT_MS: u32 = 1_000;
pub const MAX_TIMEOUT_MS: u32 = 600_000;
pub const DEFAULT_TIMEOUT_MS: u32 = 60_000;
/// Memory bounds and default, in MiB.
pub const MIN_MEMORY_MIB: u32 = 16;
pub const MAX_MEMORY_MIB: u32 = 128;
pub const DEFAULT_MEMORY_MIB: u32 = 64;
/// Most scripts in one profile.
pub const MAX_SCRIPTS: usize = 500;

/// One script of a profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Script {
    pub id: ScriptId,
    /// Unique in the profile, without case.
    pub name: String,
    pub source: String,
    #[serde(default)]
    pub settings: ScriptSettings,
    /// Incremented by every save (an editor that saves an older revision
    /// is refused).
    #[serde(default)]
    pub revision: u64,
}

/// A script's own settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScriptSettings {
    #[serde(default = "default_timeout")]
    pub timeout_ms: u32,
    #[serde(default)]
    pub stop_on_error: bool,
    #[serde(default = "default_memory", rename = "memoryMiB")]
    pub memory_mib: u32,
    #[serde(default)]
    pub description: String,
}

fn default_timeout() -> u32 {
    DEFAULT_TIMEOUT_MS
}

fn default_memory() -> u32 {
    DEFAULT_MEMORY_MIB
}

impl Default for ScriptSettings {
    fn default() -> Self {
        Self {
            timeout_ms: DEFAULT_TIMEOUT_MS,
            stop_on_error: false,
            memory_mib: DEFAULT_MEMORY_MIB,
            description: String::new(),
        }
    }
}

/// Why a script is invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ScriptError {
    #[error("the name is empty, too long or holds control characters")]
    Name,
    #[error("the source is larger than 256 KiB")]
    SourceTooLarge,
    #[error("the source holds a NUL character")]
    Source,
    #[error("the timeout is out of range")]
    Timeout,
    #[error("the memory limit is out of range")]
    Memory,
    #[error("the description is too long or holds control characters")]
    Description,
}

fn is_plain(text: &str, max: usize, newlines: bool) -> bool {
    text.chars().count() <= max
        && !text
            .chars()
            .any(|c| (c.is_control() && !(newlines && matches!(c, '\n' | '\t'))) || is_bidi(c))
}

/// Invisible and direction-changing characters (format characters, line
/// and paragraph separators, variation selectors, tags), which make a name
/// read other than it is or two names look the same.
fn is_bidi(c: char) -> bool {
    matches!(c,
        '\u{00AD}' | '\u{061C}' | '\u{180E}' | '\u{200B}'..='\u{200F}'
        | '\u{2028}'..='\u{202E}' | '\u{2060}'..='\u{206F}' | '\u{FE00}'..='\u{FE0F}'
        | '\u{FEFF}' | '\u{FFF9}'..='\u{FFFB}' | '\u{E0000}'..='\u{E007F}'
        | '\u{E0100}'..='\u{E01EF}')
}

impl Script {
    /// A new script, as the editor creates it.
    pub fn new(name: impl Into<String>, source: impl Into<String>) -> Self {
        Self {
            id: ScriptId::new(),
            name: name.into(),
            source: source.into(),
            settings: ScriptSettings::default(),
            revision: 1,
        }
    }

    pub fn validate(&self) -> Result<(), ScriptError> {
        let name = self.name.trim();
        if name.is_empty() || name != self.name || !is_plain(name, MAX_NAME, false) {
            return Err(ScriptError::Name);
        }
        if self.source.len() > MAX_SOURCE {
            return Err(ScriptError::SourceTooLarge);
        }
        if self.source.contains('\0') {
            return Err(ScriptError::Source);
        }
        let s = &self.settings;
        if !(MIN_TIMEOUT_MS..=MAX_TIMEOUT_MS).contains(&s.timeout_ms) {
            return Err(ScriptError::Timeout);
        }
        if !(MIN_MEMORY_MIB..=MAX_MEMORY_MIB).contains(&s.memory_mib) {
            return Err(ScriptError::Memory);
        }
        if !is_plain(&s.description, MAX_DESCRIPTION, true) {
            return Err(ScriptError::Description);
        }
        Ok(())
    }

    /// The hash its trust is bound to.
    pub fn source_sha256(&self) -> String {
        source_sha256(&self.source)
    }
}

/// The lowercase hex SHA-256 of `source`.
pub fn source_sha256(source: &str) -> String {
    use std::fmt::Write as _;
    let digest = Sha256::digest(source.as_bytes());
    let mut hex = String::with_capacity(64);
    for byte in digest {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// A script's machine-local trust record.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptTrust {
    /// The source hash the user trusted; `None` when the script is not
    /// trusted (never reviewed, or changed other than by the editor).
    pub source_sha256: Option<String>,
    /// When the user last trusted it (Unix milliseconds).
    pub reviewed_at: Option<i64>,
    /// The grants, dormant while the script is not trusted.
    pub grants: ScriptGrants,
}

/// Whether a script may run now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TrustState {
    /// Trusted, and its source is the one trusted.
    Trusted,
    /// Imported, duplicated, overwritten or changed outside the editor:
    /// it never runs until the user reviews and trusts it.
    NeedsReview,
}

impl ScriptTrust {
    /// The state of `script` under this record (`None`: no record).
    pub fn state(record: Option<&Self>, script: &Script) -> TrustState {
        match record.and_then(|r| r.source_sha256.as_deref()) {
            Some(hash) if hash == script.source_sha256() => TrustState::Trusted,
            _ => TrustState::NeedsReview,
        }
    }

    /// The grants a run of `script` gets: the record's, only while the
    /// script is trusted and the grants are valid; otherwise none.
    pub fn effective_grants(record: Option<&Self>, script: &Script) -> Option<ScriptGrants> {
        let record = record?;
        (Self::state(Some(record), script) == TrustState::Trusted
            && record.grants.validate().is_ok())
        .then(|| record.grants.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_script_is_checked() {
        let ok = Script::new("Deploy", "deckpilot.log('hi')");
        assert_eq!(ok.validate(), Ok(()));
        for name in [
            "",
            " x",
            "x ",
            "a\nb",
            "a\u{202E}b",
            "De\u{200B}ploy",
            "a\u{FEFF}",
            "a\u{00AD}b",
            "a\u{2028}b",
            "a\u{E0041}",
            &"x".repeat(65),
        ] {
            let s = Script {
                name: name.into(),
                ..ok.clone()
            };
            assert_eq!(s.validate(), Err(ScriptError::Name), "{name:?}");
        }
        let big = Script {
            source: "x".repeat(MAX_SOURCE + 1),
            ..ok.clone()
        };
        assert_eq!(big.validate(), Err(ScriptError::SourceTooLarge));
        let mut s = ok.clone();
        s.settings.timeout_ms = 999;
        assert_eq!(s.validate(), Err(ScriptError::Timeout));
        let mut s = ok.clone();
        s.settings.memory_mib = 129;
        assert_eq!(s.validate(), Err(ScriptError::Memory));
        let mut s = ok;
        s.settings.description = "line one\nline two".into();
        assert_eq!(s.validate(), Ok(()));
    }

    #[test]
    fn a_script_runs_only_while_its_trusted_source_is_unchanged() {
        let mut script = Script::new("Deploy", "one()");
        assert_eq!(ScriptTrust::state(None, &script), TrustState::NeedsReview);
        assert_eq!(ScriptTrust::effective_grants(None, &script), None);
        let record = ScriptTrust {
            source_sha256: Some(script.source_sha256()),
            reviewed_at: Some(1),
            grants: ScriptGrants {
                projects_read: true,
                ..ScriptGrants::default()
            },
        };
        assert_eq!(
            ScriptTrust::state(Some(&record), &script),
            TrustState::Trusted
        );
        assert!(
            ScriptTrust::effective_grants(Some(&record), &script).is_some_and(|g| g.projects_read)
        );
        script.source = "two()".into();
        assert_eq!(
            ScriptTrust::state(Some(&record), &script),
            TrustState::NeedsReview
        );
        assert_eq!(ScriptTrust::effective_grants(Some(&record), &script), None);
        let dormant = ScriptTrust {
            source_sha256: None,
            ..record
        };
        assert_eq!(ScriptTrust::effective_grants(Some(&dormant), &script), None);
    }

    #[test]
    fn the_hash_is_sha256() {
        assert_eq!(
            source_sha256(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn the_record_is_strict_and_camel_case() {
        let s = Script::new("A", "x");
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["settings"]["memoryMiB"], 64);
        assert_eq!(v["settings"]["timeoutMs"], 60_000);
        let mut bad = v;
        bad["grants"] = serde_json::json!({});
        assert!(
            serde_json::from_value::<Script>(bad).is_err(),
            "no grants in a script"
        );
    }
}
