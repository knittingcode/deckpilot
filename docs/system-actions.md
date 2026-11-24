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

`system.open-directory` (Open folder) keeps running as it is (opening a
subfolder of a folder resource), but since Open opens folders too it is
**deprecated**: hidden from new assignments and kept for saved ones
([3.5](#35-open-folder-and-open)); nothing is registered twice.

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

#### The key's icon

An Open key shows what it opens: the document icon (`file`, Open's own)
for a file, the open folder (`folder-open`, Open folder's) for a folder,
and the document icon when nothing is chosen or the type cannot be told.
The same icons of the UI icon set are used everywhere: the Device Editor's
canvas, the Inspector's key preview, previews of saved layouts and the
device itself.

Which icon a key shows, first match wins:

1. the key's own icon or image (the Inspector's *Icon*, `KeyAppearance`),
   drawn by the renderer over everything below;
2. the action's own icon, chosen in the action editor. New Open actions
   are made without one (`icon: null`), so an icon set later is a choice.
   Actions saved before this change were made with a copy of Open's icon,
   `file`; they are never rewritten, and an own icon of exactly `file` is
   treated as that default, not as a choice. A stored `file` cannot tell
   the two apart, so choosing *File* by hand also shows the automatic icon;
   to keep the document icon on a folder key, give the key its own icon
   (1.);
3. the automatic icon of the resolved target (`folder-open` or `file`);
4. Open's default document icon (`file`).

Nothing automatic is ever stored: a new Open action has no icon, an
older one keeps the `file` it was made with, and the key has no icon of
its own. A custom icon is never changed when the
target changes; removing it shows the automatic icon again.

**One computation.** The icon is chosen in the core's
`device_runtime::face_for` (`action_symbol`), the function the agent uses
for the device and the app's `preview_layout` uses for the canvas, the key
preview and layout previews: they cannot disagree. The `system` module
says how (`ActionHandler::automatic_symbol` of Open); the core resolves
the target (`deckpilot_core::target_kind`). Modules without the
`machine-resources` capability are told nothing about this computer's
targets.

**How the type is found.** The location is resolved as Open resolves it
(the resource key, or the project role for the active project, then the
optional `path` inside it, through `paths::existing_inside`: links
followed, never out of the resource), and only its filesystem metadata is
asked (`stat`). The target is never read, opened with its application or
run. A macOS application bundle is a folder on disk but starts as an
application: it shows the default icon. Unknown, so the document icon:
nothing chosen, an unbound key, a project role without a project, a
binding that is not a file or folder (`application`, `url`, …), a value
that is not a local absolute path (network paths such as `\\server\share`
or `//server/share` are never looked at), a `path` or location given by a
variable (known only when the action runs), a missing file or folder, a
permission error, a link out of the resource, or a lookup that takes more
than 250 ms (a sleeping network drive), and a FIFO, socket or device
file (only `stat`, never opened). Every lookup's time counts against a
budget of 1 s per event: after it, the targets not looked at yet are
unknown until the next event, so a page of keys on a slow mount holds
the agent for about a second at most, once; a path whose lookup has not
returned is not looked up again until it does.

**When it is looked at again.** Answers are cached by resolved path
(`TargetCache`) and never polled. The agent forgets them when the machine's
resource bindings, the modules or the profile are loaded again (saving an
action or a binding does that), when a live preview gives a device a
working layout, and when a device switches layout. The app's previews
forget theirs whenever anything is written to the database (an action, a
binding, a layout) or another layout is previewed. A file that is replaced
by a folder underneath, with nothing saved in DeckPilot, keeps its old
icon until the next such event.

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
its bundle id checked again. Wherever a process is checked again (before a
signal, and while waiting for it to exit), its id, executable, owner and
start time must all still match, so a later process the system gave the
same id, even of the same program, is never taken for the matched one. A
start time that cannot be read is never a match: nothing is signalled and
the process counts as gone. The only source is `sysinfo` (`/proc` on Linux,
`proc_pidinfo` on macOS, `GetProcessTimes` on Windows), which reads it for
the user's own processes on all three systems; `the_real_snapshot_knows_this_process`
and the real close tests assert it on each CI runner.

DeckPilot's own processes are protected by name (the app, the agent, the
plugin host) and, on macOS and Windows, by their installation folder (the
`.app` bundle, the program folder). Descendants of the agent are not
protected as such: an application Open Application starts directly (Linux,
Windows) is the agent's child, and closing it is the point.

**Imports.** A resource key is predictable only until it is bound: the
pickers add a random suffix (`app.safari-k3x9q2`). An import lists every
Close Application action (`ImportPreview.close_actions`), the profiles'
and the layout library's (actions of layouts no profile uses), one entry
per copy, with whether it forces and whether its key is bound here. Before
anything is planned, every Close Application action whose id the user did
not allow in the review (`ImportOptions.confirmed_close_actions`) is made
unbound in every copy, by id (`import.close-<id>`, `force` off, its
`application` and `force` overrides and mappings removed in profiles and
library layouts alike), so the library's copies, stored after the
profiles', cannot restore it. Allowing an id allows every copy of it.

Outputs: `instances` (matched), `closed` (all gone), `forced`. Not running
is a success with `instances` 0.

### 3.5 Open folder and Open

The Action Library offers **Website, Open, Open Application and Close
Application** in the System category. Open folder (`system.open-directory`)
is marked `deprecated` in its definition, the mechanism Wait uses for
Delay: it is left out of the Action Library, the action editor's types and
the workflow catalog's new nodes (`creatableTypes`), while

- saved Open folder actions, their keys, gestures, Multi Action steps and
  workflow nodes keep running unchanged (the handler is still registered,
  same id, same parameters), keep their folder icon, and stay editable
  with the folders-only picker;
- a saved Open folder action is still listed in the workflow catalog
  (saved actions are listed whatever their type), so existing workflow
  steps render and can be placed again;
- nothing is rewritten: no saved id, parameter or icon is changed.

**Optional migration (not run).** An Open folder action can be turned into
an equivalent Open action by hand, and a future version could offer it as
an explicit, user-confirmed step: create an Open action with the same
`location` and `path`, rebind its keys, gestures and steps to it, then
delete the old one. It is backward compatible in behaviour because Open
resolves the same parameters with the same containment rules
(`existing_inside`); the differences are that Open also accepts a file
(Open folder refuses a file with `not-found`) and that its key's icon then
follows the target (a folder keeps the open folder). It is not done
automatically: it rewrites user data, and a profile exported after it
needs a DeckPilot whose Open takes a folder without `path`.

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
| Discovery | standard application folders and the Finder | App Paths, Start menu | XDG data dirs |
| Browsers | `LaunchServices` `https` handlers (macOS 12+; `Info.plist` before) | `StartMenuInternet` | desktop entries with `x-scheme-handler/http(s)` |
| Default browser | `LaunchServices` | `UrlAssociations\https\UserChoice` | `mimeapps.list` |
| Icons | `NSWorkspace` | `SHGetFileInfoW` | `hicolor` PNGs, `/usr/share/pixmaps` (no SVG) |
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

The inspector, the Action Library and workflow nodes draw the four actions
(and Open folder) with machine-local pickers
(`components/machine-resource-field.tsx`). The simple mode is the default;
no resource key, project role or internal name shows in it.

| Action | Field | Simple mode |
| --- | --- | --- |
| Open Application, Close Application | Application | A searchable list of the installed applications with their icons (`list_applications`, `application_icon`), and *Another application…*: the native dialog limited to `.app` (macOS), `.exe` and `.lnk` (Windows), `.desktop` files or any file (Linux), turned into a stable id by `describe_application` |
| Website | Open with | *Default browser (Name)* first, then every installed browser with its icon (`list_browsers`); a browser that is not installed any more is said so, and the page opens in the default browser (`fallback: default-browser`) |
| Open | File, folder or application | Its name and path, *Choose file…* and *Choose folder…* (the native dialogs of the Tauri dialog plugin) with what each does and that the key's icon follows; the file inside a folder (`path`) is an optional disclosure |
| Open folder (saved actions only) | Folder | Its name and path, *Choose folder…* |
| Close Application | Force quit | A toggle, off, with what graceful and forced mean |

A choice is bound in Settings › Resources under a generated key
(`app.safari-k3x9q2`, `file.report-pdf-…`, `folder.downloads-…`), shown
with a *This computer only* badge, and never exported. *Advanced* switches
a field to the resource key or project role control; a saved action whose
key is not a picker's (`shop.repository`) or that uses a project role opens
in advanced mode and runs unchanged.

**Native services** (desktop backend, `commands/machine.rs`, all reading
`deckpilot-apps` off the main thread, nothing started, no shell; each
answers the main window only and refuses other windows):

- `list_applications`: `discover()` (bundles and the Finder on macOS; App
  Paths, registered browsers and Start-menu shortcuts on Windows; XDG
  desktop entries on Linux).
- `list_browsers`: `browsers::browsers()`: macOS `LaunchServices`
  (`URLsForApplicationsToOpenURL`, `URLForApplicationToOpenURL`); Windows
  `StartMenuInternet` and the `https` `UserChoice` (else
  `HKCR\https\shell\open\command`); Linux desktop entries with
  `x-scheme-handler/http(s)` and the `mimeapps.list` files
  (`$XDG_CURRENT_DESKTOP-mimeapps.list` first in each folder, `https` then
  `http` in each file, in XDG precedence).
- `application_icon(id)`: only for an id `discover()` or `browsers()`
  reported, or that a picked file was described as when it is an
  application the system knows (a bundle with an identifier, an installed
  desktop entry, an App Paths registration). A plain path described by
  hand never gets an icon, so no file a caller names is ever read, and a
  network path (UNC, `\\?\`) is refused. Drawn by the system
  (`NSWorkspace iconForFile` into a new bitmap inside an autorelease pool;
  `SHGetFileInfoW` with COM initialized on the thread; on Linux the
  `hicolor` theme or `/usr/share/pixmaps`, where `..` is refused and every
  candidate's real path, links resolved, must be inside those folders, or,
for NixOS and Guix themes, a PNG in `/nix/store` or `/gnu/store` linked
from inside them),
  decoded, scaled to at most 64×64 and encoded again as a PNG of at most
  64 KiB. At most 8 requests run at a time; up to 512 icons are cached,
  the oldest dropped first, and a failed icon is asked for again.
- `application_present(id)`: whether a bound application is still
  installed (what Open Application would open exists), so an application
  chosen outside the listed folders is not called missing; unknown (no
  alert) for an `AppUserModelID` or a bare name, which cannot be checked
  without starting it.
- `describe_application(path)`: a picked file as its stable id.
- `resource_status(key)`: whether what a picker bound under `key` is still
  there (`ok`, `missing`, `unbound`, `unchecked`), checked as
  `check_resources` checks it for Settings › Resources: a folder must be a
  folder, a picked file (kind `other`, an absolute path) must exist, an
  application is checked as Open Application resolves it (bundle id,
  desktop-file id, App Paths name or path; an `AppUserModelID` or a bare
  name is `unchecked`), and network paths are never looked at. It takes a
  key, never a path.

Website validation, the inspector panels (Save and Test, the long press of
Open Application) and the localized names are as in #409. Dropping a file
onto a field is not offered: the webview's drop gives no paths, and the
window's drop events are not wired to fields.

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
| Open folder hidden from new assignments only | `open_folder_is_kept_for_saved_actions_only` (system module), `every_builtin_action_declares_its_contexts` (contracts), `saved_open_folder_keys_keep_running_with_their_folder_icon` (`crates/deckpilot-modules/tests/open_icon.rs`), `types_that_need_a_value_are_saved_as_drafts_until_configured` (desktop), `layout-editor/open-unified.test.tsx` (library, action editor and workflow catalog) |
| The key's icon | `target_kind::tests` (core: metadata only, links, missing, network, unresolved, the cache cleared by events, macOS bundles), `open_shows_a_folder_or_a_document_for_its_target` (system module), `crates/deckpilot-modules/tests/open_icon.rs` (file, folder, switching both ways, clearing, missing, unknown and unresolved targets, project roles, custom icons and their removal, saved layouts store no icon, the device and the preview draw the same frames), `open_keys_preview_the_icon_of_what_they_open` (desktop `preview_layout`), the `open` golden set (`crates/deckpilot-render/tests/golden.rs`), the drawn key in `layout-editor/open-unified.test.tsx` |
