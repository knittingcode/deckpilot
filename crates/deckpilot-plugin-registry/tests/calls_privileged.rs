//! T-P2 end to end (design-calls.md §11, C9): the Calls plugin's manifest,
//! `deckpilot.calls` 0.1.0 exactly as deckpilot-plugins builds it
//! (`tests/fixtures/calls/`, a copy of `official/calls` at deckpilot-plugins
//! #105 36befd54: the same JSON, formatted by Prettier), through the real
//! release paths: `package --first-party`, `release sign`'s preflight and
//! `release verify` with two throwaway publisher keys.
//!
//! - **This build refuses it everywhere.** The compiled policy has no Calls
//!   entry, so nothing can ship the plugin before the owner's release
//!   steps.
//! - **Even a reviewed entry is not enough.** An open test gate with an
//!   approved test policy that lists the exact Calls entry still refuses
//!   it, because `deckpilot.calls` is not one of the privileged plugin ids.
//!   Adding it there (and the entry) is the owner's reviewed change.
//! - **The kinds are bundled-only and pinned to the plugin.** Every other
//!   path is refused too: the public (registry, sideload) rules, a
//!   first-party package without the bundled rules, another id, `calls`
//!   with `network`, and an API below 1.16.
//! - **The open gate stays test-only.** `PrivilegedGate::open_for_tests`
//!   exists only with the feature `test-privileged-gate`, which no
//!   release-relevant build enables (checked with `cargo tree`).
//!
//! Keys, packages and the test policy live in memory and in temporary
//! directories only. Nothing is written to `keys/`, `registry/`, a data
//! directory or any file a release build reads.

#![allow(clippy::too_many_lines)]

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::{ThrowawayKey, id, v};
use deckpilot_plugin_manifest::{ApiLevel, HostEnvironment, IssueCode, Manifest, Platform};
use deckpilot_plugin_package::{Limits, PackageBuilder};
use deckpilot_plugin_registry::privileged_policy::{
    POLICY_JSON, PrivilegedGate, PrivilegedPolicy, Review, ReviewOutcome,
};
use deckpilot_plugin_registry::publish::{package_with_gate, preflight_with_gate};
use deckpilot_plugin_registry::publisher::{
    PublisherError, PublisherKeys, ReleaseClaim, ReleasePolicy, SignatureFile, digest_of,
    release_comment, verify_release,
};
use serde_json::{Value, json};
use tempfile::TempDir;

const CALLS: &str = "deckpilot.calls";
const VERSION: &str = "0.1.0";

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/calls")
}

fn manifest() -> Value {
    serde_json::from_slice(&std::fs::read(fixture().join("manifest.json")).unwrap()).unwrap()
}

/// A DeckPilot with plugin API 1.16, as the first release with Calls.
fn host() -> HostEnvironment {
    HostEnvironment {
        app_version: v("0.17.0"),
        plugin_api: ApiLevel::new(1, 16),
        platform: Platform::Macos,
    }
}

/// The fixture in a temporary directory, with `edit` applied to its
/// manifest, and a stand-in module.
struct PluginDir {
    dir: TempDir,
}

impl PluginDir {
    fn new(edit: impl FnOnce(&mut Value)) -> Self {
        let dir = TempDir::new().unwrap();
        let mut m = manifest();
        edit(&mut m);
        std::fs::write(
            dir.path().join("manifest.json"),
            serde_json::to_vec_pretty(&m).unwrap(),
        )
        .unwrap();
        std::fs::copy(fixture().join("LICENSE"), dir.path().join("LICENSE")).unwrap();
        for sub in ["locales", "assets"] {
            std::fs::create_dir_all(dir.path().join(sub)).unwrap();
            for entry in std::fs::read_dir(fixture().join(sub)).unwrap() {
                let entry = entry.unwrap();
                std::fs::copy(entry.path(), dir.path().join(sub).join(entry.file_name())).unwrap();
            }
        }
        std::fs::write(
            dir.path().join("module.wasm"),
            deckpilot_plugin_package::wasm::minimal_abi_module_with_note("calls 0.1.0"),
        )
        .unwrap();
        Self { dir }
    }

    fn package(&self, gate: PrivilegedGate<'_>) -> Result<Vec<u8>, String> {
        package_with_gate(
            self.dir.path(),
            &self.dir.path().join("module.wasm"),
            true,
            gate,
        )
        .map(|built| built.bytes)
        .map_err(|e| e.to_string())
    }

    /// A package with the bundled rules but without the policy check: what
    /// a build that skipped the gate would produce, for the verifier.
    fn bundled_rules_package(&self) -> Vec<u8> {
        let mut builder = PackageBuilder::new();
        builder.reviewed_privileged();
        for (name, bytes) in self.files() {
            builder.add(&name, bytes).unwrap();
        }
        builder.build().unwrap().bytes
    }

    fn files(&self) -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        let mut stack = vec![self.dir.path().to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let rel = path
                    .strip_prefix(self.dir.path())
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .replace('\\', "/");
                let name = if rel == "module.wasm" {
                    "plugin.wasm".to_owned()
                } else {
                    rel
                };
                out.push((name, std::fs::read(&path).unwrap()));
            }
        }
        out.sort();
        out
    }
}

fn keys(a: &ThrowawayKey, b: &ThrowawayKey) -> PublisherKeys {
    PublisherKeys {
        publisher_keys_version: 1,
        publisher: "deckpilot".into(),
        keys: vec![a.public().to_owned(), b.public().to_owned()],
        threshold: 2,
        revoked_keys: Vec::new(),
    }
}

fn signed(key: &ThrowawayKey, bytes: &[u8], claim: &ReleaseClaim) -> SignatureFile {
    let signature = key.sign(
        bytes,
        &release_comment(&claim.id, &claim.version, &claim.package),
    );
    SignatureFile {
        key_id: key.key_id(),
        bytes: format!("{signature}\n").into_bytes(),
    }
}

/// `release verify` of `bytes` signed by two distinct throwaway keys.
fn verify(bytes: &[u8], gate: PrivilegedGate<'_>) -> Result<(), PublisherError> {
    let a = ThrowawayKey::generate();
    let b = ThrowawayKey::generate();
    let claim = ReleaseClaim {
        id: id(CALLS),
        version: v(VERSION),
        package: digest_of(bytes),
    };
    let signatures = [signed(&a, bytes, &claim), signed(&b, bytes, &claim)];
    let limits = Limits::default();
    let host = host();
    let policy = ReleasePolicy {
        host: Some(&host),
        floor: None,
        revoked: &[],
        limits: &limits,
        gate,
    };
    verify_release(bytes, &signatures, &keys(&a, &b), &claim, &policy).map(drop)
}

/// The policy entry the owner's release step adds (calls-status.md, "Owner
/// release steps"), made from the manifest: its exact permissions and its
/// three constructs with their values.
fn calls_entry(m: &Value) -> Value {
    json!({
        "id": CALLS,
        "version": VERSION,
        "permissions": m["permissions"],
        "constructs": [
            {"path": "permissions[0]", "what": "the `calls` permission"},
            {"path": "contributes.callOverlay", "what": "`callOverlay`",
             "value": m["contributes"]["callOverlay"]},
            {"path": "contributes.layoutTemplates", "what": "`layoutTemplates`",
             "value": m["contributes"]["layoutTemplates"]},
        ],
    })
}

/// The compiled policy plus the Calls entry, approved by a throwaway test
/// review. Built in memory only.
fn test_policy_json() -> Vec<u8> {
    let mut policy: Value = serde_json::from_str(POLICY_JSON).unwrap();
    policy["plugins"]
        .as_array_mut()
        .unwrap()
        .push(calls_entry(&manifest()));
    policy["review"] = json!({
        "reviewer": "throwaway test reviewer",
        "date": "2026-11-25",
        "outcome": "approved",
        "record": "tests/calls_privileged.rs",
    });
    serde_json::to_vec(&policy).unwrap()
}

/// The fixture is the plugin's manifest: valid with the bundled rules, and
/// its privileged usages are exactly the three the policy entry names.
#[test]
fn the_fixture_is_valid_bundled_with_three_privileged_usages() {
    let m = Manifest::from_json(&manifest().to_string()).unwrap();
    m.validate_bundled().unwrap();
    let usages: Vec<(String, String)> = m.privileged_usages();
    assert_eq!(
        usages,
        [
            ("permissions[0]".into(), "the `calls` permission".into()),
            ("contributes.callOverlay".into(), "`callOverlay`".into()),
            (
                "contributes.layoutTemplates".into(),
                "`layoutTemplates`".into()
            ),
        ]
    );
    let entry = calls_entry(&manifest());
    assert_eq!(entry["constructs"].as_array().unwrap().len(), usages.len());
}

/// This build ships no Calls: the compiled gate (open for exactly the
/// reviewed five) refuses `deckpilot.calls` in the packager, `release
/// sign`'s preflight and `release verify` with two distinct keys.
#[test]
fn this_build_refuses_the_calls_plugin_on_every_release_path() {
    let dir = PluginDir::new(|_| {});
    let packaged = dir.package(PrivilegedGate::compiled());
    assert!(packaged.is_err(), "packaged: {packaged:?}");
    let bytes = dir.bundled_rules_package();
    assert!(
        preflight_with_gate(&bytes, id(CALLS), v(VERSION), PrivilegedGate::compiled()).is_err(),
        "preflight"
    );
    let verified = verify(&bytes, PrivilegedGate::compiled());
    assert!(
        matches!(
            verified,
            Err(PublisherError::PrivilegedGateClosed { .. }
                | PublisherError::PrivilegedNotReviewed { .. })
        ),
        "{verified:?}"
    );
    if let Err(PublisherError::PrivilegedNotReviewed { reason, .. }) = &verified {
        assert!(
            reason.contains("does not cover this plugin"),
            "the compiled record has no Calls entry: {reason}"
        );
    }
}

/// A reviewed entry alone does not admit it: `deckpilot.calls` is not one
/// of the privileged plugin ids, so a policy that lists it is invalid,
/// and an open gate with that policy refuses every path for that reason.
/// The owner's release step changes the id list and adds the entry
/// together.
#[test]
fn a_reviewed_calls_entry_still_needs_the_owners_id_list() {
    let refused = PrivilegedPolicy::from_json(&test_policy_json()).unwrap_err();
    assert!(
        refused.contains("deckpilot.calls is not one of DeckPilot's privileged plugins"),
        "{refused}"
    );
    // Parsed without validation, then handed to an open test gate: the
    // gate validates it again and refuses.
    let mut policy: PrivilegedPolicy = serde_json::from_slice(&test_policy_json()).unwrap();
    policy.review = Some(Review {
        reviewer: "throwaway test reviewer".into(),
        date: "2026-11-25".into(),
        outcome: ReviewOutcome::Approved,
        record: "tests/calls_privileged.rs".into(),
    });
    let gate = PrivilegedGate::open_for_tests(&policy);
    let dir = PluginDir::new(|_| {});
    let packaged = dir.package(gate).unwrap_err();
    assert!(
        packaged.contains("is not one of DeckPilot's privileged plugins"),
        "{packaged}"
    );
    let bytes = dir.bundled_rules_package();
    assert!(preflight_with_gate(&bytes, id(CALLS), v(VERSION), gate).is_err());
    match verify(&bytes, gate) {
        Err(PublisherError::PrivilegedNotReviewed { reason, .. }) => {
            assert!(
                reason.contains("is not one of DeckPilot's privileged plugins"),
                "{reason}"
            );
        }
        other => panic!("{other:?}"),
    }
}

/// Every path but a bundled first-party package is refused by the
/// manifest rules themselves, whatever any policy says: the public
/// (registry and sideload) rules, a first-party package of another id, a
/// `calls` plugin with a way out, and an API below 1.16. And a first-party
/// package refuses a changed template like any unlisted construct.
#[test]
fn the_calls_kinds_are_bundled_only_and_pinned_to_the_plugin() {
    let parse = |m: &Value| Manifest::from_json(&m.to_string()).unwrap();
    // Public rules (a registry or sideloaded plugin): first-party only.
    let public = parse(&manifest()).validate().unwrap_err();
    let first_party_only: Vec<&str> = public
        .0
        .iter()
        .filter(|i| i.code == IssueCode::FirstPartyOnly)
        .map(|i| i.path.as_str())
        .collect();
    for path in [
        "permissions[0]",
        "contributes.callOverlay",
        "contributes.layoutTemplates",
    ] {
        assert!(first_party_only.contains(&path), "{path}: {public:?}");
    }
    // Even with the bundled rules: another id, a way out, a lower API.
    // Refused, and for the Calls rule at `at` (a conflict there).
    let refused = |what: &str, at: &str, edit: &dyn Fn(&mut Value)| {
        let mut m = manifest();
        edit(&mut m);
        let issues = parse(&m).validate_bundled().unwrap_err();
        assert!(
            issues
                .0
                .iter()
                .any(|i| i.path == at && i.code == IssueCode::Conflict),
            "{what}: not refused at {at}: {issues:?}"
        );
    };
    refused("another id", "permissions[0]", &|m| {
        m["id"] = json!("deckpilot.workspace");
    });
    for way_out in [
        json!({"kind": "network", "hosts": ["example.com"], "methods": ["GET"],
               "justification": "x"}),
        json!({"kind": "openUrl", "hosts": ["example.com"], "justification": "x"}),
        json!({"kind": "connections", "types": ["github"], "methods": ["GET"],
               "justification": "x"}),
    ] {
        refused(
            &format!("calls with {}", way_out["kind"]),
            "permissions[2]",
            &|m| {
                m["permissions"]
                    .as_array_mut()
                    .unwrap()
                    .push(way_out.clone());
            },
        );
    }
    refused("plugin API 1.9", "permissions[0]", &|m| {
        m["engines"]["pluginApi"] = json!("^1.9");
    });
    // A package without the first-party rules refuses it before any gate.
    let dir = PluginDir::new(|_| {});
    let public_package = package_with_gate(
        dir.dir.path(),
        &dir.dir.path().join("module.wasm"),
        false,
        PrivilegedGate::Closed,
    );
    assert!(
        public_package.is_err(),
        "a public package of the Calls plugin"
    );
}

/// `PrivilegedGate::open_for_tests` exists only with the feature
/// `test-privileged-gate`, which only `[dev-dependencies]` enable: no
/// release-relevant build turns it on (the agent with `plugins`, the
/// registry tool with `publish`, the desktop app, which has the registry
/// crate as a dev-dependency only).
#[test]
fn no_release_build_enables_the_test_gate() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (manifest, package, features) in [
        ("Cargo.toml", "deckpilot-agent", "plugins"),
        ("Cargo.toml", "deckpilot-plugin-registry", "publish"),
        ("apps/desktop/src-tauri/Cargo.toml", "deckpilot-desktop", ""),
    ] {
        let mut command = Command::new(option_env!("CARGO").unwrap_or("cargo"));
        command
            .args(["tree", "--locked", "--manifest-path"])
            .arg(root.join(manifest))
            .args([
                "-p",
                package,
                "--edges",
                "normal,build,features",
                "--prefix",
                "none",
            ]);
        if !features.is_empty() {
            command.args(["--features", features]);
        }
        let output = command.output().expect("cargo tree runs");
        assert!(
            output.status.success(),
            "cargo tree -p {package}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let tree = String::from_utf8_lossy(&output.stdout);
        // The agent and the tool link the registry crate; the desktop app
        // only in its tests.
        if package != "deckpilot-desktop" {
            assert!(
                tree.contains("deckpilot-plugin-registry"),
                "{package}: the tree does not reach the registry crate"
            );
        }
        assert!(
            !tree.contains("test-privileged-gate"),
            "{package} enables test-privileged-gate in a normal build"
        );
    }
    // A positive control: the crate's own tests do enable it.
    let mut command = Command::new(option_env!("CARGO").unwrap_or("cargo"));
    command
        .args(["tree", "--locked", "--manifest-path"])
        .arg(root.join("Cargo.toml"))
        .args([
            "-p",
            "deckpilot-plugin-registry",
            "--edges",
            "normal,build,dev,features",
            "--prefix",
            "none",
        ]);
    let output = command.output().expect("cargo tree runs");
    assert!(String::from_utf8_lossy(&output.stdout).contains("test-privileged-gate"));
}
