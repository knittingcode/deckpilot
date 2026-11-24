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
`credentials`, `openUrl` or `tools`. `Manifest::privileged_usages` lists
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

Agent tests (`apps/agent/tests/calls.rs`, with a simulated MK.2):

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

## In the app (IPC protocol 19)

The agent answers `getCallsStatus` with a `CallsStatus` and pushes it again
as `callsChanged` whenever it changes, to desktop clients of protocol 19.
`CallsStatus` holds:

- whether Calls is installed;
- the call's state name and the provider id;
- whether any control is possible;
- per connected device: the overlay setting, whether it shows the call, and
  whether it went back to work;
- per model: the managed layout's id, name, edited flag and template
  version.

It never holds a meeting title, a participant or a process name.

- `setCallsOverlay { device, on }` turns the overlay off or on for one
  device. An overlay shown now goes at once.
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

The per-device setting is kept by the running agent; saving it across
restarts comes with the plugin's settings (C7). Opening the managed layouts
in the Device Editor, and the state preview there, are still to come (C6b).

Still to come: a provider in a build (C5); the plugin (C7), whose
enabling with consent installs Calls; and the IPC event for the editor
(C6, a release blocker).
