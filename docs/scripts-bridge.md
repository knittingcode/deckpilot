# The Scripts host bridge (plugin API 1.13)

The Scripts plugin (`deckpilot.scripts`, `knittingcode/deckpilot-plugins`
`official/scripts`) runs a user's JavaScript in QuickJS, inside its wasm
module, inside the sandboxed plugin host. Every effect a script has
crosses one `host_call` of the bridge described here, and the agent
decides each one against the script's **current** grants. This page is
the host side of the contract; the guest side is "The bridge contract" in
the plugin's README. The design is `design-projects-scripts.md` §4.5, §5
and §7.

Status: the ABI and the host engine are in place (this page's first two
sections). The agent's executor (file system, processes), the `scripts`
permission and the manifest kinds follow in their own pull requests; until
then the broker refuses every bridge operation to every plugin.

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
