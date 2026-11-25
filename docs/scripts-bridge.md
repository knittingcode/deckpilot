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
  projects and the file system;
- next: processes (`process.run`, `shell.run`, answered `unavailable`
  until then), the manifest kinds (no manifest can declare `scripts` yet,
  so nothing is granted in practice), and the script store (P7; until
  then the agent keeps no scripts and every run ends with
  `scriptMissing`).

## Operations

Each request is `{"op": "<op>", "args": {…}}`, a `CapabilityRequest`
variant with unknown fields refused (`plugins/runtime/crates/abi/src/scripts.rs`).
The argument structs mirror the guest's marshallers (`src/api/*.rs`) field
for field. `plugins/runtime/crates/abi/tests/fixtures/script-requests.json`
holds one request of every operation, as the guest sends it, and a list of
requests that must be refused; both repositories' tests read it.

| Op | Args | Ok value | Scope |
| --- | --- | --- | --- |
| `script.load` | `{script}` | `ScriptLoaded {source, platform, settings {timeoutMs, stopOnError, memoryMib}, grants, run {id, origin, userInitiated}, nowUs}` | run |
| `script.poll` | `{}` | `{cancelled, remainingMs, nowUs}` | run |
| `script.log` | `{level, message ≤ 8 KiB}` | `null` | run |
| `script.sleep` | `{ms ≤ 600,000}` | `{nowUs}` | run |
| `script.confirm` | `{token}` | `{confirmed}` | run |
| `project.list` | `{}` | `{projects: [{id, key, name}]}` (shared with plugin API 1.9) | projects |
| `project.repositories` | `{project: id \| null}` | `{project, repositories: [Repository]}` | projects |
| `fs.exists`, `fs.stat`, `fs.list` | `{path}` | `{exists}`, `{stat \| null}`, `{entries}` | fs |
| `fs.mkdir` | `{path, recursive}` | `{}` | fs |
| `fs.readFile` | `{path, encoding, maxBytes?, offset?, length?}` | `{content, bytes, size, eof}` | fs |
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
  be less than asked and the guest reads on from `offset + bytes`;
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
projects with any. `project.list` is the bridge's for a plugin with
`scripts` and without `projectContext`. Revoking the permission refuses
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
  `repositories.paths`.

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
  absolute link, is refused (`symlinkEscape`). `stat` and `list` never
  follow the final name. `readFile` opens without blocking (a FIFO never
  hangs the run) and reads only a regular file;
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
  grant says `destructive: "allow"`, which needs no token. A removal
  stops if more entries appear than were planned;
- **writes** go to a temporary file in the target's folder, renamed over
  the target on commit (with `createOnly`, linked to it, which never
  replaces an existing file); a folder is never written over. An abort, a
  revoked grant between chunks, or the end of the run removes the
  temporary file;
- **limits**: 4 MiB per file read or written (`maxBytes`, the chunked
  total), 512 KiB per read and its JSON within the reply cap (text that
  escapes heavily is read in shorter pieces), at most 4 open chunked
  writes, 10,000 entries per `list`, 100,000 per removal, 1 GiB copied per
  run, 128 folders deep.

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
