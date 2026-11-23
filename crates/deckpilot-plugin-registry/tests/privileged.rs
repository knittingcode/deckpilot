//! The privileged-kinds gate and the reviewed privileged policy
//! (`src/privileged_policy.rs`, `docs/marketplace/publisher-signature.md`,
//! "Privileged plugins"): `package --first-party`, `release sign`'s
//! preflight and post-sign check, `release verify` and `verify_release`,
//! closed (this build) and opened in the test only, with the real manifests
//! of the five privileged plugins and throwaway publisher keys. Every
//! refusal, and the accepted reviewed case.

// One flow per test, with the cases spelled out.
#![allow(
    clippy::too_many_lines,
    clippy::too_many_arguments,
    clippy::type_complexity
)]

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use common::{ThrowawayKey, id, v};
use deckpilot_plugin_manifest::{ApiLevel, HostEnvironment, Platform};
use deckpilot_plugin_package::{Limits, PackageBuilder};
use deckpilot_plugin_registry::RevokedVersions;
use deckpilot_plugin_registry::privileged_policy::{
    self, MIN_THRESHOLD, POLICY_JSON, PRIVILEGED_PLUGIN_IDS, PrivilegedGate, PrivilegedPolicy,
    Review, ReviewOutcome,
};
use deckpilot_plugin_registry::publish::{
    VerifyRequest, check_signed_with_gate, package_with_gate, preflight_with_gate,
    verify_release_files, verify_release_files_with_gate,
};
use deckpilot_plugin_registry::publisher::{
    PublisherError, PublisherKeys, ReleaseClaim, ReleaseFloor, ReleasePolicy, RevokedRelease,
    SignatureFile, digest_of, release_comment, verify_release,
};
use serde_json::{Value, json};
use tempfile::TempDir;

const CLOSED_MESSAGE: &str = "only for DeckPilot's own plugins shipped inside the app";

/// What each official plugin's release ships but its module
/// (`plugins/first-party/contracts/deckpilot.<name>`).
fn first_party() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/first-party/contracts")
}

/// The compiled policy with an approved review (the review is the test's).
fn approved() -> &'static PrivilegedPolicy {
    let mut policy = PrivilegedPolicy::from_json(POLICY_JSON.as_bytes()).unwrap();
    policy.review = Some(Review {
        reviewer: "throwaway test reviewer".into(),
        date: "2026-11-21".into(),
        outcome: ReviewOutcome::Approved,
        record: "tests/privileged.rs".into(),
    });
    Box::leak(Box::new(policy))
}

fn open() -> PrivilegedGate<'static> {
    PrivilegedGate::open_for_tests(approved())
}

fn host() -> HostEnvironment {
    HostEnvironment {
        app_version: v("0.10.0"),
        plugin_api: ApiLevel::new(1, 9),
        platform: Platform::Macos,
    }
}

/// A copy of the release files of `deckpilot.<name>` at `version` as the plugin
/// repository would package it, with `edit` applied to the manifest, and a
/// stand-in module.
struct PluginDir {
    dir: TempDir,
}

impl PluginDir {
    fn new(name: &str, version: &str, edit: impl FnOnce(&mut Value)) -> Self {
        let dir = TempDir::new().unwrap();
        let source = first_party().join(format!("deckpilot.{name}"));
        let mut manifest: Value =
            serde_json::from_slice(&std::fs::read(source.join("manifest.json")).unwrap()).unwrap();
        manifest["version"] = version.into();
        edit(&mut manifest);
        std::fs::write(
            dir.path().join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        std::fs::copy(source.join("LICENSE"), dir.path().join("LICENSE")).unwrap();
        for sub in ["locales", "assets"] {
            let Ok(entries) = std::fs::read_dir(source.join(sub)) else {
                continue;
            };
            std::fs::create_dir_all(dir.path().join(sub)).unwrap();
            for entry in entries {
                let entry = entry.unwrap();
                std::fs::copy(entry.path(), dir.path().join(sub).join(entry.file_name())).unwrap();
            }
        }
        std::fs::write(
            dir.path().join("module.wasm"),
            deckpilot_plugin_package::wasm::minimal_abi_module_with_note(&format!(
                "{name} {version}"
            )),
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

    /// The package's files by package path.
    fn files(&self) -> Vec<(String, Vec<u8>)> {
        walk(self.dir.path())
            .into_iter()
            .map(|entry| {
                let rel = entry.strip_prefix(self.dir.path()).unwrap();
                let rel = rel.to_str().unwrap().replace('\\', "/");
                let name = if rel == "module.wasm" {
                    "plugin.wasm".to_owned()
                } else {
                    rel
                };
                (name, std::fs::read(&entry).unwrap())
            })
            .collect()
    }

    /// The package written with the bundled rules but **without** the
    /// policy check (`PackageBuilder::reviewed_privileged` directly).
    fn bundled_rules_package(&self) -> Vec<u8> {
        let mut builder = PackageBuilder::new();
        builder.reviewed_privileged();
        for (name, bytes) in self.files() {
            builder.add(&name, bytes).unwrap();
        }
        builder.build().unwrap().bytes
    }

    /// The package written with no content check at all: what an attacker
    /// or a mistaken build could produce.
    fn unchecked_package(&self) -> Vec<u8> {
        let mut tar = tar::Builder::new(Vec::new());
        for (path, bytes) in self.files() {
            let mut header = tar::Header::new_ustar();
            header.set_path(&path).unwrap();
            header.set_entry_type(tar::EntryType::Regular);
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_mtime(0);
            header.set_cksum();
            tar.append(&header, bytes.as_slice()).unwrap();
        }
        let tar = tar.into_inner().unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut gz, &tar).unwrap();
        gz.finish().unwrap()
    }
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else {
            out.push(path);
        }
    }
    out.sort();
    out
}

fn keys(current: &[&ThrowawayKey], threshold: u64) -> PublisherKeys {
    PublisherKeys {
        publisher_keys_version: 1,
        publisher: "deckpilot".into(),
        keys: current.iter().map(|k| k.public().to_owned()).collect(),
        threshold,
        revoked_keys: Vec::new(),
    }
}

fn claim(bytes: &[u8], plugin: &str, version: &str) -> ReleaseClaim {
    ReleaseClaim {
        id: id(plugin),
        version: v(version),
        package: digest_of(bytes),
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

fn verify(
    bytes: &[u8],
    signatures: &[SignatureFile],
    keys: &PublisherKeys,
    claim: &ReleaseClaim,
    gate: PrivilegedGate<'_>,
    floor: Option<&ReleaseFloor>,
    revoked: &[RevokedRelease],
    host: &HostEnvironment,
) -> Result<deckpilot_plugin_registry::publisher::VerifiedRelease, PublisherError> {
    let limits = Limits::default();
    let policy = ReleasePolicy {
        host: Some(host),
        floor,
        revoked,
        limits: &limits,
        gate,
    };
    verify_release(bytes, signatures, keys, claim, &policy)
}

fn not_reviewed<T: std::fmt::Debug>(result: Result<T, PublisherError>, text: &str) {
    match result {
        Err(PublisherError::PrivilegedNotReviewed { reason, .. }) => {
            assert!(reason.contains(text), "`{text}` not in: {reason}");
        }
        other => panic!("expected PrivilegedNotReviewed ({text}), got {other:?}"),
    }
}

fn name_of(id: &str) -> &str {
    id.strip_prefix("deckpilot.").unwrap()
}

// ------------------------------------------------------------- closed

/// The opening change: the constant is open; the compiled gate is open
/// exactly when the record's review is approved, and with it the
/// packager, `release sign` and `release verify` admit the reviewed
/// releases; while the review is pending they refuse them as closed.
#[test]
fn this_build_s_gate_opens_only_with_the_approved_review() {
    const { assert!(privileged_policy::PRIVILEGED_GATE_OPEN) };
    let compiled = privileged_policy::compiled_policy().unwrap();
    let approved = compiled.is_approved();
    assert_eq!(
        PrivilegedGate::compiled() == PrivilegedGate::Closed,
        !approved
    );
    let a = ThrowawayKey::generate();
    let b = ThrowawayKey::generate();
    for plugin in PRIVILEGED_PLUGIN_IDS {
        let dir = PluginDir::new(name_of(plugin), "0.1.1", |_| {});
        let packaged = dir.package(PrivilegedGate::compiled());
        let preflight = preflight_with_gate(
            &dir.unchecked_package(),
            id(plugin),
            v("0.1.1"),
            PrivilegedGate::compiled(),
        );
        let bytes = dir.bundled_rules_package();
        let claim = claim(&bytes, plugin, "0.1.1");
        let signatures = [signed(&a, &bytes, &claim), signed(&b, &bytes, &claim)];
        let verified = verify(
            &bytes,
            &signatures,
            &keys(&[&a, &b], 2),
            &claim,
            PrivilegedGate::compiled(),
            None,
            &[],
            &host(),
        );
        if approved {
            packaged.unwrap();
            preflight.unwrap();
            verified.unwrap();
        } else {
            assert!(packaged.unwrap_err().contains(CLOSED_MESSAGE), "{plugin}");
            assert!(preflight.is_err(), "{plugin}");
            assert!(
                matches!(verified, Err(PublisherError::PrivilegedGateClosed { .. })),
                "{plugin}"
            );
        }
    }
}

/// The production gate (`PrivilegedGate::compiled()`, whatever the
/// record's outcome, pending or approved) refuses what the record does not
/// list: another version, an extra construct, another plugin id with
/// reviewed privileges. In the packager, `release sign`'s preflight and
/// `verify_release` with two distinct keys.
#[test]
fn the_compiled_gate_refuses_what_the_record_does_not_list() {
    let a = ThrowawayKey::generate();
    let b = ThrowawayKey::generate();
    let cases: [(&str, &str, &str, Box<dyn Fn(&mut Value)>); 6] = [
        ("another version", "workflows", "0.1.2", Box::new(|_| {})),
        (
            "a broader permission",
            "projects",
            "0.1.1",
            Box::new(|m| m["permissions"][1]["open"] = json!(true)),
        ),
        (
            "another executable",
            "git",
            "0.1.1",
            Box::new(|m| {
                m["permissions"][0]["tools"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({
                        "executable": "docker",
                        "subcommands": [{"name": "ps"}]
                    }));
            }),
        ),
        (
            "another git subcommand",
            "git",
            "0.1.1",
            Box::new(|m| {
                m["permissions"][0]["tools"][0]["subcommands"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"name": "fetch"}));
            }),
        ),
        (
            "an extra construct",
            "workflows",
            "0.1.1",
            Box::new(|m| m["contributes"]["statusItems"] = json!(["activeProject"])),
        ),
        (
            "another plugin id",
            "git",
            "0.1.1",
            Box::new(|m| m["id"] = json!("deckpilot.github")),
        ),
    ];
    for (what, name, version, edit) in cases {
        let dir = PluginDir::new(name, version, |m| edit(m));
        let manifest: Value =
            serde_json::from_slice(&std::fs::read(dir.dir.path().join("manifest.json")).unwrap())
                .unwrap();
        let plugin = manifest["id"].as_str().unwrap().to_owned();
        assert!(
            dir.package(PrivilegedGate::compiled()).is_err(),
            "{what}: packaged"
        );
        let bytes = dir.unchecked_package();
        assert!(
            preflight_with_gate(&bytes, id(&plugin), v(version), PrivilegedGate::compiled())
                .is_err(),
            "{what}: preflight"
        );
        let claim = claim(&bytes, &plugin, version);
        let signatures = [signed(&a, &bytes, &claim), signed(&b, &bytes, &claim)];
        let result = verify(
            &bytes,
            &signatures,
            &keys(&[&a, &b], 2),
            &claim,
            PrivilegedGate::compiled(),
            None,
            &[],
            &host(),
        );
        assert!(
            matches!(
                result,
                Err(PublisherError::PrivilegedGateClosed { .. }
                    | PublisherError::PrivilegedNotReviewed { .. })
            ),
            "{what}: {result:?}"
        );
    }
}

/// The production gate never admits a privileged release with fewer than
/// two distinct pinned keys, even exactly the reviewed one: one key with
/// threshold 1, two keys with threshold 1, threshold 2 with one signature.
#[test]
fn the_compiled_gate_needs_two_distinct_keys() {
    let a = ThrowawayKey::generate();
    let b = ThrowawayKey::generate();
    let bytes = PluginDir::new("git", "0.1.1", |_| {}).bundled_rules_package();
    let claim = claim(&bytes, "deckpilot.git", "0.1.1");
    let by_a = signed(&a, &bytes, &claim);
    let by_b = signed(&b, &bytes, &claim);
    for (signatures, pinned) in [
        (vec![by_a.clone()], keys(&[&a], 1)),
        (vec![by_a.clone(), by_b.clone()], keys(&[&a, &b], 1)),
        (vec![by_a.clone()], keys(&[&a, &b], 2)),
    ] {
        let result = verify(
            &bytes,
            &signatures,
            &pinned,
            &claim,
            PrivilegedGate::compiled(),
            None,
            &[],
            &host(),
        );
        assert!(
            matches!(
                result,
                Err(PublisherError::PrivilegedGateClosed { .. }
                    | PublisherError::PrivilegedNotReviewed { .. }
                    | PublisherError::BelowThreshold { .. })
            ),
            "threshold {}: {result:?}",
            pinned.threshold
        );
    }
}

/// `package --first-party` refuses every privileged plugin with the gate
/// closed, compiled or given, with the first-party rules' own message.
#[test]
fn the_packager_refuses_every_privileged_plugin_while_closed() {
    for plugin in PRIVILEGED_PLUGIN_IDS {
        let dir = PluginDir::new(name_of(plugin), "0.1.1", |_| {});
        let error = dir.package(PrivilegedGate::Closed).unwrap_err();
        assert!(error.contains(CLOSED_MESSAGE), "{plugin}: {error}");
    }
    // The other first-party plugins package as before.
    let github = PluginDir::new("github", "0.2.9", |_| {});
    github.package(PrivilegedGate::Closed).unwrap();
}

#[test]
fn release_sign_and_verify_refuse_every_privileged_plugin_while_closed() {
    let a = ThrowawayKey::generate();
    let b = ThrowawayKey::generate();
    for plugin in PRIVILEGED_PLUGIN_IDS {
        let bytes = PluginDir::new(name_of(plugin), "0.1.1", |_| {}).unchecked_package();
        let error = preflight_with_gate(&bytes, id(plugin), v("0.1.1"), PrivilegedGate::Closed)
            .unwrap_err()
            .to_string();
        assert!(error.contains("refusing to sign"), "{error}");
        assert!(error.contains("publisher-review-request.md"), "{error}");
        let claim = claim(&bytes, plugin, "0.1.1");
        let signatures = [signed(&a, &bytes, &claim), signed(&b, &bytes, &claim)];
        let result = verify(
            &bytes,
            &signatures,
            &keys(&[&a, &b], 2),
            &claim,
            PrivilegedGate::Closed,
            None,
            &[],
            &host(),
        );
        assert!(
            matches!(result, Err(PublisherError::PrivilegedGateClosed { .. })),
            "{plugin}: {result:?}"
        );
    }
}

// --------------------------------------------------------------- open

/// The accepted case: each reviewed plugin, exactly as reviewed, packaged,
/// preflighted, signed by two distinct keys and verified with the gate
/// open; the verified files are those of the package, with its privileged
/// kinds, and the verification says how it was admitted.
#[test]
fn the_open_gate_admits_exactly_the_reviewed_releases() {
    let a = ThrowawayKey::generate();
    let b = ThrowawayKey::generate();
    for plugin in PRIVILEGED_PLUGIN_IDS {
        let dir = PluginDir::new(name_of(plugin), "0.1.1", |_| {});
        let bytes = dir.package(open()).unwrap();
        assert_eq!(bytes, dir.bundled_rules_package(), "deterministic");
        let claim = preflight_with_gate(&bytes, id(plugin), v("0.1.1"), open()).unwrap();
        let signatures = [signed(&a, &bytes, &claim), signed(&b, &bytes, &claim)];
        // Each key's own post-sign check (one key per job).
        for (key, file) in [(&a, &signatures[0]), (&b, &signatures[1])] {
            let text = String::from_utf8(file.bytes.clone()).unwrap();
            check_signed_with_gate(&bytes, text.trim(), key.public(), &claim, open()).unwrap();
        }
        let verified = verify(
            &bytes,
            &signatures,
            &keys(&[&a, &b], 2),
            &claim,
            open(),
            None,
            &[],
            &host(),
        )
        .unwrap_or_else(|e| panic!("{plugin}: {e}"));
        assert_eq!(verified.signers().len(), 2);
        let how = verified.privileged().unwrap();
        assert!(
            how.starts_with(&format!("reviewed: {plugin} 0.1.1")),
            "{how}"
        );
        assert!(how.contains("throwaway test reviewer"), "{how}");
        assert_ne!(verified.manifest().privileged_usages().len(), 0);
        assert_eq!(verified.manifest().id.as_str(), plugin);
    }
}

#[test]
fn the_open_gate_needs_two_distinct_pinned_keys_that_signed() {
    let a = ThrowawayKey::generate();
    let b = ThrowawayKey::generate();
    let c = ThrowawayKey::generate();
    let bytes = PluginDir::new("git", "0.1.1", |_| {})
        .package(open())
        .unwrap();
    let claim = claim(&bytes, "deckpilot.git", "0.1.1");
    let by_a = signed(&a, &bytes, &claim);
    let by_b = signed(&b, &bytes, &claim);
    let run = |signatures: &[SignatureFile], keys: &PublisherKeys| {
        verify(&bytes, signatures, keys, &claim, open(), None, &[], &host())
    };
    // One pinned key, threshold 1: never enough for a privileged release.
    not_reviewed(
        run(std::slice::from_ref(&by_a), &keys(&[&a], 1)),
        "needs 2 distinct",
    );
    // Two pinned keys, threshold 1, both signed: the pinned threshold must
    // itself be at least 2.
    not_reviewed(
        run(&[by_a.clone(), by_b.clone()], &keys(&[&a, &b], 1)),
        "pinned threshold 1",
    );
    // Threshold 2, one signed: the ordinary threshold refuses.
    assert!(matches!(
        run(std::slice::from_ref(&by_a), &keys(&[&a, &b], 2)),
        Err(PublisherError::BelowThreshold { valid: 1, .. })
    ));
    // The same signature twice counts once.
    let again = SignatureFile {
        key_id: b.key_id(),
        bytes: by_a.bytes.clone(),
    };
    assert!(matches!(
        run(&[by_a.clone(), again], &keys(&[&a, &b], 2)),
        Err(PublisherError::BelowThreshold { valid: 1, .. })
    ));
    // A key that is not pinned does not count.
    let by_c = signed(&c, &bytes, &claim);
    assert!(matches!(
        run(&[by_a.clone(), by_c], &keys(&[&a, &b], 2)),
        Err(PublisherError::BelowThreshold { valid: 1, .. })
    ));
    // A policy asking for three: two are not enough.
    let mut three = approved().clone();
    three.min_threshold = 3;
    let three: &'static PrivilegedPolicy = Box::leak(Box::new(three));
    let result = verify(
        &bytes,
        &[by_a.clone(), by_b.clone()],
        &keys(&[&a, &b, &c], 2),
        &claim,
        PrivilegedGate::open_for_tests(three),
        None,
        &[],
        &host(),
    );
    not_reviewed(result, "needs 3 distinct");
    // Two distinct pinned keys: admitted.
    run(&[by_a, by_b], &keys(&[&a, &b], 2)).unwrap();
    assert_eq!(MIN_THRESHOLD, 2);
}

/// Everything that is not exactly the record fails closed, in every tool:
/// a new construct, a broader permission, an extra executable, another
/// plugin, another version, a pending review.
#[test]
fn the_open_gate_refuses_anything_but_the_record() {
    let a = ThrowawayKey::generate();
    let b = ThrowawayKey::generate();
    let pinned = keys(&[&a, &b], 2);
    let cases: Vec<(&str, &str, &str, Box<dyn Fn(&mut Value)>, &str)> = vec![
        (
            "a new construct (status items, with the same permissions)",
            "workflows",
            "0.1.1",
            Box::new(|m| m["contributes"]["statusItems"] = json!(["activeProject"])),
            "contributes.statusItems (`statusItems`) is not reviewed",
        ),
        (
            "a broader resource parameter",
            "git",
            "0.1.1",
            Box::new(|m| {
                m["contributes"]["actions"][0]["parameters"][0]["kind"]["kinds"] =
                    json!(["directory", "file"]);
            }),
            "differs from the reviewed value",
        ),
        (
            "a broader process permission (another git subcommand)",
            "git",
            "0.1.1",
            Box::new(|m| {
                m["permissions"][0]["tools"][0]["subcommands"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"name": "fetch"}));
            }),
            "permissions[0] (tools) differ",
        ),
        (
            "an extra executable",
            "git",
            "0.1.1",
            Box::new(|m| {
                m["permissions"][0]["tools"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({
                        "executable": "docker",
                        "subcommands": [{"name": "ps"}]
                    }));
            }),
            "differ",
        ),
        (
            "a changed process permission (Docker without its separate modify grant)",
            "docker",
            "0.1.1",
            Box::new(|m| {
                m["permissions"][0]["tools"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("modify");
            }),
            "permissions[0] (tools) differ",
        ),
        (
            "a broader resources permission",
            "projects",
            "0.1.1",
            Box::new(|m| m["permissions"][1]["open"] = json!(true)),
            "permissions[1] (resources) differ",
        ),
        (
            "a new permission",
            "workflows",
            "0.1.1",
            Box::new(|m| {
                m["permissions"].as_array_mut().unwrap().push(json!({
                    "kind": "projectContext",
                    "select": true,
                    "justification": "t:permissions.hostViews"
                }));
            }),
            "2 permission(s) declared",
        ),
        (
            "another version",
            "workspace",
            "0.1.2",
            Box::new(|_| {}),
            "covers version 0.1.1 only",
        ),
    ];
    for (what, name, version, edit, text) in cases {
        let dir = PluginDir::new(name, version, |m| edit(m));
        let plugin = format!("deckpilot.{name}");
        // The packager refuses it.
        let error = dir.package(open()).unwrap_err();
        assert!(
            error.contains("refusing to package") && error.contains(text),
            "{what}: {error}"
        );
        // So does release sign's preflight, on a package built without the
        // check.
        let bytes = dir.unchecked_package();
        let error = preflight_with_gate(&bytes, id(&plugin), v(version), open())
            .unwrap_err()
            .to_string();
        assert!(error.contains(text), "{what}: {error}");
        // And release verify, even signed by both keys.
        let claim = claim(&bytes, &plugin, version);
        let signatures = [signed(&a, &bytes, &claim), signed(&b, &bytes, &claim)];
        not_reviewed(
            verify(
                &bytes,
                &signatures,
                &pinned,
                &claim,
                open(),
                None,
                &[],
                &host(),
            ),
            text,
        );
    }
}

#[test]
fn the_open_gate_refuses_another_plugin_with_reviewed_privileges() {
    let a = ThrowawayKey::generate();
    let b = ThrowawayKey::generate();
    // deckpilot.github with deckpilot.git's privileges.
    let dir = PluginDir::new("git", "0.1.1", |m| m["id"] = json!("deckpilot.github"));
    let error = dir.package(open()).unwrap_err();
    assert!(error.contains("does not cover this plugin"), "{error}");
    let bytes = dir.unchecked_package();
    let claim = claim(&bytes, "deckpilot.github", "0.1.1");
    let signatures = [signed(&a, &bytes, &claim), signed(&b, &bytes, &claim)];
    not_reviewed(
        verify(
            &bytes,
            &signatures,
            &keys(&[&a, &b], 2),
            &claim,
            open(),
            None,
            &[],
            &host(),
        ),
        "does not cover this plugin",
    );
    // A claim of another plugin for the reviewed bytes: the manifest is not
    // the claimed release.
    let bytes = PluginDir::new("git", "0.1.1", |_| {})
        .package(open())
        .unwrap();
    let error = preflight_with_gate(&bytes, id("deckpilot.docker"), v("0.1.1"), open())
        .unwrap_err()
        .to_string();
    assert!(error.contains("not the claimed release"), "{error}");
}

#[test]
fn a_pending_or_missing_review_opens_nothing() {
    let dir = PluginDir::new("workflows", "0.1.1", |_| {});
    let mut pending = approved().clone();
    pending.review.as_mut().unwrap().outcome = ReviewOutcome::Pending;
    let pending: &'static PrivilegedPolicy = Box::leak(Box::new(pending));
    let error = dir
        .package(PrivilegedGate::open_for_tests(pending))
        .unwrap_err();
    assert!(error.contains("no approved security review"), "{error}");
    // The compiled policy, while its review is pending.
    let compiled = privileged_policy::compiled_policy().unwrap();
    if !compiled.is_approved() {
        let error = dir
            .package(PrivilegedGate::open_for_tests(compiled))
            .unwrap_err();
        assert!(error.contains("no approved security review"), "{error}");
    }
}

/// Admission changes nothing about revocation, rollback and compatibility.
#[test]
fn reviewed_releases_still_meet_revocation_rollback_and_compatibility() {
    let a = ThrowawayKey::generate();
    let b = ThrowawayKey::generate();
    let pinned = keys(&[&a, &b], 2);
    let bytes = PluginDir::new("docker", "0.1.1", |_| {})
        .package(open())
        .unwrap();
    let claim = claim(&bytes, "deckpilot.docker", "0.1.1");
    let signatures = [signed(&a, &bytes, &claim), signed(&b, &bytes, &claim)];
    let run = |floor: Option<&ReleaseFloor>, revoked: &[RevokedRelease], host: &HostEnvironment| {
        verify(
            &bytes,
            &signatures,
            &pinned,
            &claim,
            open(),
            floor,
            revoked,
            host,
        )
    };
    let verified = run(None, &[], &host()).unwrap();
    // Rollback: below a shipped version.
    let floor = ReleaseFloor {
        version: v("0.1.2"),
        files: BTreeMap::default(),
    };
    assert!(matches!(
        run(Some(&floor), &[], &host()),
        Err(PublisherError::Rollback { .. })
    ));
    // The same version only with the same files.
    let mut files = verified.file_digests();
    let same = ReleaseFloor {
        version: v("0.1.1"),
        files: files.clone(),
    };
    run(Some(&same), &[], &host()).unwrap();
    files.insert("plugin.wasm".into(), digest_of(b"other"));
    let other = ReleaseFloor {
        version: v("0.1.1"),
        files,
    };
    assert!(matches!(
        run(Some(&other), &[], &host()),
        Err(PublisherError::SameVersionOtherFiles { .. })
    ));
    // Revoked.
    let revoked = [RevokedRelease {
        id: id("deckpilot.docker"),
        versions: RevokedVersions::List(vec![v("0.1.1")]),
        reason: "test".into(),
    }];
    assert!(matches!(
        run(None, &revoked, &host()),
        Err(PublisherError::Revoked { .. })
    ));
    // Not compatible with an older plugin API.
    let old = HostEnvironment {
        plugin_api: ApiLevel::new(1, 8),
        ..host()
    };
    assert!(matches!(
        run(None, &[], &old),
        Err(PublisherError::Incompatible(_))
    ));
}

/// `release verify` (the files and the listing `artifacts.sh` reads): the
/// closed compiled gate refuses; the open one prints how it admitted the
/// release, and the unpacked files are the package's.
#[test]
fn release_verify_reports_the_admission_line() {
    let work = TempDir::new().unwrap();
    let a = ThrowawayKey::generate();
    let b = ThrowawayKey::generate();
    let bytes = PluginDir::new("projects", "0.1.1", |_| {})
        .package(open())
        .unwrap();
    let claim = claim(&bytes, "deckpilot.projects", "0.1.1");
    let package = work.path().join("deckpilot.projects-0.1.1.dpplugin");
    std::fs::write(&package, &bytes).unwrap();
    for key in [&a, &b] {
        let file = signed(key, &bytes, &claim);
        std::fs::write(
            work.path().join(format!(
                "deckpilot.projects-0.1.1.dpplugin.{}.minisig",
                key.key_id()
            )),
            file.bytes,
        )
        .unwrap();
    }
    let keys_path = work.path().join("keys.json");
    std::fs::write(&keys_path, serde_json::to_vec(&keys(&[&a, &b], 2)).unwrap()).unwrap();
    let unpack = work.path().join("unpacked");
    let request = VerifyRequest {
        package: &package,
        signatures_dir: None,
        keys: Some(&keys_path),
        claim: claim.clone(),
        host: host(),
        released_pins: None,
        revocations: None,
        audit: None,
        unpack: Some(&unpack),
        now: common::t0(),
    };
    if !privileged_policy::compiled_policy().unwrap().is_approved() {
        let error = verify_release_files(&request).unwrap_err().to_string();
        assert!(error.contains("refused"), "{error}");
        assert!(!unpack.exists());
    }
    let listing = verify_release_files_with_gate(&request, open()).unwrap();
    let lines: Vec<&str> = listing
        .lines()
        .filter(|l| l.starts_with("  privileged: "))
        .collect();
    assert_eq!(lines.len(), 1, "{listing}");
    assert!(
        lines[0].starts_with("  privileged: reviewed: deckpilot.projects 0.1.1"),
        "{listing}"
    );
    assert!(unpack.join("manifest.json").is_file());
}

/// No production code opens the gate. `PrivilegedGate::Open` holds a
/// `ReviewedGate` that only `privileged_policy.rs` can build (in
/// `compiled()`, and in the test-only `open_for_tests`); this lint is the
/// second line: in the production code of every Rust file of the
/// repository (test files, `#[cfg(test)]` and `#[cfg(any(test, …))]` items
/// left out) it refuses
///
/// * any use of `open_for_tests`, and any `ReviewedGate { policy … }`
///   outside `compiled()`;
/// * `use` of `PrivilegedGate::Open`, a glob of `PrivilegedGate` or of
///   `privileged_policy`, and a bare `Open(` in a file that imports it;
/// * `PrivilegedGate::Open(…)` that is not a pattern, also across lines;
/// * a call of `*_with_gate(…)` or `.with_gate(…)` whose arguments are
///   not `PrivilegedGate::compiled()` or `PrivilegedGate::Closed`, also
///   across lines.
#[test]
fn only_tests_construct_an_open_gate() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut offenders = Vec::new();
    for dir in ["crates", "apps", "tools", "plugins"] {
        for file in rust_files(&root.join(dir)) {
            let rel = file
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if is_test_file(&rel) {
                continue;
            }
            let text = std::fs::read_to_string(&file).unwrap();
            let policy_module = rel.ends_with("deckpilot-plugin-registry/src/privileged_policy.rs");
            for problem in gate_lint(&production_code(&text), policy_module) {
                offenders.push(format!("{rel}: {problem}"));
            }
        }
    }
    assert!(offenders.is_empty(), "{offenders:#?}");
}

/// The lint catches what it must, and lets the real readers pass.
#[test]
fn the_gate_lint_catches_every_way_to_open_the_gate() {
    let caught = |code: &str| !gate_lint(&production_code(code), false).is_empty();
    for bad in [
        "let g = PrivilegedGate::open_for_tests(&p);",
        "use deckpilot_plugin_registry::privileged_policy::PrivilegedGate::Open;",
        "use deckpilot_plugin_registry::privileged_policy::PrivilegedGate::{Closed, Open};",
        "use deckpilot_plugin_registry::privileged_policy::PrivilegedGate::*;",
        "use deckpilot_plugin_registry::privileged_policy::*;",
        "use deckpilot_plugin_registry::privileged_policy::PrivilegedGate::{\n    Open as O,\n};",
        "fn f() { let g = PrivilegedGate::Open(\n    r,\n); }",
        "fn f() { let g = PrivilegedGate\n    ::Open(r); }",
        "fn f() { verify_with_gate(&request, gate); }",
        "fn f() { package_with_gate(\n  dir,\n  module,\n  true,\n  other,\n); }",
        "fn f() { official.with_gate(g); }",
        "fn f() { let g = ReviewedGate { policy: p }; }",
    ] {
        assert!(caught(bad), "not caught: {bad}");
    }
    for good in [
        "fn f() { let PrivilegedGate::Open(reviewed) = gate else { return; }; }",
        "fn f() { match gate { PrivilegedGate::Open(r) => r, PrivilegedGate::Closed => x } }",
        "fn f() { package_with_gate(dir, module, true, PrivilegedGate::compiled()) }",
        "fn f() { preflight_with_gate(\n  p,\n  id,\n  v,\n  PrivilegedGate::Closed,\n) }",
        "pub fn verify_with_gate(request: &R, gate: PrivilegedGate<'_>) -> X { y }",
        "#[cfg(test)]\nfn t() { verify_with_gate(&r, PrivilegedGate::open_for_tests(&p)); }",
        "#[cfg(test)]\nmod tests { use super::*; fn t() { x.with_gate(PrivilegedGate::open_for_tests(&p)); } }",
        "// PrivilegedGate::open_for_tests(&p) in a comment",
    ] {
        assert!(!caught(good), "flagged: {good}");
    }
}

fn is_test_file(rel: &str) -> bool {
    rel.contains("/tests/")
        || rel.contains("/benches/")
        || rel.ends_with("/tests.rs")
        || rel.ends_with("_tests.rs")
}

/// The code of `text` outside comments and test-only items: a `#[cfg(…)]`
/// attribute that names `test` removes the item after it (up to its `;` or
/// its balanced `{…}`). Whitespace is collapsed to single spaces.
fn production_code(text: &str) -> String {
    let mut code = String::new();
    for line in text.lines() {
        let line = match line.find("//") {
            Some(at) if !line[..at].contains('"') => &line[..at],
            _ => line,
        };
        code.push_str(line);
        code.push(' ');
    }
    let bytes = code.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        if code[i..].starts_with("#[cfg(") {
            let end = code[i..].find(']').map_or(bytes.len(), |e| i + e + 1);
            let attribute = &code[i..end];
            if attribute.contains("test") {
                // Skip the item: to its `;` or its balanced braces.
                let mut j = end;
                let mut depth = 0usize;
                while j < bytes.len() {
                    match bytes[j] {
                        b'{' => depth += 1,
                        b'}' => {
                            depth = depth.saturating_sub(1);
                            if depth == 0 {
                                j += 1;
                                break;
                            }
                        }
                        b';' if depth == 0 => {
                            j += 1;
                            break;
                        }
                        _ => {}
                    }
                    j += 1;
                }
                i = j;
                continue;
            }
        }
        let c = code[i..].chars().next().unwrap();
        out.push(c);
        i += c.len_utf8();
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The arguments of the call whose `(` is at `open` in `code`.
fn arguments(code: &str, open: usize) -> &str {
    let mut depth = 0usize;
    for (k, c) in code[open..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return &code[open + 1..open + k];
                }
            }
            _ => {}
        }
    }
    &code[open..]
}

fn gate_lint(code: &str, policy_module: bool) -> Vec<String> {
    let mut problems = Vec::new();
    let tight = code.replace(" ::", "::").replace(":: ", "::");
    if tight.contains("PrivilegedGate::open_for_tests") && !policy_module {
        problems.push("uses PrivilegedGate::open_for_tests".to_owned());
    }
    let constructions = tight.matches("ReviewedGate { policy").count();
    if constructions > usize::from(policy_module) {
        problems.push("builds a ReviewedGate".to_owned());
    }
    for statement in tight.split(';') {
        let statement = statement.trim();
        let Some(path) = statement
            .strip_prefix("use ")
            .or_else(|| statement.strip_prefix("pub use "))
            .or_else(|| statement.strip_prefix("pub(crate) use "))
            .or_else(|| statement.strip_prefix("pub(super) use "))
        else {
            continue;
        };
        let path = path.replace(' ', "");
        if path.contains("PrivilegedGate::Open")
            || path.contains("PrivilegedGate::*")
            || path.contains("privileged_policy::*")
            || (path.contains("PrivilegedGate::{") && path.contains("Open"))
        {
            problems.push(format!("imports the open gate: use {path}"));
        }
    }
    // A bare `Open(` is only possible after such an import (reported
    // above); every qualified one must be a pattern.
    let mut from = 0;
    while let Some(at) = tight[from..].find("PrivilegedGate::Open(") {
        let start = from + at;
        let open = start + "PrivilegedGate::Open".len();
        let args = arguments(&tight, open);
        let after = tight[open + args.len() + 2..].trim_start();
        let before = tight[..start].trim_end();
        let pattern = before.ends_with("let")
            || after.starts_with("=>")
            || after.starts_with('|')
            || (after.starts_with('=') && !after.starts_with("=="));
        if !pattern {
            problems.push(format!("PrivilegedGate::Open({args}) is not a pattern"));
        }
        from = open;
    }
    for (needle, call) in [("_with_gate(", true), (".with_gate(", false)] {
        let mut from = 0;
        while let Some(at) = tight[from..].find(needle) {
            let start = from + at;
            let open = start + needle.len() - 1;
            let name_start = tight[..start]
                .rfind(|c: char| !(c.is_alphanumeric() || c == '_'))
                .map_or(0, |p| p + 1);
            let definition = tight[..name_start].trim_end().ends_with("fn");
            let args = arguments(&tight, open);
            let compiled_only = args.contains("PrivilegedGate::compiled()")
                || args
                    .trim()
                    .trim_end_matches(',')
                    .trim_end()
                    .ends_with("PrivilegedGate::Closed");
            if !definition && !compiled_only {
                let what = if call {
                    "a *_with_gate call"
                } else {
                    "a .with_gate call"
                };
                problems.push(format!("{what} with another gate: ({args})"));
            }
            from = open;
        }
    }
    problems
}

fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if path.is_dir() {
            if name != "target" && name != "node_modules" && !name.starts_with('.') {
                out.extend(rust_files(&path));
            }
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    out
}
