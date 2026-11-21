//! **Experimental.** The privileged-kinds gate of the first-party publisher
//! signature and the **reviewed privileged policy** that an opened gate
//! admits (`docs/marketplace/publisher-signature.md`, "Privileged plugins").
//!
//! Five of DeckPilot's own plugins use plugin API 1.9's privileged kinds
//! (`tools`, which is process execution, `resources`, `projectContext`,
//! `hostViews`, `openUrl.fromParams`, `resource` parameters,
//! `node.changesContext`, host variable sources, host views, `statusItems`,
//! `dashboardCards`): [`PRIVILEGED_PLUGIN_IDS`]. A signed release artifact
//! of one of them is refused unless **all** of this holds:
//!
//! 1. the compiled constant [`PRIVILEGED_GATE_OPEN`] is `true` (a reviewed
//!    code change; there is no flag, setting, key or environment variable);
//! 2. the compiled policy ([`POLICY_JSON`], `publisher/privileged-policy.json`)
//!    is valid and records the independent security review with outcome
//!    `approved` ([`Review`]);
//! 3. the policy lists the plugin's id with exactly the claimed version;
//! 4. the manifest's `permissions` are exactly the reviewed JSON, and its
//!    privileged usages ([`Manifest::privileged_usages`]) are exactly the
//!    reviewed constructs, each with the reviewed JSON value (a new
//!    construct, a broader permission, an extra executable, another plugin
//!    or another version is refused: [`check`]);
//! 5. at least [`MIN_THRESHOLD`] (and the policy's `minThreshold`) distinct
//!    pinned publisher keys signed it (`publisher::verify_release`).
//!
//! Process execution is limited here independently of the record: only
//! `deckpilot.git` may run `git` and only `deckpilot.docker` may run
//! `docker` ([`PROCESS_ALLOW_LIST`]); a policy that says otherwise does not
//! parse.
//!
//! Admission is **authenticity, not authorization**: an admitted release is
//! read with the bundled rules (`verify_bundled_files`) so that an
//! application build can stage it as a bundled plugin, and it reaches users
//! only that way, pinned in the agent. Nothing here grants a permission;
//! the store's consent rules apply as for every bundled plugin, and a
//! registry copy of a privileged plugin stays refused by the store
//! (`check_privileged_source`, X12).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use deckpilot_plugin_manifest::{Manifest, PluginId};
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::publisher::PublisherError;

/// **Open**, by the reviewed change knittingcode/deckpilot#388 (owner
/// decision 2026-11-21 after the final independent review,
/// `docs/marketplace/reviews/2026-11-21-privileged-gate-review.md`). It
/// admits exactly the reviewed privileged policy, and only while
/// [`POLICY_JSON`] records that review as approved and complete
/// ([`Review::is_approved`]): otherwise [`PrivilegedGate::compiled`] stays
/// closed. Nothing at run time or build time changes either.
pub const PRIVILEGED_GATE_OPEN: bool = true;

/// The reviewed privileged policy, compiled in.
pub const POLICY_JSON: &str = include_str!("../publisher/privileged-policy.json");

/// The only `privilegedPolicyVersion`.
pub const POLICY_VERSION: u32 = 1;

/// Largest policy document.
pub const MAX_POLICY_BYTES: usize = 256 * 1024;

/// Distinct publisher keys a privileged release always needs, whatever
/// the pinned threshold or the policy say.
pub const MIN_THRESHOLD: u64 = 2;

/// The plugins a policy may name: DeckPilot's own plugins that use the
/// privileged kinds. Any other id makes the policy invalid.
pub const PRIVILEGED_PLUGIN_IDS: [&str; 5] = [
    "deckpilot.docker",
    "deckpilot.git",
    "deckpilot.projects",
    "deckpilot.workflows",
    "deckpilot.workspace",
];

/// Process execution (`tools`), ever: only these plugins, each only its one
/// executable.
pub const PROCESS_ALLOW_LIST: [(&str, &str); 2] =
    [("deckpilot.docker", "docker"), ("deckpilot.git", "git")];

/// The gate a check runs with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivilegedGate<'a> {
    /// Every privileged usage is refused ([`PublisherError::PrivilegedGateClosed`]).
    Closed,
    /// Exactly what the policy lists is admitted. [`ReviewedGate`] cannot
    /// be built outside this module: an open gate comes only from
    /// [`PrivilegedGate::compiled`] or, in tests,
    /// [`PrivilegedGate::open_for_tests`] (feature `test-privileged-gate`,
    /// a dev-dependency feature only).
    Open(ReviewedGate<'a>),
}

/// The policy an open gate admits. Its field is private, so no code outside
/// this module can make one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReviewedGate<'a> {
    policy: &'a PrivilegedPolicy,
}

impl<'a> ReviewedGate<'a> {
    /// The policy this open gate admits.
    pub fn policy(&self) -> &'a PrivilegedPolicy {
        self.policy
    }
}

#[cfg(any(test, feature = "test-privileged-gate"))]
impl<'a> PrivilegedGate<'a> {
    /// **Tests only.** An open gate for `policy`, whatever
    /// [`PRIVILEGED_GATE_OPEN`] says. Compiled only for this crate's unit
    /// tests and with the feature `test-privileged-gate`, which only
    /// `[dev-dependencies]` enable (this crate's own, the agent's): never in
    /// a release build. `tests/privileged.rs`
    /// (`only_tests_construct_an_open_gate`) refuses any production use.
    pub fn open_for_tests(policy: &'a PrivilegedPolicy) -> Self {
        Self::Open(ReviewedGate { policy })
    }
}

impl PrivilegedGate<'static> {
    /// This build's gate: [`PrivilegedGate::Closed`] unless
    /// [`PRIVILEGED_GATE_OPEN`] and the compiled policy is valid and
    /// records an approved review. Fail closed: an invalid policy or a
    /// pending review keeps it closed.
    pub fn compiled() -> Self {
        if !PRIVILEGED_GATE_OPEN {
            return Self::Closed;
        }
        match compiled_policy() {
            Ok(policy) if policy.is_approved() => Self::Open(ReviewedGate { policy }),
            _ => Self::Closed,
        }
    }
}

/// The compiled policy, parsed once.
pub fn compiled_policy() -> Result<&'static PrivilegedPolicy, String> {
    static POLICY: OnceLock<Result<PrivilegedPolicy, String>> = OnceLock::new();
    POLICY
        .get_or_init(|| PrivilegedPolicy::from_json(POLICY_JSON.as_bytes()))
        .as_ref()
        .map_err(Clone::clone)
}

/// `publisher/privileged-policy.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PrivilegedPolicy {
    #[serde(rename = "$comment", default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    pub privileged_policy_version: u32,
    /// The independent security review; `None` until it is recorded.
    pub review: Option<Review>,
    /// Distinct publisher keys a privileged release needs; at least
    /// [`MIN_THRESHOLD`].
    pub min_threshold: u64,
    pub plugins: Vec<ReviewedPlugin>,
}

/// The review's sign-off.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Review {
    /// Who reviewed (not the author).
    pub reviewer: String,
    /// `YYYY-MM-DD`.
    pub date: String,
    pub outcome: ReviewOutcome,
    /// Where the review is recorded (the sign-off row and its report).
    pub record: String,
}

impl Review {
    /// An approved sign-off, all of it: the outcome `approved`, a reviewer
    /// named (not empty, not a `pending…` placeholder), a `YYYY-MM-DD`
    /// date and a record. Anything less opens nothing, whatever the
    /// outcome says.
    pub fn is_approved(&self) -> bool {
        let reviewer = self.reviewer.trim();
        self.outcome == ReviewOutcome::Approved
            && !reviewer.is_empty()
            && !reviewer.to_ascii_lowercase().starts_with("pending")
            && is_date(self.date.trim())
            && !self.record.trim().is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReviewOutcome {
    /// Requested, not done: the gate stays closed.
    Pending,
    Approved,
}

/// One reviewed plugin version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ReviewedPlugin {
    pub id: PluginId,
    /// The only version admitted.
    pub version: Version,
    /// The manifest's `permissions`, exactly (JSON equality, in order).
    pub permissions: Vec<Value>,
    /// Every privileged usage, exactly.
    pub constructs: Vec<Construct>,
}

/// One privileged usage, as [`Manifest::privileged_usages`] names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Construct {
    pub path: String,
    pub what: String,
    /// The manifest's JSON at `path`; absent for a `permissions[…]` usage
    /// (the permission itself is in [`ReviewedPlugin::permissions`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
}

impl PrivilegedPolicy {
    /// Parses and validates a policy document.
    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_POLICY_BYTES {
            return Err(format!(
                "the privileged policy is {} bytes; at most {MAX_POLICY_BYTES}",
                bytes.len()
            ));
        }
        let policy: Self = serde_json::from_slice(bytes)
            .map_err(|e| format!("the privileged policy does not parse: {e}"))?;
        policy.validate()?;
        Ok(policy)
    }

    /// Whether the review is recorded and approved.
    pub fn is_approved(&self) -> bool {
        self.review.as_ref().is_some_and(Review::is_approved)
    }

    /// The document's rules: version 1, `minThreshold` at least
    /// [`MIN_THRESHOLD`], a well-formed review if any, only
    /// [`PRIVILEGED_PLUGIN_IDS`], each once, each with at least one
    /// construct, distinct construct paths, a `value` exactly for the
    /// constructs outside `permissions`, every `permissions[n]` construct
    /// naming a listed permission, and process execution only as
    /// [`PROCESS_ALLOW_LIST`] allows.
    pub fn validate(&self) -> Result<(), String> {
        if self.privileged_policy_version != POLICY_VERSION {
            return Err(format!(
                "privilegedPolicyVersion {} (only {POLICY_VERSION})",
                self.privileged_policy_version
            ));
        }
        if self.min_threshold < MIN_THRESHOLD {
            return Err(format!(
                "minThreshold {}: a privileged release needs at least {MIN_THRESHOLD} distinct keys",
                self.min_threshold
            ));
        }
        if let Some(review) = &self.review {
            if review.reviewer.trim().is_empty() || review.record.trim().is_empty() {
                return Err("review: reviewer and record must be given".into());
            }
            if !is_date(&review.date) {
                return Err(format!("review: date `{}` is not YYYY-MM-DD", review.date));
            }
        }
        let mut ids = BTreeSet::new();
        for plugin in &self.plugins {
            let id = plugin.id.as_str();
            if !PRIVILEGED_PLUGIN_IDS.contains(&id) {
                return Err(format!(
                    "{id} is not one of DeckPilot's privileged plugins ({})",
                    PRIVILEGED_PLUGIN_IDS.join(", ")
                ));
            }
            if !ids.insert(id) {
                return Err(format!("{id} is listed twice"));
            }
            if plugin.constructs.is_empty() {
                return Err(format!("{id}: no privileged construct is listed"));
            }
            let mut paths = BTreeSet::new();
            for construct in &plugin.constructs {
                if construct.what.trim().is_empty() {
                    return Err(format!("{id}: {}: `what` is empty", construct.path));
                }
                if !paths.insert(construct.path.as_str()) {
                    return Err(format!("{id}: {} is listed twice", construct.path));
                }
                match permission_index(&construct.path) {
                    Some(index) => {
                        if construct.value.is_some() {
                            return Err(format!(
                                "{id}: {}: a permission's construct has no `value` (the \
                                 permission is in `permissions`)",
                                construct.path
                            ));
                        }
                        if index >= plugin.permissions.len() {
                            return Err(format!(
                                "{id}: {} names no listed permission",
                                construct.path
                            ));
                        }
                    }
                    None if construct.value.is_none() => {
                        return Err(format!("{id}: {}: `value` is missing", construct.path));
                    }
                    None => {}
                }
            }
            check_process_allow_list(id, &plugin.permissions)?;
        }
        Ok(())
    }

    /// The reviewed entry of `id`, if any (whatever its version).
    pub fn plugin(&self, id: &PluginId) -> Option<&ReviewedPlugin> {
        self.plugins.iter().find(|p| &p.id == id)
    }

    /// Distinct keys a privileged release needs under this policy and
    /// `pinned_threshold`.
    pub fn needed_signers(&self, pinned_threshold: u64) -> u64 {
        MIN_THRESHOLD.max(self.min_threshold).max(pinned_threshold)
    }
}

/// `n` of a construct path `permissions[n]` or `permissions[n].<field>`.
fn permission_index(path: &str) -> Option<usize> {
    let rest = path.strip_prefix("permissions[")?;
    let (n, tail) = rest.split_once(']')?;
    if !(tail.is_empty() || tail.starts_with('.')) {
        return None;
    }
    n.parse().ok()
}

fn is_date(text: &str) -> bool {
    let b = text.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

/// `tools` only for the allow-listed plugin, with exactly its executable.
fn check_process_allow_list(id: &str, permissions: &[Value]) -> Result<(), String> {
    for permission in permissions {
        if permission.get("kind").and_then(Value::as_str) != Some("tools") {
            continue;
        }
        let Some((_, allowed)) = PROCESS_ALLOW_LIST.iter().find(|(owner, _)| *owner == id) else {
            return Err(format!(
                "{id} may not run processes: `tools` is only for {}",
                allow_list_text()
            ));
        };
        let executables: Vec<&str> = permission
            .get("tools")
            .and_then(Value::as_array)
            .map(|tools| {
                tools
                    .iter()
                    .map(|t| t.get("executable").and_then(Value::as_str).unwrap_or(""))
                    .collect()
            })
            .unwrap_or_default();
        if executables != [*allowed] {
            return Err(format!(
                "{id} may run only `{allowed}`, the policy lists {executables:?}"
            ));
        }
    }
    Ok(())
}

fn allow_list_text() -> String {
    PROCESS_ALLOW_LIST
        .iter()
        .map(|(id, exe)| format!("{id} ({exe})"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// What [`check`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Privileged<'a> {
    /// The manifest uses no privileged kind: the ordinary first-party rules
    /// apply.
    None,
    /// Every privileged usage is exactly the reviewed one.
    Reviewed {
        policy: &'a PrivilegedPolicy,
        plugin: &'a ReviewedPlugin,
    },
}

impl Privileged<'_> {
    /// One line for logs and `release verify`: `none`, or the review.
    pub fn describe(&self) -> String {
        match self {
            Self::None => "none".into(),
            Self::Reviewed { policy, plugin } => {
                let review = policy.review.as_ref();
                format!(
                    "reviewed: {} {} ({} construct(s)), review by {} on {} ({})",
                    plugin.id,
                    plugin.version,
                    plugin.constructs.len(),
                    review.map_or("?", |r| r.reviewer.as_str()),
                    review.map_or("?", |r| r.date.as_str()),
                    review.map_or("?", |r| r.record.as_str()),
                )
            }
        }
    }
}

/// The gate for the manifest `manifest_bytes` of a release claimed as `id`
/// `version`. A manifest that does not parse is refused
/// ([`PublisherError::Package`]). No privileged usage:
/// [`Privileged::None`]. Closed: [`PublisherError::PrivilegedGateClosed`]
/// with every usage. Open: [`Privileged::Reviewed`] only when the policy is
/// valid, its review approved, and the manifest is exactly the reviewed
/// one for its id and version (see the module documentation); otherwise
/// [`PublisherError::PrivilegedNotReviewed`]. The signature threshold is
/// the caller's check ([`PrivilegedPolicy::needed_signers`]).
pub fn check<'a>(
    gate: PrivilegedGate<'a>,
    manifest_bytes: &[u8],
    id: &PluginId,
    version: &Version,
) -> Result<Privileged<'a>, PublisherError> {
    let manifest = Manifest::from_slice(manifest_bytes)
        .map_err(|e| PublisherError::Package(format!("manifest.json: {e}")))?;
    let usages = manifest.privileged_usages();
    if usages.is_empty() {
        return Ok(Privileged::None);
    }
    let PrivilegedGate::Open(reviewed) = gate else {
        return Err(PublisherError::PrivilegedGateClosed {
            usages: usages.into_iter().map(|(path, _)| path).collect(),
        });
    };
    let policy = reviewed.policy();
    let refuse = |reason: String| PublisherError::PrivilegedNotReviewed {
        id: id.to_string(),
        version: version.to_string(),
        reason,
    };
    policy.validate().map_err(refuse)?;
    if !policy.is_approved() {
        return Err(refuse(
            "the policy records no approved security review".into(),
        ));
    }
    if &manifest.id != id || &manifest.version != version {
        return Err(refuse(format!(
            "the manifest is {}@{}, not the claimed release",
            manifest.id, manifest.version
        )));
    }
    let plugin = policy
        .plugin(id)
        .ok_or_else(|| refuse("the review does not cover this plugin".into()))?;
    if &plugin.version != version {
        return Err(refuse(format!(
            "the review covers version {} only",
            plugin.version
        )));
    }
    let raw: Value = serde_json::from_slice(manifest_bytes)
        .map_err(|e| PublisherError::Package(format!("manifest.json: {e}")))?;
    let declared = raw
        .get("permissions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if declared != plugin.permissions {
        return Err(refuse(permissions_difference(
            &plugin.permissions,
            &declared,
        )));
    }
    let reviewed: BTreeMap<&str, &Construct> = plugin
        .constructs
        .iter()
        .map(|c| (c.path.as_str(), c))
        .collect();
    let mut unreviewed = Vec::new();
    let mut seen = BTreeSet::new();
    for (path, what) in &usages {
        seen.insert(path.as_str());
        let Some(construct) = reviewed.get(path.as_str()) else {
            unreviewed.push(format!("{path} ({what}) is not reviewed"));
            continue;
        };
        if &construct.what != what {
            unreviewed.push(format!(
                "{path} is {what}, the review has {}",
                construct.what
            ));
            continue;
        }
        if construct.value.is_some() && construct.value.as_ref() != value_at(&raw, path) {
            unreviewed.push(format!("{path} ({what}) differs from the reviewed value"));
        }
    }
    for path in reviewed.keys() {
        if !seen.contains(path) {
            unreviewed.push(format!("{path} is reviewed but not declared"));
        }
    }
    if !unreviewed.is_empty() {
        return Err(refuse(unreviewed.join("; ")));
    }
    Ok(Privileged::Reviewed { policy, plugin })
}

fn permissions_difference(reviewed: &[Value], declared: &[Value]) -> String {
    let kind = |v: &Value| {
        v.get("kind")
            .and_then(Value::as_str)
            .unwrap_or("?")
            .to_owned()
    };
    if reviewed.len() != declared.len() {
        return format!(
            "{} permission(s) declared ({}), {} reviewed ({})",
            declared.len(),
            declared.iter().map(kind).collect::<Vec<_>>().join(", "),
            reviewed.len(),
            reviewed.iter().map(kind).collect::<Vec<_>>().join(", "),
        );
    }
    let differing: Vec<String> = reviewed
        .iter()
        .zip(declared)
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(n, (_, b))| format!("permissions[{n}] ({})", kind(b)))
        .collect();
    format!(
        "{} differ(s) from the reviewed permissions",
        differing.join(", ")
    )
}

/// The JSON at a usage path (`a.b[0].c`), if present.
fn value_at<'v>(doc: &'v Value, path: &str) -> Option<&'v Value> {
    let mut current = doc;
    for segment in path.split('.') {
        let (name, mut indexes) = match segment.find('[') {
            Some(at) => (&segment[..at], &segment[at..]),
            None => (segment, ""),
        };
        current = current.get(name)?;
        while let Some(rest) = indexes.strip_prefix('[') {
            let (n, tail) = rest.split_once(']')?;
            current = current.get(n.parse::<usize>().ok()?)?;
            indexes = tail;
        }
        if !indexes.is_empty() {
            return None;
        }
    }
    Some(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The five privileged manifests of this repository, at the reviewed
    /// version: the source the policy was reviewed against.
    fn source(name: &str) -> &'static str {
        match name {
            "docker" => include_str!("../../../plugins/first-party/docker/manifest.json"),
            "git" => include_str!("../../../plugins/first-party/git/manifest.json"),
            "projects" => include_str!("../../../plugins/first-party/projects/manifest.json"),
            "workflows" => include_str!("../../../plugins/first-party/workflows/manifest.json"),
            "workspace" => include_str!("../../../plugins/first-party/workspace/manifest.json"),
            _ => unreachable!(),
        }
    }

    fn at_version(name: &str, version: &str) -> Vec<u8> {
        let mut manifest: Value = serde_json::from_str(source(name)).unwrap();
        manifest["version"] = version.into();
        serde_json::to_vec_pretty(&manifest).unwrap()
    }

    fn approved() -> PrivilegedPolicy {
        let mut policy = PrivilegedPolicy::from_json(POLICY_JSON.as_bytes()).unwrap();
        policy.review = Some(Review {
            reviewer: "test reviewer".into(),
            date: "2026-11-21".into(),
            outcome: ReviewOutcome::Approved,
            record: "test".into(),
        });
        policy
    }

    /// An approval opens only when the whole sign-off is there: a named
    /// reviewer (not a `pending` placeholder), a date and a record.
    #[test]
    fn an_approval_needs_a_named_reviewer_a_date_and_a_record() {
        let review = |reviewer: &str, date: &str, record: &str| Review {
            reviewer: reviewer.into(),
            date: date.into(),
            outcome: ReviewOutcome::Approved,
            record: record.into(),
        };
        assert!(review("A. Reviewer", "2026-11-22", "report").is_approved());
        for refused in [
            review("", "2026-11-22", "report"),
            review("   ", "2026-11-22", "report"),
            review("pending: the independent reviewer", "2026-11-22", "report"),
            review("Pending", "2026-11-22", "report"),
            review("A. Reviewer", "", "report"),
            review("A. Reviewer", "22.11.2026", "report"),
            review("A. Reviewer", "2026-11-22", ""),
            review("A. Reviewer", "2026-11-22", "  "),
            Review {
                outcome: ReviewOutcome::Pending,
                ..review("A. Reviewer", "2026-11-22", "report")
            },
        ] {
            assert!(!refused.is_approved(), "{refused:?}");
            let mut policy = compiled_policy().unwrap().clone();
            policy.review = Some(refused);
            assert!(!policy.is_approved());
        }
        // A missing date or record does not even parse.
        for review in [
            r#"{"reviewer": "A. Reviewer", "outcome": "approved", "record": "x"}"#,
            r#"{"reviewer": "A. Reviewer", "date": "2026-11-22", "outcome": "approved"}"#,
        ] {
            let mut doc: Value = serde_json::from_str(POLICY_JSON).unwrap();
            doc["review"] = serde_json::from_str(review).unwrap();
            assert!(PrivilegedPolicy::from_json(&serde_json::to_vec(&doc).unwrap()).is_err());
        }
    }

    /// The record's `review` is the last row of the Sign-off table in
    /// docs/marketplace/publisher-review-request.md (reviewer, date,
    /// outcome), and no review means an empty row: the two are changed
    /// together, by the reviewer.
    #[test]
    fn the_review_is_the_last_sign_off_row() {
        let doc = include_str!("../../../docs/marketplace/publisher-review-request.md");
        let table = doc
            .split("## Sign-off")
            .nth(1)
            .expect("the review request has a Sign-off section");
        let row = table
            .lines()
            .take_while(|l| l.trim().is_empty() || l.trim_start().starts_with('|'))
            .filter(|l| l.trim_start().starts_with('|'))
            .last()
            .expect("the Sign-off table has a row");
        let cells: Vec<&str> = row
            .trim()
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect();
        assert!(cells.len() >= 3, "{row}");
        let policy = compiled_policy().unwrap();
        match &policy.review {
            None => assert_eq!(&cells[..3], ["", "", ""], "{row}"),
            Some(review) => {
                let outcome = match review.outcome {
                    ReviewOutcome::Pending => "pending",
                    ReviewOutcome::Approved => "approved",
                };
                assert_eq!(
                    &cells[..3],
                    [review.reviewer.as_str(), review.date.as_str(), outcome],
                    "{row}"
                );
            }
        }
    }

    /// The opening change: the constant is open, the record names the
    /// review, and the compiled gate opens exactly when that review is
    /// approved (closed while it is pending).
    #[test]
    fn the_gate_opens_only_with_the_approved_review() {
        const { assert!(PRIVILEGED_GATE_OPEN) };
        let policy = compiled_policy().unwrap();
        let review = policy
            .review
            .as_ref()
            .expect("the opening change records the review");
        assert!(
            review.record.contains("publisher-review-request.md"),
            "{review:?}"
        );
        assert_eq!(policy.min_threshold, MIN_THRESHOLD);
        match review.outcome {
            ReviewOutcome::Approved => {
                assert_eq!(
                    PrivilegedGate::compiled(),
                    PrivilegedGate::open_for_tests(policy)
                );
            }
            ReviewOutcome::Pending => {
                assert!(!policy.is_approved());
                assert_eq!(PrivilegedGate::compiled(), PrivilegedGate::Closed);
            }
        }
    }

    #[test]
    fn the_policy_names_the_five_privileged_plugins_at_their_reviewed_version() {
        let policy = compiled_policy().unwrap();
        let ids: Vec<&str> = policy.plugins.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, PRIVILEGED_PLUGIN_IDS);
        for plugin in &policy.plugins {
            assert_eq!(plugin.version, Version::new(0, 1, 1), "{}", plugin.id);
        }
    }

    /// The policy is exactly what this repository's sources of the five
    /// plugins declare (at the reviewed version): every permission and
    /// every privileged construct. A change to a plugin's privileges fails
    /// here until the policy, and so its review, is updated.
    #[test]
    fn the_policy_is_exactly_what_the_sources_declare() {
        let policy = approved();
        for plugin in &policy.plugins {
            let name = plugin.id.as_str().strip_prefix("deckpilot.").unwrap();
            let manifest = at_version(name, &plugin.version.to_string());
            let found = check(
                PrivilegedGate::open_for_tests(&policy),
                &manifest,
                &plugin.id,
                &plugin.version,
            )
            .unwrap_or_else(|e| panic!("{}: {e}", plugin.id));
            assert!(matches!(found, Privileged::Reviewed { .. }));
        }
    }

    #[test]
    fn the_process_allow_list_is_git_and_docker_only() {
        let policy = compiled_policy().unwrap();
        for plugin in &policy.plugins {
            let tools: Vec<&Value> = plugin
                .permissions
                .iter()
                .filter(|p| p["kind"] == "tools")
                .collect();
            match plugin.id.as_str() {
                "deckpilot.git" => assert_eq!(tools[0]["tools"][0]["executable"], "git"),
                "deckpilot.docker" => assert_eq!(tools[0]["tools"][0]["executable"], "docker"),
                _ => assert!(tools.is_empty(), "{}", plugin.id),
            }
        }
        // A policy that lets another plugin, or another executable, run
        // processes does not parse.
        let mut doc: Value = serde_json::from_str(POLICY_JSON).unwrap();
        let workflows = doc["plugins"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|p| p["id"] == "deckpilot.workflows")
            .unwrap();
        workflows["permissions"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"kind": "tools", "tools": [{"executable": "git"}]}));
        let error = PrivilegedPolicy::from_json(&serde_json::to_vec(&doc).unwrap()).unwrap_err();
        assert!(error.contains("may not run processes"), "{error}");
        let mut doc: Value = serde_json::from_str(POLICY_JSON).unwrap();
        assert_eq!(doc["plugins"][1]["id"], "deckpilot.git");
        doc["plugins"][1]["permissions"][0]["tools"][0]["executable"] = "docker".into();
        let error = PrivilegedPolicy::from_json(&serde_json::to_vec(&doc).unwrap()).unwrap_err();
        assert!(error.contains("may run only `git`"), "{error}");
        doc["plugins"][1]["permissions"][0]["tools"][0]["executable"] = "git".into();
        doc["plugins"][1]["permissions"][0]["tools"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"executable": "git"}));
        assert!(PrivilegedPolicy::from_json(&serde_json::to_vec(&doc).unwrap()).is_err());
    }

    #[test]
    fn policy_documents_are_checked() {
        let edit = |f: &dyn Fn(&mut Value)| {
            let mut doc: Value = serde_json::from_str(POLICY_JSON).unwrap();
            f(&mut doc);
            PrivilegedPolicy::from_json(&serde_json::to_vec(&doc).unwrap())
        };
        assert!(edit(&|_| {}).is_ok());
        for (what, change) in [
            (
                "version 2",
                &(|d: &mut Value| {
                    d["privilegedPolicyVersion"] = 2.into();
                }) as &dyn Fn(&mut Value),
            ),
            ("threshold 1", &|d: &mut Value| {
                d["minThreshold"] = 1.into();
            }),
            ("unknown field", &|d: &mut Value| {
                d["open"] = true.into();
            }),
            ("another plugin", &|d: &mut Value| {
                d["plugins"][0]["id"] = "deckpilot.github".into();
            }),
            ("a third-party plugin", &|d: &mut Value| {
                d["plugins"][0]["id"] = "acme.git".into();
            }),
            ("listed twice", &|d: &mut Value| {
                let first = d["plugins"][0].clone();
                d["plugins"].as_array_mut().unwrap().push(first);
            }),
            ("no construct", &|d: &mut Value| {
                d["plugins"][0]["constructs"] = serde_json::json!([]);
            }),
            ("a construct without its value", &|d: &mut Value| {
                let constructs = d["plugins"][0]["constructs"].as_array_mut().unwrap();
                let c = constructs
                    .iter_mut()
                    .find(|c| c.get("value").is_some())
                    .unwrap();
                c.as_object_mut().unwrap().remove("value");
            }),
            ("a permission construct with a value", &|d: &mut Value| {
                d["plugins"][0]["constructs"][0]["value"] = 1.into();
            }),
            ("a construct naming no permission", &|d: &mut Value| {
                d["plugins"][0]["constructs"][0]["path"] = "permissions[9]".into();
            }),
            ("a review without reviewer", &|d: &mut Value| {
                d["review"] = serde_json::json!({"reviewer": " ", "date": "2026-11-21", "outcome": "approved", "record": "x"});
            }),
            ("a review with a bad date", &|d: &mut Value| {
                d["review"] = serde_json::json!({"reviewer": "r", "date": "21.11.2026", "outcome": "approved", "record": "x"});
            }),
            ("an unknown outcome", &|d: &mut Value| {
                d["review"] = serde_json::json!({"reviewer": "r", "date": "2026-11-21", "outcome": "yes", "record": "x"});
            }),
        ] {
            assert!(edit(change).is_err(), "{what}");
        }
    }

    #[test]
    fn closed_refuses_every_privileged_manifest_and_passes_the_others() {
        let policy = compiled_policy().unwrap();
        for plugin in &policy.plugins {
            let name = plugin.id.as_str().strip_prefix("deckpilot.").unwrap();
            let manifest = at_version(name, "0.1.1");
            let error = check(
                PrivilegedGate::Closed,
                &manifest,
                &plugin.id,
                &plugin.version,
            )
            .unwrap_err();
            assert!(
                matches!(&error, PublisherError::PrivilegedGateClosed { usages } if usages.len() == plugin.constructs.len()),
                "{error}"
            );
        }
        let github = include_str!("../../../plugins/first-party/github/manifest.json");
        let manifest = Manifest::from_slice(github.as_bytes()).unwrap();
        assert_eq!(
            check(
                PrivilegedGate::Closed,
                github.as_bytes(),
                &manifest.id,
                &manifest.version
            )
            .unwrap(),
            Privileged::None
        );
    }

    /// Everything that is not exactly the reviewed manifest is refused with
    /// the gate open.
    #[test]
    #[allow(clippy::too_many_lines)] // one case after the other
    fn open_admits_exactly_the_record_and_nothing_more() {
        let policy = approved();
        let open = PrivilegedGate::open_for_tests(&policy);
        let git = PluginId::parse("deckpilot.git").unwrap();
        let v011 = Version::new(0, 1, 1);
        let edit = |f: &dyn Fn(&mut Value)| {
            let mut m: Value = serde_json::from_slice(&at_version("git", "0.1.1")).unwrap();
            f(&mut m);
            serde_json::to_vec(&m).unwrap()
        };
        let not_reviewed = |bytes: &[u8], id: &PluginId, version: &Version, text: &str| match check(
            open, bytes, id, version,
        ) {
            Err(PublisherError::PrivilegedNotReviewed { reason, .. }) => {
                assert!(reason.contains(text), "{reason} (expected {text})");
            }
            other => panic!("{other:?} (expected {text})"),
        };
        assert!(matches!(
            check(open, &edit(&|_| {}), &git, &v011).unwrap(),
            Privileged::Reviewed { .. }
        ));
        // Another version, in the claim and the manifest.
        let v012 = Version::new(0, 1, 2);
        not_reviewed(
            &edit(&|m| m["version"] = "0.1.2".into()),
            &git,
            &v012,
            "covers version 0.1.1 only",
        );
        // A manifest that is not the claimed release.
        not_reviewed(&edit(&|_| {}), &git, &v012, "not the claimed release");
        // Another plugin with the same privileges.
        let mut other: Value = serde_json::from_slice(&edit(&|_| {})).unwrap();
        other["id"] = "deckpilot.github".into();
        let github = PluginId::parse("deckpilot.github").unwrap();
        not_reviewed(
            &serde_json::to_vec(&other).unwrap(),
            &github,
            &v011,
            "does not cover this plugin",
        );
        // An extra git subcommand, another executable, a broader option.
        not_reviewed(
            &edit(&|m| {
                m["permissions"][0]["tools"][0]["subcommands"]
                    .as_array_mut()
                    .unwrap()
                    .push(serde_json::json!({"name": "fetch"}));
            }),
            &git,
            &v011,
            "permissions[0] (tools) differ",
        );
        not_reviewed(
            &edit(&|m| {
                m["permissions"][0]["tools"].as_array_mut().unwrap().push(
                    serde_json::json!({"executable": "docker", "subcommands": [{"name": "ps"}]}),
                );
            }),
            &git,
            &v011,
            "differ",
        );
        // A broader non-privileged permission.
        not_reviewed(
            &edit(&|m| m["permissions"][2]["read"] = serde_json::json!(["project", "profile"])),
            &git,
            &v011,
            "permissions[2]",
        );
        // A new permission.
        not_reviewed(
            &edit(&|m| {
                m["permissions"].as_array_mut().unwrap().push(
                    serde_json::json!({"kind": "notifications", "justification": "t:permissions.profile"}),
                );
            }),
            &git,
            &v011,
            "5 permission(s) declared",
        );
        // A new construct: a host view.
        not_reviewed(
            &edit(&|m| m["contributes"]["statusItems"] = serde_json::json!([])),
            &git,
            &v011,
            "contributes.statusItems (`statusItems`) is not reviewed",
        );
        // A broader resource parameter.
        not_reviewed(
            &edit(&|m| {
                m["contributes"]["actions"][0]["parameters"][0]["kind"]["kinds"] =
                    serde_json::json!(["directory", "file"]);
            }),
            &git,
            &v011,
            "differs from the reviewed value",
        );
        // A construct dropped is not the reviewed manifest either.
        let policy_without = {
            let mut p = approved();
            let g = p.plugins.iter_mut().find(|p| p.id == git).unwrap();
            g.constructs.pop();
            p
        };
        assert!(matches!(
            check(
                PrivilegedGate::open_for_tests(&policy_without),
                &edit(&|_| {}),
                &git,
                &v011
            ),
            Err(PublisherError::PrivilegedNotReviewed { .. })
        ));
        // A pending review opens nothing.
        let mut pending = approved();
        pending.review.as_mut().unwrap().outcome = ReviewOutcome::Pending;
        assert!(matches!(
            check(PrivilegedGate::open_for_tests(&pending), &edit(&|_| {}), &git, &v011),
            Err(PublisherError::PrivilegedNotReviewed { reason, .. }) if reason.contains("no approved")
        ));
        let mut none = approved();
        none.review = None;
        assert!(matches!(
            check(
                PrivilegedGate::open_for_tests(&none),
                &edit(&|_| {}),
                &git,
                &v011
            ),
            Err(PublisherError::PrivilegedNotReviewed { .. })
        ));
        // An invalid policy opens nothing.
        let mut invalid = approved();
        invalid.min_threshold = 1;
        assert!(matches!(
            check(
                PrivilegedGate::open_for_tests(&invalid),
                &edit(&|_| {}),
                &git,
                &v011
            ),
            Err(PublisherError::PrivilegedNotReviewed { .. })
        ));
        // A manifest that does not parse never reaches the policy.
        assert!(matches!(
            check(open, b"{ not json", &git, &v011),
            Err(PublisherError::Package(_))
        ));
    }

    #[test]
    fn the_threshold_is_never_below_two() {
        let mut policy = approved();
        assert_eq!(policy.needed_signers(1), 2);
        assert_eq!(policy.needed_signers(3), 3);
        policy.min_threshold = 3;
        assert_eq!(policy.needed_signers(2), 3);
    }

    #[test]
    fn usage_paths_resolve() {
        let doc = serde_json::json!({"a": {"b": [{"c": 1}, {"c": [5, 6]}]}});
        assert_eq!(value_at(&doc, "a.b[0].c"), Some(&Value::from(1)));
        assert_eq!(value_at(&doc, "a.b[1].c[1]"), Some(&Value::from(6)));
        assert_eq!(value_at(&doc, "a.b[2].c"), None);
        assert_eq!(value_at(&doc, "a.x"), None);
        assert_eq!(permission_index("permissions[3]"), Some(3));
        assert_eq!(permission_index("permissions[1].fromParams"), Some(1));
        assert_eq!(permission_index("permissions[1]x"), None);
        assert_eq!(permission_index("contributes.views[0].kind"), None);
    }
}
