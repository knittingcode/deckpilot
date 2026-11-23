# Independent security review: opening the privileged-plugin gate (2026-11-21)

**Reviewer:** an independent security-review agent (Claude Code) commissioned by the coordinating
session under the owner's authorization. It did not write any of the reviewed code and did not take
part in the earlier reviews. It is not a human reviewer; the owner decided (checklist item L9) on
the basis of this report.

**Reviewed:** knittingcode/deckpilot main `784f62e7` and draft #388 at `ba3acb3c`;
knittingcode/deckpilot-plugins main `a1f2557` and draft #73 at `b1d69e0`. Read-only.

## Verdict

Yes, with conditions. No high or medium findings. Three low findings (L1–L3) and four notes
(N1–N4). None of them can open the gate or widen what it admits at run time.

## Verified

1. **The gate opens only through the record.** `PrivilegedGate::compiled()`
   (`privileged_policy.rs:100-108`) needs the constant `true`, a compiled-in policy that parses and
   validates, and the outcome `approved`.
   - No `cfg`, feature, environment variable or `build.rs` input affects it.
   - Production callers use `compiled()` or `Closed`.
   - `with_gate` is `#[cfg(test)]`, and `test-hooks` is a dev-dependency only.
2. **With the gate open, every path admits exactly the record.**
   - Paths covered:
     - the packager;
     - `release sign` (before and after signing), `release verify` and `verify_release`;
     - the `artifacts.sh` admission line and the `bundled.json` test;
     - `check-official.py`;
     - `admit_privileged`.
   - A probe ran `privileged_policy::check` against an approved policy copy. It refused all 21
     smuggling attempts:
     - duplicate keys at every level;
     - `sh`, `Git`, `/usr/bin/git`, `git.exe` and a fullwidth `ｇit`;
     - an extra subcommand, `network: 1`, unknown fields and an added `statusItems`;
     - an uppercase id, `0.1.1+x` and `0.1.1-rc.1`;
     - another version or id for the same bytes;
     - docker running git.
   - It admitted only `git`, which is the same JSON value as `git`.
3. **Bundled-only delivery.**
   - Registry copies of privileged plugins are refused on every agent path, with the gate open or
     closed:
     - the registry verification runs with the gate `Closed`;
     - `verify_package` then refuses privileged kinds under the first-party rules;
     - the store's `check_privileged_source` refuses any non-bundled source.
   - `official-entry.py` never creates a Marketplace entry for a privileged plugin.
4. **Threshold and revocations.** Every privileged path requires at least 2 distinct pinned,
   non-revoked keys and a pinned threshold of at least 2. A revoked key's signature refuses the
   release. Rollback floors and released pins apply.
5. **The record matches the 0.1.1 sources.**
   - All five manifests in deckpilot-plugins `official/` are admitted by both the Rust and the
     Python check. Construct counts are 10, 9, 13, 2 and 6.
   - The manifests equal DeckPilot's 0.1.0 copies except for `version` and `repository`.
   - The policy is byte-identical in both repositories and on both drafts.
6. **Git/Docker process model unchanged.**
   - The tool runner, executor, `tools.rs` and runtime are unchanged since `48cfb52d`.
   - Executables come from a fixed set, and arguments are arrays with no shell.
   - The working directory is a host-issued handle, and the environment is cleared.
   - Time and output are capped, with one concurrency slot per plugin.
   - Modifying runs happen only when the user starts them, and every run is audited.
7. **Migration.**
   - Grants are derived only onto bundled copies.
   - Process grants are high tier and never derived.
   - An update keeps only the grants the new version still declares.
8. **CI and release workflows.**
   - Sign-a and sign-b each hold one key and compile nothing.
   - Test signatures use the separate `role:test` domain, which `verify_exact` checks.
   - A held plugin is refused at `resolve` and is never signed.
9. **Earlier findings stay fixed:** M1, M2, L6–L9 (#380); L12, L13 (#379, #68); M3, M4, L10, L11
   (#68); N1, N3 (#69); N2 (#381). M5 is an owner setting.

**Tests run by the reviewer:**
- DeckPilot registry crate (all tests), store `privileged`, and agent `--features plugins --lib
  official` (29 passed). Run on #388 as drafted and again with the record set to `approved` as a
  simulation.
- Plugin repo: `test_check_official.py` (75), `test_check_workflows.py` (95) and
  `test_official_entry.py` (67).

## Findings

| # | Severity | Finding | Fix |
|---|---|---|---|
| L1 | Low | The production-code lint against building `PrivilegedGate::Open` is a line grep. It misses imports, glob imports and multi-line calls. | Widen the lint, or make `Open` constructible only through `compiled()` and test helpers |
| L2 | Low | `is_approved()` checks only the outcome, so a placeholder "pending…" reviewer with `approved` would open the gate | Refuse placeholder or empty reviewers; tie the record to the Sign-off row |
| L3 | Low | Once approved, the negative tests run only against a test clone of the gate | Add refusals against the production `compiled()` gate |
| N1 | Note | `check-official.py` compares loosely (`True == 1`, the last duplicate key wins) | Compare type-strictly and refuse duplicate keys |
| N2 | Note | `official.sh check` skips the comparison when there is no DeckPilot checkout | Fail when the gate is open |
| N3 | Note | A registry-delivered privileged artifact is audited as verified and raises the floor before the store refuses it | Audit it as refused, or raise the floor only after the install |
| N4 | Note | The doc heading "gate is closed" will be stale after approval | Update it in the approval commit |

**By design:** the record fixes each plugin's privileges, not its bytes. Its code and locale strings
are fixed by `artifacts.json` and the two signatures. The PR adding the five 0.1.1 entries to
`artifacts.json` is therefore a security-relevant review point and needs the reproducible-build
comparison.

## Conditions

1. L1–L3 and N1–N4 fixed (knittingcode/deckpilot #389 and its plugin-repo counterpart).
2. #388 approval commit: the real reviewer description, the date and this report, a matching
   Sign-off row, and updated docs.
3. #73 after the pin moves to a DeckPilot commit containing #388. The policy must be byte-identical,
   and the "pending" test updated.
4. Releasing any privileged plugin needs its own reviewed PR: remove `held`, then add the
   `artifacts.json` / `bundled.json` entries.

**Owner actions:**
- the second release reviewer;
- independent custody of key B;
- the M5 settings;
- a DeckPilot release containing #388.

## Not verified

- Each plugin's wasm behaviour against its record. Only git `config`, `remote` and `ls-remote`
  and docker's subcommands were checked.
- A reproducible-build attestation and a real `artifacts.sh` run; no artifact exists yet.
- Live GitHub settings and key custody.
- The sqlcipher store tests, `plugins_registry_https` and the desktop end-to-end tests.
- #388's macOS and Windows CI were still pending at review time.

## Remediation

Added after the review by the implementer. Everything above this section is
the review report verbatim (its sha256 is
`40d75cd3d8fd742d17c67f0a1ecba78e0f93eac96977471ee306b4eed3dd96e3`). Each fix
below was checked in the merged code.

| # | Fixed by | Merged commit | What changed |
| --- | --- | --- | --- |
| L1 | knittingcode/deckpilot#389 | `d015c89abdae3b28a7b35e82c974072185dafa52` | `PrivilegedGate::Open` holds a `ReviewedGate` with a private field. Only `compiled()` and the test-only `open_for_tests` (feature `test-privileged-gate`, dev-dependencies only) build one. The lint `only_tests_construct_an_open_gate` also covers imports, globs, multi-line constructions and `*_with_gate` / `.with_gate` calls, and is itself tested by `the_gate_lint_catches_every_way_to_open_the_gate`. |
| L2 | knittingcode/deckpilot#389 | `d015c89abdae3b28a7b35e82c974072185dafa52` | `Review::is_approved` needs the outcome `approved`, a named reviewer (not empty, not `pending…`), a `YYYY-MM-DD` date and a record. `the_review_is_the_last_sign_off_row` ties the record to the Sign-off table. |
| L3 | knittingcode/deckpilot#388 (this approval change) | the merge commit of #388 | Refusals against the production `PrivilegedGate::compiled()` gate, whatever the outcome: another version, an extra construct, a broader permission, another executable, another git subcommand, another plugin id (`the_compiled_gate_refuses_what_the_record_does_not_list`), fewer than two distinct keys (`the_compiled_gate_needs_two_distinct_keys`), and every registry delivery (`the_compiled_gate_refuses_every_registry_delivery_of_a_privileged_plugin`). |
| N1 | knittingcode/deckpilot-plugins#74 | `b0e07e4db0422e78b0057d0186ed7bfa5519dd30` | `check-official.py` compares type-strictly (`strict_equal`) and refuses duplicate JSON keys in every file it reads (`object_pairs_hook`). |
| N2 | knittingcode/deckpilot-plugins#74 | `b0e07e4db0422e78b0057d0186ed7bfa5519dd30` | `official.sh check` fails when the gate is open and there is no pinned DeckPilot checkout. |
| N3 | knittingcode/deckpilot#389 | `d015c89abdae3b28a7b35e82c974072185dafa52` | A privileged registry delivery that the agent's opening path admits is refused (`Refusal::BundledOnly`), audited as refused, and raises no floor. The store's X12 refusal remains the second lock. |
| N4 | knittingcode/deckpilot#388 (this approval change) | the merge commit of #388 | The gate's status lines in `publisher-signature.md` and `publisher-review-request.md` say it is open for exactly the reviewed policy. |

**Owner actions, updated.** DeckPilot is a single-maintainer project (owner
decision 2026-11-21):

- There is no second release reviewer. Release approval follows the
  protected-tag authority model.
- There is no independent custody of key B. Threshold 2 and both keys stay.

Both are recorded as **accepted risk: single maintainer (owner decision
2026-11-21)**. The M5 settings and a DeckPilot release containing #388 remain
owner actions.
