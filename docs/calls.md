# Calls

**Status:** contracts, the state engine, the return sessions, the
presentation overlay, the managed layouts and the driver that keeps them in
step. The agent wiring, the mock provider, the providers and the plugin
come in later changes. No DeckPilot provides plugin API 1.16
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
| `call.state` | none | the focused call, sanitised (`CallStateReply`, below) |
| `call.control` | `{"action": {"kind": "answer" \| "reject" \| "cancelOutgoing" \| "end" \| "micToggle" \| "micSet", "muted": bool \| "cameraToggle" \| "cameraSet", "on": bool}}` | `confirmed` or `alreadyThere`, or an error code |

The plugin never names a call, a provider, a device or a layout: the host
acts on the focused call. Unknown fields are refused.

**What `call.state` answers** (`deckpilot_plugin_abi::calls::CallStateReply`):
the state's name, the provider id, whether it is connected, `micMuted` and
`cameraOn` only as the call app reports them (absent when unknown: holding
the microphone is no mute state), whether a command waits (`pending`) or
was not confirmed (`unconfirmed`), per control action id whether it can
work now and the reason when not (`provider-has-no-api`,
`consent-not-given`…), the title only with the user's opt-in, and a
`generation`. Not the snapshot's `kind`, `direction` or `since`, never a
participant, a number, a meeting link or id, a process or an executable.

### Plugin operations: who may ask, and how (C9)

The ops are carried through the plugin runtime like plugin API 1.9's
(`plugins/runtime/crates/abi/src/calls.rs`, the broker, the agent's
executor `plugins/executor/calls.rs` and `service/plugin_calls.rs`). The
guest SDK has `Context::call_state` and `Context::call_control`.

- **Only `deckpilot.calls`**, checked three times: the broker refuses both
  for any other plugin id whatever it was granted (`CALLS_PLUGIN`), the
  executor checks again, and so does the agent. The manifest rules (bundled
  only, `^1.16`) and the reviewed privileged policy keep any other plugin
  from declaring `calls` at all.
- **Grants:** `call.state` needs `calls`; `call.control` needs
  `calls:control` too. A broker grant record with `calls` together with a
  way out of the agent (`network`, a credential, `openUrl`, a user origin,
  a tool, `openUrl:from-params`, `resources`, a connection, the Scripts
  bridge) is refused as a whole
  (`CallsWithWayOut`), the manifest rule checked again (and `resources`,
  whose `resource.open` takes a path the guest chose, too).
- **Runs:** `call.control` only in a run the user started (a key, a
  gesture, the app, or a workflow the user started): never a schedule, an
  event, the agent's start or a work session. 4 per second, and at most 2
  per run, so one press cannot keep controlling calls that come later.
  `call.state`
  needs no run and is answered inside a key's state call, so the plugin's
  keys can show the call (still under its grant); `call.control` is not.
- **The user's consents** decide on the agent's thread: without the
  consent to observe calls nothing runs (`calls-provider-offline`), and a
  control passes the gate above, so without the consent to control calls
  it is `calls-consent` and nothing reaches the call app.
- **The confirmed outcome:** a control the gate let through goes to the
  provider, and the plugin's call waits for the engine's confirmation (at
  most 8 s, at most 8 waiting in all; Calls started again answers every
  waiting control `calls-provider-offline`, and each start is a new provider
  session, so no command of the new engine ever answers an old one): `confirmed`, or `calls-unconfirmed`,
  or the provider's refusal (`calls-unsupported: <reason>` for a closed
  reason). A set of the state the call has answers `alreadyThere` and sends
  nothing.
- **Detection-only Teams:** every control is refused with its reason,
  `calls-unsupported: provider-has-no-api` (answer, reject and cancel are
  `calls-not-in-call` first, since Teams never shows a ringing or dialing
  call).
- **Locked:** both are `calls-locked`. `call.state` waits at most 400 ms
  for the agent, well within a key's state call.
- **Consent changes:** the engine's consents are set when Calls is
  installed. The broker reads the grants on every call, so revoking
  `calls:control` refuses controls at once; when the plugin's enabling
  installs Calls (with C7's release), a grant change must also update the
  engine's consents (`Engine::add_provider` applies a withdrawn consent).
- **No overlay, layout or device:** neither operation names a device, a
  frame, a page or a layout, so the plugin cannot drive an overlay or a
  managed layout. A confirmed command changes the call, and the overlay
  follows the call as it always does.
- **Audit:** every call is recorded with the operation, the grant, the
  outcome and a control's action kind; never the state, a title or anything
  of the call.
- **Refusals** start with their code (`calls-unsupported: …`), which the
  plugin's `errorCodes` translate. A run the user did not start is
  `calls-not-user-started` (the executor's re-check; it uses none of the
  run's controls), and more than 2 controls in a run, or too many waiting
  at once, is `calls-throttled`. Neither is `calls-consent`, which is the
  user's consent only. ABI codes: `calls-consent` and
  `calls-not-user-started` are `not-permitted`, `calls-throttled` is
  `throttled`, `calls-provider-offline` and `calls-locked` are
  `unavailable`, the others `failed`.

The agent's `PLUGIN_API` is not moved by this change, and the privileged
policy has no `deckpilot.calls` entry yet: both are the owner's release
steps. Until then no plugin can be granted `calls`, and the ops are inert.

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

| Grant | Tier | |
| --- | --- | --- |
| `calls` (`calls:observe`) | medium | the focused call's state |
| `calls:control` | high | `call.control`; always asked for, optional when actions name it |

## State engine (`deckpilot-calls::engine`)

Pure and deterministic: every call is given `now`, `next_wake` says when
`tick` is due, and every test runs on a virtual clock. Nothing is
persisted: after an agent restart the engine starts empty.

States: `idle`, `incoming`, `outgoing`, `connecting`, `active`, `ending`,
`ended { cause }`, `failed { code }`, `unknown`. One track per provider and
call, at most four.

| From | Event | To |
| --- | --- | --- |
| idle | ringing (incoming) / dialing / connected | incoming / outgoing / active |
| incoming | connecting / connected | connecting / active |
| incoming | ended, gone, 90 s | ended: rejected, answered elsewhere, missed |
| outgoing | connecting / connected | connecting / active |
| outgoing | ended, gone | ended: cancelled, declined, no answer |
| outgoing / connecting | 120 s / 60 s | unknown |
| active | ending / ended, gone | ending / ended: remote hang-up, left, unknown |
| active | *end* confirmed by the provider | ending |
| active | no observation for 24 h | unknown |
| ending | ended, gone, 10 s | ended |
| any live | provider disconnected | unknown |
| any live | provider protocol error | failed, shown 3 s |
| unknown | observation after a reconnect | the observed state |
| unknown | 15 s without evidence (4 h while OS signals still show a call) | ended: lost |
| ended | 2 s | idle |

Any other observation changes no state. Operating-system evidence must hold
for 2 s before a call is active and be gone for 8 s before it has ended;
the call app exiting makes the call unknown, not ended.

**Commands** pass `ops::gate`, are pending until a confirming observation
(for *end*, the provider's confirmation), and end as unconfirmed after 5 s.
One command at a time per call. Outcomes of another connection or command
are dropped.

**Focus**: incoming (newest first), then outgoing and connecting, then
active (most recently connected), then unknown, then ended. **Overlay
intent**: shown while the focused call is live (incoming page, outgoing
page, active page; unknown shows the active page), in one overlay session
from the first call to the last; a meeting that follows a meeting keeps the
session. It goes on entering `ended`. Calls known only from operating-system
signals may be kept off the overlay (on by default).

## Back to work (`deckpilot-calls::ret`)

"Wróć do pracy" suspends a device's overlay; a return session per device
decides when it comes back:

| Mode | Comes back when |
| --- | --- |
| Timer | 20, 30 or 60 s, or a custom 5 to 600 s |
| Completion | a qualifying run started on the device while suspended has really ended |
| Completion or timeout (default) | whichever comes first, at most 60 s (10 to 1800 s) |

- Qualifying runs: actions (in place and detached), workflows and Multi
  Actions by default; navigation only when configured; a key with nothing
  bound never.
- Success resumes. A failure resumes by default (or stays). A cancellation
  stays by default (or resumes). An interruption (the agent stopping)
  counts for nothing. The first qualifying end wins.
- One session per device, each with a new `ReturnId`. A second suspension
  replaces the first, and anything carrying an old id is ignored, so there
  is never a second timer or a stale restore.
- The call ending, failing or being lost ends every session without
  resuming (the overlay is popped). An incoming call preempts them.
  *Show calls* resumes at once. Memory only.

## Overlay (core `device_runtime/presentation.rs`)

A per-device presentation stack in the core `Runtime`. An overlay frame
makes a device show and run another layout **without changing anything of
what it showed**: profile, navigation, remembered pages, switched-to
layouts, layout history and working layout stay as they are, so taking the
frame away shows the exact earlier presentation. Nothing of a frame is
saved.

- A frame has its own layout (a managed layout for the device's model),
  page stack and owner-drawn faces. Its bound actions are handed to the
  owner (`RuntimeOutcome::OverlayAction`); the runtime runs nothing itself.
  Workflows, stateful keys and gestures do not run on an overlay. Pages of
  its layout open and close in its own stack.
- Its protected dismiss key, and *Back* on its first page, take it away
  (`RuntimeOutcome::OverlayDismissed`): a device can never be trapped.
- Priorities are a closed list (`Notification` < `Calls` < `System`), and
  owners are a closed list (`Calls`). A suspended frame stays on the stack
  and shows what is under it. At most 4 frames per device.
- **Input fence:** controls held down when what a device shows changes have
  their remaining input dropped until released, so a press on one
  presentation never releases, or finishes a gesture, on the other.
- **Auto Exit** of the covered folder is paused (a hold of its entry) and
  goes on with the time it had.
- The editor's live preview wins over an overlay. Frames stay with a
  disconnected device. If what was covered is gone when the frame goes,
  the device falls back as after a reload.

## Managed layouts (`deckpilot-calls::layouts`, storage schema 15)

One managed layout per device model, generated from the template of the
model's capability class (`grid-small` ≤ 6 keys, `grid` 7–15, `grid-large`,
`keys-dials`, `no-display`). Nothing assumes an MK.2. Ids are UUID v5: the layout of the model and
the template's major version, pages and action instances of the layout and
their key. Controls are placed relative to the grid: a template position
the model has is kept, and one it lacks moves to the first free key, so
nothing is dropped on a small grid. The model's last key is always the
protected dismiss key. A model without keys, or one too small for a page,
is refused rather than given an overlay it could not leave.

Storage (`managed_layouts` table, additive migration 15), following D2:

- A layout is created only when the owner has none for the model, in one
  transaction, and never touches another layout. A layout without a row is
  the user's, whatever its name, and is never adopted.
- A layout is **edited** when the canonical JSON of the layout with its
  actions differs from the snapshot taken when it was generated.
- A new template version regenerates an unedited layout in place. An
  edited one is never rewritten (`UpdateAvailable`).
- **Restore defaults** first keeps the user's version as an ordinary layout
  of their own, with copies of its actions and assigned to no profile, and
  then regenerates.
- Removing takes the unedited layouts, and the edited ones only when asked.
  Imports never delete a managed layout, and deleting one as an ordinary
  layout is refused.
- A managed layout is never a profile's: assigning one, or saving a
  profile that names one, is refused. Managed layouts are not in the
  Layouts list and are not exported; each machine seeds its own.
- *Restore defaults* copies the layout's whole action closure (a Multi
  Action's steps too), and the copies name each other.

## Driver (`deckpilot-calls::driver`)

Keeps the engine's overlay intent, the return sessions and the runtime's
frames in step. All of it runs on the engine clock.

- While the focused call is live, every connected device with a managed
  layout and the overlay on shows its frame on the page for the call's
  state. The per-device setting defaults to on for devices with key
  displays. When no call is live, every frame goes and every return
  session ends without resuming.
- Faces show the confirmed state. A command waiting for confirmation shows
  "Waiting…", one that was not confirmed shows "Not confirmed", and an
  unsupported control is disabled with its reason. Teams is detection-only,
  so its controls show "The call app offers no control".
- A dismissed device stays on its own presentation while the call it was
  dismissed for has the focus. *Show calls*, or another call taking the
  focus (a new incoming call), brings the overlay back.
- Gestures: the gesture engine records the device's presentation
  generation when a sequence starts. A gesture that fires after the
  presentation changed is stale and is dropped, so it never runs on the
  other presentation.
- *Back to work* suspends the frame with a return session whose settings
  are the action's parameters (`mode`, `seconds`, `maxSeconds`,
  `onFailure`, `onCancel`, `countNavigation`, `returnAllDevices`). Runs
  started from the restored presentation are reported with `run_started`
  and `run_finished`.
- The watchdog pops any Calls frame the driver does not hold.

## Mock provider (`deckpilot-calls::testing`, feature `testing`)

The only provider that exercises **full control**: answer, reject, cancel,
end, microphone and camera. It is deterministic and runs on the engine
clock:

- a scripted timeline (ring, dial, join, hang up, the call app restarting,
  a lost connection);
- configurable capabilities, including a detection-only set like the
  Teams adapter's;
- a configurable response per action: confirmed after a delay (the call
  then shows the change), rejected with a code, never answered, or
  unknown.

The agent enables `deckpilot-calls/testing` only through its own
`test-hooks` feature (`apps/agent/Cargo.toml`), which no release build
has: a shipped agent contains no mock provider.

It never drives a real call app. Its tests (`testing/tests.rs`) run the
owner's list on the mock, the driver and a real core runtime:

- full control, with confirmed, rejected, unanswered and unknown outcomes;
- detection only, where nothing is sent;
- the call app restarting, and a lost provider;
- *Back to work*:
  - 20, 30 and 60 s, and a custom time;
  - after success, failure and cancellation, in each setting;
  - after a workflow's real end;
  - navigation counted only when set;
  - the 60 s maximum, and an early completion winning;
  - the meeting ending mid-return;
  - an agent restart;
  - a device disconnecting and reconnecting;
  - an incoming call preempting;
  - other devices, and `returnAllDevices`;
  - repeated presses with one session and no duplicate timer;
  - no stale restore;
  - the folder and page kept;
  - nothing run during the switch.

## In the agent (`apps/agent/src/service/calls.rs`)

**A locked device never shows a call.** Locking the configuration pops
every Calls frame at once, so the device draws "Locked", and none is pushed
while it stays locked, whatever the call does. Return sessions end, because
nothing resumes behind the lock. The engine keeps following the call, and
after unlocking a call still going on is shown again at once. Calls
installed while locked shows nothing until the configuration is unlocked.

A call already going on when its managed layout is seeded (Calls installed
mid-call, or while the configuration was locked) shows its overlay as soon
as the layout is stored, not at the call's next change. A layout that
cannot be stored while the database is busy is tried again after 1 s, then
after waits doubling up to a minute.

The agent holds the driver and a provider, and does nothing for Calls
unless a provider is installed. No build installs one today: the
detection-only Teams adapter and the plugin come later. Tests install the
mock through the test hooks (`AgentHandle::install_calls_mock`,
`calls_mock`, `calls_view`).

- **Clock.** The engine clock is the runtime's folder clock. The loop wakes
  when the driver's next deadline is due, and polls the provider at most
  every 250 ms. Nothing is spawned.
- **Overlay keys** go to the driver. A command it lets through goes to the
  provider, and a refusal goes back to the engine.
- **Runs of a suspended device** are followed to their real end, next to
  the folder holds:
  - an action run in place, and navigation: at once;
  - a plugin's action: at its `DetachedFinished`;
  - a workflow: at its `Execution::Finished` (succeeded, failed,
    cancelled; interrupted counts for nothing).
- **Connecting.** A device that connects during a call gets the overlay.
  One that was away past its return deadline comes back to the call.
- **Consent.** Calls is installed only with the user's consent, taken from
  the plugin's grants: observing (`calls:observe`), controlling
  (`calls:control`), the title opt-in. Nothing in the agent assumes it.
  Only the test-hook path grants it.
- **Managed layouts** are seeded into the configuration for every model
  that connects (`managed_layouts`, retried while the configuration is
  locked). The overlay shows the stored layout, so the user's edits and a
  restored version are what it shows.
- **A plugin action's end** counts as it ended: success, a cancellation
  (`cancelled`) as a cancellation, other errors as failures. An action
  that could not run at all is no completion.

Agent tests (`apps/agent/tests/calls.rs`, with a simulated MK.2; the
overlay is also drawn on an XL and a Plus, every class with key displays):

- the overlay drawn on the device (T-E2E);
- a workflow's real end (T-R3);
- the 60 s maximum over a slow run (T-R4);
- navigation not counting, an action run in place counting (T-R9, T-R2);
- the meeting ending mid-return (T-R5);
- unplugged and plugged in again (T-R6);
- an incoming call preempting (T-R7);
- the release of "Wróć do pracy" running nothing underneath, and a long
  press held across the switch running nothing (T-R10);
- a failed action and a cancelled workflow, for each `onFailure` and
  `onCancel` value (T-R2);
- a Multi Action brought back at its last step (T-R3);
- the stored layout seeded, a user edit shown, and an agent restart
  without a stale suspension (T-R6).

### A real plugin action, end to end (`tests/calls_plugin_action.rs`)

A spawned agent, the real sandboxed plugin host and the bundled test
fixture `example-default` built to wasm. Calls runs with the mock
provider, in a meeting from the start, through the test-only option
`--test-calls-mock <completion|default>` (the "Back to work" mode).

- **A test-hooks build only.** Only a `test-hooks` build has the option,
  and it needs `--headless`. A normal build has neither the option nor the
  mock: `plugins_feature.rs` checks that neither the default build nor
  `--features plugins` enables `test-hooks` or `deckpilot-calls/testing`.
- **The test, in mode B** (so no timer can bring the call back):
  1. "Back to work" shows the user's own key;
  2. 1.5 s later the device is still back at work;
  3. the user's key starts the plugin's `greet` action, which runs detached
     in the host;
  4. its `DetachedFinished` (Succeeded) brings the overlay back.
- **Log lines.** The agent logs both steps at `info` ("the device went
  back to work", "back on the call after a run ended"). They carry the
  device and the run's end, never a title.

## In the app (IPC protocol 19)

The agent answers `getCallsStatus` with a `CallsStatus` and pushes it again
as `callsChanged` whenever it changes, to subscribed desktop clients of
protocol 19 (hygiene: the IPC token is the access boundary, and any client
holding it may ask `getCallsStatus`). The managed layouts in it are read
from the store once and again only after seeding, *Restore defaults*, a
configuration reload or a lock, never on the Calls tick.
`CallsStatus` holds:

- whether Calls is installed;
- the call's state name and the provider id;
- whether any control is possible;
- per connected device: the overlay setting, whether it shows the call,
  whether it went back to work, and the call page it shows as the editor's
  preview (`preview`: `incoming`, `outgoing` or `active`; left out when
  there is none, so an older agent's status reads as no preview);
- per model: the managed layout's id, name, edited flag and template
  version;
- its `revision`, which counts up whenever anything else in it changes
  (the same content keeps its number) within one run of the agent. An
  answer and a `callsChanged` may reach the app in either order: the app
  keeps the higher revision, so an older status never replaces a newer one.

It never holds a meeting title, a participant or a process name.

- `setCallsOverlay { device, on }` turns the overlay off or on for one
  connected device (another is refused with `callsUnknownDevice`). An
  overlay shown now goes at once.
- `restoreCallsLayout { model, copyName }` is *Restore defaults*. It answers
  the id of the kept copy of the user's version, or `null` for an unedited
  layout.

Settings › Devices shows a **Calls** section (en and pl) only when the agent
has the capability and Calls is installed. The section has:

- the state;
- for detection-only Teams, "Controls: Not supported", with the reason
  (Microsoft retired its device API);
- a switch per device;
- the managed layouts with an "Edited" badge, and *Restore defaults*. Its
  confirmation names the copy that keeps the user's version
  ("Calls (my version, <date>)").

### Call pages and *Back to work* (C7)

Each managed layout in `CallsStatus` also carries:

- **`pages`:** its call pages (incoming, outgoing, active), and each one's
  controls with their Calls action. Each control says whether the provider
  supports it now (`supported`), and if not, why (`reason`, a stable code
  such as `provider-has-no-api`). The support is worked out from the
  provider's capabilities whenever the status is built, so it is never
  cached with the layout. "Back to work" is always supported.
- **`back`:** its *Back to work* settings:
  - `mode`, one of:
    - `timer { seconds }` (5–600; presets 20, 30, 60);
    - `completion`;
    - `completionOrMax { maxSeconds }` (10–1800; the default, with 60);
  - `onFailure` and `onCancel` (`resume` or `stay`);
  - `allDevices`.

`back` is read from the layout's first bound "Back to work" key (a user
may have bound it to more keys; the templates bind one), so Settings shows
that key's settings. `back` is absent when no key is bound (the template of
a device without displays has none).

`setCallsBackToWork { model, back }` sets them on every bound "Back to
work" key, in one transaction, each keeping its other parameters. It
refuses times out of range. They are the parameters of the layout's "Back to work" action, so
**a change is a user edit of the managed layout**: it counts as edited,
a template update then never rewrites it, and *Restore defaults* keeps it
first. The driver uses the new settings from the next press.

In Settings › Devices, each managed layout gets two more sections (en and
pl):

- **Call pages:** every control of every page, with "Supported" or "Not
  supported" and the reason. A disabled control stays listed, so muscle
  memory does not move. For the detection-only Teams adapter, the reason
  says Microsoft retired the Teams device API on 30 June 2026. Nothing
  claims a Teams control works.
- **Back to work:**
  - the mode;
  - for the timer, 20, 30 or 60 s or a custom time;
  - for the default mode, the maximum;
  - what a failed or cancelled run does;
  - all devices.

  Each change is applied at once. A time out of range is shown as such and
  not saved.

The per-device setting is kept by the running agent; saving it across
restarts comes with the plugin's settings (C7). Opening the managed layouts
in the Device Editor's layout chooser is left out for now (it needs a
non-profile load and save path, an owner decision); the preview of a call
page is below.

### The editor's preview of a call page (C6b)

`previewCallsPage { device, page }` shows one call page (incoming,
outgoing or active) of `device`'s managed layout on the device, for the
Device Editor. `endCallsPreview { device }` ends it. **This differs from the
design, which drove the preview through the mock provider.** The mock
exists only in `test-hooks` builds, so the shipped preview uses a fixed
snapshot instead (decided 2026-11-25):

- **Contents:** constant text only. The page's state, no title, no
  participant, no number, the microphone and camera unknown. Nothing comes
  from a provider or a real call except the provider's capabilities, so a
  control it does not support shows disabled with its reason. The keys run
  nothing.
- **Who may ask:** only a connection at protocol 19 that declares itself a
  desktop (editor) client. Any other client is refused with
  `Unauthorized` (`callsPreviewDesktopOnly`); a protocol 18 client never
  sends it. The client kind is the client's own declaration in its
  `hello`, so this is a policy guard, not authentication: the IPC token is
  the access boundary, and any holder of the token can declare itself a
  desktop client.
- **Priority:** a lock and a real call always win:
  - refused while locked;
  - refused while a call shows (`callsPreviewInCall`);
  - refused when the device takes no preview frame
    (`callsPreviewRefused`); a preview already there stays;
  - ended when a call comes.
- **When it ends:**
  - on the end request;
  - when the connection that asked closes;
  - on lock;
  - on the device's disconnect;
  - when dismissed on the device.

  It never leaves an overlay frame behind. Whichever way it ends,
  `callsChanged` no longer names it on the device's entry (a disconnected
  device leaves the list).
- **One per device:** a new preview replaces the old. The new frame is
  pushed first, so when the device takes none the old one stays.
- **In the app:** the preview lives on a connection of its own in the
  backend (`commands/calls.rs`, `CallsPreview`), one at a time. It is ended
  with `endCallsPreview` when the Calls settings close, the main window
  hides, reloads or closes, or the app quits; the connection closing ends
  it too. A per-meeting capability reason may show on a preview; it is
  informational only.
- **The app follows the agent:** the settings hold one preview (the device,
  the page, and whether the agent confirmed it). A new preview starts
  unconfirmed. Once `begin` answers, the app asks `getCallsStatus`; the
  agent answers in order, so that status is the one after the begin, and
  from its revision on the status decides: a status that shows the preview
  on that device confirms it, one that does not means the agent ended it
  (a lock, a call, a dismiss, a disconnect), confirmed or not. An older
  status decides nothing, and the answer itself goes through the revision
  check, so it never rolls back a newer `callsChanged`. If that answer
  fails, the preview is ended. A restarted agent has no preview, and an
  answer of an earlier agent run (to a request made before the restart) is
  dropped. A status
  that names the page on another device of the same model is not this
  preview, and the page buttons are pressed only for the device that
  shows it.

The IPC event for the app and the editor (C6, a release blocker) is
`callsChanged`, above. It carries the managed layouts too, and is sent
again after a configuration reload, so an edit made in the Device Editor
is seen. Still to come:

- a live signal source for the Teams provider (C5, after the macOS
  permission check);
- the plugin, whose enabling with consent installs Calls;
- the managed layouts in the Device Editor, with the state preview (C6b).

## Operating-system signals and Teams (`deckpilot-calls::os_signals`)

The `os-signals` provider with its Teams profile is detection only:

- **Inferred call.** A call or meeting is probably active while Teams holds
  an active audio capture session. The engine believes it after 2 s and
  lets it go 8 s after the session is gone.
- **Teams processes:** `MSTeams`, `Microsoft Teams` and
  `Microsoft Teams (work or school)` on macOS; `ms-teams.exe` and
  `Teams.exe` on Windows. They are compared by executable name only (never
  a path or a command line), and the names are dropped: only the call
  observation leaves the module. The records' `Debug` leaves names out,
  and the `pw-dump` parser reads only `media.class` and
  `application.process.binary` (never `media.name`, which may hold a
  title). A record over 4 MiB is refused.
- **The name can be spoofed:** any process named like Teams that holds the
  microphone shows a call (an indicator only; no control exists). Before a
  live source ships, it must pass a verified identity: the bundle id
  `com.microsoft.teams2` with its code-signing team id on macOS, the
  package family `MSTeams_8wekyb3d8bbwe` on Windows.
- **What is never claimed:**
  - The microphone's mute state is **unknown**: holding the microphone is
    never shown as "not muted", and `micReadState` is reported
    unsupported.
  - The camera is not attributed on macOS (`isInUseByAnotherApplication`
    names no app), and Windows has no supported signal for it.
  - On Linux Teams is a browser tab: nothing is attributed, and the
    provider reports `unsupported { platformNotSupported }`.
- **Every control is unsupported** (`providerHasNoApi`). A command is
  refused before anything happens, and the faces show the reason.
- **Signal sources.** The sources produce platform-neutral `Signals`
  through `SignalSource`:
  - Core Audio process objects with `kAudioProcessPropertyIsRunningInput`,
    and `AVCaptureDevice.isInUseByAnotherApplication`, on macOS;
  - WASAPI capture sessions on Windows;
  - PipeWire streams (`pw-dump`) on Linux.

  This change has the parsers of their outputs and a fixture source. **No
  live query is made by any build:** no platform source exists yet.
- **Owner verification, a release blocker:** whether the macOS Core Audio
  and AVCapture queries trigger a TCC (microphone or camera) prompt. A
  signal that would prompt is dropped, never prompted for. Also to check
  on a real machine: Teams' current process names, and whether Teams keeps
  the microphone stream open while muted (assumed yes).
- **Default:** the OS-signals provider is off until the user enables it
  (OD-C3). It inspects other processes' audio sessions.
