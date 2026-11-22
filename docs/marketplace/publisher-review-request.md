# Security review request: first-party publisher signature

Status: **approved** on 2026-11-21 (Sign-off below; report and
remediation: [reviews/2026-11-21-privileged-gate-review.md](reviews/2026-11-21-privileged-gate-review.md)).
The privileged-kinds gate is open for exactly the reviewed privileged
policy.

Current state (2026-11-22): all nine official plugins were released on
2026-11-21 and each is signed by both production keys (threshold 2):
`deckpilot.github` 0.2.1; `deckpilot.gitlab`, `deckpilot.jira` and
`deckpilot.notifications` 0.1.2; `deckpilot.workflows`, `deckpilot.git`,
`deckpilot.docker`, `deckpilot.projects` and `deckpilot.workspace` 0.1.1.
The production key test passed (`official-test-sign.yml` run
44954714900 on `deckpilot.publisher-test--v1`). `official-release` uses
the tag-authority model of the single-maintainer exception (below). The
dated updates that follow are kept as the record of how it got there.

Update 2026-11-21: on the owner's decision, the two production publisher
public keys are pinned (threshold 2,
[publisher-signature.md](publisher-signature.md#provisioning-2026-11-21));
they ship with the next application release. That does not open the gate
and needs no sign-off for the closed state: verification stays fail
closed, the artifact lock is empty, and nothing has been released with
the keys. Both secret keys are held by one person in one 1Password vault
of one account: not independent custody, an accepted risk of the
single-maintainer exception (listed under K).

Update 2026-11-21 (release governance): DeckPilot is a single-maintainer
project. On the owner's decision, official releases run under a
documented single-maintainer exception to the team-managed default policy
([publisher-signature.md](publisher-signature.md#release-governance),
`docs/release-governance.md` in the plugin repository): the owner is the
sole release approver, and the admin-only release tag authorizes the run
(tag authority, as for `registry-release`). Threshold 2, the two signing
jobs, the keyless `verify` and every check are unchanged. This does not
change the privileged-kinds gate or this review's scope: the gate opened
only on the approved sign-off below. Live since 2026-11-21: in
`official-release`, there is no required reviewer, administrators cannot
bypass it, deployments come only from `deckpilot.*--v*` tags, and the
reviewer variables are removed.

Update 2026-11-21 (privileged plugins): everything needed to distribute
the five privileged plugins as signed artifacts is implemented **with the
gate closed**: the reviewed privileged policy (the record of exactly what
each plugin's reviewed version may declare), its check in every tool, and
the tests with the gate opened in tests only
([publisher-signature.md](publisher-signature.md#privileged-plugins-the-reviewed-privileged-policy)).
Opening the gate is a separate **draft** pull request in each repository
that only flips the gate constant and fills in the review record; it
needs this review's sign-off (item L of [the host side](#host-side-the-agents-install-and-update-path))
and the owner's approval, and is not merged before both.

**The opening change** (knittingcode/deckpilot#388) sets
`PRIVILEGED_GATE_OPEN = true` and records the approved review in
`publisher/privileged-policy.json`, on the owner's decision of 2026-11-21
after the final independent review ("yes, with conditions", every finding
fixed: [reviews/2026-11-21-privileged-gate-review.md](reviews/2026-11-21-privileged-gate-review.md)).
A pending or incomplete review would keep `PrivilegedGate::compiled()`
closed (the tests assert both states). Merge order: this change first,
then a DeckPilot release that carries it, then the plugin repository's
`tools/deckpilot.env` pin to that release, and only then the plugin
repository's opening draft (whose `check --deckpilot` refuses an open
gate while the pinned DeckPilot lacks the same record).

## Scope

The verification of signed first-party release artifacts
([publisher-signature.md](publisher-signature.md)) and the release
pipeline of `knittingcode/deckpilot-plugins` that produces them.

| Area | Files |
| --- | --- |
| Verifier | `crates/deckpilot-plugin-registry/src/publisher.rs`, `src/sig.rs` (reused) |
| Signing and CLI | `crates/deckpilot-plugin-registry/src/publish/release.rs`, `src/publish/keys.rs` (`--role publisher`, `from_text`), `src/publish/cli.rs` (`release sign`, `release verify`, and the key test `release test-message`, `test-sign`, `test-verify`) |
| Pinned keys and lock | `crates/deckpilot-plugin-registry/publisher/first-party.json`, `plugins/first-party/artifacts.json` |
| Privileged gate and policy | `crates/deckpilot-plugin-registry/src/privileged_policy.rs`, `publisher/privileged-policy.json` (the record), `src/publish/pack.rs` (`package --first-party`), `deckpilot-plugin-package` `PackageBuilder::reviewed_privileged`, `Manifest::privileged_usages`, `plugins/first-party/artifacts.sh` (the `privileged:` line), `tests/privileged.rs` |
| Tests | `crates/deckpilot-plugin-registry/tests/publisher.rs`, the unit tests in `publisher.rs` |
| Agent (host side) | `apps/agent/src/plugins/official.rs`, its tests, and the calls in `service.rs`, `registry.rs`, `fetch.rs` (checklist below) |
| Plugin repository | `official/`, `official/privileged-policy.json` (the record's copy), `tools/official.sh`, `tools/check-official.py`, `.github/workflows/official.yml`, `official-release.yml`, `official-test-sign.yml` (the key test) |

## What the reviewer must check

1. **Trust anchor.** Keys come only from the compiled-in
   `publisher/first-party.json`; nothing in an artifact, manifest, id,
   URL, repository name or environment selects or adds a key. `--keys`
   is a tool option only, and the output says it is not the pinned keys.
   Is it acceptable for that option to exist in a release-built CLI?
2. **Signature binding.** The trusted comment binds id, version, sha256
   and size; minisign's global signature covers it; `sig::trusted_comment`
   refuses anything after the four signature lines. Look for replay of a
   valid signature onto another version, plugin or file, and for
   confusion with registry (`deckpilot-registry v1 role:`) or updater
   signatures, in both directions.
3. **Order of checks.** The claim's size and sha256 are checked before any
   parsing; the archive is parsed only after a valid signature. Confirm
   no untrusted byte reaches the tar or gzip reader first.
4. **Threshold and signature files.** One file per signing key,
   `<package>.<KEYID>.minisig` (`signature_file_name`, `key_id_in_file_name`,
   `is_key_id`; `KEYID` as minisign prints it: little-endian key number,
   16 uppercase hex). In `check_signatures` / `check_file` confirm that a
   file counts only for the one pinned current key whose id is both in its
   name and inside its signature, at most once per key; that duplicates
   under another name, two files of one key, unknown keys, mismatched
   names, wrong comments, malformed and oversized files never count; that
   fewer than `threshold` distinct keys refuses (no partial or
   warning-only outcome); that `validate` refuses a threshold of 0 or
   above the number of current keys and a key id naming two keys; and that
   nothing at run time lowers the threshold. `release sign` uses one key
   per invocation; check that the release workflow signs A and B in
   separate jobs (no job holds both secrets) and that the combining job
   runs `release verify` against the pinned keys.
5. **Revocation.** `revokedKeys` beats any other valid signature;
   revoked releases in the lock are refused; a lock cannot both lock and
   revoke a release. Is build-time revocation (no run-time list) enough
   for artifacts that are re-pinned in the agent anyway?
6. **Rollback.** The floor is the highest version in
   `released-pins.json` for the target platform; equal versions need equal
   files. Check the "other platforms do not count" rule and what happens
   when a platform has no recorded pins.
7. **Authenticity vs. authorization.** Confirm that nothing derives a
   grant, tier or the bundled source from a signature: a verified
   artifact becomes a bundled plugin only through a reviewed
   `artifacts.json` change and the pins compiled into the agent, and the
   registry and sideload paths (X12, `is_bundled_id`) are untouched.
8. **The gate.** `check_gate` refuses every 1.9 privileged kind
   (`IssueCode::FirstPartyOnly`), process execution (`tools`) included.
   Decide whether the gate may open, and under which additional
   conditions (the opening path already needs at least 2 distinct keys
   that actually signed; for example also a hardware-backed key, or
   reproducible-build attestation for Git and Docker).
9. **Key handling.** The two secret keys are created offline by the
   owner, stored only as secrets of the `official-release` environment of
   deckpilot-plugins (`DECKPILOT_PUBLISHER_KEY_A`,
   `DECKPILOT_PUBLISHER_KEY_A_PASSWORD`, `DECKPILOT_PUBLISHER_KEY_B`,
   `DECKPILOT_PUBLISHER_KEY_B_PASSWORD`), each passed to its own
   `release sign --key-env … --password-env …` job (never written to
   disk), and never available to pull-request workflows. Review the environment protection rules
   (tags `deckpilot.*--v*` only, no administrator bypass, authorized by the
   admin-only tag ruleset or a required reviewer: the plugin repository's
   `tools/check-environment.sh official-release`) and that no workflow in
   `knittingcode/deckpilot` can sign (`check-release-separation.py`).
10. **Release pipeline.** `official.yml` runs untrusted pull-request code
   with no secret and throwaway keys only; `official-release.yml` runs
   only on a release tag, in the protected environment, builds from the
   tagged commit with the pinned toolchain and `Cargo.lock`, and refuses
   to sign what the verifier would refuse (`release sign` runs the
   verifier first).
11. **Reproducibility.** A release must be rebuildable bit for bit from
    the tag (two clean builds compared in CI). Decide whether the
    application build should rebuild and compare instead of, or in
    addition to, trusting the signature.

## Host side: the agent's install and update path

Scope: `apps/agent/src/plugins/official.rs` and `official/tests.rs`; the
calls in `apps/agent/src/plugins/service.rs` (`pins`, `fetch`, `update`,
`apply_verdicts`, `open`); `Registry::signature` (`registry.rs`) and
`HttpsSource::signature` (`fetch.rs`); the end-to-end test
`first_party_plugins_need_the_root_authorization_and_a_verified_publisher`
(`apps/agent/tests/plugins_registry_https.rs`). Design:
[publisher-signature.md](publisher-signature.md#the-agent-installing-and-updating-official-plugins).
The reviewer must not be the author; check each item against the code,
not this list, and record failures as findings.

A. **Every registry download of an official id is publisher-verified.**
   0. `Official::is_official` (the nine `BUNDLED_PLUGIN_IDS`, the compiled
      lock's ids, the running bundle's pins; never the `deckpilot.` prefix)
      decides; first-party community `deckpilot.*` plugins follow the
      registry's rules alone and never get privileged kinds.
   1. `is_official` gates `Official::verify` in `PluginService::fetch`,
      and every path that installs or updates from a registry goes through
      `fetch` (`install_steps`, `update`, `dependencies.rs`
      `verify_downloads`). Search for other callers of
      `Registry::package`, `manager.install`, `manager.update` with a
      registry source.
   2. Without provisioned keys, a valid lock and readable floors,
      `pins` refuses before any byte is downloaded (`Official::ready`).
      The production keys are pinned now, so `ready` passes in a release
      build; check that the download path still needs a threshold-2
      verification by the pinned keys
      (`this_build_pins_the_production_keys_and_counts_no_other`).
   3. File and dev installs of `deckpilot.*` stay refused
      (`InstallSource::allows`, `install_steps`).
B. **The claim and the trust anchor.** The claim is the registry's
   verified pin (`target.expected`), never the package's own manifest; the
   keys are `PublisherKeys::pinned()` except in `test-hooks` builds
   (`TEST_KEYS_ENV`). Confirm no release build enables `test-hooks`
   (agent `Cargo.toml`, `release.yml`, `deckpilot-dev package`), and that
   nothing in an artifact, URL, registry document or setting selects a key.
C. **Order and bounds.** `PluginService::official_signatures` downloads
   exactly one file per pinned current key id
   (`Official::signature_key_ids`, never a revoked key's), each capped at
   `MAX_SIGNATURE_BYTES`, from the package's base URL; missing files are
   tolerated only while the threshold can still be met, otherwise the
   install is refused before verification and audited; `verify_release` checks size and sha256 before parsing,
   signatures before the archive; `verify_package` then re-checks against
   the registry's manifest hash and permissions; the store receives only
   that `VerifiedPackage`.
D. **Rollback floor.** `effective_floor` takes the highest of the
   recorded floor, the compiled bundled pin and the compiled
   `released-pins.json` floor for this platform; equal versions with other
   files are refused; `Compiled::check_locked` adds the artifact lock's
   minimum; `record_release` persists before the store acts and never
   lowers a floor; a corrupt or unknown-version `floors.json` refuses everything
   and is never rewritten. Decide whether a local file (outside the
   encrypted `plugins.db`) is an acceptable home: deleting it resets only
   the recorded floors, the bundled pins still apply.
E. **Revocation at run time.** `revoked_installed` covers registry copies
   of `deckpilot.*` only, by compiled lock and by revoked signer of the
   recorded release; `apply_verdicts` runs at every start, also without a
   registry (quarantine only), and after every refresh with the registry's
   verdicts underneath (`official::Verdicts`); `check_verdict` (enable,
   every invocation) and `rollback` apply the official revocations too.
   Signers are recorded for every verified version; a registry copy whose
   version has no record, or whose record cannot be read, is not run
   (fail closed), except a retained version whose installed files match the
   compiled bundled pin (`bundled_match`). A `floors.json` with unknown
   fields is read but never rewritten (`read_floors`, `covered`).
   Check that a quarantined plugin cannot be enabled, and
   that an official revocation cannot be lifted by a registry `Run`
   verdict while the agent still revokes it.
F. **Authenticity is not authorization.** Nothing in `official.rs` writes
   a grant, consent, tier or source. The store's install and update get
   the user's consent only; a transition from the bundle derives nothing
   (unlike seeding); escalations stay staged (`AwaitingConsent`). Confirm
   `decided_by` for any grant touched by this path is `user`.
G. **The gate.** The agent's gate is the registry crate's
   `PrivilegedGate::compiled()` (`PRIVILEGED_GATE_OPEN` is `false`), read
   in `Official::with` only; the registry delivery verifies with the gate
   closed, and `admit_privileged` returns the original refusal unless the
   gate is open and every condition holds (an approved review in the
   policy; at least 2 distinct pinned keys that actually signed, at least
   the policy's `minThreshold` and the pinned threshold; the registry's
   manifest hash and permissions; exactly the reviewed manifest,
   `privileged_policy::check`), and then uses only the bundled reader.
   The store still refuses every privileged construct from a non-bundled
   source (`check_privileged_source`), and `check_gate` refuses an
   unparsable manifest. Even an admitted registry delivery is refused
   (`Refusal::BundledOnly`), audited as refused, before any floor is
   raised. `PrivilegedGate::Open` holds a `ReviewedGate` that only
   `compiled()` and the test-only `open_for_tests` (feature
   `test-privileged-gate`, dev-dependencies only) can build; confirm the
   feature never reaches a release build, and check the heuristics of the
   lint `only_tests_construct_an_open_gate` (imports, globs, multi-line
   constructions, `*_with_gate` and `.with_gate` calls).
H. **Transition.** `check_transition` runs only for a record whose source
   is `bundled`, before `trial_load`; it refuses dropped action ids,
   settings and credential slots. The bundled copy stays active on every
   refusal and through crash recovery (`an_interrupted_transition_recovers_and_retries`).
   Seeding leaves a registry copy alone; `restoreBundledPlugin` still
   restores the bundle after an uninstall.
I. **Audit log.** Every verification (accepted or refused, including
   unparsable signatures and floor failures), every refusal in `pins`
   before a download and every missing or undownloadable signature writes
   one chained line; five rotated generations are kept; a write failure is
   logged and does not change the outcome. The limits are documented in
   publisher-signature.md ("Limits of the audit log"). Decide
   whether tamper-evidence (hash chain, head hash in the agent log) is
   enough or a signed or remote log is required.
J. **Migration.** `migration.rs` derives grants only onto a bundled copy;
   `PluginManager::derive_grants` refuses any other source.
K. **Known open items** (not blockers for the closed state):
   `release verify --keys` exists in release builds of the CLI (Agent A's
   finding 3; the plugin repository's CI relies on it with throwaway
   keys). (The former item "only one `.minisig` per release is fetched" is
   resolved: one `<package>.<KEYID>.minisig` per pinned key, threshold 2
   reachable at run time.)
   Key custody: both production secret keys are held by one person in one
   1Password vault of one account; threshold 2 protects against one leaked
   secret or signing job, not against that account. No independent
   custody: **accepted risk (single maintainer)**, the owner's decision of
   2026-11-21. Likewise the earlier finding M5 (someone other than the
   person who pushed the tag approves the signing run): accepted risk
   (single maintainer). Both end with the transition to team governance
   ([publisher-signature.md](publisher-signature.md#release-governance)).
   The key test (`release test-sign`, `test-verify`): confirm that a
   test signature never counts for a release (another trusted comment
   domain; `a_key_test_signature_is_never_a_release_signature`) and that
   `test-sign` signs nothing but the fixed message.

L. **Privileged plugins: the checklist for opening the gate.** Review
   these against the code and the draft pull requests (DeckPilot and
   deckpilot-plugins, titled "requires the owner's approval and the
   independent security review"), not against this list. Every item must
   hold before the outcome `approved` is recorded:
   1. **Delivery path.** Privileged plugins reach users only as bundled
      copies staged by `artifacts.sh` from a locked, verified artifact and
      pinned in the agent. A registry copy is refused with the gate open:
      `check_privileged_source` for every non-bundled source, the agent's
      registry verification with `gate: Closed`, `admit_privileged` only
      as a second lock (`the_opening_path_needs_the_review_two_keys_and_exactly_the_policy`
      ends with the store's refusal). Search for any other caller that
      installs a privileged package with a non-bundled source.
   2. **The record.** `publisher/privileged-policy.json`: for each of the
      five plugins, the one version, the exact `permissions` and every
      construct with its value. Read each entry against the plugin's
      source and behaviour (its `src/` in `knittingcode/deckpilot-plugins`
      `official/<name>/` at the release tag) and decide whether those
      exact privileges are acceptable: the git and docker subcommands and
      options against X3's allow-lists and denylists, `network`
      (`ls-remote`) and `modify` (compose up/down/restart) as separate
      grants, `resources` (`paths`, `open`), `projectContext.select`,
      `openUrl.fromParams` for Workspace, host views, status items and
      dashboard cards for Projects. Confirm that
      `the_policy_is_exactly_what_the_sources_declare` ties the record to
      `plugins/first-party/<name>/manifest.json`, and that the plugin
      repository's copy is byte-identical (`check-official.py --deckpilot`).
   3. **Exactness.** `privileged_policy::check` admits only exactly the
      record: permissions by JSON equality in order; constructs by path,
      label and value, none added, none dropped; the claimed id and
      version equal to the manifest's and the record's. Try to find a
      manifest change that broadens what a plugin can do without changing
      `permissions` or a privileged construct (for example a new action,
      setting, view or locale that the host treats as privileged); if one
      exists, the record must cover it before the gate opens.
   4. **Process allow-list.** `PROCESS_ALLOW_LIST` (git for
      `deckpilot.git`, docker for `deckpilot.docker`, nothing else) is
      enforced when the policy parses, independently of the record; the
      host's own X3 limits are unchanged (argument arrays, no shell, the
      bound project folder as working directory, time and output limits,
      consent, audit).
   5. **Every tool, one rule.** `package --first-party`
      (`pack::package_with_gate`), `release sign` (`preflight`, then
      `check_signed` after the key is dropped), `release verify` and
      `verify_release` (at least 2 distinct pinned keys that signed, and a
      pinned threshold of at least 2, for a privileged release),
      `artifacts.sh` (exactly one `privileged:` line, `reviewed: <id>
      <version> …` for a privileged artifact), the *Rust* check of
      `bundled.json` artifact entries (the reviewed version only), the
      plugin repository's `check-official.py`, and the agent. All take
      the compiled gate; the `*_with_gate` functions exist for tests.
   6. **Fail closed.** `PrivilegedGate::compiled()` stays closed with the
      constant `false`, with an invalid policy, and with a review that is
      missing or `pending`. Nothing at run time or build time (flag,
      environment, setting, key, file) changes it. The release CLI's
      `--keys` (item K) cannot admit a privileged release signed by
      production keys alone; with test keys `build.sh` marks the bundle
      `NOT-FOR-RELEASE` and `check-bundled-plugins.sh` refuses it.
   7. **Grants, migration, rollback.** A staged privileged copy is an
      ordinary bundled plugin: seeding derives low-tier grants only;
      process execution and the other high-tier grants are only the
      user's (consent on first use, every run audited); an update keeps a
      grant only where still declared and covered, stages anything broader
      while the old copy runs; the migration engine derives onto bundled
      copies only (J); the bundled copy never goes backwards, and a
      release below `released-pins.json` or the bundled pin, or the same
      version with other files, is refused
      (`a_reviewed_artifact_updates_the_bundled_copy_under_the_store_s_consent_rules`,
      `an_unchanged_reviewed_artifact_keeps_every_grant`,
      `reviewed_releases_still_meet_revocation_rollback_and_compatibility`).
   8. **The draft changes nothing else.** Each draft pull request changes
      only the gate constant and the record's `review` (and, in the
      plugin repository, its copy of the record); the CI of both is green.
   9. **Conditions from item 8 above** (a hardware-backed key,
      reproducible-build attestation for Git and Docker): decide which are
      required before opening, and record them in the sign-off. Key
      custody and a second human reviewer are not conditions: accepted
      risk, single maintainer (owner decision 2026-11-21).

   **Recording the outcome.** The reviewer fills in, in the DeckPilot
   draft, `review` of `publisher/privileged-policy.json`:
   `{"reviewer": "<name>", "date": "<YYYY-MM-DD>", "outcome": "approved",
   "record": "docs/marketplace/publisher-review-request.md#sign-off; <report link>"}`
   (or leaves `pending` and records findings), the same in the plugin
   repository's copy, and a row of the Sign-off table below. Until then
   the drafts keep the gate closed in effect (`pending`), and they are
   merged only by the owner.

Run: `cargo test -p deckpilot-plugin-registry --all-features --test publisher`
(the multi-signature rules), `--test privileged` and `--lib` (the
privileged gate and policy),
`cargo test -p deckpilot-agent --features plugins --lib official`
and `cargo test -p deckpilot-agent --features plugins --test
plugins_registry_https`.

## Out of scope

The registry roles (U-10, U-12), the plugin host sandbox, the 1.9
broker and denial matrix (reviewed in
[module-migration.md §7](module-migration.md#plugin-api-19-security-reviews-2026-11-20)).

## Sign-off

| Reviewer | Date | Outcome | Gate |
| --- | --- | --- | --- |
| | | | closed |
| pending: the independent security reviewer (not the author) | 2026-11-21 | pending | constant open; closed in effect until approved |
| Independent security-review agent (Claude Code, not the implementer), commissioned under the owner's authorization; owner approval recorded 2026-11-21 | 2026-11-21 | approved | open for exactly the reviewed policy; report: [reviews/2026-11-21-privileged-gate-review.md](reviews/2026-11-21-privileged-gate-review.md) |

The gate opens only with a row here whose outcome is *approved*, the same
reviewer, date and outcome in the `review` of
`crates/deckpilot-plugin-registry/publisher/privileged-policy.json` (and of
`official/privileged-policy.json` in `knittingcode/deckpilot-plugins`),
`PRIVILEGED_GATE_OPEN = true` (and `PRIVILEGED_GATE_OPEN = True` in
`tools/check-official.py`), and the owner's merge of both draft pull
requests.
