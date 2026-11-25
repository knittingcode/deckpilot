# The Scripts host bridge (plugin API 1.13)

The Scripts plugin (`deckpilot.scripts`, `knittingcode/deckpilot-plugins`
`official/scripts`) runs a user's JavaScript in QuickJS, inside its wasm
module, inside the sandboxed plugin host. Every effect a script has
crosses one `host_call` of the bridge described here, and the agent
decides each one against the script's **current** grants. This page is
the host side of the contract; the guest side is "The bridge contract" in
the plugin's README. The design is `design-projects-scripts.md` §4.5, §5
and §7.

Status:

- in place: the ABI and the host engine; the `scripts` permission in the
  broker; the agent's executor for the run's own operations, the
  projects, the file system and processes; the manifest's kinds at
  plugin API 1.13 (validator, store grant, supervisor limits);
- in place (P7, part 1): the script store (schema 16) with its
  machine-local trust and grants, and the trust lifecycle
  (`docs/data-model.md`, "Scripts");
- in place (P7, parts 2 and 3): scripts in exports and imports (never
  their trust or grants), and the agent's script store as the bridge's
  host, with the repositories' remotes (K4);
- in place (P7, part 4a): the Scripts page (`apps/desktop/src/modules/
  scripts/`, shown while the Scripts plugin is enabled and lists the
  `scripts` view): the profile's scripts, a CodeMirror 6 editor (parser
  diagnostics only, nothing runs while typing; completion of the
  `deckpilot` API; invisible and direction-changing characters drawn as
  marks), the script's settings, rename, duplicate (no trust), the
  guarded delete (its uses listed; clearing the keys is opt-in), and the
  review of a script that is not trusted: its code read only, a warning
  for hidden characters, the permissions that come back, and "Trust this
  script", which names exactly the hash and permissions shown;
- in place (P7, part 4b): the Permissions tab (projects, files by
  repository or a folder picked with the system's dialog, programs by
  name or picked with the system's dialog with where they run, their
  environment and, for git, unrestricted arguments; shells; deleting;
  revoke all), refusing a container runtime ("Container runtimes are not
  available to scripts yet"), a program acting as another user and a
  shell as a program, and flagging a launcher as running any program;
  the core's `script` parameter kind (`ParameterKind::Script`, a script
  id stored as text, so the Scripts plugin's `run` action now converts
  to an action type) with its picker; and the Action Library's "Run …"
  entries, one per script, each placing an action that names the script
  by id;
- next: running from the page (it needs the bundled plugin), the agent's
  `PLUGIN_API` 1.13 and the bundled plugin (P8).

## Operations

Each request is `{"op": "<op>", "args": {…}}`, a `CapabilityRequest`
variant with unknown fields refused (`plugins/runtime/crates/abi/src/scripts.rs`).
The argument structs mirror the guest's marshallers (`src/api/*.rs`) field
for field. `plugins/runtime/crates/abi/tests/fixtures/script-requests.json`
holds one request of every operation, as the guest sends it, and a list of
requests that must be refused; both repositories' tests read it.

| Op | Args | Ok value | Scope |
| --- | --- | --- | --- |
| `script.load` | `{script}` | `ScriptLoaded {source, platform, settings {timeoutMs, stopOnError, memoryMib}, grants, run {id, origin, userInitiated}, remainingMs, nowUs}` | run |
| `script.poll` | `{}` | `{cancelled, remainingMs, nowUs}` | run |
| `script.log` | `{level, message ≤ 8 KiB}` | `null` | run |
| `script.sleep` | `{ms ≤ 600,000}` | `{nowUs}` | run |
| `script.confirm` | `{token}` | `{confirmed}` | run |
| `project.list` | `{}` | `{projects: [{id, key, name}]}` (shared with plugin API 1.9) | projects |
| `project.repositories` | `{project: id \| null}` | `{project, repositories: [Repository]}` | projects |
| `fs.exists`, `fs.stat`, `fs.list` | `{path}` | `{exists}`, `{stat \| null}`, `{entries}` | fs |
| `fs.mkdir` | `{path, recursive}` | `{}` | fs |
| `fs.readFile` | `{path, encoding, maxBytes?, offset?, length?, ifVersion?}` | `{content, bytes, size, eof, version?}` | fs |
| `fs.writeFile` | `{path, content ≤ 32 KiB, encoding, createOnly}` | `{}` | fs |
| `fs.write.open` | `{path, createOnly}` | `{handle}` | fs |
| `fs.write.chunk` | `{handle, content ≤ 32 KiB, encoding}` | `{}` | fs |
| `fs.write.commit`, `fs.write.abort` | `{handle}` | `{}` | fs |
| `fs.copy` | `{src, dst, recursive, overwrite, confirmation?}` | `{}` | fs |
| `fs.move` | `{src, dst, overwrite, confirmation?}` | `{}` | fs |
| `fs.remove` | `{path, recursive, dryRun, confirmation?}` | `RemovalPlan` | fs |
| `fs.removeContents` | `{path, dryRun, keep, confirmation?}` | `RemovalPlan` | fs |
| `process.run` | `{command, args, cwd?, env, timeoutMs?, stdin?}` | `ProcessResult` | process |
| `shell.run` | `{shell, script, cwd?, env, timeoutMs?}` | `ProcessResult` | shell |
| `shell.available` | `{}` | `{shells}` | shell |

A **path** is `{"path": "<absolute>"}` or `{"repository": "<key>",
"relative": "a/b"}`, in the canonical form the guest sends; anything else
is refused as `bad-request` (`PathArg::validate`):

- an absolute path is fully qualified: `/a/b`, `C:\a\b` or `C:/a/b`, or
  UNC `\\server\share\a` (the agent accepts UNC only inside a UNC
  scope); never verbatim (`\\?\`) or device (`\\.\`), and without `.`,
  `..` or empty names (a trailing separator is fine) or a stream (`:`
  after the drive);
- a relative part is `/`-separated names, never `.`, `..`, empty, `\` or
  `:` (`""` is the repository's root);
- no part holds a control character or a bidirectional formatting
  character.

Whether a path stays inside a granted scope is decided by the agent at
every call.

A **command** is a bare name (`^[A-Za-z0-9][A-Za-z0-9._+-]*$`, not ending
in `.`) or an absolute path of a drive or the Unix root (never UNC), and
never a batch file (`.bat`, `.cmd`), which only `shell.run` with `cmd`
may start (CVE-2024-24576). An **environment** entry has a name
`[A-Za-z_][A-Za-z0-9_]*` that `scripts::env_denied` does not refuse:
`PATH`, `PATHEXT`, `HOME` and the other profile and temp variables,
`XDG_*`, every `GIT_*` but the author and committer, `SSH_*`, `DOCKER_*`
but `DOCKER_BUILDKIT`, `LD_*`, `DYLD_*`, interpreter options and paths
(`PYTHON*`, `PERL*`, `RUBY*`, `NODE_OPTIONS`, `NODE_PATH`, `JAVA_*`,
`RUSTC*`, `CARGO_*`, …), shell start-up files (`BASH_ENV`, `ENV`,
`ZDOTDIR`, …), proxies, certificate bundles, `COMSPEC`, `PSModulePath`
and `DECKPILOT_*` are refused whatever the grant.

A refusal is a `CallError` whose optional `script` member
(`ScriptFault {code, message, repository?, command?, path?,
confirmation?: {token}}`) carries the script-level detail; `code` is an
`ErrorCode` of the guest's `deckpilot.d.ts`. The host keeps the member
only on the bridge's own operations (`CallResult::for_op`,
`scripts::carries_script_detail`), so no other guest, whose `CallError`
refuses unknown fields, ever receives it.

## Frame caps: chunked I/O (decision D1)

No frame cap of ABI 1 is raised for this plugin:

- a request is at most 64 KiB (`MAX_HOST_CALL_REQUEST`), a reply at most
  1 MiB (`MAX_HOST_CALL_REPLY`), and a frame to the supervisor at most
  256 KiB;
- `fs.readFile` returns at most 512 KiB from `offset` (`MAX_READ_CHUNK`;
  its base64 is about 683 KiB). `maxBytes` (at most 4 MiB) bounds the
  whole file. A request without `length` is a whole-file read and is
  refused with `tooLarge` when the file is larger than one chunk, so a
  guest built before D1 never receives a silently cut file; `offset`
  needs `length`. In UTF-8 a chunk ends on a character boundary, and the
  agent shortens it further when its JSON escaping would not fit the
  reply cap (text full of control characters or quotes), so `bytes` may
  be less than asked and the guest reads on from `offset + bytes`. The
  first chunk takes the whole file (at most 4 MiB) into one snapshot and
  refuses with `fileChanged` when it changed while being read (its size,
  or its identity, size or modification time after the read). A reply
  that did not reach `eof` has a `version`; every later chunk names it
  (`ifVersion`, required at an offset above 0) and is served from that
  snapshot (at most 4 reads at once per run, until `eof`), after the
  path, the grants and the protected folders are checked again, and is
  refused with `fileChanged` when the file at the path is no longer that
  file in that state. So the chunks of a read are always one version of
  the file; what remains is a rewrite during the first chunk's own read
  that keeps both the size and the modification time, which the
  snapshot cannot see;
- a file larger than 32 KiB is written with `fs.write.open`, then
  `fs.write.chunk` (at most 32 KiB of content each, `MAX_WRITE_CHUNK`;
  the guest sizes a chunk by its serialized request, so text that JSON
  escapes heavily goes in smaller chunks or as base64),
  then `fs.write.commit`; at most 4 MiB in all. The agent writes a
  temporary file next to the target and renames it over the target on
  commit; `fs.write.abort`, or the end of the run, removes it;
- `process.run`: at most 256 arguments, 32 KiB each, 48 KiB in total, and
  at most 48 KiB of stdin (a larger input goes through a file the script
  writes). `shell.run`: a script of at most 48 KiB.

The guest keeps its 4 MiB file limit and chunks internally (a P5
follow-up in the plugins repository).

## Call budget and the continuous-CPU deadline

- **Calls.** `GuestLimits` (host protocol, `load`) carries two optional
  budgets that the supervisor sets for `deckpilot.scripts` only, from its
  privileged `runtime.hostCalls`: `hostCalls` (at most 4,096,
  `MAX_SCRIPT_HOST_CALLS`) in place of the 256 calls per invocation, and
  `polls` (at most 8,192, `MAX_SCRIPT_POLLS`), which counts `script.poll`
  apart (decision D2). The host clamps both to at least 1. Past a budget
  the host answers each call itself with `throttled` and the script
  detail `limit`, so the script fails with a message and the host lives;
  only a guest that keeps calling 64 more times (`OVERRUN`) traps (a
  guest ABI violation, exit code 13). A budgeted guest's request over
  64 KiB is answered `bad-request` with `tooLarge` likewise, never read.
  Every other guest is unchanged: no budget, 256 calls, a poll is an
  ordinary call, and the 257th call or an oversized request traps.
- **Deadline.** The host's ticker counts 10 ms epochs; a guest traps when
  a segment of continuous execution reaches `deadline_ms`
  (`runtime.limits.continuousMs`, 2 s by default, at most 10 s). A call
  that waits on the agent (`process.run`, `shell.run`, `script.sleep`,
  `script.confirm`, the `fs.*` ops, and every operation of other plugins)
  starts a new segment, so a long process never trips it. `script.poll`
  and `script.log` do **not** (`scripts::resets_deadline`), and neither
  does a reply the host gives itself (a malformed or locally refused
  request): the guest gets back only what was left of its segment, and
  traps at once when nothing is left. A busy loop that polls or logs
  every iteration therefore still traps at `continuousMs`. The deadline
  bounds *continuous* execution, not the total: fuel bounds the run's
  total CPU (per invocation), and the agent's invocation deadline its
  wall time.

Tests: `plugins/runtime/crates/abi/src/scripts/tests.rs` (the fixture, the
caps of D1, the script detail) and `plugins/runtime/crates/host/tests/scripts_bridge.rs`
(a poll loop and a log loop trap at the deadline, a malformed poll gives no
time back, a loop of slow `process.run` calls finishes, 4,097 calls trap,
polls are counted apart, the budgets are per invocation and clamped).

## The broker: the `scripts` permission (the ceiling)

`ApprovedPermission::Scripts {fs, process, shell}` (plugin API 1.13,
bundled first-party only through the manifest) becomes the plugin's
ceiling. The broker allows a bridge operation only for `deckpilot.scripts`,
only with the permission, and only within the sub-ceiling it needs: `fs`
for the `fs.*` operations, `process` for `process.run`, `shell` for
`shell.run` and `shell.available`; the run's own operations and the
projects with any. `project.list` of `deckpilot.scripts` is always the
bridge's (its `projects.read` decides), whatever else it holds. Revoking the permission refuses
every operation at the next call. The bridge has its own rate (2,000 calls
per second, `RateLimits::scripts`) in place of the shared 200, since a run
polls and logs often and the host already bounds each run.

## The agent: what every call is checked against

`apps/agent/src/plugins/executor/scripts/` decides each allowed call again:

1. the plugin is `deckpilot.scripts`, and the invocation has a script run:
   the action's own `script` parameter (`scripts::attach`);
2. the script's **current** machine-local grants (`ScriptGrants` in
   `deckpilot-core`, from the `ScriptHost`, read at every call, `None` for
   a script that is not trusted). An invalid record counts as no grants;
3. `script.load` comes first, once, for that script only, and refuses an
   untrusted (`scriptNeedsReview`) or missing (`scriptMissing`) one;
4. the run's time: the script's `timeoutMs` from its start, never past the
   invocation's deadline; a Stop or a past deadline ends every later call
   (`cancelled`, `timedOut`);
5. the operation's own rules (below).

Every call but `script.poll` and `script.log` writes one audit line: the
operation, the run id, the scope's label, the outcome and counts; never a
path, a file's content or an environment value.

- `script.load` answers the source, the platform, the settings, the run
  (`id`, `origin`: `key`, `gesture`, `editor`, `workflow` or `trigger`,
  `userInitiated`) and the grant summary, which names a directory scope by
  its label and an absolute command by its file name, never a path.
- `script.poll` answers `cancelled`, `remainingMs` and `nowUs`.
- `script.log` keeps the first 1,000 lines or 256 KiB of the run, then one
  marker, and writes kept lines to the agent's log with control characters
  escaped.
- `script.sleep` waits in 25 ms slices, at most the run's time left.
- `project.list` and `project.repositories` need `projects.read`, answer
  the active profile only, and give a repository's folder only with
  `repositories.paths`. A repository's `remotes` (decision K4) list every
  remote, in the stored order: `name`, `primary`, `url`,
  `defaultBranch`, the `connectionId` of the Git Providers connection it
  is linked to on this computer and the provider's `remoteRepositoryId`.
  Both ids are in script objects only, never in text variables. `remote`
  (the primary's `url` and `defaultBranch`) stays for older guests.
- The agent's script store answers a run (`service/plugin_scripts.rs`):
  the run's script, always of the **active profile** (never the profile
  or layout a binding belongs to), only while it is trusted (otherwise
  `scriptNeedsReview`, and no grants; a script the profile does not have
  is `scriptMissing`), and nothing while DeckPilot is locked. Its grants
  are read again at every call **for the source the run loaded** (its
  SHA-256, against the trusted hash, without reading the source again):
  when that is no longer the trusted source (trust revoked, imported
  over, edited and trusted again for a new source) or DeckPilot is
  locked, the run's next operation is refused (`scriptNeedsReview`) and
  the run ends. A program it already started is stopped by its own
  grant check (every 20 ms) as before. A repository's folder is its machine
  binding's, canonical, and only when it is an existing folder.

## The file system

Paths are resolved against the script's scopes: each repository scope
(`{"repository": "<key>"}` or `"*"`, every repository of the run's
current project that has a folder here) and each directory scope (a folder
picked with the native picker, by its canonical path and a label). The
longest scope root that is a prefix of the path decides; an absolute path
counts inside a repository's scope only with `repositories.paths`; nothing
else is reachable (`outsideScope`). Access is `read`, `write` or `delete`.

The scope's root is opened with `cap-std`, then checked: its canonical path
must be the recorded one and, on Unix, the opened folder must be the one at
that path; a root swapped for a link is refused (`outsideScope`, "scope
moved"). Every operation then works relative to that open folder:

- **reads** (`exists`, `stat`, `list`, `readFile`) may pass through a
  relative link that stays inside the scope; one that leads out, and every
  absolute link, is refused (`symlinkEscape`). `exists` and `stat` never
  follow the final name. `readFile` (and a copy's source) opens without
  blocking (a FIFO never hangs the run) and reads only a regular file;
- **DeckPilot's private folders** (its data with the database and the IPC
  token, its logs, configuration and caches, and the app) are never read
  or changed, nor anything that holds them, also through a link: a scope
  over the home folder still never reaches them, and listing the home
  folder itself (or `~/Library`) is refused for the same reason. They are
  recognized by their text and by their **file identity** (volume and
  file number) checked **on the handles the operation uses**: the scope's
  open root and each folder above it (walked up through `..`), and each
  folder of the path as it is opened, one at a time, so no rename between
  a check by path and the open slips through, and another name for the
  same folder (macOS's `/System/Volumes/Data` firmlink, a bind mount, a
  share) is refused too. A folder made after the agent started is learned
  when it first exists. Shares of this computer are never paths: by any
  of its names (NetBIOS, DNS with or without its domain, `.local`, a
  trailing dot), `localhost`, any address (loopback, `0.0.0.0`, LAN
  addresses, numeric forms such as `2130706433`, `*.ipv6-literal.net`)
  or an administrative `C$` share (names known only to the resolver,
  such as hosts-file aliases, cannot be told by their text: the identity
  checks are the backstop). A read follows a link by hand, folder by
  folder, each opened without following and checked: only a relative link
  that stays inside the scope, at most 40 links per path; the file itself
  is opened without following one. A process's working folder is opened
  the same way and must be the very folder its path names at the start;
- **names**: an 8.3 short name (`GIT~1`) is refused everywhere (Windows
  shares resolve them on every system), and on Windows a device name and a
  name ending in a dot or a space, so no name reaches a folder under
  another name than the one checked;
- **changes** never pass through a link: each folder of the path is opened
  without following it. A link is removed, replaced or copied as a link,
  never followed;
- **`.git`**: no change to a path inside a `.git` folder (OD-6, also
  writes, so no script plants a hook), and no removal or overwrite of a
  tree that holds one;
- **protected roots**: no change that removes, empties or replaces the file
  system or a drive root, the user's home or a folder holding it, the
  system's folders, DeckPilot's data and logs or the app; no removal of a
  repository's folder or a tree holding one; a scope's root is never
  removed, moved or replaced, and emptied (`removeContents`) only with
  `rootDestructive`;
- **destructive operations** (`remove` of a folder that is not empty,
  `removeContents`, `copy` or `move` over an existing target): `delete`
  access (`write` for an overwrite). `dryRun` returns the plan (counts,
  bytes, up to 50 sample paths, relative to the scope's label) with read
  access and changes nothing. With `destructive: "ask"` (the default) the
  first request is answered `needsConfirmation` with a token for the
  agent's plan; `script.confirm` shows that plan in a dialog, only in a run
  the user started; the repeated request with the token is carried out.
  The token is 128 random bits, kept in the run only, bound to the op and a
  digest of its resolved paths and options, single-use (removed before
  anything else is checked), refused unconfirmed, declined or after
  120 s, and dropped with the run. The plan is computed again before the
  operation runs; a plan that grew is refused and a new token issued. A
  run nobody started is refused (`notUserInitiated`) unless the script's
  grant says `destructive: "allow"`, which needs no token; with `ask`, a
  run nobody started removes or replaces nothing at all, not even one
  file: no write over an existing file (a chunked write commits only if
  nothing appeared at its target meanwhile), and no move; The token is bound to every entry of the plan (its path, kind,
  size, file id and time), not only to the counts: other files in place
  of the confirmed ones are asked again. Replacing a folder that is not
  empty needs `delete` access. A removal stops if more entries appear
  than were planned. `keep` spares a `.git` only at a repository's own
  root; elsewhere a tree holding one is never emptied. Note that `delete`
  access removes single files and empty folders without a confirmation in
  runs the user starts;
- **copies and moves** carry a link only when it is relative, of the
  form `../../name/…` (every `..` first), and still leads inside the
  scope from its new place; a move of a link, or of a folder holding a
  link, that would lead out (or any absolute link) is refused;
- **writes** go to a temporary file in the target's folder, renamed over
  the target on commit (with `createOnly`, linked to it, which never
  replaces an existing file; on a volume without hard links, the target
  is created exclusively (atomic on local volumes, SMB and NFSv3 or
  later, not NFSv2) and the content copied into it from the temporary
  file's own handle, removed again when the copy fails; never a check then
  a rename); a folder is never written over. An open chunked write is
  committed only into the very folder it was opened in. An abort, a
  revoked grant between chunks, or the end of the run removes the
  temporary file;
- **limits**: 4 MiB per file read or written (`maxBytes`, the chunked
  total), 512 KiB per read and its JSON within the reply cap (text that
  escapes heavily is read in shorter pieces), at most 4 open chunked
  writes, 10,000 entries (and about 900 KiB of names) per `list`, 100,000
  per removal, 1 GiB copied per run, 48 folders deep (so a walk holds few
  file descriptors).

A run keeps the project it had at `script.load`: switching the active
project during the run changes nothing for it. On Windows the root is
checked by its canonical path only (no file id), and paths are compared
without the verbatim prefix `canonicalize` adds; the window between that
check and the open is the residual the design documents.

Tests (temporary folders only; the fixture checks the folder is not inside
the real home, and a protected "home" inside it stands for the real one):
`apps/agent/src/plugins/executor/scripts/tests.rs` (load, poll, log,
sleep, projects, the token's single use, binding, decline, growth, expiry
and the run nobody started, the audit) and `.../scripts/fs/tests.rs`
(`scripts_fs_mkdir_*`, `scripts_fs_remove_file_*`,
`scripts_fs_remove_recursive_*`, `scripts_fs_remove_contents_*`,
`scripts_fs_protected_*`, `scripts_fs_traversal_*`, `scripts_fs_symlink_*`
with a thread that swaps a folder for a link to the protected home during
removals, chunked reads and writes, copy and move, revocation).

## Processes

`process.run`, `shell.run` and `shell.available` run through the tools
runner of `tool.run` (`plugins/tools`, `deckpilot_core::process`): the
program runs in its own process group, its stdin written (at most
48 KiB) and closed, its output captured, and it is stopped (`SIGTERM`,
`SIGKILL` after 2 s, `taskkill /T /F` on Windows) when its time is up,
the run is stopped (`cancelled`), or the grant it used is revoked
(`notGranted`): the script's grant for that command, or the plugin's
`process`/`shell` ceiling, checked every 20 ms while it runs. At most 4
runs of the plugin at once.

- **The command** must be one the script was granted, by the same name
  or path. `git` is the user's configured tool (`gitUnavailable` when
  not found); another bare
  name is looked up on the agent's `PATH` (with `PATHEXT`, `.exe` and
  `.com`, on Windows); an absolute grant must exist
  (`commandNotFound`). The file found is canonicalized and is never a
  batch file.
- **Arguments** are an array; there is no shell, glob or expansion.
- **git** without `unrestrictedArgs` (OD-8) is limited to work in the
  run's repository: the first argument is one of `add`, `blame`,
  `branch`, `checkout`, `cherry-pick`, `clean`, `commit`, `describe`,
  `diff`, `fetch`, `gc`, `grep`, `log`, `ls-files`, `ls-remote`, `merge`,
  `mv`, `pull`, `push`, `rebase`, `remote`, `reset`, `restore`,
  `rev-parse`, `revert`, `rm`, `shortlog`, `show`, `stash`, `status`,
  `switch`, `tag` (no global option: the host sets its own). Never
  `config` (aliases, `core.sshCommand`), `clone`, `init`, `worktree`,
  `submodule`, `bisect`, `difftool`, `mergetool`, `filter-branch`,
  `archive`, `format-patch`, `bundle` or `help`. The options that run a
  program, read or write elsewhere or change the configuration
  (`--upload-pack`, `--receive-pack`, `--exec`, `--output`, `--no-index`,
  `--ext-diff`, `--textconv`, `--config`, `--template`, `--strategy`,
  `--open-files-in-pager`, and those that read a file from anywhere:
  `--contents`, `--file`, `--pathspec-from-file`, `--exclude-from`, …)
  are refused, and so is every prefix of them git would accept, down to
  one letter (`--u=`); so are, by subcommand and anywhere in a cluster of
  short options (`-qx`), `rebase -x/-s/-X`, `pull -u/-s/-X`, `fetch` and
  `ls-remote -u`, `merge -s/-X/-F`, `grep -O/-f`, `commit -F/-t`,
  `tag -F`, `blame -S`, `ls-files -X`, and the `ext::` and `fd::`
  transports. `tool.run`'s hardening (`core.fsmonitor=false`, the allowed
  protocols, no prompts) is prepended. The host's variables
  (`GIT_TERMINAL_PROMPT=0`, `GCM_INTERACTIVE=never`, an empty
  `GIT_ASKPASS`, `GIT_OPTIONAL_LOCKS=0`, `GIT_PROTOCOL_FROM_USER=0`, and
  `GIT_CEILING_DIRECTORIES` at the folder above the scope, so git never
  uses a repository above it; not with `cwd: any`, which has no scope) are set after the script's and win.
  Restricted git still runs the repository's own hooks and filters (the
  user's repository); with `unrestrictedArgs` a git grant runs any
  program the user could, and is shown as such.
- **What a command may be** (`deckpilot_core::script_grants::
  command_class`, by its file name without case and program extension,
  for the granted name, the path found and the file it resolves to):
  - **container runtimes, virtual machines and namespace tools are not
    available to scripts** in this version (an open owner decision, kept
    at the safest option): `docker`, `podman`, `nerdctl`, `colima`,
    `lima`/`limactl`, `containerd`/`ctr`, `runc`/`crun`, `buildah`,
    `finch`, Rancher Desktop's and OrbStack's tools, Apple's `container`,
    `wsl`, `lxc`/`lxd`/`incus`, `multipass`, `nsenter`, `unshare`,
    `chroot`, `systemd-nspawn`, `lxc-*`, `machinectl`, QEMU (`qemu-*`),
    `virsh`, VirtualBox, Parallels, VMware's `vmrun`, UTM, Vagrant, Tart,
    and every `docker-*` or `podman-*` helper (`CONTAINER_RUNTIMES`). They are effectively full access to
    the computer; when the owner decides, they become a high-risk
    consent or stay off;
  - programs that act as another user (`sudo`, `sudoedit`, `doas`,
    `pkexec`, `su`, `runuser`, `run0`, `systemd-run`, `runas`,
    `osascript`, `launchctl`, …) are never grantable;
  - shells (`sh`, `bash`, `zsh`, `pwsh`, `cmd`, …, and `busybox` and
    `toybox`, which have a shell applet) run only through `shell.run` and
    its own grant;
  - a grant naming one of these is refused when it is saved, a stored
    record naming one grants nothing, and a granted command whose name
    or resolved file is one is refused at run time (`notGranted`, with
    the reason, e.g. "container runtimes are not available to scripts
    yet"); a bare name is ASCII;
  - programs that run any other program (interpreters such as `python`
    or `node`, `env`, `xargs`, build tools, package runners, `open`,
    `ssh`, `curl`, `RUNS_ANY_PROGRAM`) are grantable, but such a grant is
    as broad as a shell, a container runtime included (`curl
    --unix-socket` reaches a container daemon's socket), and the
    Permissions tab shows it as such. The refusal above stops only a
    direct call. A copy of a refused program under another name is not
    recognized; nor is a program reached on another computer (`ssh`).
- **`cwd`** follows the grant's rule: `repositories` (a repository of the
  run's project), `fsScopes` (one of the script's scopes) or `any` (an
  existing folder, never one of DeckPilot's private folders). It is
  resolved like a file system path, canonicalized, must be a folder
  inside its scope (a link that leads out is `symlinkEscape`), and is
  checked again right before the start. A shell runs only in a repository
  or a scope. Without `cwd`, an empty folder of the agent's. The design's
  `fchdir` in the child would close the window between that last check
  and the start; the agent forbids `unsafe` code, which `pre_exec`
  needs, so the window stays on every system, as the design already
  documents for Windows: using it needs write access to a folder inside
  the scope, which the user granted.
- **The environment** is the tools runner's base (`PATH`, `HOME`, the
  user and temporary folders, Windows' system variables, `LC_ALL=C`; no
  secrets), `LANG` and `SSH_AUTH_SOCK`, plus the script's entries when
  the grant says `env: "allowed"`, each a valid name the denylist does not
  refuse, nor the agent's own additions (`GCM_*`, `GO*`, `COMPOSE_*`,
  `SSLKEYLOGFILE`, compilers and their flags, `VIMINIT`, `EXINIT`,
  `GLIBC_TUNABLES`, `CURL_*`, `WGETRC`, `HG*`, …). `shell.run` takes the
  script's entries under the shell grant. Credentials in URLs
  (`https://user:token@…`) are removed from the output, as with
  `tool.run`.
- **Time**: `min(timeoutMs or the script's timeout, the run's time
  left)`; a timeout is a result with `timedOut: true`. **Output**: the
  script's `maxOutputKiB` per stream, both together at most 960 KiB (under
  1 MiB with the rest of the reply), then shortened until the result's
  JSON fits the reply (`truncated: true`). A non-zero
  exit is `ok: false`, never a fault.
- **`shell.run`** needs the shell in the script's `shell` grant and a
  shell this computer has; the script is one argument (`-c`, `-Command`
  with `-NoProfile -NonInteractive`, or `/d /c`). `shell.available`
  lists the granted shells that exist. On Windows `cmd` receives the
  script with Rust's standard quoting, which `cmd.exe` parses its own
  way: a script with quotes may need care.
- **Known gaps** (documented, not closed here): a descendant that leaves
  the process group (`setsid`, a daemon) survives the stop, and on Windows
  `taskkill /T` misses orphans of a child that already exited (a job
  object would close this); such a descendant also keeps the run's
  output and stdin threads alive, and is not counted in the 4 concurrent
  runs. The program is started by the path it was found at (so a program
  that reads its own name sees the granted one); the file it is is
  checked. Narrowing a grant while a program runs (its cwd rule, env,
  `unrestrictedArgs`) stops it like a revocation.

Tests (Unix, harmless stand-ins only: shell scripts for `git` and other
tools in a temporary `bin`, `sh` for the shell; never docker compose,
never git pull, never a real repository):
`apps/agent/src/plugins/executor/scripts/process/tests.rs`.

## The manifest (plugin API 1.13)

`crates/deckpilot-plugin-manifest/src/scripts.rs` and both schema copies:

| Construct | Shape | Rules |
| --- | --- | --- |
| permission `scripts` | `{kind, fs?, process?, shell?, justification}` | `deckpilot.scripts` only, privileged, high tier, at least one of the three; no `network`, `credentials` or `openUrl` beside it |
| parameter kind `script` | `{type: "script"}` | privileged, needs the `scripts` permission; never a setting |
| `code` language `javascript` | `{type: "code", language: "javascript"}` | `^1.13`, open to every plugin |
| view kind `scripts` | `{kind: "scripts"}` | a host view (`hostViews`), privileged |
| `runtime.limits.continuousMs` | 250 to 10,000 | `deckpilot.scripts` only, privileged |
| `runtime.hostCalls` | 1 to 4,096 | `deckpilot.scripts` only, privileged |

Each needs `^1.13`. The store records the permission as the grant
`scripts` (`GrantScope::Scripts`, high tier, consented once), which the
agent passes to the broker as `ApprovedPermission::Scripts`. The
supervisor gives `deckpilot.scripts` its `continuousMs` as `deadline_ms`
and its `hostCalls` together with the 8,192 polls (`guest_limits`); every
other plugin gets the defaults whatever its manifest says.
