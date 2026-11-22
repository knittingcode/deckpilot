//! Official releases against a real plugin store in a temporary data
//! directory, with throwaway publisher keys made per test and the bundled
//! test fixture `plugins/first-party/test-fixtures/example-default` (a
//! minimal ABI 1 module stands in for its `plugin.wasm`).

use std::path::Path;
use std::sync::Arc;

use deckpilot_plugin_bundled::{BUNDLED_DIR, BundleClass, Pins};
use deckpilot_plugin_manifest::{ApiLevel, Platform};
use deckpilot_plugin_package::{MANIFEST, PackageBuilder};
use deckpilot_plugin_registry::RevokedVersions;
use deckpilot_plugin_registry::publish::{KeyRole, SigningKey, generate_key};
use deckpilot_plugin_store::{
    Consent, CrashOnce, Failpoint, GrantKey, GrantSet, GrantStatus, ManagerOptions, Operation,
    PluginManager, PluginState, StoreKey, UpdateOutcome,
};
use serde_json::{Value, json};

use super::super::bundled::{self, Bundle, Seeded};
use super::*;

const ID: &str = "deckpilot.example-default";
const FIXTURE: &str = "../../plugins/first-party/test-fixtures/example-default";
const REGISTRY: &str = "https://registry.example/";

fn id() -> PluginId {
    PluginId::parse(ID).unwrap()
}

fn v(text: &str) -> Version {
    Version::parse(text).unwrap()
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(FIXTURE)
            .join(name),
    )
    .unwrap()
}

fn host() -> HostEnvironment {
    HostEnvironment {
        app_version: v("0.10.0"),
        plugin_api: ApiLevel::new(1, 9),
        platform: Platform::current().unwrap_or(Platform::Macos),
    }
}

fn notifications() -> Value {
    json!([{ "kind": "notifications", "justification": "Test fixture: low tier." }])
}

fn with_network() -> Value {
    json!([
        { "kind": "notifications", "justification": "Test fixture: low tier." },
        {
            "kind": "network",
            "hosts": ["api.example.com"],
            "justification": "Test fixture: medium tier, needs consent."
        }
    ])
}

fn with_setting(manifest: &mut Value) {
    manifest["contributes"]["settings"] = json!({
        "portabilityVersion": 1,
        "fields": [{
            "key": "greeting",
            "label": "Greeting",
            "description": "Test fixture: a setting kept through the transition.",
            "required": false,
            "kind": { "type": "text", "maxLength": 32 },
            "portability": "portable"
        }]
    });
}

/// The plugin's files at `version` (the module carries the version, so
/// every version has other bytes; `note` changes them within a version).
fn files(version: &str, permissions: &Value, note: &str, edit: impl FnOnce(&mut Value)) -> Files {
    let mut manifest: Value = serde_json::from_slice(&fixture("manifest.json")).unwrap();
    manifest["version"] = version.into();
    manifest["permissions"] = permissions.clone();
    with_setting(&mut manifest);
    edit(&mut manifest);
    let mut files = Files::new();
    files.insert(
        MANIFEST.into(),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    );
    files.insert(
        "plugin.wasm".into(),
        deckpilot_plugin_package::wasm::minimal_abi_module_with_note(&format!("{version}{note}")),
    );
    files.insert("LICENSE".into(), fixture("LICENSE"));
    files.insert("assets/icon-128.png".into(), fixture("assets/icon-128.png"));
    files
}

fn package_of(files: &Files) -> Vec<u8> {
    let mut builder = PackageBuilder::new();
    builder.first_party();
    for (path, bytes) in files {
        builder.add(path, bytes.clone()).unwrap();
    }
    builder.build().unwrap().bytes
}

/// A `.dpplugin` written directly (no content checks): for manifests the
/// builder refuses, such as the privileged kinds.
fn raw_package(files: &Files) -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    for (path, bytes) in files {
        let mut header = tar::Header::new_ustar();
        header.set_path(path).unwrap();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(0);
        header.set_cksum();
        tar.append(&header, bytes.as_slice()).unwrap();
    }
    let tar = tar.into_inner().unwrap();
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&tar).unwrap();
    gz.finish().unwrap()
}

/// What a verified registry chain would pin for `package`.
fn expected_for(package: &[u8], version: &str) -> ExpectedPackage {
    let files = read_archive(package, &Limits::default()).unwrap();
    let manifest: Value = serde_json::from_slice(&files[MANIFEST]).unwrap();
    ExpectedPackage {
        id: id(),
        version: v(version),
        package: digest(package),
        manifest_sha256: sha256(&files[MANIFEST]),
        permissions: Some(manifest["permissions"].as_array().unwrap().clone()),
        first_party: true,
    }
}

struct Key {
    signing: SigningKey,
}

impl Key {
    fn new(dir: &Path, name: &str) -> Self {
        let path = dir.join(name);
        generate_key(KeyRole::Publisher, &path, None).unwrap();
        Self {
            signing: SigningKey::load(&path, None).unwrap(),
        }
    }

    fn public(&self) -> String {
        self.signing.public().to_owned()
    }

    fn key_id(&self) -> String {
        publisher::key_id(&self.public()).unwrap()
    }

    /// The `<package>.<KEYID>.minisig` file for `package` as `id` `version`.
    fn sign(&self, package: &[u8], version: &str) -> SignatureFile {
        let comment = publisher::release_comment(&id(), &v(version), &digest(package));
        SignatureFile {
            key_id: self.key_id(),
            bytes: self.signing.sign(package, &comment).unwrap().into_bytes(),
        }
    }
}

fn keys(current: &[&Key], threshold: u64, revoked: &[&Key]) -> PublisherKeys {
    PublisherKeys {
        publisher_keys_version: 1,
        publisher: "deckpilot".into(),
        keys: current.iter().map(|k| k.public()).collect(),
        threshold,
        revoked_keys: revoked.iter().map(|k| k.public()).collect(),
    }
}

/// A temporary data directory, application resources and keys.
struct Env {
    root: tempfile::TempDir,
    key: Key,
}

impl Env {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let key = Key::new(root.path(), "publisher.key");
        Self { root, key }
    }

    fn data(&self) -> PathBuf {
        self.root.path().join("data")
    }

    fn state(&self) -> PathBuf {
        self.data().join(STATE_DIR)
    }

    fn open(&self) -> PluginManager {
        self.open_with(ManagerOptions::default())
    }

    fn open_with(&self, options: ManagerOptions) -> PluginManager {
        PluginManager::open(&self.data(), &StoreKey::from_bytes([0x42; 32]), options).unwrap()
    }

    fn official(&self) -> Official {
        self.official_with(keys(&[&self.key], 1, &[]), Vec::new())
    }

    fn official_with(&self, keys: PublisherKeys, revoked: Vec<RevokedRelease>) -> Official {
        self.official_compiled(
            keys,
            Compiled {
                revoked,
                ..Compiled::default()
            },
        )
    }

    fn official_compiled(&self, keys: PublisherKeys, compiled: Compiled) -> Official {
        Official::with(&self.state(), Ok(keys), Ok(compiled))
    }

    /// The bundle at `version` with `permissions`, pinned to its files.
    fn bundle(&self, version: &str, permissions: &Value) -> Bundle {
        let dir = self.root.path().join("resources").join(BUNDLED_DIR);
        let plugin = dir.join(ID);
        let _ = std::fs::remove_dir_all(&plugin);
        for (path, bytes) in files(version, permissions, "", |_| {}) {
            let target = plugin.join(&path);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, bytes).unwrap();
        }
        let pins = Pins::compute(&dir, &[(id(), v(version), BundleClass::Default)]).unwrap();
        Bundle {
            dir: Some(dir),
            pins,
            trust: Ok(()),
        }
    }

    fn audit(&self) -> Vec<ChainedLine> {
        std::fs::read_to_string(self.state().join(AUDIT_FILE))
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }
}

/// One release: its package, what the registry pins and its signature.
struct Release {
    package: Vec<u8>,
    expected: ExpectedPackage,
    signature: SignatureFile,
}

impl Release {
    fn new(env: &Env, version: &str, permissions: &Value) -> Self {
        Self::with(env, version, permissions, "", |_| {})
    }

    fn with(
        env: &Env,
        version: &str,
        permissions: &Value,
        note: &str,
        edit: impl FnOnce(&mut Value),
    ) -> Self {
        let package = package_of(&files(version, permissions, note, edit));
        let expected = expected_for(&package, version);
        let signature = env.key.sign(&package, version);
        Self {
            package,
            expected,
            signature,
        }
    }

    fn delivery<'a>(
        &'a self,
        bundle: Option<&'a Bundle>,
        host: &'a HostEnvironment,
    ) -> Delivery<'a> {
        Delivery {
            expected: &self.expected,
            package: &self.package,
            signatures: std::slice::from_ref(&self.signature),
            pin: bundle.and_then(|b| b.pins.get(&id())),
            host: Some(host),
            now: Utc::now(),
        }
    }
}

fn registry_source() -> InstallSource {
    InstallSource::Registry {
        registry: REGISTRY.into(),
    }
}

fn refusal(result: Result<VerifiedPackage, Refusal>) -> Refusal {
    match result {
        Ok(package) => panic!("verified {}", package.manifest().version),
        Err(refusal) => refusal,
    }
}

/// The bundled copy seeded (enabled where plugins run), with a setting
/// and a credential the transition must keep.
fn seeded(env: &Env, bundle: &Bundle) -> PluginManager {
    let mut manager = env.open();
    bundled::seed(&mut manager, bundle);
    manager
        .set_settings(
            &id(),
            &std::iter::once(("greeting".to_owned(), json!("Yo"))).collect(),
        )
        .unwrap();
    manager
}

fn state_of(manager: &PluginManager) -> PluginRecord {
    manager.store().plugin(&id()).unwrap().unwrap()
}

/// A registry that revokes nothing.
struct NoVerdicts;

impl VerdictSource for NoVerdicts {
    fn verdict(&self, _id: &PluginId, _version: &Version) -> InstalledVerdict {
        InstalledVerdict::Run
    }
}

// ------------------------------------------------------------ the keys

#[test]
fn this_build_pins_the_production_keys_and_counts_no_other() {
    // The two production publisher keys are pinned (threshold 2): this
    // build can verify an official release before any download, but only
    // one both production keys signed. A release signed by any other key
    // is refused and audited, and raises no floor.
    let env = Env::new();
    let official = Official::open(&env.state());
    official.ready().unwrap();
    let (ids, threshold) = official.signature_key_ids().unwrap();
    assert_eq!(ids, publisher::PRODUCTION_KEY_IDS);
    assert_eq!(threshold, 2);
    let release = Release::new(&env, "0.1.1", &notifications());
    let host = host();
    let refused = refusal(official.verify(&release.delivery(None, &host)));
    assert!(
        matches!(
            &refused,
            Refusal::Publisher(PublisherError::BelowThreshold {
                valid: 0,
                threshold: 2,
                ..
            })
        ),
        "{refused:?}"
    );
    let audit = env.audit();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].record.outcome, publisher::AuditOutcome::Refused);
    assert!(official.floor(&id()).is_none());
    // An empty key set is still refused before any download (fail closed).
    let empty = PublisherKeys {
        publisher_keys_version: publisher::KEYS_VERSION,
        publisher: "deckpilot".into(),
        keys: Vec::new(),
        threshold: 1,
        revoked_keys: Vec::new(),
    };
    let unprovisioned = env.official_with(empty, Vec::new());
    assert!(
        unprovisioned
            .ready()
            .unwrap_err()
            .contains("no first-party publisher key"),
        "{:?}",
        unprovisioned.ready()
    );
    let refused = refusal(unprovisioned.verify(&release.delivery(None, &host)));
    assert_eq!(refused, Refusal::Publisher(PublisherError::NotProvisioned));
}

#[test]
fn a_release_verifies_and_raises_the_floor() {
    let env = Env::new();
    let official = env.official();
    official.ready().unwrap();
    let release = Release::new(&env, "0.1.1", &notifications());
    let host = host();
    let package = official.verify(&release.delivery(None, &host)).unwrap();
    assert_eq!(package.manifest().version, v("0.1.1"));
    let floor = official.floor(&id()).unwrap();
    assert_eq!(floor.version, v("0.1.1"));
    assert_eq!(floor.package, digest(&release.package));
    assert_eq!(floor.signers.len(), 1);
    // Persisted: a new agent sees it.
    assert_eq!(env.official().floor(&id()), Some(floor));
    let audit = env.audit();
    assert_eq!(audit[0].record.outcome, publisher::AuditOutcome::Verified);
    assert_eq!(audit[0].prev, GENESIS);
}

// ----------------------------------------------------------- tampering

#[test]
fn tampered_packages_and_swapped_signatures_are_refused() {
    let env = Env::new();
    let official = env.official();
    let host = host();
    let genuine = Release::new(&env, "0.1.1", &notifications());

    // A flipped byte (the registry's pin still names the genuine bytes).
    let mut tampered = Release::new(&env, "0.1.1", &notifications());
    let last = tampered.package.len() - 1;
    tampered.package[last] ^= 1;
    assert!(matches!(
        refusal(official.verify(&tampered.delivery(None, &host))),
        Refusal::Publisher(PublisherError::DigestMismatch { .. })
    ));

    // A compromised registry pins other bytes (its own package): the
    // publisher signature of the genuine release does not cover them.
    let mut swapped = Release::with(&env, "0.1.1", &notifications(), "-evil", |_| {});
    swapped.signature = genuine.signature.clone();
    assert!(matches!(
        refusal(official.verify(&swapped.delivery(None, &host))),
        Refusal::Publisher(PublisherError::BelowThreshold { valid: 0, .. })
    ));

    // The signature of another version, replayed.
    let other = Release::new(&env, "0.1.2", &notifications());
    let mut replayed = Release::new(&env, "0.1.1", &notifications());
    replayed.signature = other.signature.clone();
    assert!(matches!(
        refusal(official.verify(&replayed.delivery(None, &host))),
        Refusal::Publisher(PublisherError::BelowThreshold { valid: 0, .. })
    ));

    // A key this agent does not pin.
    let stranger = Key::new(env.root.path(), "stranger.key");
    let mut foreign = Release::new(&env, "0.1.1", &notifications());
    foreign.signature = stranger.sign(&foreign.package, "0.1.1");
    assert!(matches!(
        refusal(official.verify(&foreign.delivery(None, &host))),
        Refusal::Publisher(PublisherError::BelowThreshold { valid: 0, .. })
    ));

    // Garbage instead of a signature.
    let mut garbage = Release::new(&env, "0.1.1", &notifications());
    garbage.signature.bytes = b"not base64 !".to_vec();
    assert!(matches!(
        refusal(official.verify(&garbage.delivery(None, &host))),
        Refusal::Publisher(PublisherError::BelowThreshold { valid: 0, .. })
    ));

    // The registry's permissions (the consent dialog) must be the
    // manifest's.
    let mut relabeled = Release::new(&env, "0.1.1", &notifications());
    relabeled.expected.permissions = Some(Vec::new());
    assert!(matches!(
        refusal(official.verify(&relabeled.delivery(None, &host))),
        Refusal::Package(_)
    ));

    // A registry that does not authorize the namespace.
    let mut unauthorized = Release::new(&env, "0.1.1", &notifications());
    unauthorized.expected.first_party = false;
    assert!(matches!(
        refusal(official.verify(&unauthorized.delivery(None, &host))),
        Refusal::Package(_)
    ));

    // Nothing above moved the floor; every refusal is audited.
    assert!(official.floor(&id()).is_none());
    let audit = env.audit();
    assert_eq!(audit.len(), 7);
    assert!(
        audit
            .iter()
            .all(|l| l.record.outcome == publisher::AuditOutcome::Refused)
    );
}

#[test]
fn the_audit_log_is_hash_chained() {
    let env = Env::new();
    let official = env.official();
    let host = host();
    for version in ["0.1.1", "0.1.2", "0.1.0"] {
        let release = Release::new(&env, version, &notifications());
        let _ = official.verify(&release.delivery(None, &host));
    }
    let path = env.state().join(AUDIT_FILE);
    let text = std::fs::read_to_string(&path).unwrap();
    check_audit_chain(&text, GENESIS).unwrap();
    // Removing a line, or editing one, breaks the chain.
    let lines: Vec<&str> = text.lines().collect();
    let removed = format!("{}\n{}\n", lines[0], lines[2]);
    assert_eq!(check_audit_chain(&removed, GENESIS), Err(2));
    let edited = text.replacen("\"verified\"", "\"refused\"", 1);
    assert_eq!(check_audit_chain(&edited, GENESIS), Err(2));
    // The last line chains to nothing yet: its hash is in the agent's log
    // (`official audit line`), the anchor to compare a log against.
}

// ------------------------------------------------------------ rollback

#[test]
fn never_below_the_floor_and_the_same_version_only_with_the_same_files() {
    let env = Env::new();
    let official = env.official();
    let host = host();
    let release = Release::new(&env, "0.1.2", &notifications());
    official.verify(&release.delivery(None, &host)).unwrap();

    let older = Release::new(&env, "0.1.1", &notifications());
    assert!(matches!(
        refusal(official.verify(&older.delivery(None, &host))),
        Refusal::Publisher(PublisherError::Rollback { .. })
    ));
    let other_files = Release::with(&env, "0.1.2", &notifications(), "-rebuilt", |_| {});
    assert!(matches!(
        refusal(official.verify(&other_files.delivery(None, &host))),
        Refusal::Publisher(PublisherError::SameVersionOtherFiles { .. })
    ));
    // The same release again (a retried install) verifies.
    official.verify(&release.delivery(None, &host)).unwrap();
    // A new agent (a restart) keeps the floor.
    let restarted = env.official();
    assert!(matches!(
        refusal(restarted.verify(&older.delivery(None, &host))),
        Refusal::Publisher(PublisherError::Rollback { .. })
    ));
}

#[test]
fn the_bundled_copy_is_a_floor_too() {
    let env = Env::new();
    let official = env.official();
    let host = host();
    let bundle = env.bundle("0.2.0", &notifications());
    let older = Release::new(&env, "0.1.9", &notifications());
    assert!(matches!(
        refusal(official.verify(&older.delivery(Some(&bundle), &host))),
        Refusal::Publisher(PublisherError::Rollback { .. })
    ));
    // The bundled version with other files.
    let same = Release::with(&env, "0.2.0", &notifications(), "-artifact", |_| {});
    assert!(matches!(
        refusal(official.verify(&same.delivery(Some(&bundle), &host))),
        Refusal::Publisher(PublisherError::SameVersionOtherFiles { .. })
    ));
    official
        .verify(&Release::new(&env, "0.2.1", &notifications()).delivery(Some(&bundle), &host))
        .unwrap();
}

#[test]
fn unreadable_floors_refuse_and_are_never_reset() {
    let env = Env::new();
    std::fs::create_dir_all(env.state()).unwrap();
    std::fs::write(env.state().join(FLOORS_FILE), b"{ not json").unwrap();
    let official = env.official();
    assert!(official.ready().is_err());
    let host = host();
    let release = Release::new(&env, "0.1.1", &notifications());
    assert!(matches!(
        refusal(official.verify(&release.delivery(None, &host))),
        Refusal::Unavailable(_)
    ));
    assert_eq!(
        std::fs::read(env.state().join(FLOORS_FILE)).unwrap(),
        b"{ not json"
    );
}

// ---------------------------------------------------------- revocation

#[test]
fn revoked_keys_and_releases_are_refused() {
    let env = Env::new();
    let host = host();
    let release = Release::new(&env, "0.1.1", &notifications());

    let revoked_key = env.official_with(keys(&[], 1, &[&env.key]), Vec::new());
    // A key set whose only key is revoked is not provisioned at all.
    assert!(revoked_key.ready().is_err());
    let second = Key::new(env.root.path(), "second.key");
    let revoked_key = env.official_with(keys(&[&second], 1, &[&env.key]), Vec::new());
    assert!(matches!(
        refusal(revoked_key.verify(&release.delivery(None, &host))),
        Refusal::Publisher(PublisherError::RevokedKey { .. })
    ));

    let lock = vec![RevokedRelease {
        id: id(),
        versions: RevokedVersions::List(vec![v("0.1.1")]),
        reason: "test".into(),
    }];
    let revoked_release = env.official_with(keys(&[&env.key], 1, &[]), lock);
    let refused = refusal(revoked_release.verify(&release.delivery(None, &host)));
    assert!(matches!(
        refused,
        Refusal::Publisher(PublisherError::Revoked { .. })
    ));
    assert_eq!(
        refused.to_ipc(&id()).reason.as_deref(),
        Some(reason::REVOKED)
    );
}

#[test]
fn an_installed_release_is_revoked_by_a_later_agent() {
    let env = Env::new();
    let host = host();
    let official = env.official();
    let release = Release::new(&env, "0.1.1", &notifications());
    let package = official.verify(&release.delivery(None, &host)).unwrap();
    let mut manager = env.open();
    manager
        .install(&package, &registry_source(), &Consent::of(vec![], "user"))
        .unwrap();
    let records = manager.store().plugins().unwrap();
    assert!(official.revoked_installed(&records).is_empty());

    // An agent whose pinned keys revoke the key that signed it.
    let second = Key::new(env.root.path(), "second.key");
    let later = env.official_with(keys(&[&second], 1, &[&env.key]), Vec::new());
    let revoked = later.revoked_installed(&records);
    assert_eq!(revoked.len(), 1, "{revoked:?}");
    let verdicts = Verdicts {
        official: &revoked,
        registry: &NoVerdicts,
    };
    let changes = manager.apply_verdicts(&verdicts).unwrap();
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert_eq!(state_of(&manager).state, PluginState::Quarantined);

    // An agent whose lock revokes the release.
    let lock = vec![RevokedRelease {
        id: id(),
        versions: RevokedVersions::All,
        reason: "withdrawn".into(),
    }];
    let locked = env.official_with(keys(&[&env.key], 1, &[]), lock);
    assert_eq!(locked.revoked_installed(&records).len(), 1);
    // A bundled copy is never affected (the pins vouch for it).
    let other = Env::new();
    let bundle = other.bundle("0.1.0", &notifications());
    let bundled_manager = seeded(&other, &bundle);
    let lock = vec![RevokedRelease {
        id: id(),
        versions: RevokedVersions::All,
        reason: "withdrawn".into(),
    }];
    let locked = other.official_with(keys(&[&other.key], 1, &[]), lock);
    assert!(
        locked
            .revoked_installed(&bundled_manager.store().plugins().unwrap())
            .is_empty()
    );
}

// ------------------------------------------------------- compatibility

#[test]
fn an_incompatible_release_is_refused() {
    let env = Env::new();
    let official = env.official();
    let release = Release::with(&env, "0.1.1", &notifications(), "", |m| {
        m["engines"]["deckpilot"] = json!(">=9.0.0");
    });
    let host = host();
    let refused = refusal(official.verify(&release.delivery(None, &host)));
    assert!(matches!(
        refused,
        Refusal::Publisher(PublisherError::Incompatible(_))
    ));
    assert_eq!(
        refused.to_ipc(&id()).reason.as_deref(),
        Some(reason::INCOMPATIBLE)
    );
    let old_api = HostEnvironment {
        plugin_api: ApiLevel::new(1, 0),
        ..host.clone()
    };
    let newer_api = Release::with(&env, "0.1.1", &notifications(), "", |m| {
        m["engines"]["pluginApi"] = json!("^1.9");
    });
    assert!(matches!(
        refusal(official.verify(&newer_api.delivery(None, &old_api))),
        Refusal::Publisher(PublisherError::Incompatible(_))
    ));
    // No known host: refused, never checked without one.
    let mut unknown = release.delivery(None, &host);
    unknown.host = None;
    assert!(matches!(
        refusal(official.verify(&unknown)),
        Refusal::Unavailable(_)
    ));
}

// ------------------------------------------------- threshold 2 (multisig)

/// `release` delivered with exactly `signatures`.
fn delivery_with<'a>(
    release: &'a Release,
    signatures: &'a [SignatureFile],
    host: &'a HostEnvironment,
) -> Delivery<'a> {
    Delivery {
        signatures,
        ..release.delivery(None, host)
    }
}

#[test]
#[allow(clippy::too_many_lines)] // one flow, step by step
fn threshold_two_needs_two_distinct_pinned_keys() {
    let env = Env::new();
    let second = Key::new(env.root.path(), "second.key");
    let stranger = Key::new(env.root.path(), "stranger.key");
    let official = env.official_with(keys(&[&env.key, &second], 2, &[]), Vec::new());
    assert_eq!(
        official.signature_key_ids().unwrap(),
        (vec![env.key.key_id(), second.key_id()], 2)
    );
    let host = host();
    let release = Release::new(&env, "0.1.1", &notifications());
    let by_first = env.key.sign(&release.package, "0.1.1");
    let by_second = second.sign(&release.package, "0.1.1");
    let below = |r: Result<VerifiedPackage, Refusal>, n: usize| {
        matches!(
            refusal(r),
            Refusal::Publisher(PublisherError::BelowThreshold { valid, threshold: 2, .. })
                if valid == n
        )
    };

    // One valid signature, or a missing second file.
    assert!(below(
        official.verify(&delivery_with(
            &release,
            std::slice::from_ref(&by_first),
            &host
        )),
        1
    ));
    assert!(below(
        official.verify(&delivery_with(&release, &[], &host)),
        0
    ));
    // The same signature under the second key's name.
    let renamed = SignatureFile {
        key_id: second.key_id(),
        ..by_first.clone()
    };
    assert!(below(
        official.verify(&delivery_with(
            &release,
            &[by_first.clone(), renamed],
            &host
        )),
        1
    ));
    // The same key twice.
    assert!(below(
        official.verify(&delivery_with(
            &release,
            &[by_first.clone(), by_first.clone()],
            &host
        )),
        1
    ));
    // A key this agent does not pin, under its own name or a pinned one's.
    let foreign = stranger.sign(&release.package, "0.1.1");
    let disguised = SignatureFile {
        key_id: second.key_id(),
        ..foreign.clone()
    };
    for other in [foreign, disguised] {
        assert!(below(
            official.verify(&delivery_with(&release, &[by_first.clone(), other], &host)),
            1
        ));
    }
    // Another version's or another package's signatures by both keys.
    let other_version = Release::new(&env, "0.1.2", &notifications());
    let replayed = [
        env.key.sign(&other_version.package, "0.1.2"),
        second.sign(&other_version.package, "0.1.2"),
    ];
    assert!(below(
        official.verify(&delivery_with(&release, &replayed, &host)),
        0
    ));
    // A compromised registry pins other bytes (its own package) under the
    // genuine two signatures.
    let evil = Release::with(&env, "0.1.1", &notifications(), "-evil", |_| {});
    let genuine = [by_first.clone(), by_second.clone()];
    assert!(below(
        official.verify(&delivery_with(&evil, &genuine, &host)),
        0
    ));
    // ... or rewrites the permissions the consent dialog shows, or the
    // manifest hash: the registry's pins are still checked.
    let mut relabeled = Release::new(&env, "0.1.1", &notifications());
    relabeled.expected.permissions = Some(Vec::new());
    assert!(matches!(
        refusal(official.verify(&delivery_with(&relabeled, &genuine, &host))),
        Refusal::Package(_)
    ));
    let mut other_manifest = Release::new(&env, "0.1.1", &notifications());
    other_manifest.expected.manifest_sha256 = sha256(b"another manifest");
    assert!(matches!(
        refusal(official.verify(&delivery_with(&other_manifest, &genuine, &host))),
        Refusal::Package(_)
    ));
    assert!(official.floor(&id()).is_none());

    // Both keys: verified, both recorded as signers.
    official
        .verify(&delivery_with(&release, &genuine, &host))
        .unwrap();
    let floor = official.floor(&id()).unwrap();
    assert_eq!(floor.version, v("0.1.1"));
    let mut signers = floor.signers.clone();
    signers.sort();
    let mut expected = vec![
        publisher::fingerprint(&env.key.public()).unwrap(),
        publisher::fingerprint(&second.public()).unwrap(),
    ];
    expected.sort();
    assert_eq!(signers, expected);

    // A newer release, then a downgrade signed by both keys: refused.
    let newer = Release::new(&env, "0.1.2", &notifications());
    let both_newer = [
        env.key.sign(&newer.package, "0.1.2"),
        second.sign(&newer.package, "0.1.2"),
    ];
    official
        .verify(&delivery_with(&newer, &both_newer, &host))
        .unwrap();
    assert!(matches!(
        refusal(official.verify(&delivery_with(&release, &genuine, &host))),
        Refusal::Publisher(PublisherError::Rollback { .. })
    ));
}

#[test]
fn a_revoked_key_never_counts_with_threshold_two() {
    let env = Env::new();
    let second = Key::new(env.root.path(), "second.key");
    let old = Key::new(env.root.path(), "old.key");
    let host = host();
    let release = Release::new(&env, "0.1.1", &notifications());
    let by_first = env.key.sign(&release.package, "0.1.1");
    let by_second = second.sign(&release.package, "0.1.1");
    let by_old = old.sign(&release.package, "0.1.1");
    // The revoked key is not fetched at all.
    let official = env.official_with(keys(&[&env.key, &second], 2, &[&old]), Vec::new());
    assert!(
        !official
            .signature_key_ids()
            .unwrap()
            .0
            .contains(&old.key_id())
    );
    // Revoked plus one valid key: refused (and the revoked one never counts).
    assert!(matches!(
        refusal(official.verify(&delivery_with(
            &release,
            &[by_first.clone(), by_old.clone()],
            &host
        ))),
        Refusal::Publisher(PublisherError::RevokedKey { .. })
    ));
    // Revoked plus both valid keys, if it were presented: refused.
    assert!(matches!(
        refusal(official.verify(&delivery_with(
            &release,
            &[by_first.clone(), by_second.clone(), by_old],
            &host
        ))),
        Refusal::Publisher(PublisherError::RevokedKey { .. })
    ));
    // The two current keys alone: verified.
    official
        .verify(&delivery_with(&release, &[by_first, by_second], &host))
        .unwrap();
}

// ---------------------------------------------------------- transition

/// Bundled 0.1.0 → official 0.1.1 through the store's update: the id, the
/// setting and the grants stay; the copy becomes registry-managed.
#[test]
fn the_transition_keeps_the_configuration_and_the_grants() {
    let env = Env::new();
    let bundle = env.bundle("0.1.0", &notifications());
    let mut manager = seeded(&env, &bundle);
    let before = state_of(&manager);
    let official = env.official();
    let host = host();
    let release = Release::new(&env, "0.1.1", &notifications());
    let package = official
        .verify(&release.delivery(Some(&bundle), &host))
        .unwrap();
    check_transition(&before.active().unwrap().manifest, package.manifest()).unwrap();
    let outcome = manager.update(&package, &registry_source()).unwrap();
    assert!(
        matches!(outcome, UpdateOutcome::Applied { .. }),
        "{outcome:?}"
    );
    let after = state_of(&manager);
    assert_eq!(after.source, registry_source());
    assert_eq!(after.state, before.state);
    assert_eq!(after.active().unwrap().version, v("0.1.1"));
    assert_eq!(
        manager.store().settings(&id()).unwrap()["greeting"],
        json!("Yo")
    );
    let grants = manager.store().grants(&id()).unwrap();
    assert_eq!(
        grants.iter().map(|g| g.key.to_string()).collect::<Vec<_>>(),
        ["notifications"]
    );
    assert!(grants.iter().all(|g| g.status == GrantStatus::Granted));

    // Idempotent: the same release again changes nothing.
    official
        .verify(&release.delivery(Some(&bundle), &host))
        .unwrap();
    assert!(manager.update(&package, &registry_source()).is_err());
    assert_eq!(state_of(&manager), after);

    // Offline start (no registry): seeding leaves the managed copy alone,
    // and it stays as it was.
    let report = bundled::seed(&mut manager, &bundle);
    assert!(
        matches!(&report[0].1, Seeded::Skipped(why) if why.contains("registry")),
        "{report:?}"
    );
    drop(manager);
    let mut reopened = env.open();
    reopened.recover().unwrap();
    bundled::seed(&mut reopened, &bundle);
    assert_eq!(state_of(&reopened), after);
}

#[test]
fn new_privileges_wait_for_consent_and_the_bundled_copy_keeps_running() {
    let env = Env::new();
    let bundle = env.bundle("0.1.0", &notifications());
    let mut manager = seeded(&env, &bundle);
    let before = state_of(&manager);
    let official = env.official();
    let host = host();
    let release = Release::new(&env, "0.1.1", &with_network());
    let package = official
        .verify(&release.delivery(Some(&bundle), &host))
        .unwrap();
    let outcome = manager.update(&package, &registry_source()).unwrap();
    let UpdateOutcome::AwaitingConsent { diff, .. } = outcome else {
        panic!("{outcome:?}")
    };
    assert!(
        diff.escalations()
            .iter()
            .any(|key| key.to_string().starts_with("network")),
        "{diff:?}"
    );
    let staged = state_of(&manager);
    assert_eq!(staged.active().unwrap().version, v("0.1.0"));
    assert_eq!(staged.state, before.state);
    let granted: Vec<String> = manager
        .store()
        .grants(&id())
        .unwrap()
        .iter()
        .filter(|g| g.status == GrantStatus::Granted)
        .map(|g| g.key.to_string())
        .collect();
    assert_eq!(granted, ["notifications"], "nothing new is granted");
}

#[test]
fn a_transition_that_drops_an_action_or_setting_is_refused() {
    let bundled: Manifest = {
        let f = files("0.1.0", &notifications(), "", |_| {});
        Manifest::from_slice(&f[MANIFEST]).unwrap()
    };
    let renamed: Manifest = {
        let f = files("0.1.1", &notifications(), "", |m| {
            m["contributes"]["actions"][0]["id"] = json!("hello");
        });
        Manifest::from_slice(&f[MANIFEST]).unwrap()
    };
    let problem = check_transition(&bundled, &renamed).unwrap_err();
    assert!(problem.contains("actions greet"), "{problem}");
    let no_setting: Manifest = {
        let f = files("0.1.1", &notifications(), "", |m| {
            m["contributes"].as_object_mut().unwrap().remove("settings");
        });
        Manifest::from_slice(&f[MANIFEST]).unwrap()
    };
    assert!(
        check_transition(&bundled, &no_setting)
            .unwrap_err()
            .contains("settings greeting")
    );
}

/// A crash in the middle of the transition: recovery leaves the bundled
/// copy active and intact, the floor stays raised, and the retry with the
/// same bytes completes; other bytes for that version stay refused.
#[test]
fn an_interrupted_transition_recovers_and_retries() {
    for point in [
        Failpoint::JournalWritten,
        Failpoint::FilesUnpacked,
        Failpoint::StepRecorded,
    ] {
        let env = Env::new();
        let bundle = env.bundle("0.1.0", &notifications());
        let manager = seeded(&env, &bundle);
        let before = state_of(&manager);
        drop(manager);
        let official = env.official();
        let host = host();
        let release = Release::new(&env, "0.1.1", &notifications());
        let package = official
            .verify(&release.delivery(Some(&bundle), &host))
            .unwrap();
        let crash = Arc::new(CrashOnce::new(Operation::Update, point));
        let mut crashing = env.open_with(ManagerOptions {
            failpoints: crash.clone(),
            ..ManagerOptions::default()
        });
        assert!(crashing.update(&package, &registry_source()).is_err());
        assert!(crash.fired(), "{point:?}");
        drop(crashing);

        let mut manager = env.open();
        manager.recover().unwrap();
        bundled::seed(&mut manager, &bundle);
        let recovered = state_of(&manager);
        assert_eq!(recovered.state, before.state, "{point:?}");
        let rolled_forward = recovered.source == registry_source();
        if rolled_forward {
            // Recovery completed the verified update: all of it.
            assert_eq!(recovered.active().unwrap().version, v("0.1.1"));
        } else {
            // Recovery undid it: the bundled copy, active and intact.
            assert_eq!(recovered.source, InstallSource::Bundled, "{point:?}");
            assert_eq!(recovered.active().unwrap().version, v("0.1.0"));
            deckpilot_plugin_bundled::verify_installed(
                &manager
                    .layout()
                    .resolve(&recovered.active().unwrap().install_dir),
                &bundle.pins.plugins[0],
            )
            .unwrap();
        }
        assert_eq!(
            manager.store().settings(&id()).unwrap()["greeting"],
            json!("Yo")
        );

        // The floor held through the crash.
        let restarted = env.official();
        assert_eq!(restarted.floor(&id()).unwrap().version, v("0.1.1"));
        let rebuilt = Release::with(&env, "0.1.1", &notifications(), "-rebuilt", |_| {});
        assert!(matches!(
            refusal(restarted.verify(&rebuilt.delivery(Some(&bundle), &host))),
            Refusal::Publisher(PublisherError::SameVersionOtherFiles { .. })
        ));
        let again = restarted
            .verify(&release.delivery(Some(&bundle), &host))
            .unwrap();
        if !rolled_forward {
            manager.update(&again, &registry_source()).unwrap();
        }
        assert_eq!(state_of(&manager).source, registry_source());
        assert_eq!(
            manager.store().settings(&id()).unwrap()["greeting"],
            json!("Yo")
        );
    }
}

#[test]
fn a_refused_release_leaves_the_bundled_copy_untouched() {
    let env = Env::new();
    let bundle = env.bundle("0.1.0", &notifications());
    let manager = seeded(&env, &bundle);
    let before = state_of(&manager);
    let official = env.official();
    let host = host();
    let mut tampered = Release::new(&env, "0.1.1", &notifications());
    let last = tampered.package.len() - 1;
    tampered.package[last] ^= 1;
    refusal(official.verify(&tampered.delivery(Some(&bundle), &host)));
    assert_eq!(state_of(&manager), before);
    assert!(official.floor(&id()).is_none());
}

// ------------------------------------------------- review fixes (2026-11-21)

/// Install `version` (signed by `key`) from the registry, or update to it.
fn install_or_update(manager: &mut PluginManager, official: &Official, release: &Release) {
    let host = host();
    let package = official.verify(&release.delivery(None, &host)).unwrap();
    if manager.store().plugin(&id()).unwrap().is_some() {
        let outcome = manager.update(&package, &registry_source()).unwrap();
        assert!(
            matches!(outcome, UpdateOutcome::Applied { .. }),
            "{outcome:?}"
        );
    } else {
        let all: Vec<GrantKey> = GrantSet::from_manifest(package.manifest())
            .keys()
            .cloned()
            .collect();
        manager
            .install(&package, &registry_source(), &Consent::of(all, "user"))
            .unwrap();
    }
}

/// Finding 1: every verified version keeps its signers, so a rollback to an
/// older version signed by a now-revoked key is refused, not only the
/// floor version.
#[test]
fn a_rollback_to_a_release_signed_by_a_revoked_key_is_refused() {
    let env = Env::new();
    let second = Key::new(env.root.path(), "second.key");
    let mut manager = env.open();
    let both = env.official_with(keys(&[&env.key, &second], 1, &[]), Vec::new());
    install_or_update(
        &mut manager,
        &both,
        &Release::new(&env, "0.1.1", &notifications()),
    );
    let mut newer = Release::new(&env, "0.1.2", &notifications());
    newer.signature = second.sign(&newer.package, "0.1.2");
    install_or_update(&mut manager, &both, &newer);
    assert_eq!(state_of(&manager).active().unwrap().version, v("0.1.2"));

    // A later agent revokes the key that signed 0.1.1 only.
    let later = env.official_with(keys(&[&second], 1, &[&env.key]), Vec::new());
    let records = manager.store().plugins().unwrap();
    let revoked = later.revoked_installed(&records);
    assert_eq!(
        revoked.keys().cloned().collect::<Vec<_>>(),
        [(id(), v("0.1.1"))],
        "{revoked:?}"
    );
    let verdicts = Verdicts {
        official: &revoked,
        registry: &NoVerdicts,
    };
    let refused = manager.rollback(&id(), &verdicts);
    assert!(refused.is_err(), "{refused:?}");
    assert_eq!(state_of(&manager).active().unwrap().version, v("0.1.2"));
    // Without the revocation the rollback goes through.
    let none = BTreeMap::new();
    manager
        .rollback(
            &id(),
            &Verdicts {
                official: &none,
                registry: &NoVerdicts,
            },
        )
        .unwrap();
    assert_eq!(state_of(&manager).active().unwrap().version, v("0.1.1"));
    // ... and then the active version is the revoked one.
    let records = manager.store().plugins().unwrap();
    assert!(
        later
            .revoked_installed(&records)
            .contains_key(&(id(), v("0.1.1")))
    );
}

/// Finding 2: the floors compiled into the agent (released pins for this
/// platform, the artifact lock) hold on a fresh install with nothing
/// recorded and no bundled copy.
#[test]
fn compiled_floors_hold_on_a_fresh_install() {
    let env = Env::new();
    let host = host();
    let shipped = files("0.3.0", &notifications(), "", |_| {});
    let released_pins = json!({
        "releasedPinsVersion": 1,
        "releases": [{
            "release": "0.9.0",
            "platform": "linux",
            "plugins": [{
                "id": ID,
                "version": "0.3.0",
                "files": shipped
                    .iter()
                    .map(|(path, bytes)| (path.clone(), json!(digest(bytes))))
                    .collect::<serde_json::Map<_, _>>()
            }]
        }, {
            "release": "0.9.0",
            "platform": "macos",
            "plugins": [{ "id": ID, "version": "9.0.0", "files": {} }]
        }]
    });
    let empty_lock = br#"{"artifactsVersion":1,"plugins":[],"revoked":[]}"#;
    let compiled = Compiled::parse(
        empty_lock,
        &serde_json::to_vec(&released_pins).unwrap(),
        Some(Platform::Linux),
    )
    .unwrap();
    // Other platforms do not count.
    assert_eq!(compiled.released[&id()].version, v("0.3.0"));
    let official = env.official_compiled(keys(&[&env.key], 1, &[]), compiled);
    assert!(matches!(
        refusal(
            official.verify(&Release::new(&env, "0.2.9", &notifications()).delivery(None, &host))
        ),
        Refusal::Publisher(PublisherError::Rollback { .. })
    ));
    assert!(matches!(
        refusal(official.verify(
            &Release::with(&env, "0.3.0", &notifications(), "-other", |_| {}).delivery(None, &host)
        )),
        Refusal::Publisher(PublisherError::SameVersionOtherFiles { .. })
    ));
    official
        .verify(&Release::new(&env, "0.3.1", &notifications()).delivery(None, &host))
        .unwrap();

    // The lock: never below its release, at its version only its package.
    let locked = Release::new(&env, "0.5.0", &notifications());
    let lock = json!({
        "artifactsVersion": 1,
        "plugins": [{
            "id": ID,
            "version": "0.5.0",
            "sha256": digest(&locked.package).sha256.to_string(),
            "size": locked.package.len(),
            "url": format!(
                "https://github.com/knittingcode/deckpilot-plugins/releases/download/\
                 {ID}--v0.5.0/{ID}-0.5.0.dpplugin"
            ),
            "keyIds": [env.key.key_id()]
        }],
        "revoked": []
    });
    let other = Env::new();
    let compiled = Compiled::parse(
        &serde_json::to_vec(&lock).unwrap(),
        br#"{"releasedPinsVersion":1,"releases":[]}"#,
        Some(Platform::Linux),
    )
    .unwrap();
    let official = other.official_compiled(keys(&[&env.key], 1, &[]), compiled);
    let older = Release::new(&env, "0.4.9", &notifications());
    assert!(matches!(
        refusal(official.verify(&older.delivery(None, &host))),
        Refusal::Publisher(PublisherError::Rollback { .. })
    ));
    let rebuilt = Release::with(&env, "0.5.0", &notifications(), "-rebuilt", |_| {});
    assert!(matches!(
        refusal(official.verify(&rebuilt.delivery(None, &host))),
        Refusal::Publisher(PublisherError::SameVersionOtherFiles { .. })
    ));
    official.verify(&locked.delivery(None, &host)).unwrap();
}

#[test]
fn this_builds_compiled_data_parses() {
    Compiled::this_build().unwrap();
    for platform in [Platform::Macos, Platform::Windows, Platform::Linux] {
        Compiled::parse(
            ARTIFACT_LOCK.as_bytes(),
            RELEASED_PINS.as_bytes(),
            Some(platform),
        )
        .unwrap();
    }
}

/// Finding 7: revocation fails closed. A registry copy of an official
/// plugin this agent has no record of verifying (the floors file deleted)
/// or whose record cannot be read is not run.
#[test]
fn revocation_fails_closed_without_the_record() {
    let env = Env::new();
    let mut manager = env.open();
    let official = env.official();
    install_or_update(
        &mut manager,
        &official,
        &Release::new(&env, "0.1.1", &notifications()),
    );
    let records = manager.store().plugins().unwrap();
    assert!(official.revoked_installed(&records).is_empty());

    std::fs::remove_file(env.state().join(FLOORS_FILE)).unwrap();
    let deleted = env.official();
    let revoked = deleted.revoked_installed(&records);
    assert!(
        revoked[&(id(), v("0.1.1"))].reason.contains("no record"),
        "{revoked:?}"
    );

    std::fs::write(env.state().join(FLOORS_FILE), b"garbage").unwrap();
    let unreadable = env.official();
    let revoked = unreadable.revoked_installed(&records);
    assert!(
        revoked[&(id(), v("0.1.1"))].reason.contains("cannot read"),
        "{revoked:?}"
    );
    // The quarantine follows.
    let changes = manager
        .apply_verdicts(&Verdicts {
            official: &revoked,
            registry: &NoVerdicts,
        })
        .unwrap();
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert_eq!(state_of(&manager).state, PluginState::Quarantined);
}

/// Finding 8: refusals before verification are audited, and the rotation
/// keeps several generations, chained across files.
#[test]
fn early_refusals_are_audited_and_rotation_keeps_generations() {
    let env = Env::new();
    let official = env.official().with_audit_limit(600);
    official.refused_before_verification(
        &id(),
        Some(&v("0.1.1")),
        None,
        "refused before any download: test",
        Utc::now(),
    );
    let audit = env.audit();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].record.outcome, publisher::AuditOutcome::Refused);
    assert!(
        audit[0]
            .record
            .reason
            .as_deref()
            .unwrap()
            .contains("before any download")
    );

    for n in 0..40 {
        official.refused_before_verification(&id(), None, None, &format!("line {n}"), Utc::now());
    }
    let generation = |n: usize| env.state().join(format!("{AUDIT_FILE}.{n}"));
    for n in 1..=AUDIT_GENERATIONS {
        assert!(generation(n).exists(), "generation {n}");
    }
    assert!(!generation(AUDIT_GENERATIONS + 1).exists());
    // Oldest kept generation first, the live file last: one chain.
    let mut prev: Option<String> = None;
    for path in (1..=AUDIT_GENERATIONS)
        .rev()
        .map(generation)
        .chain(std::iter::once(env.state().join(AUDIT_FILE)))
    {
        let text = std::fs::read_to_string(&path).unwrap();
        let first: ChainedLine = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        let start = prev.clone().unwrap_or(first.prev.clone());
        prev = Some(check_audit_chain(&text, &start).unwrap());
    }
}

// ------------------------------------------------- re-review (N2, floors)

fn make_writable_and_flip(path: &Path) {
    let mut permissions = std::fs::metadata(path).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    std::fs::set_permissions(path, permissions).unwrap();
    let mut bytes = std::fs::read(path).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    std::fs::write(path, bytes).unwrap();
}

/// N2: after the update from the bundle to a registry release, the
/// retained bundled version has no release record but is the bundled copy
/// this agent pins: it is trusted, and the rollback to it goes through. A
/// tampered retained copy is still refused, with an accurate reason.
#[test]
fn the_retained_bundled_copy_is_trusted_and_a_tampered_one_is_not() {
    for tampered in [false, true] {
        let env = Env::new();
        let bundle = env.bundle("0.1.0", &notifications());
        let mut manager = seeded(&env, &bundle);
        let official = env.official();
        let host = host();
        let release = Release::new(&env, "0.1.1", &notifications());
        let package = official
            .verify(&release.delivery(Some(&bundle), &host))
            .unwrap();
        manager.update(&package, &registry_source()).unwrap();
        let record = state_of(&manager);
        let previous = record.previous().unwrap().clone();
        assert_eq!(previous.version, v("0.1.0"));
        if tampered {
            make_writable_and_flip(
                &manager
                    .layout()
                    .resolve(&previous.install_dir)
                    .join("plugin.wasm"),
            );
        }
        let records = manager.store().plugins().unwrap();
        let withheld = official.withheld(&records, &|record, version| {
            bundled_match(&bundle, manager.layout(), record, version)
        });
        // Without the bundle to check against, it would have no record.
        let unchecked = official.revoked_installed(&records);
        assert_eq!(
            unchecked[&(id(), v("0.1.0"))].kind,
            WithheldKind::Unverified
        );
        let verdicts = Verdicts {
            official: &withheld,
            registry: &NoVerdicts,
        };
        if tampered {
            let why = &withheld[&(id(), v("0.1.0"))];
            assert_eq!(why.kind, WithheldKind::Unverified);
            assert_eq!(why.ipc_reason(), reason::INVALID_PACKAGE);
            assert!(
                why.reason.contains("not the bundled copy this agent pins"),
                "{why:?}"
            );
            assert!(!why.state_reason().starts_with("revoked"), "{why:?}");
            assert!(manager.rollback(&id(), &verdicts).is_err());
            assert_eq!(state_of(&manager).active().unwrap().version, v("0.1.1"));
        } else {
            assert!(withheld.is_empty(), "{withheld:?}");
            manager.rollback(&id(), &verdicts).unwrap();
            let rolled = state_of(&manager);
            assert_eq!(rolled.active().unwrap().version, v("0.1.0"));
            // Still trusted where it now runs.
            let records = manager.store().plugins().unwrap();
            assert!(
                official
                    .withheld(&records, &|record, version| {
                        bundled_match(&bundle, manager.layout(), record, version)
                    })
                    .is_empty()
            );
        }
    }
}

/// Info: a floors file with fields of a newer DeckPilot is read (its
/// records keep working for revocation) but never rewritten (official
/// installs are refused); another floorsVersion is refused entirely.
#[test]
fn floors_of_a_newer_agent_are_read_but_never_rewritten() {
    let env = Env::new();
    let mut manager = env.open();
    let official = env.official();
    install_or_update(
        &mut manager,
        &official,
        &Release::new(&env, "0.1.1", &notifications()),
    );
    let path = env.state().join(FLOORS_FILE);
    let mut floors: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    floors["newerRecords"] = json!({ "something": 1 });
    floors["plugins"][ID]["newerField"] = json!(true);
    let written = serde_json::to_vec_pretty(&floors).unwrap();
    std::fs::write(&path, &written).unwrap();

    let downgraded = env.official();
    let records = manager.store().plugins().unwrap();
    assert!(downgraded.revoked_installed(&records).is_empty());
    assert!(
        downgraded.ready().unwrap_err().contains("newer DeckPilot"),
        "{:?}",
        downgraded.ready()
    );
    let host = host();
    assert!(matches!(
        refusal(
            downgraded.verify(&Release::new(&env, "0.1.2", &notifications()).delivery(None, &host))
        ),
        Refusal::Unavailable(_)
    ));
    assert_eq!(std::fs::read(&path).unwrap(), written, "never rewritten");

    floors["floorsVersion"] = json!(2);
    std::fs::write(&path, serde_json::to_vec(&floors).unwrap()).unwrap();
    let other_format = env.official();
    let withheld = other_format.revoked_installed(&records);
    assert!(
        withheld[&(id(), v("0.1.1"))].reason.contains("cannot read"),
        "{withheld:?}"
    );
}

// ------------------------------------- privileged plugins (reviewed policy)
//
// The gate is closed in this build. These tests open it in the test only,
// with the compiled privileged policy and an approved test review, and use
// the real manifests of deckpilot.workflows and deckpilot.git
// (their releases' files but the module, plugins/first-party/contracts/)
// with throwaway keys.

const FIRST_PARTY: &str = "../../plugins/first-party/contracts";

fn pid(text: &str) -> PluginId {
    PluginId::parse(text).unwrap()
}

/// The real plugin `name`'s files at `version` (its manifest edited by
/// `edit`), with a stand-in module whose bytes carry the version.
fn real_files(name: &str, version: &str, edit: impl FnOnce(&mut Value)) -> Files {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(FIRST_PARTY)
        .join(format!("deckpilot.{name}"));
    let mut manifest: Value =
        serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).unwrap()).unwrap();
    manifest["version"] = version.into();
    edit(&mut manifest);
    let mut files = Files::new();
    files.insert(
        MANIFEST.into(),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    );
    files.insert(
        "LICENSE".into(),
        std::fs::read(dir.join("LICENSE")).unwrap(),
    );
    for sub in ["locales", "assets"] {
        let Ok(entries) = std::fs::read_dir(dir.join(sub)) else {
            continue;
        };
        for entry in entries {
            let entry = entry.unwrap();
            files.insert(
                format!("{sub}/{}", entry.file_name().to_str().unwrap()),
                std::fs::read(entry.path()).unwrap(),
            );
        }
    }
    files.insert(
        "plugin.wasm".into(),
        deckpilot_plugin_package::wasm::minimal_abi_module_with_note(&format!("{name} {version}")),
    );
    files
}

/// What a verified registry chain would pin for `package` as `plugin`.
fn expected_as(plugin: &str, package: &[u8], version: &str) -> ExpectedPackage {
    let mut expected = expected_for(package, version);
    expected.id = pid(plugin);
    expected
}

fn signed_as(key: &Key, plugin: &str, package: &[u8], version: &str) -> SignatureFile {
    let comment = publisher::release_comment(&pid(plugin), &v(version), &digest(package));
    SignatureFile {
        key_id: key.key_id(),
        bytes: key.signing.sign(package, &comment).unwrap().into_bytes(),
    }
}

/// The compiled privileged policy with an approved test review and `edit`.
fn reviewed(
    edit: impl FnOnce(&mut privileged_policy::PrivilegedPolicy),
) -> PrivilegedGate<'static> {
    let mut policy = privileged_policy::compiled_policy().unwrap().clone();
    policy.review = Some(privileged_policy::Review {
        reviewer: "throwaway test reviewer".into(),
        date: "2026-11-21".into(),
        outcome: privileged_policy::ReviewOutcome::Approved,
        record: "official/tests.rs".into(),
    });
    edit(&mut policy);
    PrivilegedGate::open_for_tests(Box::leak(Box::new(policy)))
}

#[test]
fn the_privileged_gate_opens_only_with_the_approved_review() {
    // The opening change: the constant is open; the compiled gate is open
    // only once the record's review is approved.
    const { assert!(privileged_policy::PRIVILEGED_GATE_OPEN) };
    let approved = privileged_policy::compiled_policy().unwrap().is_approved();
    assert_eq!(compiled_gate() == PrivilegedGate::Closed, !approved);
    let env = Env::new();
    assert_eq!(Official::open(&env.state()).gate, compiled_gate());
    let second = Key::new(env.root.path(), "second.key");
    // Closed, even with two distinct keys and exactly the reviewed release.
    let official = env
        .official_with(keys(&[&env.key, &second], 2, &[]), Vec::new())
        .with_gate(PrivilegedGate::Closed);
    let package = raw_package(&real_files("workflows", "0.1.1", |_| {}));
    let expected = expected_as("deckpilot.workflows", &package, "0.1.1");
    let signatures = [
        signed_as(&env.key, "deckpilot.workflows", &package, "0.1.1"),
        signed_as(&second, "deckpilot.workflows", &package, "0.1.1"),
    ];
    let host = host();
    let delivery = Delivery {
        expected: &expected,
        package: &package,
        signatures: &signatures,
        pin: None,
        host: Some(&host),
        now: Utc::now(),
    };
    assert!(matches!(
        refusal(official.verify(&delivery)),
        Refusal::Publisher(PublisherError::PrivilegedGateClosed { .. })
    ));
    assert!(official.floor(&pid("deckpilot.workflows")).is_none());
}

/// The opening path, behind a test-only open gate: the review record, at
/// least two distinct pinned keys that signed, exactly the reviewed
/// manifest and the registry's pins; then the store refuses the registry
/// copy all the same (X12): privileged plugins come only as bundled
/// copies.
#[test]
#[allow(clippy::too_many_lines)] // one flow, step by step
fn the_opening_path_needs_the_review_two_keys_and_exactly_the_policy() {
    const WORKFLOWS: &str = "deckpilot.workflows";
    let env = Env::new();
    let second = Key::new(env.root.path(), "second.key");
    let third = Key::new(env.root.path(), "third.key");
    let host = host();
    let package = raw_package(&real_files("workflows", "0.1.1", |_| {}));
    let expected = expected_as(WORKFLOWS, &package, "0.1.1");
    let both = [
        signed_as(&env.key, WORKFLOWS, &package, "0.1.1"),
        signed_as(&second, WORKFLOWS, &package, "0.1.1"),
    ];
    let delivery = |expected, signatures| Delivery {
        expected,
        package: &package,
        signatures,
        pin: None,
        host: Some(&host),
        now: Utc::now(),
    };
    let gated = |keys: PublisherKeys, gate| env.official_with(keys, Vec::new()).with_gate(gate);
    let closed = |r: Result<VerifiedPackage, Refusal>| {
        matches!(
            r,
            Err(Refusal::Publisher(
                PublisherError::PrivilegedGateClosed { .. }
            ))
        )
    };
    let not_reviewed = |r: Result<VerifiedPackage, Refusal>, text: &str| match r {
        Err(Refusal::Publisher(PublisherError::PrivilegedNotReviewed { reason, .. })) => {
            assert!(reason.contains(text), "`{text}` not in {reason}");
        }
        other => panic!("{other:?} (expected {text})"),
    };
    let open = reviewed(|_| {});
    // One key is never enough, whatever the policy says.
    let one = gated(keys(&[&env.key], 1, &[]), open);
    assert!(closed(one.verify(&delivery(&expected, &both[..1]))));
    // Two pinned keys with threshold 1: the pinned threshold must be 2.
    let low = gated(keys(&[&env.key, &second], 1, &[]), open);
    assert!(closed(low.verify(&delivery(&expected, &both))));
    // A policy asking for three keys.
    let three = gated(
        keys(&[&env.key, &second, &third], 2, &[]),
        reviewed(|p| p.min_threshold = 3),
    );
    assert!(closed(three.verify(&delivery(&expected, &both))));
    // No approved review recorded.
    for gate in [
        reviewed(|p| p.review = None),
        reviewed(|p| {
            p.review.as_mut().unwrap().outcome = privileged_policy::ReviewOutcome::Pending;
        }),
    ] {
        let official = gated(keys(&[&env.key, &second], 2, &[]), gate);
        assert!(closed(official.verify(&delivery(&expected, &both))));
    }
    let pinned = || keys(&[&env.key, &second, &third], 2, &[]);
    // Another version than the reviewed one.
    let other_version = reviewed(|p| {
        let w = p
            .plugins
            .iter_mut()
            .find(|p| p.id.as_str() == WORKFLOWS)
            .unwrap();
        w.version = v("0.1.2");
    });
    not_reviewed(
        gated(pinned(), other_version).verify(&delivery(&expected, &both)),
        "covers version 0.1.2 only",
    );
    // A plugin the policy does not list.
    let without = reviewed(|p| p.plugins.retain(|p| p.id.as_str() != WORKFLOWS));
    not_reviewed(
        gated(pinned(), without).verify(&delivery(&expected, &both)),
        "does not cover this plugin",
    );
    // A new construct (status items) in a release signed by both keys.
    let broader = raw_package(&real_files("workflows", "0.1.1", |m| {
        m["contributes"]["statusItems"] = json!(["activeProject"]);
    }));
    let broader_expected = expected_as(WORKFLOWS, &broader, "0.1.1");
    let broader_signed = [
        signed_as(&env.key, WORKFLOWS, &broader, "0.1.1"),
        signed_as(&second, WORKFLOWS, &broader, "0.1.1"),
    ];
    let broader_delivery = Delivery {
        expected: &broader_expected,
        package: &broader,
        signatures: &broader_signed,
        pin: None,
        host: Some(&host),
        now: Utc::now(),
    };
    not_reviewed(
        gated(pinned(), open).verify(&broader_delivery),
        "`statusItems`) is not reviewed",
    );
    // The registry's pins still hold on this path.
    let mut relabeled = expected.clone();
    relabeled.permissions = Some(Vec::new());
    assert!(matches!(
        gated(pinned(), open).verify(&delivery(&relabeled, &both)),
        Err(Refusal::Publisher(PublisherError::Package(_)))
    ));
    // Everything holds: the opening path admits it, counting exactly the
    // two real signers, and the registry delivery is refused all the same
    // (bundled copies only), audited as refused, with no floor raised.
    let official = gated(pinned(), open);
    let Err(Refusal::BundledOnly { mut signers }) = official.verify(&delivery(&expected, &both))
    else {
        panic!("a privileged registry delivery must be refused");
    };
    signers.sort();
    let mut real = vec![
        publisher::fingerprint(&env.key.public()).unwrap(),
        publisher::fingerprint(&second.public()).unwrap(),
    ];
    real.sort();
    assert_eq!(signers, real);
    let last = env.audit().last().unwrap().record.clone();
    assert_eq!(last.outcome, publisher::AuditOutcome::Refused);
    assert!(
        last.reason
            .as_deref()
            .unwrap_or("")
            .contains("bundled copy"),
        "{last:?}"
    );
    assert_eq!(last.signers, Vec::<String>::new());
    assert!(
        official.floor(&pid(WORKFLOWS)).is_none(),
        "no floor is raised"
    );
    // The store is the second lock: the package the opening path admits is
    // refused from a registry too.
    let limits = Limits::default();
    let policy = ReleasePolicy {
        host: Some(&host),
        floor: None,
        revoked: &[],
        limits: &limits,
        gate: PrivilegedGate::Closed,
    };
    let admitted = admit_privileged(
        open,
        PublisherError::PrivilegedGateClosed { usages: Vec::new() },
        &pinned(),
        &both,
        &expected,
        &package,
        &policy,
    )
    .unwrap();
    assert!(
        GrantSet::from_manifest(admitted.package.manifest())
            .keys()
            .any(GrantKey::is_first_party_only)
    );
    let mut manager = env.open();
    let refused = manager
        .install(
            &admitted.package,
            &registry_source(),
            &Consent::of(vec![], "user"),
        )
        .unwrap_err();
    assert!(
        matches!(
            refused,
            deckpilot_plugin_store::StoreError::SourceNotAllowed { .. }
        ),
        "{refused}"
    );
}

/// With the production gate (pending or approved), a registry delivery of
/// exactly the reviewed release, signed by two distinct pinned keys, is
/// refused (X12: privileged plugins only as bundled copies) and raises no
/// floor.
#[test]
fn the_compiled_gate_refuses_every_registry_delivery_of_a_privileged_plugin() {
    let env = Env::new();
    let second = Key::new(env.root.path(), "second.key");
    let official = env.official_with(keys(&[&env.key, &second], 2, &[]), Vec::new());
    assert_eq!(official.gate, compiled_gate());
    for name in ["git", "docker", "projects", "workflows", "workspace"] {
        let plugin = format!("deckpilot.{name}");
        let package = raw_package(&real_files(name, "0.1.1", |_| {}));
        let expected = expected_as(&plugin, &package, "0.1.1");
        let signatures = [
            signed_as(&env.key, &plugin, &package, "0.1.1"),
            signed_as(&second, &plugin, &package, "0.1.1"),
        ];
        let host = host();
        let result = official.verify(&Delivery {
            expected: &expected,
            package: &package,
            signatures: &signatures,
            pin: None,
            host: Some(&host),
            now: Utc::now(),
        });
        assert!(
            matches!(
                result,
                Err(Refusal::BundledOnly { .. }
                    | Refusal::Publisher(PublisherError::PrivilegedGateClosed { .. }))
            ),
            "{plugin}: {result:?}"
        );
        assert!(official.floor(&pid(&plugin)).is_none(), "{plugin}");
        assert_eq!(
            env.audit().last().unwrap().record.outcome,
            publisher::AuditOutcome::Refused
        );
    }
}

/// What an application build does with a reviewed privileged artifact
/// (plugins/first-party/artifacts.sh): `release verify` with the gate open
/// (here in the test), then the verified files unpacked as the bundled
/// copy and pinned.
fn stage_reviewed(
    env: &Env,
    files: &Files,
    plugin: &str,
    version: &str,
    gate: PrivilegedGate<'_>,
    floor: Option<&ReleaseFloor>,
    second: &Key,
) -> Result<Bundle, PublisherError> {
    let mut builder = PackageBuilder::new();
    builder.reviewed_privileged();
    for (path, bytes) in files {
        builder.add(path, bytes.clone()).unwrap();
    }
    let package = builder.build().unwrap().bytes;
    let claim = ReleaseClaim {
        id: pid(plugin),
        version: v(version),
        package: digest(&package),
    };
    let signatures = [
        signed_as(&env.key, plugin, &package, version),
        signed_as(second, plugin, &package, version),
    ];
    let limits = Limits::default();
    let host = host();
    let policy = ReleasePolicy {
        host: Some(&host),
        floor,
        revoked: &[],
        limits: &limits,
        gate,
    };
    let release = publisher::verify_release(
        &package,
        &signatures,
        &keys(&[&env.key, second], 2, &[]),
        &claim,
        &policy,
    )?;
    assert!(release.privileged().is_some());
    Ok(bundle_of(env, plugin, version, release.files()))
}

/// `files` unpacked as the bundled copy of `plugin` and pinned (class
/// `default`, so that seeding handles it).
fn bundle_of(env: &Env, plugin: &str, version: &str, files: &Files) -> Bundle {
    let dir = env.root.path().join("resources").join(BUNDLED_DIR);
    let target = dir.join(plugin);
    let _ = std::fs::remove_dir_all(&target);
    for (path, bytes) in files {
        let file = target.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, bytes).unwrap();
    }
    let pins = Pins::compute(&dir, &[(pid(plugin), v(version), BundleClass::Default)]).unwrap();
    Bundle {
        dir: Some(dir),
        pins,
        trust: Ok(()),
    }
}

fn grants_of(manager: &PluginManager, plugin: &str) -> BTreeMap<String, (GrantStatus, String)> {
    manager
        .store()
        .grants(&pid(plugin))
        .unwrap()
        .into_iter()
        .map(|g| (g.key.to_string(), (g.status, g.decided_by)))
        .collect()
}

fn granted(manager: &PluginManager, plugin: &str) -> BTreeSet<String> {
    grants_of(manager, plugin)
        .into_iter()
        .filter(|(_, (status, _))| *status == GrantStatus::Granted)
        .map(|(key, _)| key)
        .collect()
}

/// The migration of a privileged plugin from the bundled copy built from
/// source to the bundled copy staged from its reviewed artifact, with the
/// gate open in the test: the store's consent rules decide, exactly as for
/// any bundled update. Process execution (`tools:git`) is never derived by
/// seeding, only granted by the user; a grant the new version no longer
/// declares is dropped; one it broadens waits for consent while the old
/// copy keeps running; and the bundled copy never goes backwards.
#[test]
#[allow(clippy::too_many_lines)] // one flow, step by step
fn a_reviewed_artifact_updates_the_bundled_copy_under_the_store_s_consent_rules() {
    const GIT: &str = "deckpilot.git";
    let env = Env::new();
    let second = Key::new(env.root.path(), "second.key");
    let open = reviewed(|_| {});

    // The application release before: deckpilot.git 0.1.0 built from source,
    // with one git subcommand fewer and a notifications permission that
    // 0.1.1 no longer declares.
    let source = real_files("git", "0.1.0", |m| {
        let subcommands = m["permissions"][0]["tools"][0]["subcommands"]
            .as_array_mut()
            .unwrap();
        subcommands.retain(|s| s["name"] != "log");
        m["permissions"].as_array_mut().unwrap().push(json!({
            "kind": "notifications",
            "justification": "Test: dropped by the next version."
        }));
    });
    let before = bundle_of(&env, GIT, "0.1.0", &source);
    let mut manager = env.open();
    let report = bundled::seed(&mut manager, &before);
    let Seeded::AwaitingConsent(waiting) = &report[0].1 else {
        panic!("{report:?}")
    };
    // Seeding derived only low-tier grants: never process execution.
    let keys_waiting: BTreeSet<String> = waiting.iter().map(ToString::to_string).collect();
    for high in ["tools:git", "tools:git:network", "resources:open"] {
        assert!(
            keys_waiting.contains(high),
            "{high} waits: {keys_waiting:?}"
        );
        assert!(!granted(&manager, GIT).contains(high), "{high} not derived");
    }
    assert!(granted(&manager, GIT).contains("notifications"));
    // The user consents to everything 0.1.0 asks for.
    manager
        .grant(&pid(GIT), &Consent::of(waiting.clone(), "user"))
        .unwrap();
    let consented = granted(&manager, GIT);
    for key in &keys_waiting {
        assert!(consented.contains(key), "{key}");
        assert_eq!(grants_of(&manager, GIT)[key].1, "user");
    }

    // The application release after: 0.1.1 from its reviewed artifact,
    // above the 0.1.0 bundled floor.
    let floor = ReleaseFloor {
        version: v("0.1.0"),
        files: before.pins.get(&pid(GIT)).unwrap().files.clone(),
    };
    let reviewed_files = real_files("git", "0.1.1", |_| {});
    // Closed, it is not staged at all.
    assert!(matches!(
        stage_reviewed(
            &env,
            &reviewed_files,
            GIT,
            "0.1.1",
            PrivilegedGate::Closed,
            Some(&floor),
            &second
        ),
        Err(PublisherError::PrivilegedGateClosed { .. })
    ));
    let after = stage_reviewed(
        &env,
        &reviewed_files,
        GIT,
        "0.1.1",
        open,
        Some(&floor),
        &second,
    )
    .unwrap();
    let report = bundled::seed(&mut manager, &after);
    // `tools:git` is broader (the `log` subcommand): staged for consent,
    // the 0.1.0 copy keeps running with its grants.
    assert_eq!(report[0].1, Seeded::UpdateAwaitingConsent, "{report:?}");
    let record = manager.store().plugin(&pid(GIT)).unwrap().unwrap();
    assert_eq!(record.active().unwrap().version, v("0.1.0"));
    assert_eq!(granted(&manager, GIT), consented);
    let (staged, diff) = manager.pending_update(&pid(GIT)).unwrap().unwrap();
    assert_eq!(staged, v("0.1.1"));
    let escalations: Vec<String> = diff.escalations().iter().map(ToString::to_string).collect();
    assert_eq!(escalations, ["tools:git"], "{diff:?}");
    // Seeding again does not approve it.
    let report = bundled::seed(&mut manager, &after);
    assert!(
        matches!(&report[0].1, Seeded::Skipped(why) if why.contains("approval")),
        "{report:?}"
    );
    // The user approves: 0.1.1 runs; the dropped permission's grant is gone,
    // the still-valid ones are kept as the user decided them.
    manager
        .approve_update(&pid(GIT), &Consent::of(diff.escalations(), "user"))
        .unwrap();
    let record = manager.store().plugin(&pid(GIT)).unwrap().unwrap();
    assert_eq!(record.active().unwrap().version, v("0.1.1"));
    assert_eq!(record.source, InstallSource::Bundled);
    let now = granted(&manager, GIT);
    assert!(!now.contains("notifications"), "{now:?}");
    for key in [
        "tools:git",
        "tools:git:network",
        "resources",
        "resources:open",
    ] {
        assert!(now.contains(key), "{key} kept: {now:?}");
        assert_eq!(grants_of(&manager, GIT)[key].1, "user", "{key}");
    }

    // Rollback: an application that bundles 0.1.0 again never replaces
    // 0.1.1, and a 0.1.0 artifact below the 0.1.1 floor is refused.
    let report = bundled::seed(&mut manager, &before);
    assert!(
        matches!(&report[0].1, Seeded::Skipped(why) if why.contains("newer")),
        "{report:?}"
    );
    let shipped = ReleaseFloor {
        version: v("0.1.1"),
        files: after.pins.get(&pid(GIT)).unwrap().files.clone(),
    };
    let old_reviewed = reviewed(|p| {
        let g = p.plugins.iter_mut().find(|p| p.id.as_str() == GIT).unwrap();
        g.version = v("0.1.0");
    });
    assert!(matches!(
        stage_reviewed(
            &env,
            &real_files("git", "0.1.0", |_| {}),
            GIT,
            "0.1.0",
            old_reviewed,
            Some(&shipped),
            &second
        ),
        Err(PublisherError::Rollback { .. })
    ));
    // The same version with other files is refused too.
    let rebuilt = real_files("git", "0.1.1", |_| {})
        .into_iter()
        .map(|(path, bytes)| {
            if path == "plugin.wasm" {
                (
                    path,
                    deckpilot_plugin_package::wasm::minimal_abi_module_with_note("rebuilt"),
                )
            } else {
                (path, bytes)
            }
        })
        .collect::<Files>();
    assert!(matches!(
        stage_reviewed(&env, &rebuilt, GIT, "0.1.1", open, Some(&shipped), &second),
        Err(PublisherError::SameVersionOtherFiles { .. })
    ));
    // A registry copy of the next version, admitted by the agent's opening
    // path, is refused (bundled copies only), audited, and raises no floor:
    // the bundled copy stays.
    let next = raw_package(&real_files("git", "0.1.2", |_| {}));
    let next_expected = expected_as(GIT, &next, "0.1.2");
    let next_signed = [
        signed_as(&env.key, GIT, &next, "0.1.2"),
        signed_as(&second, GIT, &next, "0.1.2"),
    ];
    let host = host();
    let next_gate = reviewed(|p| {
        let g = p.plugins.iter_mut().find(|p| p.id.as_str() == GIT).unwrap();
        g.version = v("0.1.2");
    });
    let official = env
        .official_with(keys(&[&env.key, &second], 2, &[]), Vec::new())
        .with_gate(next_gate);
    let refused = official.verify(&Delivery {
        expected: &next_expected,
        package: &next,
        signatures: &next_signed,
        pin: after.pins.get(&pid(GIT)),
        host: Some(&host),
        now: Utc::now(),
    });
    assert!(
        matches!(refused, Err(Refusal::BundledOnly { .. })),
        "{refused:?}"
    );
    assert!(official.floor(&pid(GIT)).is_none(), "no floor is raised");
    let record = manager.store().plugin(&pid(GIT)).unwrap().unwrap();
    assert_eq!(record.active().unwrap().version, v("0.1.1"));
    assert_eq!(record.source, InstallSource::Bundled);
}

/// An unchanged reviewed release (the same privileges as the copy built
/// from source) updates the bundled copy and keeps every grant as the user
/// decided it; nothing new is asked for and nothing is derived.
#[test]
fn an_unchanged_reviewed_artifact_keeps_every_grant() {
    const WORKFLOWS: &str = "deckpilot.workflows";
    let env = Env::new();
    let second = Key::new(env.root.path(), "second.key");
    let before = bundle_of(
        &env,
        WORKFLOWS,
        "0.1.0",
        &real_files("workflows", "0.1.0", |_| {}),
    );
    let mut manager = env.open();
    let report = bundled::seed(&mut manager, &before);
    // hostViews is low tier: derived at seeding, enabled.
    assert_eq!(report[0].1, Seeded::Enabled, "{report:?}");
    let grants = grants_of(&manager, WORKFLOWS);
    let floor = ReleaseFloor {
        version: v("0.1.0"),
        files: before.pins.get(&pid(WORKFLOWS)).unwrap().files.clone(),
    };
    let after = stage_reviewed(
        &env,
        &real_files("workflows", "0.1.1", |_| {}),
        WORKFLOWS,
        "0.1.1",
        reviewed(|_| {}),
        Some(&floor),
        &second,
    )
    .unwrap();
    let report = bundled::seed(&mut manager, &after);
    assert_eq!(report[0].1, Seeded::Updated, "{report:?}");
    assert_eq!(grants_of(&manager, WORKFLOWS), grants);
    let record = manager.store().plugin(&pid(WORKFLOWS)).unwrap().unwrap();
    assert_eq!(record.active().unwrap().version, v("0.1.1"));
    assert_eq!(record.state, PluginState::Enabled);
}
