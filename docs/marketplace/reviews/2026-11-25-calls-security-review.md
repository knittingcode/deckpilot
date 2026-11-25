# Security review record: Calls (C8, 2026-11-25)

The record of the independent reviews of the Calls work: deckpilot #450 to
#490 and deckpilot-plugins #105. Design: `design-calls.md` §10 and §12, C8.
It is compiled from the coordinator's approvals log
(`scratchpad/integration/approvals.md`) and the review sections of the PRs.

**Reviewers.** Each review was done by an independent review agent (Claude
Code), commissioned by the coordinating session under the owner's
authorization. None of them wrote the code under review. They are not human
reviewers: the owner signs off on the release (design §13, "Needs the
owner" 1). A *coordinator delta check* is the merging coordinator comparing
a new head with an already reviewed one. It is not an independent review.

**Status.** Calls is groundwork and inert in every build:

- no build installs a provider (only the test hooks install the mock);
- no plugin can be granted `calls` (no policy entry, and the agent's plugin API is below 1.16);
- no live operating-system source exists.

Nothing here was checked against a live Teams meeting or a physical Stream
Deck.

## Summary

| PR | Item | Reviews (head → verdict) | Final |
| --- | --- | --- | --- |
| #450 | C1 contracts: types, manifest kinds `calls`, `callOverlay`, `layoutTemplates` (1.16, bundled-only, privileged) | 5739488f → CHANGES (5); 0ce3d270 → CHANGES (1 new); 9512f57a → code OK; 07e8218b → APPROVED; main merge d6f55261 → resolution check MERGE-READY; 7f934d5c → APPROVED | APPROVED |
| #452 | C2 state engine, Back to work controller | 26620a8d → CHANGES (5 MEDIUM, 8 LOW); c01a68c5 → re-check MERGE-READY | APPROVED-PENDING-CI |
| #457 | C3 overlay, fence, managed layouts (migration 15) | fd78b80a → CHANGES (9); 0d17852d → re-check; c0f462bc → migration-15 check MERGE-READY; 9976ca03 → delta (test only) | APPROVED-PENDING-CI |
| #458 | C4 mock provider | d78eccd3 → MERGE-READY (3 LOW optional) | APPROVED-PENDING-CI |
| #459 | C3b agent wiring | c5deb3c6 → CHANGES (6); ac4f38ce → re-check MERGE-READY; 105ab837, 0489b550 → delta | APPROVED-PENDING-CI |
| #462 | follow-ups (seeding, end instant) | 5ed26366 → CHANGES (2 MEDIUM, 2 LOW); ae9aff48 → re-check MERGE-READY | APPROVED-PENDING-CI |
| #463 | C5 Teams detection-only provider (OS signals) | b6bec3de → security review MERGE-READY after 7 LOW fixed; 258cd45e, 0be47889 → delta (merges only) | APPROVED-PENDING-CI |
| #464 | C6a IPC 19, Settings › Devices › Calls | first review → CHANGES (3 MEDIUM, 5 LOW); a7224fe4 → re-check MERGE-READY (5 LOW); e7972593, 270b177d → delta | APPROVED-PENDING-CI |
| #468 | C9 part: detached plugin action (T-R3), `--test-calls-mock` | 5da150a2 → MERGE-READY (5 LOW); bcb9ecb9 → delta | APPROVED-PENDING-CI |
| #471 | a locked device never shows a call | 615562d6 → privacy review MERGE-READY (3 LOW) | APPROVED-PENDING-CI |
| #473 | no overlay or overlay action while locked | 7addf9ed → privacy review MERGE-READY (3 LOW + nit) | APPROVED-PENDING-CI |
| #476 | keys fenced across lock and unlock | 09d0d619 → delta (#473's LOWs) | APPROVED-PENDING-CI |
| #474 | C7 call pages, Back to work settings | a1b8453f → CHANGES (3 MEDIUM, 3 LOW); 5ad14ded → re-check MERGE-READY (3 LOW) | APPROVED-PENDING-CI |
| #478 | #474's lows | 7ae55cef → delta | APPROVED-PENDING-CI |
| #480 | C6b part 1: `previewCallsPage` | 345d04dc → privacy review MERGE-READY (4 LOW) | APPROVED-PENDING-CI |
| #482 | C6b part 2: the app's preview | 906d3dab → CHANGES (1 MEDIUM, 1 LOW-MEDIUM, 4 LOW); e1b5a0ad → re-check MERGE-READY | APPROVED-PENDING-CI |
| #483 | T-E2E on XL and Plus; app follows previews | f8ff8cd4 → delta (test only); df80549c → CHANGES (1 MEDIUM, 5 LOW); b2e08a24 → re-check MERGE-READY (2 LOW) | APPROVED-PENDING-CI |
| #486 | `CallsStatus.revision`, preview by device | 16e91ef6 → MERGE-READY (2 LOW) | APPROVED-PENDING-CI |
| #490 | statuses of an earlier agent run dropped | fcd46f6b → delta (#486's LOWs) | APPROVED-PENDING-CI |
| #489 | C9 host wiring: `call.state`, `call.control` | 97e3f4e1 → APPROVE-WITH-FIXES (1 MEDIUM, 4 LOW, 2 NIT); b207b974 → re-check APPROVE | in the queue |
| plugins #105 | C7 `official/calls` 0.1.0 (held ahead at 1.16) | plugin coordinator's review: pending | pending |

## Findings and fixes

### #450 C1 contracts

- **CHANGES at 5739488f** (gating sound):
  - a set via a toggle could report a fake success from an inferred read state;
  - the snapshot title leaked through `Debug`;
  - the title opt-in was not enforced in code;
  - codes were not validated;
  - tests were missing.
- **Fixed, each with a test.**
- **New defect at 0ce3d270:** the snapshot's `since` used the wrong clock. It was fixed with `since_unix_ms` taken from the caller, plus a test that repeated observations keep `since`.
- **Main merge** (connections 1.14, d6f55261): the conflicts were resolved as unions. The merge added one rule: a plugin with `calls` may not declare `connections`. The reviewer found the rule correct, and its test fails without it.

### #452 C2 state engine

**MEDIUM, all fixed with tests:**

1. A late or out-of-order observation revived an ended call.
2. A microphone or camera toggle was confirmed by an inferred (OS-signal) state.
3. The candidates map was unbounded.
4. `ProviderEvent::Capabilities` could widen a detection-only provider at run time.
5. The engine accepted observations without the observe consent.

**LOW:**

6. A pending command was dropped without a `CommandDone`. Fixed.
7. `inferred_evidence()` was global. Fixed.
8. An idle snapshot showed control capabilities without the control consent. Fixed.
9. Events were uncapped. Capped at 256.
10. One provider could fill `MAX_TRACKS`. Justified; the justification holds.
11. `since` was an engine-clock value in a serialisable field. Justified; the justification holds.
12. `ReturnMode` fields were unvalidated. Fixed.
13. `Pending::acknowledged` was unused. Removed.

**Re-check follow-up A:** an observation stamped exactly at the end instant revived a call. The end instant is now inclusive (#462), with a test.

**Note B (accepted):** a withdrawn consent ends calls at their last observation.

### #457 C3 overlay and managed layouts

Findings at fd78b80a:

1. The migration must be 15.
2. There were no v0.13/v0.14 upgrade tests and no compatibility entry.
3. Exported managed layouts broke seeding elsewhere.
4. A managed layout could be assigned to a profile, and regeneration dropped it silently.
5. Dismiss did not stick while ringing.
6. The generator hard-coded MK.2 keys (it could trap a 2×4 device).
7. The input fence missed gestures.
8. Restore defaults left dangling step ids.
9. The remove path read outside its transaction.

All were fixed and tested, with migration 15, the upgrade tests and the compatibility entry. The agent wiring and tests T-R2/3/4/6/7/9/E2E moved to C3b (#459), and from there to #468 and #483.

### #458 C4 mock provider

MERGE-READY: the mock exists only under `cfg(test)` or the non-default `testing` feature, and nothing in a workflow or the app enables it.

Three optional LOWs (docs, assertions), taken up by the Calls agent.

### #459 C3b agent wiring

Findings at c5deb3c6:

- the agent-level gesture test T-R10 was missing;
- seeding into `managed_layouts` was not done;
- `install_calls` hard-coded the control consent to `true`;
- the `DetachedFinished` mapping was wrong;
- T-R2, T-R3 and T-R6 were missing;
- Prettier failed on #450's fixture.

All were fixed and tested. T-R3 at agent level was then a release blocker, covered by #468.

### #462 follow-ups

1. **MEDIUM:** seeding retried on every loop pass with a 5 s busy timeout, so the agent loop could stall, it logged 4 warnings per second, and permanent errors were retried forever. Fixed: a backoff from 1 s doubling to 60 s, retries only for busy or locked, and one warning per streak.
2. **MEDIUM:** the retry path was untested. Fixed: `a_busy_seed_is_tried_again_with_a_growing_wait_and_stored_once`.

The LOWs were fixed, or done in #468.

### #463 C5 Teams detection-only provider

The security review was MERGE-READY. Its seven LOWs were fixed at b6bec3de:

1. executable names are kept out of `Debug`;
2. `pw-dump` reads only `media.class` and `application.process.binary`, never `media.name` (which may hold a title);
3. `MicReadState` is unsupported, since holding the microphone is no mute state;
4. name spoofing is documented: a verified identity (bundle id and team id, package family) is required before a live source;
5. the consent is tested: no sampling without `observe` or after `disconnect`;
6. a disconnect resets the failure state;
7. records over 4 MiB are refused.

**Found by the C9 review:** the provider never reported `Connected`, so every Teams control answered "provider offline" instead of its reason. This is fixed in #489, with tests.

### #464 C6a IPC 19 and Settings

**First review, CHANGES:**

1. **MEDIUM:** `callsChanged` reached every subscriber. It now goes to desktop clients only. This is a policy guard; the IPC token is the boundary.
2. **MEDIUM:** there was no downgrade test and no matrix row. Both were added.
3. **MEDIUM:** the store was read on the Calls tick. The layouts are now cached, and a test counts the reads.
4. **LOW:** the "Edited" badge went stale.
5. **LOW:** unknown devices were accepted.
6. **LOW:** `copyName` was not validated. Control, bidirectional and invisible characters are now refused.
7. **LOW:** the Teams text appeared for any provider.
8. **LOW:** connect and disconnect did not publish.

All were fixed and tested.

**Re-check LOWs:**

- the agent's protocol-18 refusal is now tested;
- the doc comments are restored;
- U+2028/2029, U+200B–200D and U+FEFF are refused;
- the Polish wording is fixed;
- the event goes to the main window only.

All fixed.

### #468 detached plugin action (T-R3)

MERGE-READY. `--test-calls-mock` exists only under `test-hooks`, which only dev-dependencies reach; a `cargo tree` test proves it, with a positive control. There is no gate or consent bypass.

LOWs 1–4 were fixed. LOW 5 (an overlay survived a lock) was a release blocker, fixed by #471/#473/#476.

### #471, #473, #476 lock

- **#471 privacy review: MERGE-READY.** Every lock path hides Calls. LOWs:
  - the core drew overlays ahead of Locked → fixed in #473;
  - the test asserted through the view, not the device output → fixed;
  - a comment.
- **#473 privacy review: MERGE-READY.** It adds a single `visible_frame` check. LOWs:
  - a press before a lock could release into the overlay after the unlock;
  - the lock order;
  - an assertion weaker than claimed.

  All three were fixed in #476, with a core test.

### #474, #478 call pages and Back to work settings

**CHANGES at a1b8453f.** The review confirmed that Teams never claims support. Findings:

1. **MEDIUM:** page controls ignored the control consent. Fixed: `consent-not-given`.
2. **MEDIUM:** Back to work was shown for layouts without its key.
3. **MEDIUM:** `setCallsBackToWork` had no IPC tests.
4. **LOW:** the agent handler did not validate its input.
5. **LOW:** the write-back replaced the whole parameter map.
6. **LOW:** the compatibility rows.

All were fixed and tested.

The re-check LOWs were fixed in #478: one transaction for every key, and the false Restore-defaults sentence dropped.

### #480, #482, #483, #486, #490 preview and the app

- **#480 privacy review: MERGE-READY.** Preview faces come only from `CallSnapshot::idle`, with fixed content. LOWs:
  - "desktop" is a client's self-declaration, a policy guard and not authentication: reworded;
  - a failed push left state behind: fixed;
  - `clear()` kept previews: fixed;
  - doc comments: fixed.
- **#482 CHANGES:**
  - **MEDIUM:** a begin/stop race let a preview outlive the settings. Fixed with a generation counter, end-if-stale, and the button disabled while pending.
  - the per-layout state was lifted;
  - Polish refusal texts;
  - a CHANGELOG entry.

  All fixed. The re-check LOWs were done in #483.
- **#483 CHANGES at df80549c:**
  - **MEDIUM:** a stale confirmation could clear a fresh preview of the same page. Fixed: the confirmation lives in the preview state, with a vitest of the exact sequence.
  - LOWs, all fixed:
    - the agent could end a preview before the app saw it: a follow-up status after the begin;
    - previews were matched by model: now by device;
    - a privacy test of the device entry's keys;
    - only one end path was tested: now every end path;
    - the docs.
  - Re-check LOWs L1/L2 → #486.
- **#486 MERGE-READY:**
  - `CallsStatus.revision`, so the higher revision wins;
  - the preview keyed by device.
  - LOWs → #490: a status of an earlier agent run is dropped, plus tests.

### #489 C9 host wiring

**APPROVE-WITH-FIXES at 97e3f4e1:**

1. **MEDIUM:** after Calls was installed again, a waiting control could be answered by a new engine's command with the same token, or blocked by stale entries. Fixed: an install abandons every waiting control (`calls-provider-offline`), and each install is a new provider session. Test: `a_waiting_control_is_answered_when_calls_starts_again`.
2. **LOW:** `call.state` could use a key state call's whole 2 s. Fixed: it waits 400 ms.
3. **LOW:** one user-started run could keep controlling calls for its whole timeout. Fixed: at most 2 controls per run. Test: `one_run_controls_at_most_twice`.
4. **LOW:** `resources` was not a way out in `CallsWithWayOut`. Fixed, with a broker test.
5. **LOW:** the engine's control consent is fixed at install. Deferred; see the residual risks.
6. **NIT:** two error paths dropped the reply. Fixed: explicit refusals.
7. **NIT:** the Calls plugin must never log a title. The plugin source (#105) never reads one.

**Re-check at b207b974: APPROVE.**

**Checked and fine:**

- the plugin pin in three layers;
- grant per op;
- user-started runs only for control;
- the engine gate before anything is sent;
- data minimisation: no participant, process or executable name; the title only with the opt-in; stable codes;
- no op names a device, frame, layout or call, so the plugin cannot drive overlays;
- the agent thread never blocks;
- no reachable panic from plugin input;
- the test hooks are dev-only.

### deckpilot-plugins #105 C7

The plugin coordinator's review is pending. It is to check:

- the new `AHEAD_OF_PIN` entry and its kind (a reviewed tooling change);
- that no control capability is claimed;
- refusal on every package, sign, release and registry path;
- no 1.16-only kind in another manifest.

The PR's own checks:

- the manifest validates as bundled against #450's head, and the public rules refuse it;
- the module imports only `host_call`;
- the pinned packager refuses it;
- a contract test enforces that no text claims a Teams control.

## Accepted residual risks

- **A process named like Teams can fake "in a call"** (design §10). The worst case is a misleading overlay, because no Teams control exists. The watchdog, the protected dismiss and the per-device setting limit it. A live source needs a verified identity first: the bundle id and team id on macOS, the package family on Windows.
- **The "desktop" client kind is self-declared.** `callsChanged` and the preview are limited to desktop clients as hygiene. The IPC token is the access boundary.
- **The engine's control consent is fixed at install** (#489 review LOW 5). The broker re-reads grants on every call, so revoking `calls:control` refuses controls at once. **Blocker for the PR that installs Calls in production:** a grant change must update the engine's consents.
- **Detection only for Teams.** Mute is unknown (Teams may hold the microphone while muted), the camera is not attributed on macOS, and Linux attributes nothing. This is stated in the UI, the README and the docs; nothing claims a control.
- **The managed layouts are not exported** (the safe default). "Include Calls layouts" (§7.3) is an open owner decision.
- **A preview's per-meeting capability reason** may show on a preview. It is informational only and carries no call data.

## Release blockers and owner checks (not covered by these reviews)

- The **TCC check on a real Mac**: whether the Core Audio process-object queries and `AVCaptureDevice.isInUseByAnotherApplication` prompt for the microphone or camera. A signal that would prompt is dropped.
- The **Teams process names** on current clients, and the verified identity, before any live source ships.
- **Production install** with consent updates (above); the agent's plugin API 1.16; the privileged policy entry of `deckpilot.calls` 0.1.0 in both repositories, with the owner's sign-off recorded (`review`); bundling. The exact diffs are in the Calls status notes.
- **Visual checks on devices:** the overlay on an MK.2, an XL and a Plus; disabled controls with their reason; "Locked" over a call; the preview; Settings › Devices › Calls.
- C5c (Graph presence) was not built and not reviewed.
