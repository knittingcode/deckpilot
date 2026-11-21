//! The first-party publisher signature (`deckpilot_plugin_registry::publisher`)
//! and `deckpilot-registry release sign|verify`, with throwaway keys made
//! per test (never a production key).

mod common;

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use common::{ThrowawayKey, id, package, t0, v};
use deckpilot_plugin_manifest::{ApiLevel, HostEnvironment, PackageDigest, Platform};
use deckpilot_plugin_package::Limits;
use deckpilot_plugin_registry::RevokedVersions;
use deckpilot_plugin_registry::privileged_policy::PrivilegedGate;
use deckpilot_plugin_registry::publish::{Io, run};
use deckpilot_plugin_registry::publisher::{
    AuditOutcome, AuditRecord, MAX_SIGNATURE_FILES, PublisherError, PublisherKeys, ReleaseClaim,
    ReleaseFloor, ReleasePolicy, RevokedRelease, SignatureCheck, SignatureFile, digest_of,
    is_key_id, key_id_in_file_name, release_comment, signature_file_name, test_comment,
    test_message, verify_release, verify_test_signatures,
};
use deckpilot_plugin_registry::sig::index_comment;
use tempfile::TempDir;

fn keys(current: &[&ThrowawayKey], threshold: u64, revoked: &[&ThrowawayKey]) -> PublisherKeys {
    let keys = PublisherKeys {
        publisher_keys_version: 1,
        publisher: "deckpilot".into(),
        keys: current.iter().map(|k| k.public().to_owned()).collect(),
        threshold,
        revoked_keys: revoked.iter().map(|k| k.public().to_owned()).collect(),
    };
    keys.validate().unwrap();
    keys
}

fn host() -> HostEnvironment {
    HostEnvironment {
        app_version: v("0.10.0"),
        plugin_api: ApiLevel::new(1, 9),
        platform: Platform::Macos,
    }
}

fn claim_for(bytes: &[u8], plugin: &str, version: &str) -> ReleaseClaim {
    ReleaseClaim {
        id: id(plugin),
        version: v(version),
        package: digest_of(bytes),
    }
}

fn sign(key: &ThrowawayKey, bytes: &[u8], claim: &ReleaseClaim) -> String {
    key.sign(
        bytes,
        &release_comment(&claim.id, &claim.version, &claim.package),
    )
}

/// A signature file named for `key_id` holding `signature`.
fn named(key_id: &str, signature: &str) -> SignatureFile {
    SignatureFile {
        key_id: key_id.to_owned(),
        bytes: format!("{signature}\n").into_bytes(),
    }
}

struct Case {
    bytes: Vec<u8>,
    claim: ReleaseClaim,
    host: HostEnvironment,
    limits: Limits,
}

impl Case {
    fn new(plugin: &str, version: &str) -> Self {
        let bytes = package(plugin, version).bytes;
        let claim = claim_for(&bytes, plugin, version);
        Self {
            bytes,
            claim,
            host: host(),
            limits: Limits::default(),
        }
    }

    /// `key`'s signature of this case's bytes and claim, in its own file.
    fn signed(&self, key: &ThrowawayKey) -> SignatureFile {
        named(&key.key_id(), &sign(key, &self.bytes, &self.claim))
    }

    fn verify(
        &self,
        signatures: &[SignatureFile],
        keys: &PublisherKeys,
        floor: Option<&ReleaseFloor>,
        revoked: &[RevokedRelease],
    ) -> Result<deckpilot_plugin_registry::publisher::VerifiedRelease, PublisherError> {
        let policy = ReleasePolicy {
            host: Some(&self.host),
            floor,
            revoked,
            limits: &self.limits,
            gate: PrivilegedGate::compiled(),
        };
        verify_release(&self.bytes, signatures, keys, &self.claim, &policy)
    }
}

/// The refusal is the threshold's, with `valid` counted keys, and every
/// reason in `reasons` among the ignored files'.
fn below(result: Result<impl std::fmt::Debug, PublisherError>, valid: usize, reasons: &[&str]) {
    match result {
        Err(PublisherError::BelowThreshold {
            valid: counted,
            ignored,
            ..
        }) => {
            assert_eq!(counted, valid, "{ignored:?}");
            let all = ignored.join("; ");
            for reason in reasons {
                assert!(all.contains(reason), "`{reason}` not in: {all}");
            }
        }
        other => panic!("expected a threshold refusal, got {other:?}"),
    }
}

#[test]
fn a_signed_release_verifies_and_grants_nothing() {
    let key = ThrowawayKey::generate();
    let case = Case::new("deckpilot.alpha", "1.2.0");
    let verified = case
        .verify(&[case.signed(&key)], &keys(&[&key], 1, &[]), None, &[])
        .unwrap();
    assert_eq!(verified.manifest().id, id("deckpilot.alpha"));
    assert_eq!(verified.signers().len(), 1);
    let files = verified.file_digests();
    assert!(files.contains_key("plugin.wasm"));
    assert!(files.contains_key("manifest.json"));
    let audit = verified.audit(t0());
    assert_eq!(audit.outcome, AuditOutcome::Verified);
    assert_eq!(audit.signers, verified.signers());
    let line = audit.to_line();
    assert!(line.ends_with('\n'));
    let back: AuditRecord = serde_json::from_str(line.trim_end()).unwrap();
    assert_eq!(back, audit);
}

#[test]
fn nothing_verifies_without_a_provisioned_key() {
    let key = ThrowawayKey::generate();
    let case = Case::new("deckpilot.alpha", "1.2.0");
    let empty = keys(&[], 1, &[]);
    assert!(!empty.is_provisioned());
    assert_eq!(
        case.verify(&[case.signed(&key)], &empty, None, &[])
            .unwrap_err(),
        PublisherError::NotProvisioned
    );
}

#[test]
fn the_pinned_production_keys_count_no_other_key() {
    // The keys compiled into this build are the two production keys
    // (threshold 2): a throwaway key's signatures never count, also two of
    // them under the production keys' ids.
    let pinned = PublisherKeys::pinned().unwrap();
    assert!(pinned.is_provisioned());
    assert_eq!(pinned.threshold, 2);
    assert_eq!(
        pinned.signature_key_ids(),
        deckpilot_plugin_registry::publisher::PRODUCTION_KEY_IDS
    );
    let (a, b) = (ThrowawayKey::generate(), ThrowawayKey::generate());
    let case = Case::new("deckpilot.alpha", "1.2.0");
    below(
        case.verify(&[case.signed(&a), case.signed(&b)], &pinned, None, &[]),
        0,
        &["not a pinned publisher key"],
    );
    let ids = pinned.signature_key_ids();
    let disguised = [
        named(&ids[0], &sign(&a, &case.bytes, &case.claim)),
        named(&ids[1], &sign(&b, &case.bytes, &case.claim)),
    ];
    below(
        case.verify(&disguised, &pinned, None, &[]),
        0,
        &["the file is named for key"],
    );
}

#[test]
fn a_key_test_signature_is_never_a_release_signature() {
    let (a, b) = (ThrowawayKey::generate(), ThrowawayKey::generate());
    let test_keys = keys(&[&a, &b], 2, &[]);
    let message = test_message(3);
    let comment = test_comment(3, &digest_of(&message));
    let tested = |key: &ThrowawayKey| named(&key.key_id(), &key.sign(&message, &comment));
    // The key test verifies with both keys, and needs both.
    let verified = verify_test_signatures(&[tested(&a), tested(&b)], &test_keys, 3).unwrap();
    assert_eq!(verified.signers.len(), 2);
    below(verify_test_signatures(&[tested(&a)], &test_keys, 3), 1, &[]);
    // Another run's message or comment does not count.
    below(
        verify_test_signatures(&[tested(&a), tested(&b)], &test_keys, 4),
        0,
        &[],
    );
    // Without keys: refused.
    assert_eq!(
        verify_test_signatures(&[tested(&a), tested(&b)], &keys(&[], 1, &[]), 3).unwrap_err(),
        PublisherError::NotProvisioned
    );

    // As a release: the message claimed as any plugin is refused (its
    // signatures carry the test comment), for real ids and versions too.
    for (plugin, version) in [
        ("deckpilot.notifications", "0.1.2"),
        ("deckpilot.github", "0.2.1"),
        ("deckpilot.publisher-test", "3.0.0"),
    ] {
        let claim = claim_for(&message, plugin, version);
        let policy = ReleasePolicy {
            host: Some(&host()),
            floor: None,
            revoked: &[],
            limits: &Limits::default(),
            gate: PrivilegedGate::compiled(),
        };
        let refused = verify_release(
            &message,
            &[tested(&a), tested(&b)],
            &test_keys,
            &claim,
            &policy,
        );
        below(refused, 0, &["comment"]);
    }
    // Even the test comment over a real package's bytes never counts for
    // its release.
    let case = Case::new("deckpilot.alpha", "1.2.0");
    let package_comment = test_comment(3, &case.claim.package);
    let over_package =
        |key: &ThrowawayKey| named(&key.key_id(), &key.sign(&case.bytes, &package_comment));
    below(
        case.verify(&[over_package(&a), over_package(&b)], &test_keys, None, &[]),
        0,
        &["comment"],
    );
    // And a release signature is no key test.
    let release = [case.signed(&a), case.signed(&b)];
    below(verify_test_signatures(&release, &test_keys, 3), 0, &[]);
}

#[test]
fn the_signature_binds_id_version_and_bytes() {
    let key = ThrowawayKey::generate();
    let other = ThrowawayKey::generate();
    let pinned = keys(&[&key], 1, &[]);
    let case = Case::new("deckpilot.alpha", "1.2.0");

    // Another key.
    below(
        case.verify(&[case.signed(&other)], &pinned, None, &[]),
        0,
        &["not a pinned publisher key"],
    );

    // The right key, another version in the comment.
    let mut older = case.claim.clone();
    older.version = v("1.1.0");
    below(
        case.verify(
            &[named(&key.key_id(), &sign(&key, &case.bytes, &older))],
            &pinned,
            None,
            &[],
        ),
        0,
        &["trusted comment"],
    );

    // A registry signature of the same bytes.
    below(
        case.verify(
            &[named(
                &key.key_id(),
                &key.sign(&case.bytes, &index_comment(1, 1)),
            )],
            &pinned,
            None,
            &[],
        ),
        0,
        &["trusted comment"],
    );

    // Tampered bytes: refused against the claim before any signature.
    let mut tampered = Case::new("deckpilot.alpha", "1.2.0");
    let signature = [tampered.signed(&key)];
    let last = tampered.bytes.len() - 1;
    tampered.bytes[last] ^= 1;
    let error = tampered.verify(&signature, &pinned, None, &[]).unwrap_err();
    assert!(
        matches!(error, PublisherError::DigestMismatch { .. }),
        "{error}"
    );
    // ... and a claim rewritten to the tampered bytes fails the signature.
    tampered.claim.package = digest_of(&tampered.bytes);
    below(tampered.verify(&signature, &pinned, None, &[]), 0, &[]);

    // A package of another plugin under this claim's signature.
    let mut swapped = Case::new("deckpilot.beta", "1.2.0");
    swapped.claim.id = id("deckpilot.alpha");
    let error = swapped
        .verify(&[swapped.signed(&key)], &pinned, None, &[])
        .unwrap_err();
    assert!(matches!(error, PublisherError::Package(_)), "{error}");
}

#[test]
fn only_first_party_releases() {
    let key = ThrowawayKey::generate();
    let case = Case::new("acme.alpha", "1.0.0");
    let error = case
        .verify(&[case.signed(&key)], &keys(&[&key], 1, &[]), None, &[])
        .unwrap_err();
    assert!(
        matches!(error, PublisherError::NotFirstParty { .. }),
        "{error}"
    );
}

// ------------------------------------------------- threshold 2 (multisig)

#[test]
fn key_ids_and_file_names_follow_minisign() {
    let key = ThrowawayKey::generate();
    let id = key.key_id();
    assert!(is_key_id(&id), "{id}");
    // The id minisign itself prints in the public key's comment line.
    let text = String::from_utf8(STANDARD.decode(key.public()).unwrap()).unwrap();
    assert!(text.lines().next().unwrap().ends_with(&id), "{text}");
    // The id inside a signature is the signing key's.
    let signature = key.sign(b"x", "c");
    assert_eq!(
        deckpilot_plugin_registry::sig::signature_key_id(&signature).unwrap(),
        id
    );

    let package = "deckpilot.alpha-1.2.0.dpplugin";
    let name = signature_file_name(package, &id);
    assert_eq!(name, format!("deckpilot.alpha-1.2.0.dpplugin.{id}.minisig"));
    assert_eq!(key_id_in_file_name(package, &name), Some(id.as_str()));
    // The old single name is not a signature file of the new convention.
    for other in [
        "deckpilot.alpha-1.2.0.dpplugin.minisig",
        "deckpilot.alpha-1.2.0.dpplugin..minisig",
        "deckpilot.alpha-1.2.0.dpplugin.A.B.minisig",
        "deckpilot.alpha-1.2.0.dpplugin",
        "deckpilot.beta-1.2.0.dpplugin.0123456789ABCDEF.minisig",
    ] {
        assert_eq!(key_id_in_file_name(package, other), None, "{other}");
    }
    for bad in [
        "0123456789abcdef",
        "0123456789ABCDE",
        "0123456789ABCDEFA",
        "G123456789ABCDEF",
    ] {
        assert!(!is_key_id(bad), "{bad}");
    }
}

#[test]
fn two_distinct_keys_meet_threshold_two() {
    let a = ThrowawayKey::generate();
    let b = ThrowawayKey::generate();
    let case = Case::new("deckpilot.alpha", "1.2.0");
    let pinned = keys(&[&a, &b], 2, &[]);
    assert_eq!(pinned.signature_key_ids(), vec![a.key_id(), b.key_id()]);
    let verified = case
        .verify(&[case.signed(&a), case.signed(&b)], &pinned, None, &[])
        .unwrap();
    assert_eq!(verified.signers().len(), 2);
    assert!(
        verified
            .signature_checks()
            .iter()
            .all(|c| matches!(c, SignatureCheck::Counted { .. })),
        "{:?}",
        verified.signature_checks()
    );
    // The order of the files does not matter.
    case.verify(&[case.signed(&b), case.signed(&a)], &pinned, None, &[])
        .unwrap();
    // Two of three pinned keys.
    let c = ThrowawayKey::generate();
    let three = keys(&[&a, &b, &c], 2, &[]);
    case.verify(&[case.signed(&a), case.signed(&c)], &three, None, &[])
        .unwrap();
}

#[test]
fn one_valid_signature_is_not_enough() {
    let a = ThrowawayKey::generate();
    let b = ThrowawayKey::generate();
    let case = Case::new("deckpilot.alpha", "1.2.0");
    let pinned = keys(&[&a, &b], 2, &[]);
    below(case.verify(&[case.signed(&a)], &pinned, None, &[]), 1, &[]);
    // The second file is garbage, empty or oversized.
    for junk in [
        b"not base64 !".to_vec(),
        Vec::new(),
        vec![b'A'; deckpilot_plugin_registry::publisher::MAX_SIGNATURE_BYTES + 1],
    ] {
        let files = [
            case.signed(&a),
            SignatureFile {
                key_id: b.key_id(),
                bytes: junk,
            },
        ];
        below(case.verify(&files, &pinned, None, &[]), 1, &[&b.key_id()]);
    }
}

#[test]
fn missing_signature_files_are_refused() {
    let a = ThrowawayKey::generate();
    let b = ThrowawayKey::generate();
    let case = Case::new("deckpilot.alpha", "1.2.0");
    for pinned in [keys(&[&a], 1, &[]), keys(&[&a, &b], 2, &[])] {
        let error = case.verify(&[], &pinned, None, &[]).unwrap_err();
        assert!(
            error.to_string().contains("no other signature file"),
            "{error}"
        );
        below(Err::<(), _>(error), 0, &[]);
    }
}

#[test]
fn the_same_signature_under_two_names_counts_once() {
    let a = ThrowawayKey::generate();
    let b = ThrowawayKey::generate();
    let case = Case::new("deckpilot.alpha", "1.2.0");
    let pinned = keys(&[&a, &b], 2, &[]);
    let by_a = sign(&a, &case.bytes, &case.claim);
    below(
        case.verify(
            &[named(&a.key_id(), &by_a), named(&b.key_id(), &by_a)],
            &pinned,
            None,
            &[],
        ),
        1,
        &[&format!(
            "named for key {} but the signature names key {}",
            b.key_id(),
            a.key_id()
        )],
    );
}

#[test]
fn two_files_from_the_same_key_count_once() {
    let a = ThrowawayKey::generate();
    let b = ThrowawayKey::generate();
    let case = Case::new("deckpilot.alpha", "1.2.0");
    let pinned = keys(&[&a, &b], 2, &[]);
    // The same file given twice (a client that does not dedupe names).
    below(
        case.verify(&[case.signed(&a), case.signed(&a)], &pinned, None, &[]),
        1,
        &["a second file for the same key"],
    );
    // A second signature of the same key with another untrusted comment
    // (other bytes, same key).
    let text =
        String::from_utf8(STANDARD.decode(sign(&a, &case.bytes, &case.claim)).unwrap()).unwrap();
    let rest = text.split_once('\n').unwrap().1;
    let other = STANDARD.encode(format!("untrusted comment: again\n{rest}"));
    below(
        case.verify(
            &[case.signed(&a), named(&a.key_id(), &other)],
            &pinned,
            None,
            &[],
        ),
        1,
        &["a second file for the same key"],
    );
    // A's signature in a file named with the key id in lower case.
    below(
        case.verify(
            &[
                case.signed(&a),
                named(
                    &a.key_id().to_lowercase(),
                    &sign(&a, &case.bytes, &case.claim),
                ),
            ],
            &pinned,
            None,
            &[],
        ),
        1,
        &["does not name a key id"],
    );
}

#[test]
fn a_wrong_publisher_key_never_counts() {
    let a = ThrowawayKey::generate();
    let b = ThrowawayKey::generate();
    let stranger = ThrowawayKey::generate();
    let case = Case::new("deckpilot.alpha", "1.2.0");
    let pinned = keys(&[&a, &b], 2, &[]);
    // Under its own name: not pinned.
    below(
        case.verify(
            &[case.signed(&a), case.signed(&stranger)],
            &pinned,
            None,
            &[],
        ),
        1,
        &["not a pinned publisher key"],
    );
    // Under a pinned key's name: the names do not match.
    let by_stranger = sign(&stranger, &case.bytes, &case.claim);
    below(
        case.verify(
            &[case.signed(&a), named(&b.key_id(), &by_stranger)],
            &pinned,
            None,
            &[],
        ),
        1,
        &[&format!("the signature names key {}", stranger.key_id())],
    );
}

#[test]
fn a_file_name_and_key_id_mismatch_never_counts() {
    let a = ThrowawayKey::generate();
    let b = ThrowawayKey::generate();
    let case = Case::new("deckpilot.alpha", "1.2.0");
    let pinned = keys(&[&a, &b], 2, &[]);
    // Swapped names: each file names the other key.
    let by_a = sign(&a, &case.bytes, &case.claim);
    let by_b = sign(&b, &case.bytes, &case.claim);
    below(
        case.verify(
            &[named(&b.key_id(), &by_a), named(&a.key_id(), &by_b)],
            &pinned,
            None,
            &[],
        ),
        0,
        &["is named for key"],
    );
    // A name that is no key id at all.
    below(
        case.verify(&[case.signed(&a), named("B", &by_b)], &pinned, None, &[]),
        1,
        &["does not name a key id"],
    );
}

#[test]
fn wrong_id_version_or_bytes_never_count_for_either_key() {
    let a = ThrowawayKey::generate();
    let b = ThrowawayKey::generate();
    let pinned = keys(&[&a, &b], 2, &[]);
    let case = Case::new("deckpilot.alpha", "1.2.0");
    let both = |claim: &ReleaseClaim| {
        [
            named(&a.key_id(), &sign(&a, &case.bytes, claim)),
            named(&b.key_id(), &sign(&b, &case.bytes, claim)),
        ]
    };
    let mut other_id = case.claim.clone();
    other_id.id = id("deckpilot.beta");
    let mut other_version = case.claim.clone();
    other_version.version = v("1.2.1");
    for claim in [other_id, other_version] {
        below(
            case.verify(&both(&claim), &pinned, None, &[]),
            0,
            &["trusted comment"],
        );
    }
    // Modified bytes: refused against the claim, then against the
    // signatures when the claim follows the bytes.
    let mut modified = Case::new("deckpilot.alpha", "1.2.0");
    let files = [modified.signed(&a), modified.signed(&b)];
    let last = modified.bytes.len() - 1;
    modified.bytes[last] ^= 1;
    assert!(matches!(
        modified.verify(&files, &pinned, None, &[]),
        Err(PublisherError::DigestMismatch { .. })
    ));
    modified.claim.package = digest_of(&modified.bytes);
    // The comment names the original sha256 and the bytes changed.
    below(modified.verify(&files, &pinned, None, &[]), 0, &[]);
}

#[test]
fn revoked_keys_never_count_and_refuse() {
    let a = ThrowawayKey::generate();
    let b = ThrowawayKey::generate();
    let old = ThrowawayKey::generate();
    let case = Case::new("deckpilot.alpha", "1.2.0");
    // Alone.
    let error = case
        .verify(&[case.signed(&old)], &keys(&[&a], 1, &[&old]), None, &[])
        .unwrap_err();
    assert!(
        matches!(error, PublisherError::RevokedKey { .. }),
        "{error}"
    );
    // With a second valid key (threshold 1 met by `a`): still refused.
    let error = case
        .verify(
            &[case.signed(&a), case.signed(&old)],
            &keys(&[&a], 1, &[&old]),
            None,
            &[],
        )
        .unwrap_err();
    assert!(
        matches!(error, PublisherError::RevokedKey { .. }),
        "{error}"
    );
    // With the two valid keys of threshold 2: still refused.
    let pinned = keys(&[&a, &b], 2, &[&old]);
    let error = case
        .verify(
            &[case.signed(&a), case.signed(&b), case.signed(&old)],
            &pinned,
            None,
            &[],
        )
        .unwrap_err();
    assert!(
        matches!(error, PublisherError::RevokedKey { .. }),
        "{error}"
    );
    // Without the revoked key's file, the two valid keys pass (what the
    // agent does: it fetches only current keys' files).
    case.verify(&[case.signed(&a), case.signed(&b)], &pinned, None, &[])
        .unwrap();
    // A revoked key's file that does not verify is only ignored.
    let junk = named(&old.key_id(), &sign(&old, b"other bytes", &case.claim));
    case.verify(
        &[case.signed(&a), case.signed(&b), junk.clone()],
        &pinned,
        None,
        &[],
    )
    .unwrap();
    below(
        case.verify(&[case.signed(&a), junk], &pinned, None, &[]),
        1,
        &["a revoked key"],
    );
    // A key cannot be both current and revoked.
    let both = PublisherKeys {
        publisher_keys_version: 1,
        publisher: "deckpilot".into(),
        keys: vec![old.public().into()],
        threshold: 1,
        revoked_keys: vec![old.public_with_comment("same key")],
    };
    assert!(both.validate().is_err());
}

#[test]
fn a_downgrade_with_two_signatures_is_refused() {
    let a = ThrowawayKey::generate();
    let b = ThrowawayKey::generate();
    let case = Case::new("deckpilot.alpha", "1.2.0");
    let pinned = keys(&[&a, &b], 2, &[]);
    let floor = ReleaseFloor {
        version: v("1.3.0"),
        files: BTreeMap::new(),
    };
    let error = case
        .verify(
            &[case.signed(&a), case.signed(&b)],
            &pinned,
            Some(&floor),
            &[],
        )
        .unwrap_err();
    assert!(matches!(error, PublisherError::Rollback { .. }), "{error}");
}

#[test]
fn a_threshold_above_the_pinned_keys_is_refused() {
    let a = ThrowawayKey::generate();
    let b = ThrowawayKey::generate();
    let doc = |threshold: u64| {
        serde_json::json!({
            "publisherKeysVersion": 1,
            "publisher": "deckpilot",
            "keys": [a.public(), b.public()],
            "threshold": threshold,
            "revokedKeys": [],
        })
        .to_string()
    };
    assert!(PublisherKeys::from_json(doc(2).as_bytes()).is_ok());
    for threshold in [0, 3, u64::MAX] {
        let error = PublisherKeys::from_json(doc(threshold).as_bytes()).unwrap_err();
        assert!(matches!(error, PublisherError::Keys(_)), "{error}");
    }
    // Revoked keys do not count towards the threshold either.
    let c = ThrowawayKey::generate();
    let mut with_revoked: PublisherKeys = serde_json::from_str(&doc(2)).unwrap();
    with_revoked.keys.pop();
    with_revoked.revoked_keys = vec![b.public().into(), c.public().into()];
    assert!(with_revoked.validate().is_err());
    // A document that bypasses the parser is still refused at verification.
    let case = Case::new("deckpilot.alpha", "1.2.0");
    let mut unchecked: PublisherKeys = serde_json::from_str(&doc(2)).unwrap();
    unchecked.threshold = 3;
    let error = case
        .verify(&[case.signed(&a), case.signed(&b)], &unchecked, None, &[])
        .unwrap_err();
    assert!(matches!(error, PublisherError::Keys(_)), "{error}");
}

#[test]
fn too_many_signature_files_are_refused() {
    let a = ThrowawayKey::generate();
    let case = Case::new("deckpilot.alpha", "1.2.0");
    let files = vec![case.signed(&a); MAX_SIGNATURE_FILES + 1];
    let error = case
        .verify(&files, &keys(&[&a], 1, &[]), None, &[])
        .unwrap_err();
    assert!(
        matches!(error, PublisherError::TooManySignatures { .. }),
        "{error}"
    );
}

#[test]
fn one_key_id_must_name_one_key() {
    let a = ThrowawayKey::generate();
    let b = ThrowawayKey::generate();
    // `b`'s Ed25519 key under `a`'s key id.
    let raw_line = |key: &ThrowawayKey| {
        let text = String::from_utf8(STANDARD.decode(key.public()).unwrap()).unwrap();
        STANDARD.decode(text.lines().nth(1).unwrap()).unwrap()
    };
    let mut forged = raw_line(&b);
    forged[2..10].copy_from_slice(&raw_line(&a)[2..10]);
    let forged = STANDARD.encode(format!(
        "untrusted comment: forged\n{}\n",
        STANDARD.encode(&forged)
    ));
    let doc = PublisherKeys {
        publisher_keys_version: 1,
        publisher: "deckpilot".into(),
        keys: vec![a.public().into(), forged],
        threshold: 1,
        revoked_keys: Vec::new(),
    };
    let error = doc.validate().unwrap_err();
    assert!(error.contains("names two keys"), "{error}");
}

#[test]
fn revoked_releases_are_refused() {
    let key = ThrowawayKey::generate();
    let case = Case::new("deckpilot.alpha", "1.2.0");
    let signature = [case.signed(&key)];
    let pinned = keys(&[&key], 1, &[]);
    let revoked = [RevokedRelease {
        id: id("deckpilot.alpha"),
        versions: RevokedVersions::List(vec![v("1.2.0")]),
        reason: "leaks a token".into(),
    }];
    let error = case
        .verify(&signature, &pinned, None, &revoked)
        .unwrap_err();
    assert!(error.to_string().contains("leaks a token"), "{error}");
    let other = [RevokedRelease {
        id: id("deckpilot.alpha"),
        versions: RevokedVersions::List(vec![v("1.1.0")]),
        reason: "old".into(),
    }];
    case.verify(&signature, &pinned, None, &other).unwrap();
    let all = [RevokedRelease {
        id: id("deckpilot.alpha"),
        versions: RevokedVersions::All,
        reason: "withdrawn".into(),
    }];
    assert!(case.verify(&signature, &pinned, None, &all).is_err());
}

#[test]
fn compatibility_is_enforced() {
    let key = ThrowawayKey::generate();
    let mut case = Case::new("deckpilot.alpha", "1.2.0");
    let signature = [case.signed(&key)];
    let pinned = keys(&[&key], 1, &[]);
    // The example manifest needs DeckPilot >=0.8.0 <1.0.0 and plugin API ^1.0.
    case.host.app_version = v("1.0.0");
    let error = case.verify(&signature, &pinned, None, &[]).unwrap_err();
    assert!(matches!(error, PublisherError::Incompatible(_)), "{error}");
    case.host.app_version = v("0.10.0");
    case.host.plugin_api = ApiLevel::new(2, 0);
    let error = case.verify(&signature, &pinned, None, &[]).unwrap_err();
    assert!(matches!(error, PublisherError::Incompatible(_)), "{error}");
}

#[test]
fn rollback_and_same_version_other_files_are_refused() {
    let key = ThrowawayKey::generate();
    let case = Case::new("deckpilot.alpha", "1.2.0");
    let signature = [case.signed(&key)];
    let pinned = keys(&[&key], 1, &[]);
    let files = case
        .verify(&signature, &pinned, None, &[])
        .unwrap()
        .file_digests();

    let higher = ReleaseFloor {
        version: v("1.3.0"),
        files: BTreeMap::new(),
    };
    let error = case
        .verify(&signature, &pinned, Some(&higher), &[])
        .unwrap_err();
    assert!(matches!(error, PublisherError::Rollback { .. }), "{error}");

    let mut changed = files.clone();
    changed.insert(
        "plugin.wasm".into(),
        PackageDigest {
            sha256: digest_of(b"other").sha256,
            size: 5,
        },
    );
    let same_other = ReleaseFloor {
        version: v("1.2.0"),
        files: changed,
    };
    let error = case
        .verify(&signature, &pinned, Some(&same_other), &[])
        .unwrap_err();
    assert!(
        error.to_string().contains("plugin.wasm"),
        "names the changed file: {error}"
    );

    let same = ReleaseFloor {
        version: v("1.2.0"),
        files,
    };
    case.verify(&signature, &pinned, Some(&same), &[]).unwrap();
    let lower = ReleaseFloor {
        version: v("1.1.9"),
        files: BTreeMap::new(),
    };
    case.verify(&signature, &pinned, Some(&lower), &[]).unwrap();
}

// ------------------------------------------------------------------ CLI

struct Cli {
    dir: TempDir,
    env: HashMap<String, String>,
}

impl Cli {
    fn new() -> Self {
        Self {
            dir: TempDir::new().unwrap(),
            env: HashMap::new(),
        }
    }

    fn path(&self, name: &str) -> String {
        self.dir.path().join(name).display().to_string()
    }

    fn run(&self, args: &[&str]) -> Result<String, String> {
        let args: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
        let env = |name: &str| self.env.get(name).cloned();
        let mut stdin = std::io::empty();
        let mut stdin = std::io::BufReader::new(&mut stdin);
        let mut out = Vec::new();
        let mut io = Io {
            env: &env,
            stdin: &mut stdin,
            out: &mut out,
        };
        run(&args, &mut io)
            .map(|()| String::from_utf8(out).unwrap())
            .map_err(|e| e.to_string())
    }
}

fn keys_file(path: &str, publics: &[&str], threshold: u64, revoked: &[&str]) {
    let doc = serde_json::json!({
        "publisherKeysVersion": 1,
        "publisher": "deckpilot",
        "keys": publics,
        "threshold": threshold,
        "revokedKeys": revoked,
    });
    std::fs::write(path, serde_json::to_vec_pretty(&doc).unwrap()).unwrap();
}

const PACKAGE: &str = "deckpilot.alpha-1.2.0.dpplugin";

fn verify_args<'a>(cli: &'a Cli, built: &'a PackageDigest, extra: &[&'a str]) -> Vec<String> {
    let mut args: Vec<String> = [
        "release",
        "verify",
        "--package",
        &cli.path(PACKAGE),
        "--id",
        "deckpilot.alpha",
        "--version",
        "1.2.0",
        "--sha256",
        &built.sha256.to_string(),
        "--size",
        &built.size.to_string(),
        "--app-version",
        "0.10.0",
        "--plugin-api",
        "1.9",
        "--platform",
        "macos",
        "--audit",
        &cli.path("audit.jsonl"),
        "--now",
        "2026-11-21T10:00:00Z",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    args.extend(extra.iter().map(|s| (*s).to_owned()));
    args
}

impl Cli {
    fn generate(&self, name: &str) -> String {
        self.run(&[
            "keys",
            "generate",
            "--role",
            "publisher",
            "--out",
            &self.path(name),
            "--no-password",
        ])
        .unwrap();
        std::fs::read_to_string(self.path(&format!("{name}.pub")))
            .unwrap()
            .trim()
            .to_owned()
    }

    fn sign_with(&self, key: &str, extra: &[&str]) -> Result<String, String> {
        let package = self.path(PACKAGE);
        let mut args = vec![
            "release",
            "sign",
            "--package",
            &package,
            "--id",
            "deckpilot.alpha",
            "--version",
            "1.2.0",
        ];
        let key = self.path(key);
        args.extend(["--key", &key]);
        args.extend_from_slice(extra);
        self.run(&args)
    }

    fn verify(&self, digest: &PackageDigest, extra: &[&str]) -> Result<String, String> {
        let args = verify_args(self, digest, extra);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        self.run(&args)
    }
}

fn key_id_of(public: &str) -> String {
    deckpilot_plugin_registry::publisher::key_id(public).unwrap()
}

#[test]
#[allow(clippy::too_many_lines)] // one flow, step by step
fn cli_signs_with_two_keys_verifies_audits_and_unpacks() {
    let mut cli = Cli::new();
    let bytes = package("deckpilot.alpha", "1.2.0").bytes;
    std::fs::write(cli.path(PACKAGE), &bytes).unwrap();
    let digest = digest_of(&bytes);
    let public_a = cli.generate("publisher-a.key");
    let public_b = cli.generate("publisher-b.key");
    let (id_a, id_b) = (key_id_of(&public_a), key_id_of(&public_b));

    // Key A from an environment variable, as a CI secret (one job).
    let secret = std::fs::read_to_string(cli.path("publisher-a.key")).unwrap();
    cli.env.insert("DECKPILOT_PUBLISHER_KEY_A".into(), secret);
    let out = cli
        .run(&[
            "release",
            "sign",
            "--package",
            &cli.path(PACKAGE),
            "--id",
            "deckpilot.alpha",
            "--version",
            "1.2.0",
            "--key-env",
            "DECKPILOT_PUBLISHER_KEY_A",
        ])
        .unwrap();
    let file_a = cli.path(&format!("{PACKAGE}.{id_a}.minisig"));
    assert!(out.contains("signed deckpilot.alpha 1.2.0"), "{out}");
    assert!(out.contains(&format!("publisher key {id_a}")), "{out}");
    assert!(out.contains(&file_a), "{out}");
    assert!(!out.contains("secret"), "{out}");
    assert!(Path::new(&file_a).is_file());
    // Never overwritten.
    let error = cli
        .run(&[
            "release",
            "sign",
            "--package",
            &cli.path(PACKAGE),
            "--id",
            "deckpilot.alpha",
            "--version",
            "1.2.0",
            "--key-env",
            "DECKPILOT_PUBLISHER_KEY_A",
        ])
        .unwrap_err();
    assert!(error.contains("refusing to overwrite"), "{error}");
    // The old single-file option is gone.
    let error = cli
        .sign_with("publisher-b.key", &["--out", &cli.path("x.minisig")])
        .unwrap_err();
    assert!(error.contains("unknown option --out"), "{error}");

    // Two keys pinned, threshold 2, one signature so far: refused, with
    // the file listed.
    keys_file(&cli.path("keys.json"), &[&public_a, &public_b], 2, &[]);
    let keys = cli.path("keys.json");
    let error = cli.verify(&digest, &["--keys", &keys]).unwrap_err();
    assert!(error.contains("refused"), "{error}");
    assert!(error.contains("1 valid signature(s)"), "{error}");
    assert!(error.contains("2 required"), "{error}");
    assert!(error.contains(&format!("counted  {file_a}")), "{error}");

    // Key B (the other job), into another directory first.
    std::fs::create_dir(cli.path("job-b")).unwrap();
    let out = cli
        .sign_with("publisher-b.key", &["--out-dir", &cli.path("job-b")])
        .unwrap();
    let file_b_job = cli
        .dir
        .path()
        .join("job-b")
        .join(format!("{PACKAGE}.{id_b}.minisig"))
        .display()
        .to_string();
    assert!(out.contains(&file_b_job), "{out}");
    // Combined next to the package.
    let file_b = cli.path(&format!("{PACKAGE}.{id_b}.minisig"));
    std::fs::copy(&file_b_job, &file_b).unwrap();

    // The pinned keys are the production keys: the throwaway keys'
    // signatures do not count; refused, and audited.
    let error = cli.verify(&digest, &[]).unwrap_err();
    assert!(error.contains("0 valid signature(s)"), "{error}");
    assert!(error.contains("not a pinned publisher key"), "{error}");
    assert!(error.contains("threshold of the pinned keys"), "{error}");

    // With the test keys and unpacking.
    let unpack = cli.path("unpacked");
    let out = cli
        .verify(&digest, &["--keys", &keys, "--unpack", &unpack])
        .unwrap();
    assert!(out.contains("deckpilot.alpha 1.2.0 verified"), "{out}");
    assert!(
        out.contains("signed by 2 distinct publisher key(s)"),
        "{out}"
    );
    assert!(out.contains("threshold 2 of the keys in"), "{out}");
    assert!(out.contains("not the pinned keys"), "{out}");
    assert!(out.contains("no permission is granted"), "{out}");
    assert!(
        out.contains(&format!("counted  {file_a}: key {id_a}")),
        "{out}"
    );
    assert!(
        out.contains(&format!("counted  {file_b}: key {id_b}")),
        "{out}"
    );
    assert!(Path::new(&unpack).join("plugin.wasm").is_file());
    assert!(Path::new(&unpack).join("manifest.json").is_file());

    let audit = std::fs::read_to_string(cli.path("audit.jsonl")).unwrap();
    let records: Vec<AuditRecord> = audit
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(records.len(), 3);
    assert_eq!(records[0].outcome, AuditOutcome::Refused);
    assert!(records[0].reason.as_deref().unwrap().contains("2 required"));
    assert_eq!(records[1].outcome, AuditOutcome::Refused);
    assert!(
        records[1]
            .reason
            .as_deref()
            .unwrap()
            .contains("publisher key")
    );
    assert_eq!(records[2].outcome, AuditOutcome::Verified);
    assert_eq!(records[2].signers.len(), 2);
    assert_eq!(records[2].sha256, digest.sha256.to_string());

    // --signatures-dir reads another directory (job B's has only B).
    let error = cli
        .verify(
            &digest,
            &["--keys", &keys, "--signatures-dir", &cli.path("job-b")],
        )
        .unwrap_err();
    assert!(error.contains("1 valid signature(s)"), "{error}");

    // A third, stray file: a copy of A's signature under a stranger's id,
    // and the old single name (never read): reported or skipped, and the
    // two valid keys still verify.
    let stranger = "0123456789ABCDEF";
    let stray = cli.path(&format!("{PACKAGE}.{stranger}.minisig"));
    std::fs::copy(&file_a, &stray).unwrap();
    std::fs::copy(&file_a, cli.path(&format!("{PACKAGE}.minisig"))).unwrap();
    let out = cli.verify(&digest, &["--keys", &keys]).unwrap();
    assert!(
        out.contains(&format!(
            "ignored  {stray}: the file is named for key {stranger}"
        )),
        "{out}"
    );
    assert!(!out.contains(&format!("{PACKAGE}.minisig")), "{out}");
    std::fs::remove_file(&stray).unwrap();

    // B's signature renamed to A's id, A's removed: one key, refused.
    std::fs::remove_file(&file_a).unwrap();
    std::fs::rename(&file_b, &file_a).unwrap();
    let error = cli.verify(&digest, &["--keys", &keys]).unwrap_err();
    assert!(error.contains("0 valid signature(s)"), "{error}");
    assert!(error.contains(&format!("ignored  {file_a}")), "{error}");
    std::fs::rename(&file_a, &file_b).unwrap();
    std::fs::copy(&file_b_job, cli.path("b-backup")).unwrap();

    // Revoking key A: B alone is below threshold 2 of [B, C] ...
    let public_c = cli.generate("publisher-c.key");
    keys_file(
        &cli.path("rotated.json"),
        &[&public_b, &public_c],
        2,
        &[&public_a],
    );
    let rotated = cli.path("rotated.json");
    cli.sign_with("publisher-a.key", &[]).unwrap();
    // ... and a valid signature by revoked A refuses the release.
    let error = cli.verify(&digest, &["--keys", &rotated]).unwrap_err();
    assert!(error.contains("revoked publisher key"), "{error}");
    std::fs::remove_file(&file_a).unwrap();
    cli.sign_with("publisher-c.key", &[]).unwrap();
    let out = cli.verify(&digest, &["--keys", &rotated]).unwrap();
    assert!(out.contains("signed by 2 distinct"), "{out}");

    // A claim that names other bytes.
    let wrong = PackageDigest {
        sha256: digest_of(b"x").sha256,
        size: digest.size,
    };
    assert!(
        cli.verify(&wrong, &["--keys", &rotated])
            .unwrap_err()
            .contains("the claim says")
    );

    // A higher version already shipped (released pins).
    let pins = serde_json::json!({
        "releasedPinsVersion": 1,
        "releases": [{ "release": "0.11.0", "platform": "macos", "source": "test",
                       "plugins": [{ "id": "deckpilot.alpha", "version": "1.3.0", "files": {} }] }]
    });
    std::fs::write(cli.path("released-pins.json"), pins.to_string()).unwrap();
    let released = cli.path("released-pins.json");
    let error = cli
        .verify(&digest, &["--keys", &rotated, "--released-pins", &released])
        .unwrap_err();
    assert!(error.contains("below 1.3.0"), "{error}");
    // Other platforms' pins do not count.
    let args = verify_args(
        &cli,
        &digest,
        &["--keys", &rotated, "--released-pins", &released],
    );
    let mut args: Vec<&str> = args.iter().map(String::as_str).collect();
    let at = args.iter().position(|a| *a == "macos").unwrap();
    args[at] = "linux";
    cli.run(&args).unwrap();
}

#[test]
fn cli_refuses_to_sign_what_could_never_verify() {
    let mut cli = Cli::new();
    cli.run(&[
        "keys",
        "generate",
        "--role",
        "publisher",
        "--out",
        &cli.path("publisher.key"),
        "--no-password",
    ])
    .unwrap();
    let bytes = package("acme.alpha", "1.0.0").bytes;
    std::fs::write(cli.path("acme.dpplugin"), &bytes).unwrap();
    let error = cli
        .run(&[
            "release",
            "sign",
            "--package",
            &cli.path("acme.dpplugin"),
            "--id",
            "acme.alpha",
            "--version",
            "1.0.0",
            "--key",
            &cli.path("publisher.key"),
        ])
        .unwrap_err();
    assert!(error.contains("refusing to sign"), "{error}");
    assert!(no_signature_files(&cli));

    let bytes = package("deckpilot.alpha", "1.2.0").bytes;
    std::fs::write(cli.path("alpha.dpplugin"), &bytes).unwrap();
    let sign = |cli: &Cli, id: &str, version: &str, key_env: &str| {
        cli.run(&[
            "release",
            "sign",
            "--package",
            &cli.path("alpha.dpplugin"),
            "--id",
            id,
            "--version",
            version,
            "--key-env",
            key_env,
        ])
    };
    // The tag's id and version, not the package's: a package that says
    // otherwise is refused, before the key is loaded (MISSING is never read).
    cli.env.clear();
    for (id, version) in [("deckpilot.alpha", "1.2.1"), ("deckpilot.beta", "1.2.0")] {
        let error = sign(&cli, id, version, "MISSING").unwrap_err();
        assert!(error.contains("refusing to sign"), "{error}");
        assert!(!error.contains("MISSING"), "{error}");
    }
    // A package that passes the preflight then needs the key.
    let error = sign(&cli, "deckpilot.alpha", "1.2.0", "MISSING").unwrap_err();
    assert!(error.contains("MISSING is not set"), "{error}");
    assert!(no_signature_files(&cli));
}

fn no_signature_files(cli: &Cli) -> bool {
    std::fs::read_dir(cli.dir.path()).unwrap().all(|e| {
        !e.unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".minisig")
    })
}

// ------------------------------------------------------------ the app's files

const ARTIFACTS: &str = include_str!("../../../plugins/first-party/artifacts.json");
const RELEASED_PINS: &str = include_str!("../../../plugins/first-party/released-pins.json");

/// The immutable release URL of `id` `version`.
fn lock_url(id: &str, version: &str) -> String {
    format!(
        "https://github.com/knittingcode/deckpilot-plugins/releases/download/\
         {id}--v{version}/{id}-{version}.dpplugin"
    )
}

/// An `artifacts.json` entry (`key_ids` signature files, a stand-in digest).
fn lock_entry(id: &str, version: &str, key_ids: &[String]) -> serde_json::Value {
    serde_json::json!({ "id": id, "version": version,
                        "sha256": digest_of(b"x").sha256.to_string(), "size": 1,
                        "url": lock_url(id, version), "keyIds": key_ids })
}

fn lock_doc(plugins: Vec<serde_json::Value>, revoked: serde_json::Value) -> String {
    let mut doc = serde_json::json!({ "artifactsVersion": 1 });
    doc["plugins"] = serde_json::Value::Array(plugins);
    doc["revoked"] = revoked;
    doc.to_string()
}

fn two_key_ids() -> Vec<String> {
    vec!["0123456789ABCDEF".into(), "FEDCBA9876543210".into()]
}

#[test]
fn the_artifact_lock_parses_and_locks_nothing_yet() {
    use deckpilot_plugin_registry::publisher::ArtifactLock;
    let lock = ArtifactLock::from_json(ARTIFACTS.as_bytes()).unwrap();
    // No plugin is consumed from a release artifact yet.
    assert_eq!(lock.plugins, Vec::new());
    assert_eq!(lock.revoked, Vec::new());

    let entry = lock_entry("deckpilot.github", "0.2.1", &two_key_ids());
    let one =
        ArtifactLock::from_json(lock_doc(vec![entry], serde_json::json!([])).as_bytes()).unwrap();
    assert_eq!(
        one.plugins[0].package_url(),
        "https://github.com/knittingcode/deckpilot-plugins/releases/download/\
         deckpilot.github--v0.2.1/deckpilot.github-0.2.1.dpplugin"
    );
    assert_eq!(
        one.plugins[0].signature_url("0123456789ABCDEF"),
        "https://github.com/knittingcode/deckpilot-plugins/releases/download/\
         deckpilot.github--v0.2.1/deckpilot.github-0.2.1.dpplugin.0123456789ABCDEF.minisig"
    );
    assert_eq!(one.plugins[0].claim().version, v("0.2.1"));
    assert_eq!(one.plugins[0].url, one.plugins[0].package_url());
    assert_eq!(one.plugins[0].key_ids, two_key_ids());
}

#[test]
fn the_artifact_lock_refuses_what_it_cannot_pin() {
    use deckpilot_plugin_registry::publisher::ArtifactLock;
    let github = "deckpilot.github";
    let entry = || lock_entry(github, "0.2.1", &two_key_ids());
    let with = |field: &str, value: serde_json::Value| {
        let mut entry = entry();
        entry[field] = value;
        entry
    };
    let url = lock_url(github, "0.2.1");
    let releases = "https://github.com/knittingcode/deckpilot-plugins/releases";
    let none = || serde_json::json!([]);
    for bad in [
        // A mutable or another release's URL, another repository, plain HTTP.
        lock_doc(
            vec![with(
                "url",
                format!("{releases}/latest/download/deckpilot.github-0.2.1.dpplugin").into(),
            )],
            none(),
        ),
        lock_doc(vec![with("url", lock_url(github, "0.2.2").into())], none()),
        lock_doc(
            vec![with(
                "url",
                url.replace("/knittingcode/", "/someone/").into(),
            )],
            none(),
        ),
        lock_doc(
            vec![with("url", url.replace("https:", "http:").into())],
            none(),
        ),
        // No, malformed or duplicate key ids; a missing field.
        lock_doc(vec![with("keyIds", serde_json::json!([]))], none()),
        lock_doc(
            vec![with("keyIds", serde_json::json!(["0123456789abcdef"]))],
            none(),
        ),
        lock_doc(
            vec![with(
                "keyIds",
                serde_json::json!(["0123456789ABCDEF", "0123456789ABCDEF"]),
            )],
            none(),
        ),
        lock_doc(
            vec![{
                let mut e = entry();
                e.as_object_mut().unwrap().remove("url");
                e
            }],
            none(),
        ),
        // Not first-party, locked twice, locked and revoked, format 2.
        lock_doc(
            vec![lock_entry("acme.github", "0.2.1", &two_key_ids())],
            none(),
        ),
        lock_doc(
            vec![entry(), lock_entry(github, "0.2.2", &two_key_ids())],
            none(),
        ),
        lock_doc(
            vec![entry()],
            serde_json::json!([{ "id": github, "versions": "*", "reason": "x" }]),
        ),
        lock_doc(vec![], none()).replace("\"artifactsVersion\":1", "\"artifactsVersion\":2"),
    ] {
        assert!(ArtifactLock::from_json(bad.as_bytes()).is_err(), "{bad}");
    }
}

#[test]
fn the_released_pins_give_the_rollback_floor() {
    use deckpilot_plugin_registry::publish::floor_from_released_pins;
    let pins = RELEASED_PINS.as_bytes();
    let floor = floor_from_released_pins(pins, &id("deckpilot.github"), Platform::Macos)
        .unwrap()
        .expect("0.9.0 shipped deckpilot.github on macOS");
    assert!(floor.version >= v("0.1.0"));
    assert!(floor.files.contains_key("plugin.wasm"));
    let unknown = floor_from_released_pins(pins, &id("deckpilot.unknown"), Platform::Macos);
    assert!(unknown.unwrap().is_none());
}

// ------------------------------------- bundled.json: from source or artifact

const BUNDLED: &str = include_str!("../../../plugins/first-party/bundled.json");

/// The bundled plugins the closed privileged-kinds gate refuses from a
/// signed artifact: they are built from source here until the gate's review
/// (docs/marketplace/publisher-review-request.md). Checked against the gate
/// for every plugin whose source is still in plugins/first-party/, and kept
/// for those whose source has gone.
const PRIVILEGED: [&str; 5] = [
    "deckpilot.docker",
    "deckpilot.git",
    "deckpilot.projects",
    "deckpilot.workflows",
    "deckpilot.workspace",
];

fn first_party_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/first-party")
}

/// A `bundled.json` source entry: its directory's manifest id, never also
/// locked as an artifact.
fn check_source_entry(
    entry: &serde_json::Value,
    lock: &deckpilot_plugin_registry::publisher::ArtifactLock,
    first_party: &Path,
) -> Result<String, String> {
    let (Some(path), None, None) = (
        entry["path"].as_str(),
        entry.get("id"),
        entry.get("version"),
    ) else {
        return Err(format!(
            "{entry}: a source entry names its directory (path) only"
        ));
    };
    let manifest = std::fs::read(first_party.join(path).join("manifest.json"))
        .map_err(|e| format!("plugins/first-party/{path}/manifest.json: {e}"))?;
    let manifest: serde_json::Value =
        serde_json::from_slice(&manifest).map_err(|e| format!("{path}: {e}"))?;
    let id = manifest["id"].as_str().unwrap_or_default().to_owned();
    if lock.plugins.iter().any(|a| a.id.as_str() == id) {
        return Err(format!(
            "{id} is built from source and locked as a release artifact: one or the other"
        ));
    }
    Ok(id)
}

/// A `bundled.json` artifact entry: locked with the same version, and not a
/// plugin the gate refuses: with the gate closed no privileged plugin; with
/// it open only a privileged plugin the reviewed policy lists at exactly the
/// locked version (`release verify` then checks the package itself against
/// the policy).
fn check_artifact_entry(
    entry: &serde_json::Value,
    lock: &deckpilot_plugin_registry::publisher::ArtifactLock,
    first_party: &Path,
    gate: PrivilegedGate<'_>,
) -> Result<String, String> {
    use deckpilot_plugin_registry::publisher::is_privileged;
    let (None, Some(id), Some(version)) = (
        entry.get("path"),
        entry["id"].as_str(),
        entry["version"].as_str(),
    ) else {
        return Err(format!(
            "{entry}: an artifact entry names id and version, no path"
        ));
    };
    let locked = lock
        .plugins
        .iter()
        .find(|a| a.id.as_str() == id)
        .ok_or_else(|| format!("{id} is an artifact that artifacts.json does not lock"))?;
    if locked.version.to_string() != version {
        return Err(format!(
            "bundled.json lists {id} {version}, artifacts.json locks {id} {}",
            locked.version
        ));
    }
    let name = id.strip_prefix("deckpilot.").unwrap_or(id);
    let gated = match std::fs::read(first_party.join(name).join("manifest.json")) {
        Ok(manifest) => is_privileged(&manifest).map_err(|e| format!("{id}: {e}"))?,
        Err(_) => false,
    };
    if gated || PRIVILEGED.contains(&id) {
        let PrivilegedGate::Open(reviewed) = gate else {
            return Err(format!(
                "{id} declares plugin API 1.9 privileged kinds: the gate is closed, it is built \
                 from source"
            ));
        };
        let policy = reviewed.policy();
        if !policy.is_approved() {
            return Err(format!(
                "{id} declares plugin API 1.9 privileged kinds: the privileged policy records no \
                 approved review"
            ));
        }
        match policy.plugins.iter().find(|p| p.id.as_str() == id) {
            Some(reviewed) if reviewed.version == locked.version => {}
            Some(reviewed) => {
                return Err(format!(
                    "{id} {version} declares plugin API 1.9 privileged kinds: the privileged \
                     policy reviewed {} only",
                    reviewed.version
                ));
            }
            None => {
                return Err(format!(
                    "{id} declares plugin API 1.9 privileged kinds the privileged policy does \
                     not list"
                ));
            }
        }
    }
    Ok(id.to_owned())
}

/// A locked artifact's key ids: pinned current keys, at least the threshold.
fn check_locked_keys(
    locked: &deckpilot_plugin_registry::publisher::LockedArtifact,
    keys: &PublisherKeys,
) -> Result<(), String> {
    if !keys.is_provisioned() {
        return Err(format!(
            "{} is locked, but no publisher key is pinned to verify it",
            locked.id
        ));
    }
    let pinned = keys.signature_key_ids();
    if let Some(unknown) = locked.key_ids.iter().find(|k| !pinned.contains(k)) {
        return Err(format!(
            "{}: keyIds {unknown} is not a pinned current key",
            locked.id
        ));
    }
    if u64::try_from(locked.key_ids.len()).unwrap_or(u64::MAX) < keys.threshold {
        return Err(format!(
            "{}: {} key id(s), the pinned threshold is {}",
            locked.id,
            locked.key_ids.len(),
            keys.threshold
        ));
    }
    Ok(())
}

/// The checks `plugins/first-party/build.sh` also makes before it builds or
/// downloads anything, here so that a required CI check fails on them:
/// each entry of `bundled` is built from source (`path`) or taken from a
/// release artifact (`id`, `version`, `"source": "artifact"`), never both;
/// every artifact is locked by `lock` with the same id and version, and
/// every locked artifact is listed; no privileged plugin is an artifact;
/// the locked key ids are pinned current keys, at least the threshold.
fn check_bundled(
    bundled: &serde_json::Value,
    lock: &deckpilot_plugin_registry::publisher::ArtifactLock,
    keys: &PublisherKeys,
    first_party: &Path,
    gate: PrivilegedGate<'_>,
) -> Result<(), String> {
    let mut ids = std::collections::BTreeSet::new();
    let mut artifacts = std::collections::BTreeSet::new();
    let entries = bundled["plugins"]
        .as_array()
        .ok_or("bundled.json has no plugins list")?;
    for entry in entries {
        let class = entry["class"].as_str().unwrap_or_default();
        if class != "default" && class != "migration" {
            return Err(format!("{entry}: class `{class}` (default or migration)"));
        }
        let id = match entry["source"].as_str().unwrap_or("source") {
            "source" => check_source_entry(entry, lock, first_party)?,
            "artifact" => {
                let id = check_artifact_entry(entry, lock, first_party, gate)?;
                artifacts.insert(id.clone());
                id
            }
            other => return Err(format!("{entry}: source `{other}` (source or artifact)")),
        };
        if !ids.insert(id.clone()) {
            return Err(format!("{id} is listed twice"));
        }
    }
    for locked in &lock.plugins {
        if !artifacts.contains(locked.id.as_str()) {
            return Err(format!(
                "artifacts.json locks {} {}, which bundled.json does not list as an artifact",
                locked.id, locked.version
            ));
        }
        check_locked_keys(locked, keys)?;
    }
    Ok(())
}

#[test]
fn the_privileged_list_is_the_gate_s() {
    use deckpilot_plugin_registry::publisher::is_privileged;
    let mut gated = std::collections::BTreeSet::new();
    let mut seen = std::collections::BTreeSet::new();
    for entry in std::fs::read_dir(first_party_dir()).unwrap() {
        let dir = entry.unwrap().path();
        let Ok(manifest) = std::fs::read(dir.join("manifest.json")) else {
            continue;
        };
        let id = serde_json::from_slice::<serde_json::Value>(&manifest).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        if is_privileged(&manifest).unwrap() {
            gated.insert(id.clone());
        }
        seen.insert(id);
    }
    let listed: std::collections::BTreeSet<String> = PRIVILEGED
        .iter()
        .map(|s| (*s).to_owned())
        .filter(|id| seen.contains(id))
        .collect();
    assert_eq!(
        gated, listed,
        "update PRIVILEGED to the plugins the gate refuses"
    );
    for open in [
        "deckpilot.github",
        "deckpilot.gitlab",
        "deckpilot.jira",
        "deckpilot.notifications",
    ] {
        assert!(seen.contains(open) && !gated.contains(open), "{open}");
    }
}

#[test]
fn every_bundled_plugin_is_built_from_source_or_a_locked_artifact() {
    use deckpilot_plugin_registry::publisher::ArtifactLock;
    let lock = ArtifactLock::from_json(ARTIFACTS.as_bytes()).unwrap();
    let bundled: serde_json::Value = serde_json::from_str(BUNDLED).unwrap();
    let pinned = PublisherKeys::pinned().unwrap();
    check_bundled(
        &bundled,
        &lock,
        &pinned,
        &first_party_dir(),
        PrivilegedGate::compiled(),
    )
    .unwrap();
}

#[test]
fn bundled_json_and_the_lock_must_agree() {
    use deckpilot_plugin_registry::publisher::ArtifactLock;
    // Throwaway keys and lock entries.
    let a = ThrowawayKey::generate();
    let b = ThrowawayKey::generate();
    let two = keys(&[&a, &b], 2, &[]);
    let unpinned = keys(&[], 1, &[]);
    let artifact = |id: &str, version: &str| serde_json::json!({ "id": id, "version": version, "class": "default", "source": "artifact" });
    let source = |path: &str| serde_json::json!({ "path": path, "class": "migration" });
    let both = [a.key_id(), b.key_id()];
    let check =
        |plugins: Vec<serde_json::Value>, lock: Vec<serde_json::Value>, keys: &PublisherKeys| {
            let lock =
                ArtifactLock::from_json(lock_doc(lock, serde_json::json!([])).as_bytes()).unwrap();
            check_bundled(
                &serde_json::json!({ "plugins": plugins }),
                &lock,
                keys,
                &first_party_dir(),
                PrivilegedGate::Closed,
            )
        };
    let n = "deckpilot.notifications";
    let locked_n = || vec![lock_entry(n, "0.1.3", &both)];
    check(
        vec![source("github"), artifact(n, "0.1.3")],
        locked_n(),
        &two,
    )
    .unwrap();
    for (plugins, lock, keys, expected) in [
        // The lock and bundled.json disagree on the version or the id.
        (
            vec![artifact(n, "0.1.4")],
            locked_n(),
            &two,
            "artifacts.json locks",
        ),
        (
            vec![artifact(n, "0.1.3")],
            vec![lock_entry("deckpilot.jira", "0.1.3", &both)],
            &two,
            "does not lock",
        ),
        (vec![], locked_n(), &two, "does not list"),
        // Never both.
        (
            vec![source("notifications")],
            locked_n(),
            &two,
            "one or the other",
        ),
        (
            vec![artifact(n, "0.1.3"), source("notifications")],
            locked_n(),
            &two,
            "one or the other",
        ),
        // A privileged plugin.
        (
            vec![artifact("deckpilot.git", "0.1.1")],
            vec![lock_entry("deckpilot.git", "0.1.1", &both)],
            &two,
            "privileged",
        ),
        (
            vec![artifact("deckpilot.workflows", "0.1.1")],
            vec![lock_entry("deckpilot.workflows", "0.1.1", &both)],
            &two,
            "privileged",
        ),
        // Keys: none pinned, unknown, below the threshold.
        (
            vec![artifact(n, "0.1.3")],
            locked_n(),
            &unpinned,
            "no publisher key",
        ),
        (
            vec![artifact(n, "0.1.3")],
            vec![lock_entry(
                n,
                "0.1.3",
                &[a.key_id(), "0123456789ABCDEF".into()],
            )],
            &two,
            "not a pinned",
        ),
        (
            vec![artifact(n, "0.1.3")],
            vec![lock_entry(n, "0.1.3", &[a.key_id()])],
            &two,
            "threshold is 2",
        ),
    ] {
        let error = check(plugins, lock, keys).unwrap_err();
        assert!(error.contains(expected), "{expected}: {error}");
    }
}

/// With the gate opened (in this test only), a privileged plugin may be
/// listed as a release artifact only at exactly the version the approved
/// privileged policy reviewed; closed, or with a pending review, never.
#[test]
fn a_privileged_artifact_entry_needs_the_open_gate_and_the_reviewed_version() {
    use deckpilot_plugin_registry::privileged_policy::{
        PrivilegedPolicy, Review, ReviewOutcome, compiled_policy,
    };
    use deckpilot_plugin_registry::publisher::ArtifactLock;
    let a = ThrowawayKey::generate();
    let b = ThrowawayKey::generate();
    let two = keys(&[&a, &b], 2, &[]);
    let both = [a.key_id(), b.key_id()];
    let with_review = |outcome| {
        let mut policy: PrivilegedPolicy = compiled_policy().unwrap().clone();
        policy.review = Some(Review {
            reviewer: "throwaway test reviewer".into(),
            date: "2026-11-21".into(),
            outcome,
            record: "tests/publisher.rs".into(),
        });
        policy
    };
    let approved = with_review(ReviewOutcome::Approved);
    let pending = with_review(ReviewOutcome::Pending);
    let check = |id: &str, version: &str, gate: PrivilegedGate<'_>| {
        let lock = ArtifactLock::from_json(
            lock_doc(vec![lock_entry(id, version, &both)], serde_json::json!([])).as_bytes(),
        )
        .unwrap();
        let plugins = serde_json::json!({ "plugins": [
            { "id": id, "version": version, "class": "default", "source": "artifact" }
        ]});
        // The privileged plugin's own source entry is left out: one or the
        // other.
        check_bundled(&plugins, &lock, &two, &first_party_dir(), gate)
    };
    for id in PRIVILEGED {
        check(id, "0.1.1", PrivilegedGate::open_for_tests(&approved)).unwrap();
        for (version, gate, expected) in [
            ("0.1.1", PrivilegedGate::Closed, "the gate is closed"),
            (
                "0.1.1",
                PrivilegedGate::open_for_tests(&pending),
                "no approved review",
            ),
            (
                "0.1.2",
                PrivilegedGate::open_for_tests(&approved),
                "reviewed 0.1.1 only",
            ),
        ] {
            let error = check(id, version, gate).unwrap_err();
            assert!(error.contains(expected), "{id} {version}: {error}");
        }
    }
    // A privileged plugin the policy does not list.
    let mut without = approved.clone();
    without.plugins.retain(|p| p.id.as_str() != "deckpilot.git");
    let error = check(
        "deckpilot.git",
        "0.1.1",
        PrivilegedGate::open_for_tests(&without),
    )
    .unwrap_err();
    assert!(error.contains("does not list"), "{error}");
}

#[test]
#[allow(clippy::too_many_lines)] // one flow, step by step
fn cli_key_test_signs_only_the_test_message_and_release_verify_refuses_it() {
    let mut cli = Cli::new();
    let public_a = cli.generate("publisher-a.key");
    let public_b = cli.generate("publisher-b.key");
    let (id_a, id_b) = (key_id_of(&public_a), key_id_of(&public_b));
    keys_file(&cli.path("keys.json"), &[&public_a, &public_b], 2, &[]);
    let keys = cli.path("keys.json");
    for dir in ["job-a", "job-b", "verify"] {
        std::fs::create_dir(cli.path(dir)).unwrap();
    }
    let secret = std::fs::read_to_string(cli.path("publisher-a.key")).unwrap();
    cli.env.insert("DECKPILOT_PUBLISHER_KEY_A".into(), secret);

    // Run numbers: 1 to 9999, canonical.
    for bad in ["0", "01", "10000", "-1", "1.0", "x", ""] {
        let error = cli
            .run(&[
                "release",
                "test-message",
                "--run",
                bad,
                "--out-dir",
                &cli.path("verify"),
            ])
            .unwrap_err();
        assert!(error.contains("--run"), "{bad}: {error}");
    }
    // No other input is accepted: only the fixed message is signed.
    let error = cli
        .run(&[
            "release",
            "test-sign",
            "--run",
            "1",
            "--key-env",
            "DECKPILOT_PUBLISHER_KEY_A",
            "--out-dir",
            &cli.path("job-a"),
            "--package",
            &cli.path("x"),
        ])
        .unwrap_err();
    assert!(error.contains("unknown option --package"), "{error}");

    let out = cli
        .run(&[
            "release",
            "test-sign",
            "--run",
            "1",
            "--key-env",
            "DECKPILOT_PUBLISHER_KEY_A",
            "--out-dir",
            &cli.path("job-a"),
        ])
        .unwrap();
    assert!(out.contains(&format!("publisher key {id_a}")), "{out}");
    assert!(out.contains("never a release signature"), "{out}");
    let name = "deckpilot-publisher-test-v1.txt";
    let file_a = cli.path(&format!("job-a/{name}.{id_a}.minisig"));
    assert!(Path::new(&file_a).is_file());
    let error = cli
        .run(&[
            "release",
            "test-sign",
            "--run",
            "1",
            "--key-env",
            "DECKPILOT_PUBLISHER_KEY_A",
            "--out-dir",
            &cli.path("job-a"),
        ])
        .unwrap_err();
    assert!(error.contains("refusing to overwrite"), "{error}");
    let key_b = cli.path("publisher-b.key");
    cli.run(&[
        "release",
        "test-sign",
        "--run",
        "1",
        "--key",
        &key_b,
        "--out-dir",
        &cli.path("job-b"),
    ])
    .unwrap();
    let file_b = cli.path(&format!("job-b/{name}.{id_b}.minisig"));

    // One signature: below the threshold.
    std::fs::copy(&file_a, cli.path(&format!("verify/{name}.{id_a}.minisig"))).unwrap();
    let verify = cli.path("verify");
    let error = cli
        .run(&[
            "release",
            "test-verify",
            "--run",
            "1",
            "--signatures-dir",
            &verify,
            "--keys",
            &keys,
        ])
        .unwrap_err();
    assert!(error.contains("1 valid signature(s)"), "{error}");
    std::fs::copy(&file_b, cli.path(&format!("verify/{name}.{id_b}.minisig"))).unwrap();
    let out = cli
        .run(&[
            "release",
            "test-verify",
            "--run",
            "1",
            "--signatures-dir",
            &verify,
            "--keys",
            &keys,
        ])
        .unwrap();
    assert!(out.contains("key test 1 verified: 2 distinct"), "{out}");
    assert!(out.contains("not the pinned keys"), "{out}");
    assert!(
        out.contains(&format!(
            "counted  {}",
            Path::new(&verify)
                .join(format!("{name}.{id_a}.minisig"))
                .display()
        )),
        "{out}"
    );
    // Another run: refused.
    let error = cli
        .run(&[
            "release",
            "test-verify",
            "--run",
            "2",
            "--signatures-dir",
            &verify,
            "--keys",
            &keys,
        ])
        .unwrap_err();
    assert!(error.contains("refused"), "{error}");
    // The pinned (production) keys: the throwaway keys do not count.
    let error = cli
        .run(&[
            "release",
            "test-verify",
            "--run",
            "1",
            "--signatures-dir",
            &verify,
        ])
        .unwrap_err();
    assert!(error.contains("threshold of the pinned keys"), "{error}");
    assert!(error.contains("0 valid signature(s)"), "{error}");

    // The message, and `release verify` refusing it as a release of a real
    // plugin id and version, with the same two keys pinned.
    let out = cli
        .run(&[
            "release",
            "test-message",
            "--run",
            "1",
            "--out-dir",
            &verify,
        ])
        .unwrap();
    let message = std::fs::read(cli.path(&format!("verify/{name}"))).unwrap();
    assert_eq!(message, test_message(1));
    let digest = digest_of(&message);
    assert!(out.contains(&digest.sha256.to_string()), "{out}");
    let as_release = "deckpilot.notifications-0.1.2.dpplugin";
    std::fs::write(cli.path(&format!("verify/{as_release}")), &message).unwrap();
    for (id, file) in [(&id_a, &file_a), (&id_b, &file_b)] {
        std::fs::copy(file, cli.path(&format!("verify/{as_release}.{id}.minisig"))).unwrap();
    }
    let error = cli
        .run(&[
            "release",
            "verify",
            "--package",
            &cli.path(&format!("verify/{as_release}")),
            "--id",
            "deckpilot.notifications",
            "--version",
            "0.1.2",
            "--sha256",
            &digest.sha256.to_string(),
            "--size",
            &digest.size.to_string(),
            "--app-version",
            "0.10.0",
            "--plugin-api",
            "1.9",
            "--platform",
            "macos",
            "--keys",
            &keys,
        ])
        .unwrap_err();
    assert!(error.contains("refused"), "{error}");
    assert!(error.contains("0 valid signature(s)"), "{error}");
    assert!(
        error.contains("deckpilot-publisher-test v1 role:test"),
        "{error}"
    );
}
