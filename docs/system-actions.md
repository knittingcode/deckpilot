# System actions: Website, Open, Open Application and Close

Status: **implemented** on `feat/system-actions`, stacked on the action
contexts of PR #407. This page records the audit made before the work, the
decisions and their evidence, and the resulting contract. The action
reference stays in [integration-modules.md](integration-modules.md#system-system).

## 1. Audit

What existed before this work, read from the code (0.12.0 + #407):

| Requested | Existing id | What it did | Gap |
| --- | --- | --- | --- |
| Website | `system.open-url` (`modules/system/src/actions.rs`) | Opens `url` (text) with the platform launcher. `deckpilot_core::paths::web_url` accepted `http` and `https` with a host, and `mailto` | No choice of browser in the action (the core's `launch::url_plan` could already open a URL in a bound application, used by the `workspace` plugin); user names and passwords in the URL were accepted; a URL without a scheme was refused instead of normalised; on macOS a browser bound by bundle id could not be used (`open -a <bundle id>`) |
| Open | `system.open-file`, `system.open-directory` | Open a file inside a resource (relative `path`, required) or a folder; `existing_inside` refuses `..`, absolute paths and links that leave the resource | A bound file or application could not be opened by itself (`path` was required); errors did not say that a resource moved |
| Open Application | `system.open-app` | Starts an `application` resource (path, macOS bundle id, Linux `.desktop` id) with arguments | No discovery: the user had to type a path or id into Settings › Resources. Linux `.desktop` ids needed `gtk-launch` (GTK only); Windows had no stable id at all (App Paths names and AppUserModelIDs did not launch) |
| Close | none | — | Missing entirely. `system.process-status` only counts processes by normalised name (a query, never a target) |

Other findings:

- **One launch service.** `deckpilot_core::launch` (`plan`, `Launcher`,
  `SystemLauncher`, `RecordingLauncher`) is already shared by the `system`
  module, the plugin host's `resource.open` and `url.openParam`
  (`apps/agent/src/plugins/executor/open.rs`) and the plugin `url.open`
  opener. It never uses a shell: `open` (LaunchServices' front end) on
  macOS, `xdg-open` on Linux, `rundll32 url.dll,FileProtocolHandler` and
  `explorer` on Windows, or the program itself with an argument list.
- **URL rules.** Two rule sets existed: the built-ins' `paths::web_url`
  (lenient) and the plugin host's `literal_url` (strict: no user info,
  printable ASCII, no backslash, literal host, 2,048 bytes). They now share
  `paths::http_url_problem` (section 3.1).
- **Machine-local values.** Paths and application ids never appear in a
  profile: a parameter refers to a logical resource key
  (`ResourceRef::Fixed` or a project role) and each computer binds the key
  in its `MachineConfig` (Settings › Resources, "Zasoby"), which is never
  exported. Exports list the keys and kinds they need
  (`deckpilot-portability`, `payload.resources`), and an import reports
  `missing_resources` to bind again. This is reused unchanged.
- **Permissions.** Modules declare capabilities in their manifest
  (`Capability`, shown in Settings › Modules); the registry restricts each
  handler's `ExecutionContext` to them. The `system` module declared
  `machine-resources`, `open-external`, `process-execution` and
  `process-inspect`. Plugins have their own, consented permissions
  (`resources:open`, `url.open`, …) enforced by the broker, and those
  privileged calls run only in runs the user started
  (`RunOrigin::is_user_initiated`).
- **Built-ins and run origins.** The built-in `system.open-*` actions do
  **not** check the run's origin: a scheduled or event-started workflow can
  open a page, file or application today. This is unchanged (section 4).
- **Gestures.** Double click and long press are ordinary bindings
  (`Trigger::DoubleClick`, `Trigger::LongPress`) of an action or workflow
  on the same control (`crates/deckpilot-core/src/layout.rs`,
  `components/gesture-editor/model.ts`: `setGesture`,
  `convertToGestures`). Gesture Presets (`GesturePreset`, plugin API 1.10)
  are unreleased and live only on the owner's `feat/gesture-presets`
  branch; a preset carries **fixed** parameters, so it cannot express
  "close the application this key opens" anyway.
- **Contexts (#407).** `system.open-*` are declared key, gesture, sequence
  and workflow; the contract test `every_builtin_action_declares_its_contexts`
  holds a table of every built-in.

## 2. Architecture decision

The four actions are host capabilities, so they belong to the native
**System module**, not to a plugin. Three of them already existed there and
are **extended in place, keeping their ids and every saved parameter**:

| Name in the app | Id | Status |
| --- | --- | --- |
| Website | `system.open-url` | extended: optional `browser`; stricter, normalising URL rules |
| Open | `system.open-file` | extended: `path` optional, so a bound file, folder or application opens by itself |
| Open Application | `system.open-app` | extended: discovered applications, stable ids, native resolution per platform |
| Close Application | `system.close-app` | **new** |

`system.open-directory` stays as it is (opening a subfolder of a folder
resource); nothing is registered twice.

Platform code lives in a new crate, **`crates/deckpilot-apps`**: application
discovery, identifier parsing, Linux `.desktop` and macOS `Info.plist`
reading, process matching and termination. Like `deckpilot-presence`, it
allows `unsafe` only in two small documented modules (the AppKit calls on
macOS, the Win32 calls on Windows); the rest of the workspace still forbids
it. Launching stays in the **single** launch service of the core
(`deckpilot_core::launch`): Open, Open Application and Website all go
through `open_with` → `launch::plan` → `Launcher`. `deckpilot-apps` only
turns a stable id into what that service opens (a `.desktop` id into its
`Exec` program and arguments, an App Paths name into its executable, an
AppUserModelID into `shell:AppsFolder`).

## 3. The actions

### 3.1 Website (`system.open-url`)

Parameters: `url` (required), `browser` (optional application resource,
"Open with"; empty: the default browser). The key's title is the key's own
title, edited in the inspector's header as for every key.

`paths::website_url` validates and normalises:

- trimmed; a value without a scheme gets `https://` (`example.com/docs` →
  `https://example.com/docs`);
- only `http` and `https` with a host; `mailto:` is still accepted **for
  saved actions** (it was documented before) and only without a browser;
  every other scheme (`file:`, `javascript:`, `data:`, custom handlers) is
  refused with `invalid-url`, because those can start programs;
- the checks shared with the plugin host (`paths::http_url_problem`): no
  user name or password, no backslash, no control characters, at most
  2,048 bytes after normalisation;
- normalised by the URL parser (lower-case scheme and host, an
  internationalised host in its `xn--` form, spaces percent-encoded).

The result is opened by the shared launch service: the default browser, or
the bound browser (`open -b <bundle id> <url>` / `open -a <app> <url>` on
macOS, the `.desktop` entry's `Exec` with `%u` on Linux, the executable on
Windows). A missing browser is `not-found` before anything starts; a
launcher that fails is `open-failed` or `program-not-found`.

### 3.2 Open (`system.open-file`)

Parameters: `location` (resource: a file, folder or application), `path`
(now optional, relative inside a folder resource), `application`
(optional "Open with"). Without `path` the bound resource itself opens with
its default association (a file in its application, a folder in the file
manager, an application bundle starts). Existence is checked first;
`not-found` says the resource was moved or deleted and where to bind it
again. Containment rules (`existing_inside`) are unchanged.

### 3.3 Open Application (`system.open-app`)

Parameter: `application` (application resource), `arguments` (one per
line, unchanged). The inspector's application picker lists the installed
applications found by `deckpilot_apps::discover` and binds the chosen one
under a resource key such as `app.safari`, storing:

| Platform | Stable id stored | Discovery | Launch |
| --- | --- | --- | --- |
| macOS | bundle id (`com.apple.Safari`) | `.app` bundles in `/Applications`, `/Applications/Utilities`, `/System/Applications(/Utilities)`, `~/Applications`, read from `Contents/Info.plist` (no `mdfind`) | `open -b <id>` (LaunchServices) |
| Windows | App Paths name (`chrome.exe`), or an AppUserModelID typed in Settings › Resources | `HKCU`/`HKLM` `…\CurrentVersion\App Paths`, and Start-menu shortcuts (`.lnk`, launched as files) | App Paths: the registered executable; AUMID: `explorer.exe shell:AppsFolder\<AUMID>` |
| Linux | desktop-file id (`org.gnome.Terminal.desktop`) | `$XDG_DATA_HOME/applications`, then each `$XDG_DATA_DIRS/applications` (XDG Desktop Entry spec: first id wins, `Hidden` removes, `NoDisplay` hides, `TryExec` must exist) | the entry's `Exec`, split by the spec's quoting rules, field codes removed, started directly |

Where no stable id exists (a plain executable, a portable app, a picked
shortcut) the absolute path is stored instead. Saved values (paths, bundle
ids, `.desktop` ids, names on `PATH`) keep working.

**Long press.** The inspector's Open Application panel offers *On long
press: None / Quit application*. Choosing *Quit application* creates (or
reuses) a `system.close-app` action for the same application resource and
binds it to the key's existing `LongPress` trigger, converting a raw press
to a click first exactly as the gesture editor does (`convertToGestures`,
`setGesture`). No new gesture engine, no new trigger, no Gesture Preset and
no plugin API change: it is the same binding a user can make by hand in the
gesture editor, and it shows there.

### 3.4 Close Application (`system.close-app`)

Parameters: `application` (application resource, the same binding as Open
Application), `force` ("Force quit if it does not close", off).

1. **Identify** the application by its stable id or exact path only
   (`deckpilot_apps::Target`): a macOS bundle id (from the binding or the
   bundle's `Info.plist`), a Windows AUMID or exact executable path (App
   Paths names resolved through the registry), a Linux `.desktop` id
   resolved to the exact, canonical executable of its `Exec`, or an exact
   executable path. Process **names are never compared**, not even as a
   fallback; a `.desktop` entry whose program is a script, an interpreter
   or a sandbox launcher (`flatpak`, `snap`, `env`, `sh`, `python`…) is
   refused with `not-identifiable`, because its processes cannot be told
   apart from others.
2. **Match** the running processes of that exact application that are
   owned by the current user (the same user id; on Windows the same SID
   and session). Multiple instances: **every** instance of that exact
   application owned by the user is closed, and the result reports how
   many.
3. **Refuse** protected processes before acting (`protected-process`):
   process ids 0 and 1, DeckPilot itself, its agent and its parent,
   and a deny-list of session-critical programs by exact name or bundle id
   (macOS `WindowServer`, `loginwindow`, `launchd`, `Dock`, `Finder`,
   `SystemUIServer`, `ControlCenter`; Windows `explorer.exe`, `csrss.exe`,
   `winlogon.exe`, `wininit.exe`, `services.exe`, `lsass.exe`, `smss.exe`,
   `dwm.exe`, `svchost.exe`, `sihost.exe`, `fontdrvhost.exe`; Linux
   `systemd`, `init`, `Xorg`, `Xwayland`, `gnome-shell`, `gnome-session-*`,
   `kwin_*`, `plasmashell`, `mutter`, `sway`, `dbus-daemon`, display
   managers, `pipewire`, `pulseaudio`, `wireplumber`).
4. **Close gracefully**: macOS `NSRunningApplication.terminate` (for a
   bundle) or `SIGTERM`; Windows `WM_CLOSE` to the application's visible,
   unowned top-level windows; Linux `SIGTERM`. Then wait up to 5 s.
5. **Force** only with `force` on, for the instances still running:
   macOS `forceTerminate` or `SIGKILL`, Windows `TerminateProcess`, Linux
   `SIGKILL`. Without `force` an application that keeps running (it may be
   asking to save) is reported, never killed. When the run is cancelled or
   its deadline passes during the grace period, nothing is forced either:
   the result is `interrupted`.

Never as root or elevated: when DeckPilot runs as root (Unix) or with an
elevated token (Windows), Close Application refuses with `elevated`, since
the processes it would see may belong to another account.

Process ids are pinned before acting: on Linux a pidfd is opened, the
process's identity checked and the signal sent through the pidfd
(`pidfd_open`, `pidfd_send_signal`; a check right before `kill` where
pidfds are missing); on Windows a handle whose executable was checked is
held while the windows are asked to close, and `TerminateProcess` uses
such a handle; on macOS a bundle's instance is found by its process id and
its bundle id checked again.

DeckPilot's own processes are protected by name (the app, the agent, the
plugin host) and, on macOS and Windows, by their installation folder (the
`.app` bundle, the program folder). Descendants of the agent are not
protected as such: an application Open Application starts directly (Linux,
Windows) is the agent's child, and closing it is the point.

**Imports.** A resource key is predictable only until it is bound: the
pickers add a random suffix (`app.safari-k3x9q2`). An import lists every
Close Application action (`ImportPreview.close_actions`, with whether it
forces and whether its key is bound here) and imports each one unbound
(`import.close-<id>`, `force` off, its `application` and `force`
overrides removed) unless the user allows it in the review
(`ImportOptions.confirmed_close_actions`).

Outputs: `instances` (matched), `closed` (all gone), `forced`. Not running
is a success with `instances` 0.

## 4. Contexts, run origins and permissions

Contexts (#407 table): all four are **key, gesture, sequence, workflow**.

| Action | Contexts | Runs that are not user-initiated (schedule, event, start-up, work session, simulated press) |
| --- | --- | --- |
| `system.open-url` | key, gesture, sequence, workflow | run, as before |
| `system.open-file` | key, gesture, sequence, workflow | run, as before |
| `system.open-app` | key, gesture, sequence, workflow | run, as before |
| `system.close-app` | key, gesture, sequence, workflow | **refused** with `not-user-initiated` |

Why: the plugin broker allows privileged calls (`url.open`,
`resource.open`) only in runs the user started. The built-in openers have
never checked the origin, and restricting them now would break saved
scheduled workflows, so that is documented rather than changed (nothing is
weakened). Close is new and can lose unsaved work, so it starts with the
broker's rule: a key, a gesture, the app or a workflow the user started.

Capabilities (Settings › Modules › System): opening a page, a file or an
application keeps using `open-external` and `machine-resources`. Close
adds **`process-control`** ("Quit applications you run"), declared by the
`system` module only, and also needs `process-inspect` to find the
processes. No plugin-facing permission is added; plugins cannot reach
these actions' code.

Every value is untrusted: resource values, URLs and ids are validated
before use, nothing goes through a shell, and every program is started
with an argument list.

## 5. Platform differences

| | macOS | Windows | Linux |
| --- | --- | --- | --- |
| Default browser / file association | LaunchServices (`open`) | `rundll32 url.dll,FileProtocolHandler`, `explorer` for folders | `xdg-open` |
| Chosen browser | bundle id or `.app` | executable or App Paths name | `.desktop` id (`%u`) or executable |
| Stable app id | bundle id | App Paths name, AUMID | desktop-file id |
| Discovery | standard application folders | App Paths, Start menu | XDG data dirs |
| Graceful close | `terminate` (bundle) / `SIGTERM` (path) | `WM_CLOSE` to top-level windows | `SIGTERM` |
| Force quit | `forceTerminate` / `SIGKILL` | `TerminateProcess` | `SIGKILL` |
| Owner check | user id | SID and session | user id |
| Multi-call binaries | — | — | an executable that serves several programs (BusyBox) matches all of them, since they run the same executable |
| Packaged (AUMID) apps | — | matched by `AppUserModelID`; their windows usually belong to `ApplicationFrameHost.exe`, so `WM_CLOSE` does not reach them and only force closes them | — |

Launching keeps the core's launch service as it was (`open`, `xdg-open`,
`rundll32`/`explorer` or the program itself, always with an argument
list): `open` is the LaunchServices front end, and replacing it with
`NSWorkspace` calls would add an asynchronous completion path for no gain
in safety. What changed is how an application value is resolved before the
plan is made (`deckpilot_apps::launchable`).

Honest limitations:

- Windows: an application without a visible top-level window cannot be
  closed gracefully (only forced), which includes most packaged (AUMID)
  apps; AppUserModelIDs are not listed by the picker (they can be bound by
  hand) and a Start-menu shortcut can be opened but not closed (its target
  is not resolved). Close Application refuses to run elevated, which on a
  Windows account without UAC means always.
- Linux: an application started through a wrapper script, Flatpak or Snap
  cannot be closed (refused, never guessed). Some distributions start
  browsers through a script (`/usr/bin/firefox` on Debian), so those are
  refused too; binding the real executable works. D-Bus-activated entries
  are started through their `Exec` line, and `Terminal=true` entries are
  refused (bind the program or a terminal).
- macOS: the picker lists the standard application folders only; an
  application elsewhere can be picked with the file picker.

## 6. User interface

The app draws the four actions from their schemas as before, with:

- **Machine-local pickers** (`components/machine-resource-field.tsx`) for
  every application, browser and location parameter of the four actions,
  in the Action Library, the inspector and workflow nodes: the installed
  applications (`list_applications`, browsers listed first for Website),
  another application through the file picker (`describe_application`
  turns an `.app`, an installed `.desktop` file or an App Paths executable
  into its stable id), and a file or folder through the native picker.
  The choice is bound in Settings › Resources under a generated key
  (`app.safari`, `file.report-pdf`, `folder.downloads`; a key that holds
  another value gets a number, so no other action changes), shown with a
  *This computer only* badge and the note that it is never exported. The
  raw resource key or project role stays available under a disclosure.
- **Website validation** as the agent does it (`lib/system-actions.ts`,
  the same cases as `paths::website_url`'s tests): the refused scheme,
  credentials or character is named, and a normalised address is shown
  ("Opens https://example.com/").
- **Inspector panels** (`layout-editor/system-action-inspector.tsx`) for a
  key bound to one of the four: the action's settings with Save and Test
  (a failed test says what its error code means), the graceful or forced
  behaviour of Close Application in words, and *On long press: Nothing /
  Quit the application* for Open Application.
- **Localized names** of the System actions and their parameters (English
  and Polish), in the Action Library, the editor's library and the
  workflow catalog; the `process-control` capability in Settings › Modules.

## 7. Tests

| What | Where |
| --- | --- |
| URL validation, normalisation and refused schemes | `deckpilot-core` `paths::tests::websites_are_validated_and_normalised`; the same cases in `lib/system-actions.test.ts`; `websites_are_normalised_and_other_schemes_refused` (system module) |
| Discovery against fixtures | fake `.app` bundles with `Info.plist` (`bundle`, `discovery`), fake XDG data folders with precedence, `Hidden`, `NoDisplay`, `TryExec` (`xdg`), fake Start-menu folders and registry value parsing (`windows_apps`) |
| `.desktop` `Exec` parsing | quoting, escapes and field codes, never a shell (`desktop_entry`) |
| Launch and open through a mocked platform | `RecordingLauncher` and `RecordingApps` in the system module's tests: default and chosen browser, a bound file, folder or application opened by itself, missing and unbound resources, a failing launcher |
| Real launch and close | `crates/deckpilot-apps/tests/real.rs`, only ever against the crate's own test helper (`src/bin/deckpilot-apps-test-helper.rs`) started by the test: graceful close, a helper that ignores `SIGTERM` (or has no window on Windows) is not killed without force and is with it, `WM_CLOSE` to the helper's window on Windows, and on Linux launching and closing the helper by a desktop-file id |
| Matching, owner check, deny-list | the close procedure against a fake platform (`close::tests`): every instance of the user, never another user's, never a name match, protected processes untouched even when they match, failures reported |
| Permissions and run origins | `close_application_needs_its_capabilities`, `close_application_runs_only_when_the_user_started_it`, `closing_is_a_capability_of_the_system_module_only`, the contract tests |
| Contexts and gestures | the #407 contract table; `crates/deckpilot-modules/tests/system_actions.rs` (`quit_on_long_press_is_an_ordinary_gesture_binding`); the inspector's long-press model and panel (`layout-editor/system-actions.test.tsx`) |
| Saved configurations | `saved_open_actions_keep_their_parameters`: the old `system.open-*` parameters round-trip as JSON, validate and run |
