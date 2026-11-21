# First-party plugins

DeckPilot's own plugins (`deckpilot.*`), shipped **inside the installers**
as app components (`docs/marketplace/module-migration.md` §4). Their
sources live in `knittingcode/deckpilot-plugins` (`official/<name>`),
which builds, packages, signs and releases them; the application's release
build takes those signed releases (`artifacts.json`), verifies them with
the pinned publisher keys and bundles them. Only the bundling, the locks,
the releases' contracts and the test fixtures are here.

Experimental, like all of the plugin marketplace. The plugins of
`bundled.json` are bundled. The `migration`-class ones are the only
implementation of the former built-in `github`, `gitlab`, `jira`, `git`,
`docker`, `workspace` and `projects` modules (removed in WP-C5 and WP-C6):
their legacy action ids are unavailable with the reason until the plugin
serves. Among them `deckpilot.github` (class `migration`,
[README](https://github.com/knittingcode/deckpilot-plugins/blob/main/official/github/README.md)), to which a plugin-enabled agent
routes the legacy `github.*` action types once a computer's migration is
cut over
(`docs/marketplace/module-migration.md` §6 and §9, P9),
`deckpilot.gitlab` (class `migration`, [README](https://github.com/knittingcode/deckpilot-plugins/blob/main/official/gitlab/README.md))
and `deckpilot.jira` (class `migration`, [README](https://github.com/knittingcode/deckpilot-plugins/blob/main/official/jira/README.md)),
routed the same way through the alias blocks `gitlab.*` and `jira.*`, for
GitLab.com or the server the user had entered and allows (P11), and
`deckpilot.notifications` (class `default`, the first of the default set,
[README](https://github.com/knittingcode/deckpilot-plugins/blob/main/official/notifications/README.md)), seeded on every
computer and routed through the alias block `notifications.*` (P10), and
`deckpilot.workflows` (class `default`), which owns the Workflows screens.
Git, Docker, Workspace and Projects (plugin API 1.9) are routed since
WP-C1. The published release
pipeline does not package plugins at all (see "Packaging" below).

## Layout

| Path | What |
| --- | --- |
| `bundled.json` | The plugins a release bundles: taken from a signed release artifact, `{"id": "deckpilot.<name>", "version": "X.Y.Z", "class": "default" \| "migration", "source": "artifact"}` (see "Plugins from release artifacts"), or built here from source, `{"path": "<dir>", "class": ...}` (only the test fixtures are); never both |
| `released-pins.json` | The pins of the published releases, per platform: the id, version and the size and sha256 of every file of each bundled plugin (see "Versions") |
| `artifacts.json` | The signed release artifacts from `knittingcode/deckpilot-plugins` a build bundles (`{"id", "version", "url", "sha256", "size", "keyIds"}` each), and the releases no build may use ([publisher-signature.md](../../docs/marketplace/publisher-signature.md)). All nine bundled plugins come from here |
| `artifacts.sh` | The release-artifact half of `build.sh` (sourced by it): fetch, check, verify and unpack each artifact entry |
| `contracts/` | `deckpilot.<name>/`: every file the locked release of each bundled plugin ships but its module (`manifest.json`, `LICENSE`, `assets/`, `locales/`), byte for byte, for the tests that need them at compile time: the module contracts (`crates/deckpilot-modules/tests/contracts.rs`), the reviewed privileged policy and the publisher tests. The agent's end-to-end tests (`apps/agent/tests/support/official.rs`) and the *Plugins in the agent* dry run check them against the verified release; a new release updates its contract in the same pull request as its `artifacts.json` entry |
| `test-build.sh` | Tests of `build.sh`'s artifact path with throwaway keys made in the test (*Plugins in the agent* workflow) |
| `support/` | `deckpilot-first-party-support`: a library the plugins for a user-entered server (`gitlab`, `jira`, `github` for GitHub Enterprise Server) share (URLs without a URL parser, the time limit counted with `Date` headers, text helpers). `knittingcode/deckpilot-plugins` depends on it at its pinned DeckPilot commit; never bundled by itself |
| `build.sh` | Stages the listed release artifacts after verifying them (and builds the listed source plugins, the test fixtures, to wasm: `wasm32-unknown-unknown`, `guest` profile, reproducible on one platform), and pins them all |
| `test-fixtures/` | **Test fixtures, never shipped**: `example-default` (`deckpilot.example-default`, class `default`, one action, one low-tier permission), `legacy-fixture` (`deckpilot.legacy-fixture`, class `migration`: the replacement of the agent's fixture legacy module in the module migration tests, with a low-, a medium- and a high-tier permission) and their own `bundled.json`, used only with `build.sh --test-fixtures` |

The plugins' own tests, the differences from the built-ins they replaced
(each plugin's README) and their sources are in
[`official/`](https://github.com/knittingcode/deckpilot-plugins/tree/main/official)
of `knittingcode/deckpilot-plugins`; here the agent tests them as bundled
(`apps/agent/tests/plugins_<name>.rs`, `plugins_cutover*.rs`,
`core_without_plugins.rs`).

## What the build produces

```sh
plugins/first-party/build.sh --out target/bundled-plugins            # the release set
plugins/first-party/build.sh --out target/bundle --test-fixtures     # tests, dry runs
# with release artifacts listed: pre-download, then build without network
plugins/first-party/build.sh --fetch-only --artifacts-dir target/plugin-artifacts
plugins/first-party/build.sh --out target/bundled-plugins --artifacts-dir target/plugin-artifacts --offline
```

All options (`build.sh --help`): `--out DIR`, `--test-fixtures`,
`--target-dir DIR` (Cargo's, and where downloads and the registry tool go
by default), `--artifacts-dir DIR` (the artifact cache), `--offline` (never
download), `--fetch-only` (download and check size and sha256, then stop),
`--registry-tool FILE` (a `deckpilot-registry` binary; otherwise built
with `cargo build -p deckpilot-plugin-registry --features publish` when an
artifact is listed), `--app-version V`, `--plugin-api MAJOR.MINOR` and
`--platform macos|linux|windows` (the host the artifact is checked for;
defaults: the workspace version, 1.9, this platform). Tests only:
`--test-only-publisher-keys FILE` with `DECKPILOT_TEST_ONLY_PUBLISHER_KEYS=1`
(see below). Besides `<out>/bundled/`, an artifact build writes
`<out>/artifacts-audit.jsonl`, one `release verify` record per artifact,
which is not shipped.

```text
<out>/bundled/
  pins.json                              id, version, class, size and sha256 of every file
  deckpilot.<name>/manifest.json
  deckpilot.<name>/plugin.wasm
  deckpilot.<name>/LICENSE
  deckpilot.<name>/assets/icon-128.png   the icons and visual assets (PNG, JPEG) the manifest names
  deckpilot.<name>/locales/<tag>.json    the translations the manifest declares
```

Never a `.dpplugin`, never an archive of its own, never a signature: a
bundled plugin is trusted by the **pins compiled into the agent**
(option B of §4.4), plus the app bundle's code seal on macOS (option A).

## How the pins are trusted

1. `build.sh` writes `bundled/pins.json`.
2. The agent is compiled with `DECKPILOT_BUNDLED_PINS=<out>/bundled/pins.json`:
   `crates/deckpilot-plugin-bundled/build.rs` copies the file into the
   build and `include_bytes!` embeds it. Without the variable (every build
   today) nothing is pinned and nothing is seeded.
3. `tools/release-pipeline/check-bundled-plugins.sh <out>/bundled <agent>`
   checks the staged files against `pins.json` and `pins.json` against
   the agent's `--print-bundled-pins`.
4. At start-up or unlock the agent seeds the bundled plugins from the
   application's resources (`apps/agent/src/plugins/bundled.rs`), checking
   every file against the compiled pins, never against the `pins.json`
   next to them, and checks the installed copy again at every load.

## Versions

A released version of a bundled plugin never changes. An installation
replaces its installed copy only with a **higher** bundled version
(`apps/agent/src/plugins/bundled.rs`, and for a cut-over migration-class
plugin the migration engine), and checks the installed copy
against the pins compiled into the agent at every load: a plugin rebuilt
with other files under a version an earlier release shipped (a changed
SDK or support crate is enough) is **broken** on every installation
upgraded from that release. So every change to a bundled plugin's files
gets a new version, released from `knittingcode/deckpilot-plugins` (its
`manifest.json`, `Cargo.toml`, `Cargo.lock` and README there) and locked
here (`artifacts.json`, `bundled.json`, `contracts/`): a patch
version for a rebuild or a fix, a minor version for new actions,
settings, permissions or a higher plugin API (0.2.0 of `deckpilot.github`
for GitHub Enterprise).

`released-pins.json` records what each release shipped, per platform
(`macos`, `linux`, `windows`; `build.sh` is reproducible on one platform,
and both macOS apps are built on the same runner). It does not depend on
where the checkout is: Cargo hashes a member outside the workspace root
(`../first-party/...` of `plugins/runtime`) by its absolute path into the
code, so `build.sh` builds in a copy of the workspace rooted at `plugins/`,
and a release's pins recorded on a developer's Mac match the runner's.
Format 1:

```json
{ "releasedPinsVersion": 1,
  "releases": [ { "release": "0.9.0", "platform": "macos", "source": "<where the pins come from>",
                  "plugins": [ { "id": "deckpilot.<name>", "version": "X.Y.Z",
                                 "files": { "<path>": { "sha256": "<hex>", "size": 0 } } } ] } ] }
```

`tools/release-pipeline/check-released-pins.sh <out>/bundled/pins.json`
fails when a built plugin has the id and version of one a release recorded
for this platform, with other files (another file set, size or sha256),
and names the files; a new version, or the same files, pass. It runs in
`check-bundled-plugins.sh` (so in the release and *Packaging* builds and
`deckpilot-dev package`) and in the bundle dry run of the *Plugins in the
agent* workflow, and its cases are in
`tools/release-pipeline/test-bundled-plugins.sh`. Only `macos` has
recorded pins so far (0.9.0, from the published app); a platform without
any passes with a note, and the macOS checks catch a source change for
every platform.

Release preparation appends the release's pins
([docs/releasing.md](../../docs/releasing.md), "Version management"):

```sh
plugins/first-party/build.sh --out target/bundled-plugins
tools/release-pipeline/check-released-pins.sh --record X.Y.Z target/bundled-plugins/bundled/pins.json \
  --platform macos --source "plugins/first-party/build.sh at the release commit"
```

`--record` runs the same check first, changes nothing when the same pins
are recorded again and refuses other pins for a release and platform
already recorded.


`deckpilot-dev package` with `DECKPILOT_RELEASE_PLUGINS=1` builds the
plugins, the agent with the `plugins` feature and their pins, and the
plugin host from `plugins/runtime`, then bundles with
`apps/desktop/src-tauri/tauri.plugins.conf.json` on top of
`tauri.release.conf.json`: the host as an external binary next to the
agent, `bundled/` as a resource. On macOS the host is signed first with
its one entitlement and copied into `Contents/MacOS` instead
(`tauri.plugins.macos.conf.json`; [docs/releasing.md](../../docs/releasing.md),
"Packaging the plugins"). Without the variable (the default) the
package has neither. The release workflow (`.github/workflows/release.yml`)
and the *Packaging* workflow's package checks build every installer this
way, and `tools/ci/check-packages.sh` requires the host and exactly the
plugins of `bundled.json` in each.

## Plugins from release artifacts

The sources of the official plugins now live in
`knittingcode/deckpilot-plugins` (`official/`), which builds, packages and
releases them, each release `deckpilot.<name>--v<version>` with the package
`deckpilot.<name>-<version>.dpplugin` and two signature files
`<package>.<KEYID>.minisig` by the first-party publisher keys. A
`bundled.json` entry with `"source": "artifact"` makes `build.sh` bundle
that release:

1. the entry of `artifacts.json` with the same id and version is the
   reviewed claim: `url` (exactly the immutable release URL, never
   `latest`), `sha256`, `size`, `keyIds`;
2. the package and one signature file per key id come from
   `--artifacts-dir` or are downloaded from that URL (never with
   `--offline`); nothing is downloaded for source entries;
3. size and sha256 are checked first; then `deckpilot-registry release
   verify` with the **pinned** publisher keys and their threshold,
   `released-pins.json` (no downgrade; the same version only with the same
   files), the lock's `revoked` list, the audit log and `--unpack` into
   `bundled/<id>/`; every listed key must have counted;
4. the pins step pins the unpacked files like a source build's.

Any mismatch refuses the build: tampered bytes, a missing file or
signature, fewer than the threshold of pinned keys, a revoked key or
release, a downgrade, another id or version, a mutable URL. `build.sh`
and the required *Rust* checks
(`crates/deckpilot-plugin-registry/tests/publisher.rs`) also refuse a
plugin that is both a source and an artifact, a lock and `bundled.json`
that disagree on an id or version, and an artifact entry for a plugin with
plugin API 1.9's privileged kinds (Docker, Git, Projects, Workflows,
Workspace) at any version but exactly the one the reviewed privileged
policy lists (the gate is open for exactly that policy since 2026-11-21);
`artifacts.sh` then requires `release verify`'s `privileged: reviewed: …`
line. Details:
[publisher-signature.md](../../docs/marketplace/publisher-signature.md#wave-b-consuming-an-artifact).

**Tests** (`test-build.sh`) use throwaway keys generated in the test,
passed with `--test-only-publisher-keys`, which needs
`DECKPILOT_TEST_ONLY_PUBLISHER_KEYS=1` and marks the bundle
`NOT-FOR-RELEASE` (refused by `check-bundled-plugins.sh`). A release build
never passes it. Run locally:

```sh
cargo build --locked -p deckpilot-plugin-registry --features publish --bin deckpilot-registry
plugins/first-party/test-build.sh target/first-party-artifacts-test target/debug/deckpilot-registry
```

### The switch (2026-11-21)

All nine bundled plugins come from their signed releases (`bundled.json`
lists each with `"source": "artifact"`, `artifacts.json` locks each with
both production key ids, `BDCDC80985EB60B5` and `8C3C4E42700FA702`):

| Plugin | Version (source build it replaces) | Class | Privileged |
| --- | --- | --- | --- |
| `deckpilot.docker` | 0.1.1 (0.1.0) | `migration` | reviewed policy |
| `deckpilot.git` | 0.1.1 (0.1.0) | `migration` | reviewed policy |
| `deckpilot.github` | 0.2.1 (0.2.0) | `migration` | none |
| `deckpilot.gitlab` | 0.1.2 (0.1.1) | `migration` | none |
| `deckpilot.jira` | 0.1.2 (0.1.1) | `migration` | none |
| `deckpilot.notifications` | 0.1.2 (0.1.1) | `default` | none |
| `deckpilot.projects` | 0.1.1 (0.1.0) | `migration` | reviewed policy |
| `deckpilot.workflows` | 0.1.1 (0.1.0) | `default` | reviewed policy |
| `deckpilot.workspace` | 0.1.1 (0.1.0) | `migration` | reviewed policy |

Each release was rebuilt from its tag (`deckpilot.<name>--v<version>`, all
at deckpilot-plugins `f86ea926`) with `tools/official.sh fresh-build` on
Linux x86-64, the release runners' platform, and the package matched the
published asset byte for byte (sha256 and size). A build on macOS gives
the same files but `plugin.wasm` (the host platform changes the module, as
`released-pins.json` records per platform); the package's sha256 and size
are those of the published Linux build, which is what `artifacts.json`
locks.

The agent tests run these files too (`apps/agent/tests/support/official.rs`:
`build.sh` verifies and unpacks them, CI stages them once per job in
`DECKPILOT_E2E_OFFICIAL_PLUGINS`), and the upgrade from the copies v0.10.0
and v0.11.0 built from source is tested for a migration-class plugin
(`plugins_cutover.rs`, `plugins_cutover_git.rs`: a cut-over copy moves to
the released version with the user's settings, credentials, grants and
keys, and never backwards) and a default-class one
(`plugins_notifications.rs`).

At the application release, record the bundle's pins in
`released-pins.json` as usual (`check-released-pins.sh --record`, per
platform): the artifacts' versions and files become the floor.

The sources here (`plugins/first-party/<name>/`, their workspace members
and `Cargo.lock` entries) were removed afterwards, with the tests and CI
that built them; the files each release ships but its module stayed as its
contract (`contracts/`).
