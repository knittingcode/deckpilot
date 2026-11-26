# Changelog

All notable changes to DeckPilot. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/). The section of a version is used
as the notes of its GitHub release and of the in-app update.

**Release status.** The first stable release is 0.6.0 (2026-11-17). The
public pre-releases before it were 0.6.0-alpha.3 and 0.6.0-alpha.4
(0.6.0-alpha.1 and 0.6.0-alpha.2 were not published). 0.5.0 existed only
as two signed draft prereleases (0.5.0-1 and 0.5.0-2) made to test signing,
notarization and packaging, since deleted; they were never made public and
are not offered by the updater. The versions before 0.5.0 were never
released either. 0.6.0 is the first version intended for publication.

## [Unreleased]

### Added

- Projects (builds with plugins): a project repository can be linked to a
  repository on one of your connections. Choose the connection, find the
  repository there, check what will be linked and confirm; the link keeps
  the provider's own id (`gh:…`, `gl:…`), so a rename on the server keeps
  it, and only the app writes it. In the project editor, each saved
  repository lists its remotes and has a "Link remote…" button; the link is
  saved at once, not with the project form.
- Link remote suggests the remotes of the repository's folder: DeckPilot
  reads them with `git remote -v` (read-only; it never fetches, pulls or
  writes, and leaves out any user name, password or token in a URL), and
  for a connection on the same server "Find origin on …" asks the
  provider for exactly that repository. You still confirm the link; when
  the provider does not know it, you search by its path.
- Settings › Connections (builds with plugins): the servers plugins connect
  to, per plugin that declares them, with their state (connected, token
  rejected, missing permission, can't reach the server, certificate not
  trusted, turned off), the account, Default and Read-only. Add a
  connection to GitHub.com or GitLab.com, or to GitHub Enterprise Server or
  a self-managed GitLab at the address you open in the browser (a port or
  a path is fine), with the token's permissions explained in plain words,
  read-only unless you allow changes, and a private network only when you
  say so; it is tested at once. Test, Replace token, Turn off and Remove
  are on each connection. The token is typed once and never shown again.
  In English and Polish.
- Plugins: a connection's HTTP answers carry GitHub's `x-oauth-scopes`,
  `x-accepted-oauth-scopes` and `github-authentication-token-expiration`
  (scope names and a date), so a connection's test can name a missing
  permission or an expiring token; no other plugin request gets them. A
  connection's refusals always start with their code (`redirect-refused:
  …`).
- Plugins: connections (plugin API 1.14, host side). A plugin that
  declares connection types (DeckPilot's own bundled plugins only) can be
  connected to several servers, each with its own address, consent, token
  and status: GitHub.com and GitLab.com at their own addresses, GitHub
  Enterprise Server and self-managed GitLab at the address the user opens
  in the browser (`https` only, with a port or a path). Tokens are kept in
  the encrypted plugin database, protected like the configuration (its key
  derives from the configuration's), are never sent back to the app, logged, exported or handed to
  the plugin, and are injected by the agent only into requests to their
  own server, and only to a connection the user chose for the action (never
  one a workflow maps in); a redirect off that server ends the request, certificates
  are checked against the system's trust store with no way to skip it,
  and a server on a private network needs its own consent (this computer
  and link-local addresses never). A plugin reaches only the connections
  an action's parameters name. The app's requests to list, create, test,
  rename, turn off and remove connections are IPC protocol 20; the plugin
  store goes to schema 7. No plugin uses them yet.
- Calls (groundwork): a locked device never shows a call. Locking the
  configuration takes the Calls overlay off every device, and it comes back
  after unlocking if the call is still going on.
- Calls (groundwork, tests): "Back to work" is followed to the end of a
  real plugin action through a spawned agent and the sandboxed plugin
  host. A test-only agent option installs the mock call provider; normal
  builds do not have it, which a test checks.
- Calls (groundwork, nothing user-visible yet): the provider contract
  with capability discovery, where every capability a provider does not
  offer is explicitly unsupported with a reason; the sanitised call state;
  the `call.state` and `call.control` operations and the gate a control
  passes (no success without the call app's confirmation); and the plugin
  manifest's `calls` permission, `callOverlay` and `layoutTemplates` for
  plugin API 1.16, accepted only for the bundled Calls plugin. No
  DeckPilot provides plugin API 1.16 yet. Microsoft Teams can only be
  detected, not controlled: Microsoft retired its device API on
  2026-06-30 (`docs/calls.md`).
- Calls (groundwork): the call state engine (incoming, outgoing,
  connecting, active, ending, ended, failed, unknown) with a timeout on
  every live state, missed, rejected and remote hang-up, provider and app
  restarts, meeting-to-meeting moves and simultaneous calls; and the
  "Back to work" return sessions (timer, real completion, or whichever
  comes first, 60 s by default), one per device. Tested on a virtual clock;
  nothing user-visible yet.
- Calls (groundwork, nothing user-visible yet):
  - **Per-device overlay.** A device can show the Calls layout without
    anything of its own presentation changing: profile, page, folder and
    remembered layouts stay as they were. Keys held during the switch run
    nothing, Auto Exit pauses underneath, and a protected key always
    dismisses the overlay.
  - **Managed Calls layouts.** One per device model, generated for its
    capability class with stable ids. They are regenerated only while
    unedited, and "Restore defaults" first keeps your version as a layout
    of your own. This adds storage schema 15, which is additive.
- Calls (tests): a mock call provider with full control (answer, reject,
  cancel, end, microphone, camera), used only by tests. It runs the
  "Back to work" scenarios on a virtual clock. No real call app is
  driven.
- Calls (groundwork, off in every build): the agent runs the Calls overlay
  when a provider is installed. "Back to work" waits for the real end of
  what a key started: a workflow, a plugin's action, or an action run in
  place. Agent tests cover this with the mock provider and a simulated
  Stream Deck.
  A managed Calls layout that cannot be stored while the database is busy
  is tried again after 1 s, then after waits doubling up to a minute;
  other storage errors are not retried.
- Calls (groundwork, off in every build): a detection-only Microsoft Teams
  provider. It infers that a call is probably active while Teams holds the
  microphone, recognizing Teams only by its executable name. Answer, end,
  mute and camera stay unsupported: Microsoft retired the device API. The
  parsers for Core Audio, WASAPI and PipeWire are tested on fixtures, and
  no build queries the operating system yet.
- Calls (groundwork): IPC protocol 19 with `getCallsStatus`,
  `setCallsOverlay`, `restoreCallsLayout` and the `callsChanged` event.
  Settings › Devices gets a Calls section, in English and Polish, shown
  only while Calls is installed, which no build does yet. It shows the
  call's state, says plainly that Teams can only be detected, has a switch
  per device for the overlay, and has *Restore defaults*, which first keeps
  your version of the layout under a name it tells you.
- System actions: an Open key shows what it opens, the document icon for
  a file and the open folder for a folder, on the Canvas, in the key
  preview, in layout previews and on the Stream Deck alike. Nothing chosen,
  a missing file or folder, or a target that cannot be told (a variable,
  an application, a network path) shows the document icon. A key's own
  icon or image, and an icon chosen for the action, always win; the
  automatic icon is never saved. The target is looked at by its file
  metadata only, never opened or run, and again when an action, a
  resource binding, the profile or the layout changes, never on a timer.
- Projects: a project can list several repositories, each with a key
  (`cms`, unique in the project), a name, a description, the folder's
  resource (bound on each computer, never exported) and a default. An
  existing project with a `repository` role gets that one as its
  repository `main` when the configuration is upgraded (schema 14); its
  roles, resources and folder bindings are not changed. Marking a default
  repository also points the `repository` role at it. Removing a
  repository never removes its folder binding, and a missing folder is
  shown, never removed. An export carries the repositories (format 5)
  only when a project has more than that one.
- Projects: the project editor has a Repositories section. Add a
  repository by choosing its folder in the system's folder dialog (its key
  and name are proposed from the folder name), then rename, describe,
  reorder, choose another folder, mark the default or remove it. A
  removal asks first and lists what else uses the folder. Keys are checked
  as they are typed, a folder that is gone is marked "Folder not found",
  and folders are set on this computer when the project is saved. The
  resource role pairs move into a collapsed advanced part.
- Action Library: **gesture presets**, one entry that puts several actions
  on the gestures of a key at once. *Brightness Control* (Device) dims by
  10% on a single press, sets 100% on a double click and 50% on a long
  press. Dropped on a key it creates ordinary actions and gesture bindings
  in one undo step (keeping an occupied key's look and its other
  gestures), or nothing when the key cannot take it. A key that acts on
  its press at once is never switched to gestures by a preset: switch it
  in the Inspector first, which asks; dropped on one
  gesture card only that gesture changes. Every gesture stays editable on
  its own; nothing of the preset is stored.
- Plugin API 1.11: `contributes.actions[].contexts` lets a plugin declare
  where each action makes sense (key, gesture, display, sequence,
  workflow), with `Action::contexts` in the SDK. Advisory like the
  built-ins' contexts: a use elsewhere is reported and still runs. Without
  the field an action keeps all five.
- Plugin API 1.10: `contributes.gesturePresets` lets a plugin declare
  presets over its own actions, with localized texts and `allowPartial`.
  Presets grant nothing; each action keeps its permissions and checks.
- Variables: structured values, read member by member.
  `{{ projects.current.repositories.cms.path }}` is a repository's folder,
  `{{ projects.current.repository.name }}` the default repository's name
  (the one marked as default, else the only one; with several and none
  marked there is no value, never the first). A whole value fills only a
  JSON parameter and never becomes text. Keys show names and keys only:
  a folder path, an id or a remote URL shows `-`, and the title picker never
  offers them. A plugin's action receives a folder path only with
  `resources:paths`. Plugin API 1.12 (manifest validation only for now):
  the types `object` and `map` of `repository` and the host sources
  `host:project.repositories` and `host:project.repository`, for DeckPilot's
  own bundled plugins. IPC protocol 18; older apps see these variables as
  JSON. The Projects plugin that declares them follows.
- Settings › Profiles › a profile: "Saved actions" lists the profile's
  saved actions by name with how many places use each, and can show only
  the unused ones. Each can be opened in the action editor; an unused one
  can also be deleted from its row, after the same confirmation as in the
  editor (the agent still refuses while anything uses it). Nothing is
  deleted by itself, and there is no "delete all". Until now a saved
  action that no key, gesture or workflow used could not be opened.

- Plugins (for the coming Scripts plugin, plugin API 1.13, not usable
  yet): the plugin runtime knows the Scripts bridge's operations (files,
  processes, the script's run) and refuses them to every plugin until
  the agent's checks for them land. Files move in chunks within the
  existing message size limits. The Scripts plugin alone can be given a
  larger budget of calls per run, and its polling and logging never
  extend its continuous CPU limit, so a busy script is still stopped
  after 2 seconds. The agent checks every file operation of a script
  against that script's own folders and access, never follows a link
  out of them, never changes a `.git` folder, the home folder, system
  folders or DeckPilot's own, and asks before it deletes or replaces
  files, showing what it computed itself. A script runs only the
  programs granted to it, with an argument list and never a shell (a
  shell only with its own grant), in its granted folders, with a clean
  environment, and the program is stopped when the script is stopped,
  runs out of time or loses its grant. Plugin manifests know the
  Scripts plugin's permission, its script parameter, its page and its
  limits (plugin API 1.13), for DeckPilot's own Scripts plugin only. A
  file a script reads in parts comes from one snapshot of the file, so
  its parts never mix two versions; a file replaced or changed between
  them fails the read.
- Storage (schema 16, not usable yet): the scripts of a profile, and on
  this computer only whether each is trusted and what it may do. A
  script made in the editor is trusted with nothing granted; an imported
  or duplicated one is never trusted and never runs until reviewed. A
  script can never be granted a container runtime (Docker, Podman and
  the like) in this version.
- Export and import of scripts (export format 6, only when there are
  scripts): an export holds a script's code and settings, never whether
  it is trusted or what it may do. An imported script arrives untrusted
  with no permissions, and nothing runs on import; the import window
  says so.
- The agent runs a script only from the active profile and only while it
  is trusted; its permissions are read again at every step. Scripts see
  each repository's remotes, with the connection and the provider's
  repository they are linked to.
- A Scripts page (while the Scripts plugin is on, not usable yet): write
  and edit the profile's scripts in a code editor with JavaScript
  highlighting, checks and completion, rename, duplicate and delete them
  (a script's uses are listed first), and review a script that came from
  elsewhere before trusting it: its code is shown read only, with
  invisible characters marked, and with the permissions it would get
  back.
- On the Scripts page, a script's permissions: which projects, files and
  folders (picked with the system's dialog), programs and shells it may
  use, and whether it may delete without asking; container runtimes
  (Docker, Podman and the like) and programs that act as another user
  are not available. The Action Library lists a "Run …" entry for every
  script, which puts the script on a key by reference.

### Changed

- System actions: the Action Library offers one Open for files and
  folders (Choose file… or Choose folder…); Open folder is no longer
  offered for new keys, actions or workflow nodes. Saved Open folder
  actions, keys, gestures and workflow steps keep running unchanged with
  their folder icon; nothing is rewritten.
- Device Editor: a label picked on the Canvas is nudged with the arrow
  keys (1 unit, 5 with Shift), as on the Inspector's label stage; each
  nudge starts from the last one. Escape or a click on the key lets go of
  the label, and the arrows move between keys again.

## [0.15.0] - 2026-11-24

### Added

- Marketplace: a sidebar with search, Discover, Installed, Updates,
  Categories and collections; plugin pages with screenshots, highlights,
  what's new, version history, publisher and trust; install, update and
  failure states. An empty collection names the categories that would
  fill it and offers "Browse all categories"; nothing is made up to fill
  it.
- Marketplace: Discover and plugin pages can show the registry's signed
  storefront (banners, editorial sections, listings and verified media).
  The registry does not publish one yet, so Discover is built from the
  catalog alone; text, links and images of a storefront are shown inertly
  (no HTML, https links only, images checked by their hash).
- Device Editor: the key labels editor. The Inspector lists a key's labels
  (up to 8) with their text (and variables, through the same picker as
  the title), position, size, colour, background, opacity and layer, and
  can add, duplicate, reorder, hide and remove them. On the Canvas a
  selected key outlines its labels; a label can be dragged to another
  place and edited in place with a double click. Labels on a double
  click or long press are kept and marked as not drawn (a key shows the
  labels of its press).
- Device Editor: a key's or gesture's saved action is managed from its
  card in the Inspector: Edit opens the Action Library's editor, which says
  when the action is shared and changes every place, and Usages lists where
  it is used. An edit that would leave a shared action without a required
  setting asks first and says how many places stop running. The separate
  Action Library page and its rail entry are gone.
- Device Editor: dropping an action type whose required settings have no
  defaults (the app of Open Application, say) places a draft: its own
  action per key or gesture, marked on the Canvas and its gesture card as
  needing configuration, with the missing settings marked next to them.
  It does not run until it is configured. Draft steps and workflow nodes
  are marked the same way and do not keep a workflow from being saved.
- Device Editor: an assigned action whose plugin is missing, disabled,
  incompatible, failed verification, was revoked, is busy or waits for a
  permission says why on its gesture card, in words, with the recovery the
  reason calls for (install, enable or update the plugin, review
  permissions, turn on the module), Replace action and Remove assignment.
  Nothing is installed, enabled or granted from the Inspector itself.
- Plugin actions that only wait for an optional permission show Grant in
  the Action Library: it opens the permission dialog with exactly the
  missing permissions, and grants only what is checked.
- The library follows the action types' declared contexts: step-only
  actions (Wait, Delay) are not offered for keys, display-only ones are
  listed apart, and a saved key or gesture whose action does not work
  there is kept, marked with the reason and offered a change (wrap it in a
  Multi Action, or move it into a workflow) that is applied only after
  confirmation.
- Escape clears the Device Editor's selection (after closing a menu,
  cancelling a drag or leaving a text field first); the deselect button is
  gone.
- Folders: **Exit after action completes**, a switch of its own in the
  folder key's Inspector next to Automatic exit. The folder is left once
  what its key started is over (an action, a plugin's action, a workflow or
  Multi Action as a whole, all of several runs), whether it succeeded,
  failed or was cancelled; a run that cannot start keeps the device in the
  folder. It is the folder's existing `autoReturn`, which used to close the
  folder when the key was pressed: folders that had it keep it, with the
  new timing; it stays off for every other folder. No schema change.
- Folders: the protected Back key shows the Automatic exit countdown in
  whole seconds under its icon, redrawn once a second (only that key), with
  a pause mark while a run started from the folder is going on. The
  device, the simulated MK.2 and the editor's Canvas in a live preview draw
  the same face (the live preview's navigation gains the optional
  `folderCountdown`; protocol 17 unchanged).
- Device Editor: the key's title field has a picker of the variables a key
  can show, which inserts `{{ reference }}` at the cursor, and shows under
  the field what the key shows as the agent resolves it (cut to 32
  characters as on the key, with each `-` that stands for a variable
  marked and why it has no value). The title itself is not changed. The
  Canvas summaries show the agent's answers instead of replacing
  `{profile}`, `{project.key}` and `{project.name}` themselves.

### Changed

- Plugin permissions are granted and revoked in Settings › Permissions;
  a plugin's page in the Marketplace shows them read-only, with a link
  there. Plugin diagnostics and module migrations are in Settings › Logs &
  Diagnostics; the experimental Settings › Plugins page is gone.
- The Inspector shows the selected key's preview in a fixed column beside
  the settings, which scroll on their own; the actions of a key are
  gesture cards.
- The Action Library always lists every action type; entries that do not
  fit the selected control are disabled with the reason (the "Compatible
  only" switch is gone).
- A dial's or strip's choices list only the saved actions that control
  already runs.
- The main window starts in the Device Editor; the Dashboard is gone (a
  session that was on it opens the Device Editor). Pages that plugins and
  modules add stay in the rail as before.
- The toolbar shows Device › Profile › Layout as one trail. The device menu
  lists the connected devices first, then the known devices that are not
  connected (by serial) and the models without a device; the layout menu
  lists only the layouts of the profile being edited that fit the device,
  a new layout and the way to the Layout Manager.
- The Layout Manager moved from the rail to Settings › Layouts. It
  duplicates, renames, sets defaults, deletes and assigns layouts to
  profiles, and can be filtered to one profile; "Manage layouts…" in the
  Device Editor's layout menu and in Settings › Profiles opens it there. An
  old link to the Layouts page opens the settings page.
- Settings › Profiles lists every profile in one table and shows a
  profile's details in a view of its own; opening a profile neither
  activates it nor changes what a device shows.
- The settings pages share one layout (title, sections, rows). Language
  names each language in that language and applies at once. Modules lists
  only DeckPilot's built-in modules, with a link to the Marketplace for
  plugins.
- Notifications show their time beside the title.
- Folders: Automatic exit pauses while a run started from the folder is
  going on and resumes with the time that was left; the time a run takes
  is not counted. Only the folder shown counts down: a folder around it
  keeps its remaining time until the device returns to it.
- Folder Inspector: Automatic exit is one compact row, "Automatic exit
  after [10] seconds", with the time edited inline (stored with Enter or
  when the field is left; Escape restores it).
- A key whose plugin is missing, disabled, incompatible, waiting for a
  permission or briefly away keeps its assignment, title, icon and look,
  and shows a compact face: a plug icon with the key's title and colours,
  no warning text. A key whose other gesture still works keeps its normal
  look. The device, the simulated device and the editor's previews draw
  the same face, and the key comes back as it was once the plugin can run
  the action again.
- Mixing versions: 0.15 keeps IPC protocol 17, the database, the plugin
  store and the export format of 0.14, so a 0.14 app and a 0.15 agent (or
  the other way round) work together. Until the previous agent has
  restarted after the update, folders with Exit after action completes
  still close when their key is pressed, Automatic exit does not pause
  during a run, the Back key shows no countdown and keys whose plugin is
  missing keep their earlier warning face.
- Going back to 0.14 or earlier keeps the configuration readable (no
  database, plugin store or export format change, and an export made by
  0.15 imports there as before). There, folders with Exit after action
  completes close again as soon as their key is pressed, Automatic exit
  counts down during a run, and the Back key shows no countdown; the
  Dashboard, the Action Library page and Settings › Plugins are back
  where they were.

### Fixed

- Marketplace: an install or update that failed and was done later
  (another window, a retry elsewhere) no longer shows its old error and
  Retry again after the plugin is uninstalled.
- The workflow node catalog cut descriptions and unavailable reasons to
  one line; they now wrap to two lines and show in full in the tooltip.
  Marketplace tiles show their whole summary on hover.
- Saved actions that read alike in the workflow node catalog say which one
  is still a draft ("not set up yet"); Random action and Multi Action
  Switch are under "Multi Actions".
- The file and folder pickers of Open and Open folder say when the bound
  file or folder was moved or deleted, as the application picker does for
  an application that is no longer installed.
- The editor's previews of keys stored with the old action ids of
  built-ins that moved to a plugin (`projects.next`, `git.status`, …)
  showed them as unavailable even while the plugin served them. They now
  follow the plugin, as the agent does.
- Settings › Resources called an application chosen with the application
  or browser picker "missing", because it is stored by its bundle id,
  desktop-file id or App Paths name rather than a path, and never checked a
  file chosen with the file picker. Applications are now checked the way
  Open Application finds them, and a chosen file that was moved or deleted
  shows as missing.
- Close Application also compares a process's start time before it signals
  or waits for it, so a new process that was given the same process id is
  never taken for the one it matched.

## [0.14.0] - 2026-11-24

### Added

- Key titles can show variables: `{{ projects.current.key }}`,
  `{{ shared.team.name }}` or `{{ system.profile.current.name }}`, alone,
  several at once or mixed with text. The device and the editor's preview
  show the same text, and keys update when the project, the profile or a
  value changes. Sensitive variables and credentials never show on a key;
  a value that is not available shows `-`. `{profile}`, `{project.key}` and
  `{project.name}` keep working, and saved titles are not changed. A title
  with variables may be up to 160 characters long; the key shows at most 32.
  DeckPilot 0.13 and earlier open layouts with such a title but refuse to
  save them, and to duplicate an action whose label is such a title, until
  the title is shortened to 32 characters.
- Close Application: a key, a gesture or a workflow you start can quit an
  application. It asks the application to quit, as its Quit command does,
  so it can ask you to save; with "Force quit if it does not close" on,
  what still runs after 5 seconds is ended. Every instance you run of
  exactly that application is closed, never another user's programs, the
  system's or DeckPilot itself, and never from a schedule or an event.
  A run that is cancelled or runs out of time while the application has
  time to quit never forces it, and DeckPilot running as root or as an
  administrator (elevated) closes nothing.
- Importing a configuration lists its Close Application actions, marks
  those that force-quit, and imports each one unbound (it closes nothing)
  unless you allow it in the import review.
- Open Application, Open, Website and Close Application offer the
  applications installed on this computer and the system's file picker in
  the inspector and the Action Library, instead of a typed resource key. A
  choice is kept on this computer only, marked as such, and never
  exported; DeckPilot stores the application's stable identifier (its
  bundle id, App Paths name or desktop entry) where it has one.
- An Open Application key can quit its application on a long press: one
  choice in the inspector binds Close Application to the key's long press.
- Website can open a page in a browser you choose instead of the default
  one, and adds `https://` to an address written without it. "Open with"
  lists the browsers installed on this computer with their icons, the
  system's default first; when the chosen browser is uninstalled, the page
  opens in the default browser and the result says so.
- The application pickers show the installed applications with their own
  icons, and "Another application…" opens the system's file dialog limited
  to applications. Open and Open folder choose files and folders with the
  system's dialogs. Resource keys and project roles moved to an Advanced
  mode of each field; saved actions that use them open in it unchanged.
- Open opens a chosen file, folder or application by itself; a file
  inside a folder is now optional.
- Keys can carry up to 8 labels besides their title: short texts at the top,
  centre, bottom or a corner, along an edge (a ribbon over the icon) or
  anywhere on the key, each with its own size, colour, background and layer,
  and with variables like titles. Labels are drawn the same on the device
  and in the editor at every key size, with a readable background over
  pictures by default; keys without labels look exactly as before. Exports
  that contain labels need this version to import. (The editor for labels
  follows.)

### Changed

- Linux: an application bound by its desktop entry starts from the entry's
  own command line, so `gtk-launch` is no longer needed, and it can open a
  file or a page.
- Open URL is now called Website, Open file is Open, and Open
  application is Open Application; their saved actions keep working
  unchanged.
- Website refuses addresses with a user name or password
  (`https://user:pass@…`) or with a backslash, which earlier versions
  opened. Saved mail addresses (`mailto:`) still open.
- Linux: a desktop entry with `Terminal=true` is no longer opened by Open
  Application (earlier versions started it with `gtk-launch`, which opens
  it in a terminal). Bind the program itself, or your terminal with the
  program as its argument.
- Windows: pages, files and folders open through `rundll32.exe` and
  `explorer.exe` by their full path in the Windows folder.
- **Behaviour change: while the Workflows plugin is disabled, keys and
  gestures bound to a graph workflow no longer run it.** Disabled plugins
  preserve bindings without executing them: the keys are kept, show
  "Workflows off", and the app's status bar says why a press did nothing.
  Turning the plugin on again restores them. Multi Actions are part of
  DeckPilot itself and keep running with the plugin off; workflows started
  from the app or by their schedules are not affected.
- Deleting a workflow that is in use now asks first and lists every place
  that uses it: keys and gestures, Sub-workflow and Loop nodes of other
  workflows, Random Action and Multi Action Switch entries, and the Work
  Session start and verification workflows. The keys are kept and show
  "Workflow missing" unless you also choose to clear them; a Work Session
  setting is cleared only when you tick it, otherwise the workflow is not
  deleted; other workflows and actions are never changed. If something
  started using the workflow while the dialog was open, the delete stops
  and shows the updated list.
- A key whose workflow was deleted now shows "Workflow missing" instead of
  "Not in this profile", which is kept for a workflow of another profile
  that shares the layout.
- A key or gesture whose workflow did not start (missing, another
  profile's, invalid, needing input values, already running, or too many
  running) now says so in the app's status bar, once per key, instead of
  only in the agent's log. A Multi Action Switch set to advance on
  failure no longer skips a state whose workflow is only temporarily
  unavailable (already running, too many running, or the Workflows plugin
  off); it still moves past one that is missing, invalid or needs
  configuration or input values.
- Saving a layout refuses a new key that starts another profile's or a
  deleted workflow, a workflow that needs input values, or a graph
  workflow while the Workflows plugin is off. Keys saved like that before
  are kept and reported, and moving or duplicating them is not new.
- Built-in actions now declare where they make sense; nothing changes yet
  for existing setups. Each built-in says whether it belongs on a key, on
  a double click or long press, on a key's live display, in a Multi Action
  or in a workflow. Saving a layout or workflow reports a use elsewhere
  without refusing it, and such a use still runs, with a warning in the
  agent's log. Wait is deprecated in favour of Delay and is no longer
  offered for new actions; saved Waits keep working unchanged.
- Mixing versions: the 0.14 app and background agent speak IPC protocol 17
  (variables in key titles and key labels). Until the previous agent has
  restarted after the update, the editor shows key titles as typed and
  lists no variables for them, and the device draws no labels; everything
  else works as before. A 0.13 or older app connected to a 0.14 agent
  works as before, and the device still shows resolved titles.
- Going back to 0.13 or earlier keeps the configuration readable (no
  database or plugin store change). There, keys show no labels and saving
  a layout drops them, an export with labels cannot be imported, a title
  with variables longer than 32 characters cannot be saved until it is
  shortened, an action saved as "Needs configuration" (and a Multi Action
  step or graph workflow using one) cannot be edited until it is
  completed, and Close Application actions, which those versions do not
  know, are kept as unavailable and never run. Workflow references, the guarded delete and
  the new key messages change no saved data.

### Fixed

- A workflow used only by another workflow's Sub-workflow or Loop node, a
  Random Action or Multi Action Switch, or the Work Session settings, could
  be deleted without any warning, which broke what used it. Every kind of
  use now counts.
- An action that needs a value, such as Open application, can now be put
  on a key or gesture, into a Multi Action or into a workflow before you
  fill it in. Until then it is saved as "Needs configuration" and is never
  run: a press, a step or a workflow that reaches it is refused before
  anything happens, and says which settings are missing. A wrong value is
  still refused when saving. Nothing changes for configured actions.
- The plugin store could fail to open ("PRAGMA key requires a key") when
  the agent or the app opened it at the same moment as another encrypted
  database, right after starting; such a connection also could not make
  the backup taken before a plugin store upgrade. It now waits for the
  encryption to be ready and otherwise refuses before reading or writing
  anything.

## [0.13.0] - 2026-11-22

### Added

- Support for a signed Marketplace storefront: the agent can fetch,
  verify and cache the editorial pages a plugin registry publishes
  (featured plugins, collections, banners and plugin listings with their
  icons and artwork), each signed and checked like the registry's
  catalog, and offline from the cache. **No storefront is published in
  the official registry yet, so nothing changes in the Marketplace for
  now**; the editorial Discover page comes in a later release. A
  storefront that is missing or fails its checks never affects the
  catalog, plugin installs or updates.

### Fixed

- Windows and Linux: the main window's close button closes the window
  again (since 0.10.0 it did nothing). With unsaved layout edits it still
  asks first. The agent keeps running in the tray as before; macOS is
  unchanged (closing hides the window to the menu bar).
- Configuration export no longer overwrites or deletes an unrelated file
  next to the chosen destination (one named like `backup.deckpilot.tmp`).
  The export is staged in a new file of its own, created exclusively in
  the destination's folder, and then replaces the destination in one step;
  a failed export leaves the previous file and every other file as they
  were, and simultaneous exports no longer interfere with each other.
- Configuration import: after an import fails before anything is written
  (for example because the configuration was locked), **Import** on the
  same review screen tries again; the file no longer has to be opened and
  decrypted again. An import is applied at most once, however often
  **Import** is clicked. When a review can no longer be imported, the
  window goes back to the password step instead of offering an **Import**
  button that cannot work, and an import that was saved but whose module
  settings could not be reloaded says so instead of reporting a failure.
- A request from the app is no longer lost, or answered with a
  "malformed message" error, when it reaches the agent in pieces while
  the agent is sending the app status updates, plugin events or the live
  preview. It now always gets its own answer.

## [0.12.0] - 2026-11-22

### Changed

- The nine official plugins (Docker, Git, GitHub, GitLab, Jira,
  Notifications, Projects, Workflows and Workspace) now come from their own,
  independently signed releases in the DeckPilot plugins repository: every
  copy bundled with the app is a published release signed by **both**
  DeckPilot publisher keys, checked when the app is built and again by the
  pins compiled into the app.
- The bundled plugins are updated to these releases (GitHub 0.2.1; GitLab,
  Jira and Notifications 0.1.2; Docker, Git, Projects, Workflows and
  Workspace 0.1.1). Installed copies are replaced by the new version when
  the app starts, including copies of Git, Docker and Projects whose move
  from the built-in still waits for your consent; settings, credentials,
  granted permissions and key bindings stay as they are. Their
  permissions are unchanged, so no new consent is requested, and keys
  that were waiting for your consent still wait for the same consent.
  Plugin API stays 1.9.

### Added

- GitHub, GitLab, Jira and Notifications are also available from the
  Marketplace, as the same two-key-signed official releases.

### Security

- The privileged plugins (Docker, Git, Projects, Workflows and Workspace,
  which run processes or reach into projects, files or the app's views)
  remain bundled-only: they are accepted only as the copies shipped with
  the app, never from the Marketplace, and only with exactly the
  permissions an independent security review approved.

### Fixed

- Installing first-party community plugins (Memory, Pomodoro, Markets,
  Break Reminder, Air Quality) from the Marketplace failed in 0.11.0: the
  DeckPilot publisher signatures meant for the nine official plugins were
  required of every `deckpilot.*` plugin. They are required again only of
  the official plugins (Docker, Git, GitHub, GitLab, Jira, Notifications,
  Projects, Workflows, Workspace); community plugins are checked as in
  0.10.0 (the signed registry, its authorization of the `deckpilot`
  publisher, the package's digest) and never get the privileges reserved
  for the official ones. When the Marketplace cannot plan an install, it
  now says why in a dialog instead of seeming to do nothing. A community
  plugin that 0.11.0 stopped (quarantined) is released at the next
  registry refresh but stays turned off: turn it on again on the Plugins
  page (Marketplace › Installed).

## [0.11.0] - 2026-11-21

### Security

- Official plugins come only from DeckPilot. An official plugin
  (`deckpilot.*`) that the Marketplace offers is installed or updated only
  as a DeckPilot release signed by **both** production publisher keys
  built into the app (key ids `BDCDC80985EB60B5` and `8C3C4E42700FA702`,
  threshold 2), on top of the registry's own signatures. A release signed
  by only one key, by an unknown or revoked key, or delivered with a
  missing or altered signature is refused. A valid signature grants no
  permission: the plugin still asks for its permissions as before.
- Rollback protection for official plugins: DeckPilot never installs an
  older official release than one it has already verified or shipped
  with the app, and the same version only with the same files. Revoked
  releases are refused at install, enable, every run and rollback, and an
  installed copy that is revoked or cannot be verified is set aside with
  the reason shown. Verifications are recorded in a tamper-evident audit
  log.
- The privileged gate stays closed. Official plugins that run processes
  or reach into projects, files or the app's own views (Docker, Git,
  Projects, Workflows and Workspace) are accepted only as the copies
  bundled with the app, never from the Marketplace, whatever their
  signature.

### Changed

- The nine bundled plugins are unchanged: the same versions and the same
  files as in 0.10.0, still built from this repository and trusted by the
  pins compiled into the app. The plugin API stays 1.9. Nothing has been
  published with the new publisher keys yet; this release is what brings
  them to installations.

## [0.10.0] - 2026-11-21

### Added

- Live preview on the device: while the Device Editor shows a layout of
  the active profile, the selected device shows **and runs** the unsaved
  draft (titles, icons, colours, text appearance, gestures, action
  parameters, folders, pages), drawn by the same renderer as the Canvas;
  nothing is saved until Save, and other devices on the same layout keep
  the saved one. Navigation follows both ways: a double-click on a folder,
  Back, a page or a layout key moves the device (never running anything),
  and the device's own folder keys, protected Back, page keys, layout
  switches, Back to previous layout and Auto Exit move the editor. Save
  keeps the device where it is without redrawing it; Revert and Undo go to
  the nearest page that is still there. Closing the editor, the window or
  the app, or the app crashing, leaves the device on the saved
  configuration; after an agent restart the draft is sent again. Leaving
  the Device Editor, closing the window and quitting ask first over
  unsaved changes. IPC protocol 16 (capability `livePreview`); with an
  older agent the editor says that changes reach the device when saved.

- One key header and title editor for every key with a display (actions,
  Multi Actions, plugin actions, layout navigation and folder keys; the
  folder's protected Back key stays without one): at the top of the
  Inspector the key's live preview (the image the Canvas shows; clicking
  it opens the icon menu, never runs the key), its visible title (for a
  folder key apart from the folder's name) and background. The **T▾**
  button next to the title opens the title appearance popover: Show title,
  Top / Middle / Bottom, Reset, font, size in px, bold / italic /
  underline, text colour and the text layout (wrap, single line, auto fit,
  marquee). Every change shows on the Canvas at once and is an undo step;
  opening it changes nothing; Reset clears the overrides and never writes
  the device's defaults. The icon menu chooses from the profile's icon
  library, imports a PNG or JPEG file, pastes one from the clipboard (the
  WebView's clipboard; other formats are refused with a message) and
  removes the key's own icon. New optional appearance fields `fontFamily`
  (only fonts built into the renderer: Inter), `bold` (Inter SemiBold),
  `italic` (a 10° slant measured with its overhang) and `underline` (the
  font's underline), drawn by the renderer for the device and the preview
  alike; layouts without them look exactly as before
  (`docs/key-text.md`, "Title appearance").
- Plugin API 1.8: an action's `title` suggests a default title, says the
  title is dynamic, suggests a position and lists the parts of the look
  the editor offers; a title the user wrote now beats a plugin's dynamic
  title (`docs/marketplace/sdk/manifest.md`, "Titles").

- Restoring DeckPilot's own plugins (IPC protocol 15, capability and
  plugin runtime feature `restoreBundled`): Settings › Plugins › Installed
  lists the bundled plugins you uninstalled under *Removed DeckPilot
  plugins*, each with **Restore**, which installs the copy that comes with
  DeckPilot again (checked against the pins compiled into the agent, and
  the app's code seal on macOS, before anything changes), clears your
  earlier removal and brings back the settings and credentials you kept;
  a module that moved to the plugin runs on it again. A broken bundled
  plugin gets **Reinstall**, which replaces its files with that copy and
  keeps its settings, credentials and permission choices. The refusal of a
  plan that needs a removed bundled plugin now names this action. Older
  apps and agents are unaffected.

- A protected system Back key in folders: on every page inside a folder
  the first key with a display (key 0, top left) is DeckPilot's own Back,
  drawn and run by the agent (also with every plugin disabled) with one
  core vector icon (`crates/deckpilot-render/assets/icons/folder-back.svg`,
  no title), returning exactly one folder level. Nothing can be bound,
  moved, dropped, pasted or cleared there, in the editor and when a layout
  is saved; it is left out of multi-selection operations, and its menu
  offers only *Go back* and *Test*. Models whose keys have no display keep
  the host's navigation (`docs/data-model.md`, "The protected Back key").
- Auto Exit for folders (`autoExit`, 1–3600 s, off by default), set in the
  Inspector of the key that opens the folder: the agent leaves the folder
  one level up that long after a device entered it, also with the app
  closed. Moving within the folder does not restart it; leaving, another
  layout or profile, locking and disconnecting cancel it; every device
  counts on its own; nothing is stored.
- Database schema 11: migration 11 marks existing folders `reserved` when
  their first key is free or already goes back, and `legacy` (Back stays on
  the first free key, nothing moves, a migration note says how to change
  it) when it runs something else. "Move Back to the first key" in the
  editor moves that action to the first free key. Encrypted backup first,
  no binding lost; imports of older exports get the same rule.

- The Device Editor's interactive preview, backend half
  (`docs/editor-navigation-and-simulation.md`). Action types may declare
  `editorNavigation` (`folder.open`, `folder.back`, `page.next`,
  `page.previous`, `layout.switch`, `layout.back`, with the parameter that
  holds the target): the deck's *Next page*, *Previous page*, *Switch
  layout* and *Back to previous layout* do, and plugins may in their
  manifest (plugin API 1.7), as data only that the editor maps to its own
  navigation without calling the plugin. Preview faces say what pressing a
  key does to the view, the automatic Back key of a folder page included.
- Simulated presses (IPC protocol 14, capability `simulateInput`): the app
  can press a key of a connected device (single, double or long) through
  the agent's own gesture engine and dispatch, with the key's timings, on a
  lane separate from the device's physical keys and without writing
  anything to the device but the key images the action changes. Refused
  while locked and unless the device shows the profile, layout and page
  the app shows. It is not a user-initiated run (plugin dialogs and
  `url.open` stay refused), and a press that may change something outside
  DeckPilot (a command, a Docker change, an HTTP request other than GET, a
  plugin action that modifies, a workflow with such a step) runs only after
  the user confirmed it; the computer's settings and plugin permissions
  still apply. New Tauri command `simulate_press`.

- Plugin localization (plugin API 1.6, IPC protocol 13): a plugin package
  may carry translations as `locales/<tag>.json` (declared in the
  manifest's new `locales`, with `defaultLocale`), signed and pinned with
  the package's other files and checked at install (JSON objects of plain
  strings, at most 64 KiB and 32 files, no markup). Manifest texts may name
  keys as `"t:key.path"`; they are shown in the app's language with a fixed
  fallback (tag, its language, the plugin's default, English, then a label
  from the key), and plugin dialogs may use keys too. The Rust SDK has
  `Context::locale` and `Context::t` (with interpolation and plurals) and
  `MockHost::localized` (`docs/marketplace/sdk/localization.md`). A plugin
  reaches only its own translations.
- The agent follows the app's language: the app sends it with the new
  `setLocale` request (protocol 13, capability `locale`); the texts the
  agent draws on keys (*Back*, *Folder*, *Unavailable*, the default names
  of the deck actions such as *Next page*) and the tray menu are in English
  or Polish, and only the keys showing such texts are redrawn when the
  language changes; titles the user wrote never change. Plugin names,
  actions and settings update live. An app or agent of protocol 12 keeps
  working in English.

- Plugin dialogs (plugin API 1.5): a plugin with the new `dialogs`
  permission (medium tier) can ask DeckPilot to show a standard
  confirmation, alert, form or selection dialog and gets the user's answer,
  checked by the agent against the request. Dialogs open only in an action
  the user started, show the plugin's name, never DeckPilot's lock or shield
  icons, always have Cancel, and have no secret fields. At most one per
  plugin is open and six are shown a minute; disabling, updating or
  uninstalling the plugin closes its dialog. The Rust SDK has
  `Context::dialog` (`docs/marketplace/sdk/dialogs.md`).

- Text modes for key titles: in the device editor's Inspector, a key's
  Text appearance chooses **Wrap** (word boundaries, explicit line breaks,
  a maximum number of lines ending with “…”), **Single line** (cut with
  “…” or clipped, never smaller), **Auto fit** (shrinks only as needed, down
  to a minimum, on one or several lines; says when it does not fit even
  then) or **Marquee** (scrolls a title that is too wide, with pauses at
  both ends, at a chosen speed), plus a text size in points and left,
  centre or right alignment. One layout implementation in the shared
  renderer serves the preview and the devices, at every key size; templates
  and plugin labels are resolved before they are fitted, so plugins get the
  modes without changes. Scrolling keys animate only on the shown page of a
  connected device, at most 15 frames a second, without writing unchanged
  frames, and stand still in the preview when the system reduces motion
  (`docs/key-text.md`). Existing layouts keep their wrapped titles.

- Database schema 12: folder keys are core navigation controls. A key
  that opens a folder of its page has exactly one binding, a press that
  opens the folder at once (no gestures); saving, importing and the agent
  enforce it (`invalid.folderControl.*`). Migration 12 (encrypted backup
  first) keeps such keys opening and moves their other bindings to the
  layout's recovered list with a migration note; the key's Inspector shows
  only the folder's settings and "Recovered gestures (n)" with *Move to a
  free key* and *Discard*. Imports of older exports get the same rule with
  a warning (`docs/data-model.md`, "Folder controls").

- A shared **Icon Library** and **Background Library** for every
  configurable key. The Icon Library has 471 built-in Lucide icons in 19
  categories (offline, searchable, drawn by the keys' renderer, so the key
  looks as its tile), your imported icons (My Icons) and plugins' icons. The
  Background Library starts with Transparent, Black and White, then solid
  colours (any custom colour), built-in gradients, your images and plugins'
  backgrounds. Keys store only the asset's id; gradients and images are
  rasterised for the device. A key whose plugin asset is unavailable keeps
  it, draws the device default (no icon of its own) and draws it again
  once the plugin is back.
- Plugins can contribute icons and backgrounds (plugin API 1.8,
  `contributes.visualAssets`): PNG icons, PNG or JPEG images and gradients,
  packaged, signed and checked like every other package file; no plugin
  code runs for them.
- Plugin API 1.9, the platform for moving Git, Docker, Workspace, Projects
  and Workflows into DeckPilot's own plugins (reviewed in
  `docs/marketplace/module-migration.md` §7): manifest permissions `tools`
  (brokered `git` and `docker` limited to an allowlist of subcommands and
  options, with denylisted flags refused, separate network and modify
  grants), `resources`, `projectContext` and `hostViews`,
  `openUrl.fromParams`, `resource` parameters and the `workflows` and
  `projects` views, with their grant tiers, capability operations, broker
  checks and SDK calls. Only plugins shipped inside the app can declare
  them; third-party and registry packages are refused. The agent carries
  out `resource` parameters and `resource.resolve` (opaque handles bound to
  one invocation and plugin, labels only, the declared kinds only, project
  roles only with an active project and Projects on, audited) and answers
  the other operations as not implemented.
- Plugin API 1.9's project context: `project.current` and `project.list`
  (ids, keys and names of the active profile's projects), and
  `project.select` and `project.cycle` only in runs the user started, never
  from a lifecycle event or a schedule, so a plugin cannot loop on the
  switch it caused. A switch is applied like one from the app and its
  `projectChanged` event names the plugin. `profile.read` answers the
  granted scopes. Variables a plugin declares with a `host:project.id`,
  `.key` or `.name` source carry the project in the context, published by
  DeckPilot only while the plugin is enabled and holds `projectContext`.
  Every operation needs an active Projects feature and is audited.
- Plugin API 1.9's `tool.run`, carried out by the agent: the user's own
  `git` or `docker` (the executable, context and time limits set for the
  built-in Git and Docker modules, or the agent's `PATH`, plus Homebrew's
  and Docker Desktop's folders on macOS; the file must be named `git` or
  `docker`), never a shell, with every argument
  checked again at run time against the denylists, the host's table, the
  manifest policy and the plugin's own declaration (a manifest can only
  narrow), typed values that never start with `-`, and a canonical
  command line with the host's fixed prefix (`-C <handle> --no-pager
  --no-optional-locks` and hardening configuration: no `gpg` on `log`, no
  `ext::`, `file://` or remote helper transports, no credential prompts;
  docker's `--context`, and the compose file, folder and project name
  taken from the binding, never from the plugin). git works only on the
  bound folder's own repository (never one above it), compose only on a
  compose file inside the bound folder. The working directory is a
  resource handle of the invocation; the environment is an allowlist (no credentials, no git or
  compose overrides, `SSH_AUTH_SOCK` only for `ls-remote` with
  `tools:git:network` or an `ssh://` docker engine). `ls-remote` contacts
  only a remote `git remote` lists. Time limits per class (10 s local git,
  20 s network, 30 s docker reads, 300 s docker changes; the plugin may
  only shorten them), output caps (128 KiB, 64 KiB for logs, 16 KiB of
  errors), URL credentials (userinfo and token or password query
  parameters) removed from the output, at most 4 runs per
  plugin and one modifying run at a time; the process group is stopped on
  the deadline, an abort and a revoked grant. Docker changes need
  `tools:docker:modify` (off by default) and a run the user or their work
  session started. Every run is audited by subcommand, option names,
  handle label, exit status and byte counts, never by value. A program that
  leaves its process group no longer keeps a run waiting past its time
  limit.
- Plugin API 1.9: optional permissions. An action can name the sub-grants
  it needs (`requiresPermissions`: `tools:docker:modify`,
  `tools:git:network`, `resources:open`, `resources:paths`,
  `projectContext:select`, `openUrl:from-params`); the plugin is enabled
  without them, and only the actions naming one wait for it ("needs the
  permission …") until it is granted. The consent dialog and Settings ›
  Plugins show each one with its own switch, off by default when it is
  high risk. Docker's Compose up, down and restart name
  `tools:docker:modify`, so the Docker plugin's reads work without it.
- Plugin API 1.9: docker's `ps`, `logs` of a container, `version` and
  `context show` run without a resource, in an empty folder of the
  agent's own (the Docker plugin's status, containers and logs no longer
  need a Docker environment or a project); `resource.open` opens a file or
  folder inside a bound folder (`path`, kept inside it, links included),
  so the Workspace plugin's `path` works as the built-in's; a key state
  call can read which key it is (`run.context`'s `keyScope`), so Git and
  Docker keys that differ only in their resources keep their own state,
  and Compose up and down keys show their toggle again.

### Changed

- The GitHub, GitLab, Jira, Git, Docker, Workspace and Projects built-in
  modules are replaced by their bundled first-party plugins
  (`deckpilot.github`, `.gitlab`, `.jira`, `.git`, `.docker`, `.workspace`,
  `.projects`), now their only implementation
  (`docs/integration-modules.md`, "Moved to first-party plugins"). Existing
  keys, actions and workflows keep their ids and are never rewritten: they
  run again once the plugin is allowed (installed, enabled and, where it
  asks, consented in Settings › Plugins), and until then they are
  unavailable with the reason; there is no built-in fallback. An upgrade
  from 0.9.x migrates as before. Settings, credentials, projects and
  resource bindings are preserved. A build or a start without plugins has
  no project switching: the agent refuses it with "the Projects plugin is
  not active", and nothing is deleted.
- In builds with plugins, Workflows and Projects are what their plugins
  make them: the Workflows screen and its navigation entry only while
  `deckpilot.workflows` is enabled, and the Projects screen, the active
  project in the status bar, its dashboard card and every project control
  (project parameters, `{project.*}` hints, "while project" conditions, a
  workflow's project permission) only while `deckpilot.projects` is.
  Enabling or disabling the
  plugin shows or hides them at once. Nothing is deleted: keys that run
  workflows still run them, and graph workflows already on keys show
  read-only with the reason; Multi Actions stay the key editor's own. The
  agent refuses project requests while no project context is active
  (`projectsOff`), the app refuses workflow and project management while
  the plugin is off (`invalid.pluginFeatureOff`), and plugin answers carry
  the new optional `statusItems` and `dashboardCards` (no protocol
  change). Git, Docker, Workspace, GitHub, GitLab, Jira and Projects no
  longer have module cards or settings pages there: their plugins' pages
  replace them. Builds without plugins keep the Workflows screen.
- This computer's resource bindings moved from the Projects screen to
  **Settings › Resources**, which works without the Projects plugin (the
  core's `system.open-*` launchers use them).
- The key header: clicking the key preview opens one menu with an **Icon**
  and a **Background** section (library, import, paste, remove; background
  library, custom colour, reset to the device default); the Background row
  under the title is gone. **Show title** is a switch under the title
  instead of a checkbox in the title popover.
- A folder key's title is the folder's name: renamed in the key header,
  shown on the key, in the breadcrumbs and the Page Navigator; the separate
  folder name field is gone. A folder key title that differed from its
  folder's name is kept (schema 13 migration, after the encrypted backup,
  and imports) and offered as the folder's name or discarded.

- Keys that set no colours take their device model's defaults, the same
  for every kind of key: on the Stream Deck MK.2 (and XL and +) a
  transparent background on the black display and white text. Action,
  workflow, folder, Back and notice keys no longer get a dark tint of their
  own; a live state's accent colour and colours you chose are unchanged.
  The device and the editor's previews draw the same colours, and the
  editor's colour fields and contrast hint show the model's defaults. A
  key's background can now be set to **Transparent**, which versions up to
  0.9.0 cannot read.

- The device editor's Inspector shows only real settings: it is empty when
  nothing is selected and for a folder's protected Back key, has no
  instruction paragraphs, and leaves out actions a control cannot run
  instead of listing them disabled.

- A key title containing a line break now breaks there.
- The IPC protocol is version 12: plugin dialogs, sent only to an app that
  announces the `pluginDialogs` feature. Apps and agents of protocol 11 work
  with it unchanged.

### Fixed

- The Notifications, GitLab and Jira plugins that come with DeckPilot are
  now version 0.1.1: they were rebuilt with the current plugin SDK under
  0.1.0, so an installation upgraded from 0.9.0 would have kept its 0.1.0
  copy and shown the plugin as broken. A release check now refuses a
  bundled plugin with the version of a released one but other files.
- *Choose from icon library…* showed only the icons imported into the
  profile, so on a new installation it was empty: the library had no
  built-in collection and plugins' icons were never listed.
- The preview's **Test action** no longer says the device shows another
  page while it shows the editor's page (it compared the page's name with
  its id).
- An icon imported (or deleted) in the editor now reaches the device's keys
  at once: the agent reloads the configuration.

## [0.9.0] - 2026-11-20

### Changed

- Settings open inside the main window instead of a window of their own,
  and the gear is highlighted while they are shown. Settings reopen at the
  section shown last. The device editor keeps its page, key, inspector and
  unsaved changes while the settings are open.
- The toolbar no longer has back and forward buttons, and ⌘[ / ⌘] (Alt+← /
  Alt+→ on Windows and Linux) and the mouse's back and forward buttons no
  longer navigate. The rail, the device and profile selectors, the
  Marketplace, the settings and plugin entries lead to every view; the
  rail's Device Editor returns to the editor from anywhere.
- Settings and the app's pages use a quieter desktop layout: section
  headings, setting rows with the control on the right, hairline separators
  and inline status instead of cards, shadows and large padding. Controls
  move under their label in narrow windows.
- Settings › Profiles lists profiles in a compact table (name, last
  modified, status, actions) with Make active and a menu for rename,
  duplicate and delete; the profile being edited shows its description and
  counts as a short details list.
- The Dashboard, Action Library, Workflows, Projects, Work Session,
  Variables and the plugin Marketplace (catalog, plugin details, installed
  plugins, updates) follow the same layout.
- The settings sidebar is more compact and starts with its search; each
  settings page shows its title and description at the top.
- Scrollbars are thin and quiet everywhere: a 6-pixel thumb on a
  transparent track that darkens while the pointer is over the scrolling
  area and more while it is dragged. High-contrast modes use the system's
  colours.
- The navigation rail is always collapsed to icons, with each page's name
  as a tooltip and a clearer mark on the current page. The toolbar's sidebar
  toggle and the ⌘B / Ctrl+B shortcut are gone.
- The encrypted storage now uses SQLCipher 4.20 (through rusqlite 0.42),
  previously 4.19. The file format and key derivation settings are
  unchanged, so existing databases open as before. The compatibility test
  opens a database written by 0.8.0 with the new build, and a database
  written by the new build with 0.8.0.
- Keys on the device and in the editor's preview are drawn with the Inter
  font instead of the pixel font, and show their action's icon above the
  title (the icon the app shows for it; an imported icon still replaces
  it). Polish letters keep working.
- The device editor's preview is centred and scales with the window. Page
  navigation (previous, page, next, add page) sits directly under it;
  renaming, deleting, the page tree and Move or Copy are in Manage pages.
- The editor's Action Library uses compact rows with category headings; a
  drag shows the action's icon and name.
- System monitor keys show their value under the title: `23%`,
  `12.4/32 GiB`, `↓ 2.1 MB/s` / `↑ 340 kB/s`, `68% used`.
- The editor's preview draws each key at the size it is shown on the
  screen, Retina displays included, instead of enlarging the device's
  72-pixel image, so icons and titles are sharp. The device still gets
  images at its own size. Empty keys are drawn in the editor's key tone in
  both themes.
- Keys are drawn with a slightly smaller icon (about two fifths of the key
  above a title, a little over half without one), a little more room
  between icon and title, and a smaller status dot, on the device and in
  the preview.
- The device editor no longer has a "Preview with project" selector.
  Labels such as `{project.key}` show the profile's active project, as on
  the device. Projects, their assignments and project actions are
  unchanged, and the editor works the same with the Projects module
  disabled or not installed. The hint about project placeholders in the
  action editor appears only with the Projects module, and the Work Session
  controller is offered only with the Work Session module (keys that use it
  keep it).
- The editor's Action Library is always shown at one fixed width; it can no
  longer be collapsed or resized, and the width and collapsed state earlier
  versions remembered are forgotten. In a window narrower than the main
  window's minimum (860 × 560), which only some Linux window managers
  allow, it sits under the editor instead of beside it.
- The toolbar has no "Device Editor" title. The device and profile
  selectors appear only where they apply: in the device editor both; on
  the Action Library, Workflows, Projects and Work Session pages the
  profile; in the settings, the Marketplace, the dashboard and the Layouts
  page neither. Settings show "Settings / section" in the toolbar and their
  title once, on the page. Leaving the editor for any view and coming back
  keeps the device, profile, layout, page and selected keys.

- The plugin database (`plugins/plugins.db`, builds with plugins) is
  upgraded to schema 5, which records the plugins installed only as
  dependencies. An encrypted copy of the previous file is written to
  `plugins/.backups` first. DeckPilot 0.8.1 refuses the upgraded plugin
  database (its other features keep working); to go back, restore that
  copy as `plugins.db`. The configuration database is not changed by this.
- The configuration database is upgraded to schema 10 (an encrypted
  backup is made first). DeckPilot 0.8.1 and earlier cannot open the
  upgraded database; to go back, restore the `deckpilot-schema9-…` backup.
  Exports of selected profiles remain readable by earlier releases; a full
  export that includes layouts no profile uses requires this version.

### Added

- Plugin API 1.4: a plugin view may declare `context` (`none`, `device`,
  `profile` or `device+profile`) in `contributes.views`, the toolbar
  selectors the app shows with it (`none` when absent). The declaration
  gives the plugin no data and no permission. Manifests without it are
  unchanged; agents up to 0.8.1 (plugin API 1.3) refuse manifests that
  use it.
- The editor's preview shows live values (e.g. CPU usage) as the device
  does, read by the agent; it says when values are stale or unavailable.
  IPC protocol 11 (`watchKeyStates`); older agents and apps keep working
  without it.

- Shared layouts. Layouts are now resources of their own: one layout can
  be used by several profiles, and editing it changes it everywhere it is
  used, while each profile keeps its own projects, variables, workflows,
  layout order, default layout per device and page state. A new Layouts
  page in the navigation rail lists every layout with the profiles that use
  it and the devices it fits; layouts can be created, renamed, duplicated
  (an independent copy), assigned to or removed from profiles, and deleted
  after confirming the profiles that use them. Removing a layout from a
  profile never deletes it. Settings › Profiles lists each profile's
  layouts, and the device editor can open, add and switch between shared
  layouts.
- Switch layout and Back to previous layout (Navigation actions) move one
  device between the active profile's layouts without changing the
  profile; each device remembers its layout across restarts, and the way
  back is kept per device and profile. Switch layout keys are titled with
  the layout they show, and Back shows when there is nothing to go back to.
- Plugin dependencies (experimental plugins, plugin API 1.4; the agent now
  provides 1.4). A plugin manifest can declare the plugins and built-in
  modules it needs, required or optional, each with a version range, and
  actions can name the optional ones they need. Installing a plugin from
  the Marketplace first shows the whole plan: every plugin and built-in
  module it needs, in install order, why each is needed, and each
  plugin's own permissions. Every package is downloaded and checked
  against the signed registry before anything is installed, nothing is
  installed until you confirm, and if a step fails, what was done is
  undone. A dependency never gets the permissions of the plugin that needs
  it. Optional integrations are installed only when you select them;
  without one, only the actions that use it are unavailable. Plugins that
  others need cannot be turned off or uninstalled on their own: the
  Marketplace shows which plugins are turned off with it. Plugins
  installed only as dependencies are offered for removal, never removed
  unless you check them. Updates and rollbacks never move a plugin to a
  version that another installed plugin does not accept. Plugin details
  list a version's dependencies and optional integrations with their state
  here. Manifests without dependencies work as before. See
  docs/marketplace/dependencies.md.

### Fixed

- Windows: the settings window, and any other window opened from the app
  (export, import, logs), was created but never shown, so Settings could not
  be opened and no profile could be created. Opening a window no longer
  blocks the thread the window needs.

### Security

- Key fonts are read with skrifa (maintained) rather than ttf-parser
  (unmaintained, RUSTSEC-2026-0192).

## [0.8.1] - 2026-11-19

### Security

- The plugin sandbox uses Wasmtime 50.0.2 (was 50.0.1), which fixes the
  Wasmtime advisories of 2026-11-16 (RUSTSEC-2026-0566 to 0573). One of
  them applies to DeckPilot's sandbox, which is built without Wasmtime's
  `gc` feature: a plugin module using a function subtype check could make
  its plugin host process abort (RUSTSEC-2026-0570). Each plugin runs in
  its own sandboxed host process, so the abort stopped only that plugin,
  never the agent or other plugins.

### Fixed

- `deckpilot-registry package` checks each file path the manifest names
  before reading it, so a path such as `assets/../../icon.svg` is refused
  without reading anything outside the plugin directory. Such a package was
  already refused; only the read happened first.

## [0.8.0] - 2026-11-18

### Added

- System monitor keys. The System module has four new read-only actions,
  CPU usage, Memory usage, Network throughput and Disk space, whose keys
  show the value live: `CPU 37%` with a chart of the recent values,
  `RAM 62%`, the bytes per second received and sent (of one network
  interface, or of all but loopback added up), and the free space of the
  volume a folder is on. A status mark turns amber or red at thresholds
  you choose (CPU and memory 70% and 90% by default, disk 20% and 10%
  free). Pressing the key reports the value, which later workflow nodes
  can use. The values are read only while such a key is shown, at most
  every 2 seconds (disk: every 30 seconds), once for all keys.
- Actions declare which controls they work on: keys, dials (encoders) or
  touch zones, and which inputs of each (press, click, long press, a turn
  either way, a tap…). Every built-in action does: one-shot actions work on
  keys; Next page, Previous page, Brightness and the project switching
  actions also work on a dial, once per turn or press. Actions of plugins
  work on keys, as before. See `docs/device-controls.md`.
- Plugin API 1.3: a plugin's manifest can declare which controls and
  inputs each of its actions works on (`controls`, for example a dial's
  turns), and up to four toolbar entries (`navigation`) that open views
  DeckPilot draws itself: the plugin's variables, its actions or its
  settings (`views`). A manifest using them needs `"pluginApi": "^1.3"`.
  Without `controls` a plugin action still works on keys only, and
  `device.requires` grants no dial. The Rust plugin SDK has
  `Action::controls(..)`. See `docs/marketplace/sdk/manifest.md`.
- Toolbar icons of plugins are checked when a package is built and
  installed: PNG or WebP up to 256 KiB and 512×512 pixels, or SVG up to
  64 KiB without scripts, event handlers, external, `data:` or
  `javascript:` links, style imports or entities. The agent re-checks and
  sanitizes them each time it serves one, and the app shows them only as
  images, with its own icon when there is none to show.
- The Device Editor explains which actions fit the selected control. In
  the Action Library, actions that cannot run from it stay visible but
  disabled, with the reason ("Requires a dial. The selected Stream Deck
  MK.2 key supports press actions only."), and **Compatible only** hides
  them. Dropping such an action on a control is refused, shown on the
  control and announced; the inspector offers each trigger only the
  actions that can run from it, says why a saved binding is kept but not
  run, and a refused save names the trigger as the editor does.
- Toolbar entries of enabled plugins, left of the notifications: each
  opens a view DeckPilot draws, the plugin's live variables, its actions
  in the Action Library or its settings. Their icons are shown only as
  images (DeckPilot's own icon when there is none), more than fit are
  under *More plugin views*, the order can be changed from an entry's
  context menu and is kept on this computer, and they go away when the
  plugin is disabled or uninstalled.

### Changed

- A key, dial or touch zone bound to an action that cannot run from it (a
  dial turn bound to a key-only action, for example) no longer runs it.
  The binding is kept as it is, shows as unavailable, and the app's status
  bar says why the first time it is used. Saving a layout refuses a new
  binding like that with the reason; bindings already saved stay and are
  listed, so every existing profile can still be saved. Imports keep them
  and warn. Nothing is removed or converted, and a binding works again as
  soon as a compatible action is installed. Key bindings are not affected:
  every built-in action still works on every key gesture.
- macOS: closing DeckPilot's window (the close button or ⌘W) now keeps the
  app running in the background and takes its icon out of the Dock, like
  Docker Desktop. The tray's *Open DeckPilot*, or opening DeckPilot from
  Finder, Spotlight or the Dock, brings the window and the Dock icon back
  at once, still unlocked. ⌘Q (Quit DeckPilot UI) and the tray's Quit work
  as before.
- A new main window. DeckPilot opens with the Device Editor. The toolbar
  at the top chooses the device (connected devices first, the other
  models as "Not connected") and the profile to edit, and has
  notifications, the Marketplace and the settings at the right; the
  Dashboard, the Action Library, Workflows and module pages are in a slim
  navigation rail at the left.
- In the Device Editor, the Action Library is a sidebar at the right that
  can be resized and collapsed (an overlay in narrow windows), and the
  key inspector is below the device. The device follows the light or
  dark theme.
- The settings open in a window of their own, at the page asked for, and
  now include Profiles. A theme chosen there applies to every window.
- The Marketplace opens from the toolbar as a view of the main window;
  installed plugins are also managed in Settings › Extensions › Plugins.
- Notifications are in the toolbar, with the number of unread ones,
  instead of the status bar.

### Fixed

- Opening the plugin database no longer takes its write lock when its
  schema is already current, and waits for another process's lock while it
  checks the key. Before, the agent and anything else opening it at the
  same time could wait on one another until one failed with "database is
  locked", and a busy database could be reported as a wrong key.
- Linux: the `.deb` and `.rpm` packages now depend on the AppIndicator
  library the background agent's tray icon uses
  (`libayatana-appindicator3-1` on Debian and Ubuntu; the `.rpm` requires
  `libayatana-appindicator3.so.1`, on Fedora from
  `libayatana-appindicator-gtk3`). Where it was not installed,
  for example on a fresh Ubuntu 24.04, the agent of 0.7.0 stopped right
  after starting ("Failed to load ayatana-appindicator3"), and the app
  showed that the background agent was not running. Installing
  `libayatana-appindicator3-1` by hand works around it in 0.7.0.
- macOS: the background agent no longer shows a DeckPilot icon in the Dock
  for a moment when it starts, or for the whole life of a second agent
  process that stops because one is already running.
- A plugin update whose module the plugin host refuses (for example one
  that does not implement the actions its manifest declares) is no longer
  installed: the update fails and the installed version keeps running.
  Before, the update was applied and every action of the plugin failed
  until it was rolled back by hand.
- Linux: the AppImage's background agent could not start on newer
  systems such as Ubuntu 24.04: the AppImage did not include the
  AppIndicator library, and the system's copy cannot be loaded next to the
  older GLib the AppImage carries ("undefined symbol:
  g_once_init_leave_pointer"). The AppImage now includes the library and
  the libraries it needs.
- The background agent keeps running without a tray icon when the tray
  cannot be created, for example when the AppIndicator library is
  missing, instead of stopping. The app can still be opened from the
  application menu.

### Security

- Plugin requests are refused for every address the plugin broker refuses,
  including tunnel and translation ranges (6to4, Teredo, local-use NAT64,
  site-local IPv6 and the 6to4 relay range), not only private ones.
- A plugin request under a server path the user allowed (such as
  `https://ghe.example.com/api/v3`) is refused when an encoded `/`, `\` or
  `.` (`%2f`, `%5c`, `%2e`) would make a `..` segment once a server
  decodes it, such as `/api/v3/..%2fadmin`, which leads outside that path.
- Control characters in a plugin's log messages are escaped, so a plugin
  cannot forge lines in DeckPilot's log.
- The plugin module the host runs is checked against its integrity record
  byte for byte as it is loaded, not only before it is read.

## [0.7.0] - 2026-11-17

### Added

- The plugin Marketplace. Settings › Extensions › Plugins lists the
  plugins of DeckPilot's plugin registry
  (<https://knittingcode.github.io/deckpilot-plugins/>): browse and search
  them, read each plugin's description, version and the permissions it
  asks for, and install it after agreeing to those permissions. Installed
  plugins can be enabled and disabled, configured, bound to keys like any
  other action, updated (with a new consent when an update asks for more),
  rolled back and uninstalled, with or without their settings.
- Plugins run as WebAssembly in a separate, sandboxed plugin host, now
  part of every installer (signed and notarized on macOS, with one
  entitlement, for executable memory). A plugin gets only what it declares
  and you granted: the network hosts it names, notifications, its keys and
  its own storage. It never reaches files, other programs or your stored
  credentials directly.
- The registry is checked on every refresh against its root keys, which
  are built into DeckPilot: its signed key set, index and timestamp, and
  every package by its size and SHA-256. A stale, rolled-back or tampered
  registry is refused, and plugins that are already installed keep
  working. Only a registry whose root-signed key set allows it may serve
  DeckPilot's own `deckpilot.*` plugins, and the plugins built into
  DeckPilot can never be replaced from a registry.
- The first Marketplace plugins, by DeckPilot: Air Quality (Polish air
  quality stations, data from GIOŚ), Markets (ECB euro reference rates,
  via Frankfurter), Pomodoro, Break & Hydration, and Memory (a game on your
  keys).

### Changed

- The GitHub, GitLab and Jira integrations and notifications now run as
  plugins bundled with DeckPilot. On the first start after the update,
  each configured integration moves to its plugin: its settings are
  copied, and a stored token or a server on a private network is used by
  the plugin only after you agree. The built-in integration keeps working
  until the move is complete, and again whenever the plugin fails.
  Configurations the plugin cannot serve yet, such as GitHub Enterprise,
  stay on the built-in integration.
- Plugins are on by default. Start the agent with `--disable-plugins`, or
  set `DECKPILOT_ENABLE_PLUGINS=0`, to turn them off; the built-in
  integrations then run as before.
- Updating from 0.6.0 does not change the configuration database: plugin
  state is kept in a separate encrypted database, created after you
  unlock DeckPilot. 0.6.0 can still open the same data, without the
  plugins' changes and without credentials entered only into a plugin
  (docs/marketplace/module-migration.md, "Rollback and downgrade").

## [0.6.0] - 2026-11-17

The first public release. It also includes the changes of the
unreleased versions 0.1.0 to 0.5.0, which are recorded in the project's
CHANGELOG.md.

### Added

- The app and the agent negotiate their protocol and the capabilities that
  come with it, so a newer app works with an older agent (and the other way
  round) for what both support, says which features need an agent restart,
  and never sends a request the agent cannot understand. Agents too old for
  this app are reported as needing an update.
- Structured forms for module actions: command arguments and Compose
  services are edited as reorderable lists (shown as the exact list passed
  to the program), HTTP headers and query parameters as rows with duplicate
  and secret warnings, the request body in a JSON editor, and credentials
  are chosen from those stored on this computer. Stored actions keep their
  format. Actions that change things say so before they are saved.
- Upgrading a 0.5.0 configuration is tested with a real database created
  by 0.5.0-2: everything is kept, the encrypted backup is restorable, and a
  wrong key or a newer schema leaves the file untouched.
- The agent reports running workflows. Updates wait for them, and
  restarting the agent from the app asks before interrupting them; nothing
  ends a workflow silently.

- System module: open applications, web pages, files and folders, run
  commands and check whether a program is running. Applications, folders
  and programs come from resources bound on each computer. Commands run
  without a shell, with an argument list, a time limit, bounded output and
  a minimal environment; running them is off until allowed in Settings ›
  System on this computer. Their output reaches later workflow nodes but is
  never kept in history.
- Git module: status, branch, changed files, last commit and remote state
  of a project's repository, and opening it. Uses the repository of the
  workflow's project or an explicitly chosen project, never changes the
  active project, and never changes a repository. Remote state is what was
  last fetched unless a refresh is asked for (`git ls-remote`, which
  changes nothing locally). Detached HEADs, branches without commits and
  missing remotes are reported as such.
- Docker module: Docker availability, containers, Compose project status,
  Compose up, down and restart, and container or service logs. Every
  command names its Docker context (remote contexts work too) and Compose
  projects come from project resources. Starting and stopping services is
  off until allowed on this computer; down never removes volumes; logs are
  bounded and never followed.
- HTTP module: requests (GET, POST, PUT, PATCH, DELETE with query,
  headers and a JSON or text body; typed status, headers and body; a value
  read by JSON pointer) and health checks. Requests to this computer and
  private networks are refused unless allowed on this computer; every
  address and redirect is checked. Credentials are stored on this computer
  and used by name — never in a workflow or an export, only over https,
  and never shown. Responses are bounded; TLS verification is always on.
- Notifications module: show a notification (title, text, severity) or a
  summary of the workflow run, as a desktop notification and in the status
  bar. Text is plain and bounded; a notification that cannot be shown does
  not stop the workflow unless marked critical. Preferences: desktop,
  status bar and the least severity shown.
- Development seed: *Start Work* and *End Work* example workflows that
  combine the Git, Docker, System and Notifications modules using only
  logical resources (development data only; a new installation stays
  empty).
- Action types declare what running them does (read-only, opens something,
  changes things) and failures carry a stable error code; both are shown in
  the workflow editor.
- GitHub, GitLab and Jira modules (read-only): the signed-in account,
  repositories and projects, pull and merge requests with review requests,
  checks and pipelines, Jira issues and saved filters, and opening their
  pages. Tokens are stored credentials used by name, sent only to the
  configured server over https and scrubbed from every result; rate limits
  are reported with the time to wait. Searches are built from fixed
  templates, never from free text.
- Sub-workflows: a workflow can declare typed inputs and outputs and be
  called from another workflow's *Call* node. Recursion is refused, nesting
  is limited, and sensitive inputs and outputs are never kept in history.
- Workflow triggers: workflows can start on a schedule (every few minutes,
  daily or weekly at a local time) or when the agent starts, a profile is
  activated, the project changes or a device connects or disconnects.
  Missed times are skipped, never caught up; a trigger never starts a
  second run of a workflow that is still running, and triggers can be
  paused per workflow.
- Workflow variables: typed variables local to one run, set with a *Set
  variable* node and read like any other output. Parallel branches must
  agree on a value before they join; sensitive values stay in sensitive
  variables.
- Live key state: actions can show a state on their key (an indicator,
  a short detail, progress, on/off). Git keys show the branch and whether
  it is clean, Docker keys show what is running, and workflow keys show
  progress while their workflow runs. States refresh only for keys that
  are shown, at most four at a time, and only changed keys are redrawn.
- Key appearance: background and text colour, font size, title position
  and custom icons per key. Icons are imported from PNG or JPEG, scaled to
  at most 256 px and re-encoded, which drops embedded metadata. The app and
  the agent draw keys with the same renderer.
- Layout editor: multiple selection, keyboard navigation, copy, cut, paste
  and duplicate (also across pages), undo and redo, drag and drop from the
  library and between keys, a context menu per key, and page navigation
  with a breadcrumb.
- Run details in the workflow builder: a timeline of the run with parallel
  nodes on their own lanes, the slowest nodes, retries with their waits,
  failure details with what the error code means, and previews of declared
  outputs (sensitive ones are never shown).
- New step workflows can be created from the workflow switcher and the
  title bar.
- Workspace module: open a file in an editor (VS Code, Cursor, JetBrains
  IDEs, Zed, Sublime) at a line and column, open up to ten web pages, and
  open a project's editor, repository, documents and pages at once from its
  resources. It uses no shell and changes nothing.
- Page tree: pages can be placed in folders, reordered, and moved or copied
  with their subpages, also to another layout when its device supports
  every key. Back returns to the page's folder; a page in a folder without
  a way back gets an automatic Back key on its first free key. Existing
  layouts get their tree inferred without being rewritten.
- Appearance in the layout editor: colours with a contrast hint, text size
  and title position, an icon library with usage counts, PNG and JPEG
  import checked before upload, and previews drawn by the same renderer as
  the device. Changes apply to every selected key and can be undone.
- One variable registry: modules declare their own read-only variables in
  their namespace (`{{ projects.current.key }}`,
  `{{ workSession.elapsedSeconds }}`), the core keeps only system
  variables, and one resolver serves actions, workflows, keys and previews.
  `system.project.current.…` and `system.work.session.…` keep working
  exactly as before, as deprecated aliases that are never rewritten. A
  module's own ids are reported as unavailable while the module is
  disabled.
- Key gestures: a key can run a different action, workflow or page for a
  click, a double click and a long press, each with its own parameter
  overrides. A press resolves to at most one gesture, and a click waits
  only for the gestures the key uses. Timings default to 300 ms for a
  double click and 700 ms for a long press (Settings › Devices) and can be
  set per key. Keys without gestures, including every existing layout,
  keep reacting to the raw press as before.
- Stateful keys and the Work Session: a key can change its operation with
  the state of a built-in state machine. The Work Session key starts a
  session (running an optional start workflow), verifies the end with an
  optional verification workflow and finishes the session with a click
  once it is ready; the session is operated from the key only. The key
  shows the state and a live timer. The Work Session page shows the live
  state, the last verification and notices, holds the settings, assigns
  the key, and lists the history of sessions. Sessions survive agent
  restarts, and only finishing stops the timer.
- Shared variables: named, typed values (`{{ shared.… }}`) defined
  globally, per profile or per project and used by actions, workflows and
  keys. The Variables page in Settings lists every variable (core, module
  and shared) with search, filters, owner, access and the current value
  when it is safe to show; shared variables are edited there, and a
  variable that is still used cannot be deleted or retyped. A variable
  picker with live previews inserts references in action parameters and
  workflow node mappings. Sensitive values are never shown or exported.
- Keys that turn pages: *Next page* and *Previous page* move a device
  along the pages of its layout (the pages in no folder), with or without
  wraparound, and *Page indicator* shows where it is, such as 2/5. Any key
  label can use `{page.number}`, `{page.count}` and `{page.name}`. Existing
  layouts keep their pages and folders as they are.
- *Switch profile* on a key: the next or previous profile, or one specific
  profile chosen from the profiles that exist. Every connected device
  follows. A key whose profile was deleted says so and changes nothing.
- A folder can close by itself after one of its keys ran (a switch on the
  folder's page in the layout editor).
- A device comes back on the page it showed: when a profile becomes active
  again, and after the agent restarts. Folders are closed.
- Folders in the layout editor: drag *New folder* onto an empty key or
  choose it in the key's menu, double-click a folder key to enter it, and
  create folders inside folders. A key that already does something is
  never overwritten. Deleting a page asks first and says what goes with it.
- The layout editor's library is one searchable list with groups that
  fold, and it offers the actions that need no settings (*Next page*,
  *Switch profile* and the like) directly: dropped on a key, the action is
  created, or the one that exists is reused.
- *Brightness* on a key: set the display brightness of the devices to a
  level, or step it brighter or dimmer.
- Single click, double click and long press of a key are slots in the
  layout editor: drag an action, a workflow or a page onto one to set or
  replace what the gesture does, and remove it with one click.
- *Multi Action* in the layout editor: drag it onto a key or a gesture
  slot and add its steps right there, with delays between them (*Delay*,
  10 ms to 5 minutes). It is an ordinary step workflow, so it runs in the
  agent with progress, cancellation and history, and appears on the
  Workflows page too; *Edit steps* on a key opens any step workflow it
  runs. Workflows saved by earlier versions are read and kept unchanged.
- *Random action* on a key: runs one of several saved actions or
  workflows, picked at random, optionally with weights and never the same
  twice in a row. The pick runs as its own key would.
- *Multi Action Switch* on a key: two to eight states, each running an
  action, a workflow or a Multi Action, with its own title and glyph on
  the key. A press runs the current state and moves on when the run
  succeeds (optionally also when it fails); each key keeps its state
  across restarts. The states are edited in the Action Library.

### Fixed

- Key labels keep their accented letters: Polish (ą ć ę ł ń ó ś ź ż and
  their capitals) and the rest of Latin-1 and Latin Extended-A, `°`, `€`,
  and typographic quotes and dashes are drawn on the device instead of
  `?` (“Mgła”, “12°C”). Characters the key font still lacks are `?`, as
  before.
- Dragging one key onto another in the layout editor moves or swaps the
  keys again; it did nothing.
- HTTP, GitHub, GitLab and Jira actions run from a key or with *Run* in
  the app no longer end the agent (0.6.0-alpha.3); in workflows they
  already worked.
- Duplicating a workflow no longer briefly shows an empty editor.
- On Windows, the agent no longer fails to start when the previous
  instance is still closing its connection, and opening the configuration
  no longer fails while another process is writing to it.
- On Linux, starting a program that was written a moment earlier (for
  example by an installer) no longer fails with "Text file busy".
- Parallel branches of a workflow no longer wait for their slowest
  sibling: each node starts as soon as its inputs are ready, and a node
  that crashes fails the run instead of stalling it.
- Built-in actions: time limits longer than the engine allows are capped,
  switching the active project is marked as changing things, failures carry
  error codes, and every action icon exists in the app.
- Updates keep reaching installed versions now that the repository is
  named `knittingcode/deckpilot`: releases still announce their downloads
  under the address earlier versions accept, and the app accepts the
  repository's name in any case.
- A duplicated profile appears in the agent's profile list (the tray, the
  dashboard) at once, as a new one does; before, it showed only after
  something else made the agent reload.

## [0.5.0] - Not released (draft prereleases 0.5.0-1 and 0.5.0-2 only)

### Added

- Settings have their own navigation, grouped into General, Hardware,
  Security & Data, Extensions and Advanced, with one page per area. Back and
  forward work like in a browser and the place is remembered.
- Modules are managed in Settings › Modules: search, filters, and an
  explanation when a dependency prevents enabling or disabling one. Modules
  with settings get their own settings page.
- On macOS the toolbar is part of the window's titlebar. Page commands such
  as New workflow, Run workflow or Refresh devices are in the toolbar.
- A status bar shows the agent, connected devices (simulated ones
  separately), the active profile and project, background activity such as
  update downloads and workflow runs, and notifications.
- Workflow graphs (engine): conditions, parallel branches with joins,
  bounded loops, delays, retries, timeouts, cancellation and data passed
  between steps. Existing workflows keep running unchanged.
- A visual workflow editor: a searchable catalog of flow nodes and module
  actions, drag and drop, typed connections, a node inspector with data
  mapping, retries and timeouts, undo and redo, copy and paste, automatic
  arrangement, a minimap, and the engine's validation shown on the nodes.
  Step-based workflows can be opened in the editor; their steps are kept.
- Runs show on the workflow graph as they happen — running, succeeded,
  failed, skipped and cancelled nodes with durations, real progress, node
  results and declared outputs — with a run history per workflow, also for
  runs started from a Stream Deck or while the app was closed.
- Workflows run in the background agent, also while the app is closed, with
  live progress, cancellation and an execution history kept in the
  encrypted database. Runs interrupted by an agent restart are marked as
  such, and the app picks up running workflows when it opens.

### Changed

- The workflow editor: connections can be moved to another node, refused
  connections say why, nodes move with the arrow keys, new nodes never cover
  existing ones, and Arrange keeps branches apart without crossings. Data
  mappings are built from parts — earlier nodes' outputs with their types,
  and text — with a preview and warnings for missing references and values
  that do not fit. The inspector documents each node: module, permissions,
  behaviour, inputs and outputs. The view and open panels are remembered.

- Cancelling a workflow now reaches running actions: actions that support
  it (such as the new Wait action) stop at once, also when their timeout
  passes, and retries waiting for their next attempt end immediately.
  Cancelled steps are reported as cancelled, not failed. Actions that cannot
  be interrupted say so in the editor.

- Workflow runs have their own project context. A project selected in a run
  affects the nodes after it — including in parallel branches, whose changes
  a Join merges or rejects when they disagree — but not DeckPilot's active
  project or other runs. Changing the active project is a permission per
  workflow ("May change the active project"), applied when the run ends and
  only if the active project did not change meanwhile. Workflows saved
  earlier keep their behavior.

- Release validation: test releases (e.g. `v0.5.0-1`) are verified like
  final ones and are marked as pre-releases, so they never become the
  release the app updates to.

### Fixed

- An open configuration app now follows the background agent when the
  configuration is opened elsewhere (another DeckPilot window, the system
  key becoming available) instead of waiting at its unlock prompt.
- Windows release builds: OpenSSL is configured with Strawberry Perl.
- Linux test releases build `.deb` and AppImage (RPM rejects pre-release
  versions).
- Connecting to the agent gives up after a few seconds instead of waiting
  indefinitely when its endpoint is busy or does not answer (on Windows a
  busy named pipe was waited for without a limit).

## [0.4.0] - Not released

### Added

- The configuration is always encrypted (SQLCipher, AES-256). By default the
  key is a random key kept in the operating system's credential store
  (macOS Keychain, Windows Credential Manager, Secret Service), so DeckPilot
  opens at login without a password. A password (Argon2id) can be chosen
  instead, and you can switch in both directions.
- Settings → Security: unlock method, password change, lock now, automatic
  locking (sleep, screen lock on macOS and Linux, inactivity), when to ask
  for authentication, Touch ID on macOS.
- Sensitive operations (export, changing the protection, security settings,
  deleting the unencrypted copy) ask for authentication first.
- Encrypted configuration export and import (XChaCha20-Poly1305, Argon2id)
  with a preview, merging or replacing, selected profiles, conflict
  resolution, module and application settings, and resources to map on the
  new computer. Machine paths, keys and passwords are never exported.
- Native windows for prompts, export, import, update progress, the agent log
  and the workflow summary, sized to their content.
- Encrypted, verified backups before database upgrades, a migration history
  and module-owned schema versions.

### Changed

- Configurations of DeckPilot 0.3 are encrypted on the first start; the
  unencrypted original is kept until you delete it in Settings.
- Updates are only installed if their signature names the announced version.

### Security

- A missing database is never replaced silently; changes to the database
  files are serialized between the background agent and the app.
- See [docs/security.md](docs/security.md) for what each protection mode
  guarantees and its limits.

## [0.3.0] - Not released

### Added

- Configuration studio: Action Library, workflow editor, profiles and
  projects, per-computer resource mappings, device models and a visual
  layout designer that works without hardware.
- Modular features (General, Projects) and an application updater based on
  signed GitHub releases.

## [0.2.0] - Not released

### Added

- The background agent with a system tray, Stream Deck MK.2 support and an
  authenticated local IPC.

## [0.1.0] - Not released

### Added

- Project foundation: domain model, execution engine, SQLite storage and the
  configuration app.
