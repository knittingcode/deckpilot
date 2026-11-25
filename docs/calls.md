# Calls

**Status:** contracts only. The provider contract, the call snapshot, the
`call.*` operations and the manifest kinds of plugin API 1.16 exist; the
state engine, the presentation overlay, the managed layouts, the providers
and the plugin come in later changes. No DeckPilot provides plugin API 1.16
yet, so nothing of this can be installed or reached by a plugin today.

Calls ("Rozmowy" in Polish) shows the state of the user's calls and
meetings on their devices and, **only where the call app offers a
supported way**, controls them: answer, reject, cancel, end, microphone and
camera.

## Microsoft Teams: detection only

Microsoft retired the Teams desktop client's local third-party device API
on 2026-06-30 (Microsoft 365 Message Center MC1266901). No supported
mechanism controls the Teams desktop client on any platform since then:

- Graph's calling APIs are for bots in their own calls, not the user's
  client.
- The documented Teams keyboard shortcuts need accessibility automation,
  Teams in focus, and give no confirmation. DeckPilot does not use them.

So a Teams provider is **detection only**: "a call or meeting is probably
active", "Teams uses the microphone", "the camera is in use". Every Teams
control capability is `Unsupported { providerHasNoApi }`, and the keys for
it are shown disabled with that reason. Nothing claims that a Teams control
works. Full control is exercised by the mock provider of the tests only.

## Providers (`deckpilot-calls::provider`)

A provider is an adapter compiled into the agent (never loaded from the
plugin) that implements `CallProvider`: `discover`, `connect`,
`disconnect`, `poll` and `command`. Nothing in the contract reads a clock;
every call that needs the time is given `now` on the engine's monotonic
clock, so every timeout is tested on a virtual clock.

**Capability discovery.** `discover` returns an `Availability`
(`available`, `notInstalled`, `notRunning`, `needsPairing`,
`disabledInApp`, `consentDenied`, `unsupported { reason }`,
`error { code }`) and a `CapabilitySet`. The set names every capability:

| Capability | |
| --- | --- |
| `detectIncoming`, `detectOutgoing`, `detectMeeting`, `readTitle` | observing |
| `answer`, `reject`, `cancelOutgoing`, `end` | controlling the call |
| `micToggle`, `micSet`, `micReadState` | the call app's microphone mute (never the operating system's device mute) |
| `cameraToggle`, `cameraSet`, `cameraReadState` | the call app's camera |

Each has a `Support`, never a bare boolean: `supported`,
`conditional { when: hostAllows | inCall }`, `readOnly`, `inferred`
(derived from operating-system signals, not reported by the app) or
`unsupported { reason }`. The reasons are a closed list, each with an
English and a Polish text in the plugin: `providerHasNoApi`,
`platformNotSupported`, `notInMeeting`, `noPermissionInMeeting`,
`toggleOnly`, `needsPairing`, `needsOsPermission`, `consentNotGiven`.

- **The unsupported state is explicit.** A capability the provider leaves
  out is `unsupported { providerHasNoApi }` (`CapabilitySet::from_reported`
  fills it in and names it, so the service logs the contract violation
  once). A capability reported twice keeps the narrower support. A
  serialized set that does not name every capability does not parse.
- **Discovery is dynamic.** A provider reports
  `ProviderEvent::Capabilities` whenever its support changes (a meeting's
  permissions, an operating-system permission revoked); faces follow.
- **Only `supported` and `conditional` allow a command.** `readOnly` and
  `inferred` are observation only.

**Events.** `connected { session }` (a new generation per connection),
`disconnected { reason }`, `capabilities`, `call` (a full snapshot of one
call, never a delta, so a lost event is repaired by the next one),
`callGone`, and `commandOutcome { token, outcome }`. An outcome is
`confirmed`, `rejected { code }` or `unknown` (sent but not confirmable,
shown as unconfirmed, never as success). A command token carries the
provider session it was sent in; an outcome that matches no pending
command of the current session is dropped.

**Privacy in the types.** A `CallKey` (the provider's id of a call) prints
only its length. A meeting title is a `Private<String>`: no `Display`, no
`Serialize`, a `Debug` that prints nothing of it, readable only through
`reveal()`, and present only when the provider can read it **and** the
user opted in.

## The call snapshot (`deckpilot-calls::snapshot`)

What the plugin, the key faces and the IPC see of the focused call:
`state` (`idle`, `incoming`, `outgoing`, `connecting`, `active`, `ending`,
`ended { cause }`, `failed { code }`, `unknown`), `kind`, `direction`,
`micMuted` and `cameraOn` (each `known(bool)` or `unknown`, never
defaulted), the provider's `capabilities`, the `provider` id, `since`, an
opt-in `title` and a `generation`. It never holds participant names, phone
numbers, meeting links or meeting ids. Failure codes are stable codes; a
provider's own text never reaches the user.

## Host operations (`deckpilot-calls::ops`)

| Operation | Arguments | Answer |
| --- | --- | --- |
| `call.state` | none | the focused call's snapshot |
| `call.control` | `{"action": {"kind": "answer" \| "reject" \| "cancelOutgoing" \| "end" \| "micToggle" \| "micSet", "muted": bool \| "cameraToggle" \| "cameraSet", "on": bool}}` | `confirmed` or `alreadyThere`, or an error code |

The plugin never names a call or a provider: the host acts on the focused
call. Unknown fields are refused. The broker will grant both only to
`deckpilot.calls` with the `calls` permission, and `call.control` only with
`calls:control`; the ABI wire entries and the broker pin come with the
Calls bundle, which also moves the agent's plugin API to 1.16.

**The gate.** Before anything is sent, `ops::gate` checks, in this order:

1. the provider is connected and the call is not `unknown`, else
   `calls-provider-offline`;
2. the call is in a state the action applies to (answer and reject:
   incoming; cancel: outgoing or connecting; end: active or connecting;
   microphone and camera: active), else `calls-not-in-call`;
3. the user's `calls:control` consent, else `calls-consent`;
4. the capability allows a command now, else `calls-unsupported`.

A set of the microphone or camera on a provider that can only toggle reads
the **confirmed** state: it toggles only when the state differs, answers
`alreadyThere` when it does not, and is refused (`toggleOnly`) when the
state is unknown. A command never changes the state by itself; only a
confirming observation does, and an action that is not confirmed in time
ends with `calls-unconfirmed`. Workflows get the same gate: nothing is
queued for later.

| Plugin action (`deckpilot.calls.*`) | Asks for |
| --- | --- |
| `answer`, `reject`, `cancel-outgoing`, `end` | the same action |
| `mic-toggle`, `mic-mute`, `mic-unmute` | `micToggle`, `micSet { muted }` |
| `camera-toggle`, `camera-on`, `camera-off` | `cameraToggle`, `cameraSet { on }` |

`show-calls` and `return-to-work` change what a device shows, not a call,
and are not `call.control` actions.

## Manifest kinds (plugin API 1.16)

`calls`, `contributes.callOverlay` and `contributes.layoutTemplates` are
described in the
[SDK permissions](marketplace/sdk/permissions.md#plugin-api-116-the-bundled-calls-plugin-only).
They are privileged: only the bundled plugin `deckpilot.calls` declares
them, with `^1.16`, and a plugin with `calls` has no `network`,
`credentials`, `openUrl`, `tools` or `connections`. `Manifest::privileged_usages` lists
all three, so the reviewed privileged policy pins them like the 1.9 kinds.

**End to end (T-P2,
`crates/deckpilot-plugin-registry/tests/calls_privileged.rs`).** The Calls
plugin's own manifest (deckpilot-plugins `official/calls`) is run through
`package --first-party`, `release sign`'s preflight and `release verify`,
signed by two throwaway keys:

- This build refuses it everywhere: the compiled policy has no Calls entry.
- An approved test policy with its exact entry still refuses it, because
  `deckpilot.calls` is not a privileged plugin id. Adding the id and the
  entry is the owner's release step.
- The public rules, another id, `calls` with a way out, and an API below
  1.16 are refused by the manifest rules themselves.
- No release-relevant build enables the test-only open gate (`cargo tree`).

| Grant | Tier | |
| --- | --- | --- |
| `calls` (`calls:observe`) | medium | the focused call's state |
| `calls:control` | high | `call.control`; always asked for, optional when actions name it |
