# Registry publishing tooling

> **Not live yet.** The registry repository
> [`knittingcode/deckpilot-plugins`](https://github.com/knittingcode/deckpilot-plugins)
> exists, with its workflows and its Pages site
> (<https://knittingcode.github.io/deckpilot-plugins/>). Its environments
> `registry-release` and `registry-timestamp`, their secrets and every
> production key are still pending the owner's approval of the trust
> configuration, so no signed registry is published. This tool has been
> used only with throwaway keys in tests.

Registry metadata and plugin packages are published and signed only from
the registry repository, never with or alongside a DeckPilot application
release, and with keys and environments of their own: see the distribution
contract, [`docs/marketplace/distribution.md`](../../docs/marketplace/distribution.md).

`deckpilot-registry` is the offline half of sections 9 to 11 of the plugin
marketplace design (`docs/marketplace/architecture.md`, pull request #191):
it writes the static registry site that `RegistryClient` in this crate
verifies, and it checks the result with that same client before anything
is deployed. It has no network code. It does not upload releases, call the
GitHub API or deploy Pages; the workflows below do that around it.

```sh
cargo build -p deckpilot-plugin-registry --features publish --locked --release
target/release/deckpilot-registry help
```

The `publish` feature only adds the `publish` module and the binary
(dependencies `minisign`, `zeroize`, and for `storefront check|build` and
`verify-site --require-storefront`: `image` with PNG, JPEG and WebP
decoders, `unicode-normalization` and `jsonschema`). Without it the crate is
the verifier library and nothing else: the client walks storefront image
headers (`media.rs`) but never decodes pixels.

## Working directory

Every command works on plain files in `--dir` (default `.`). `site/v1/` is
exactly what Pages serves under `https://knittingcode.github.io/deckpilot-plugins/v1/`.

```text
roots.json                             embedded root role (public): the app's
                                       DECKPILOT_REGISTRY_ROOT_PUBKEYS
registry/publishers/<login>.json       publisher entries of the index
registry/plugins/<id>/plugin.json      listing and detail metadata (--plugin-detail)
registry/plugins/<id>/versions/<v>.json  version history, append-only
registry/revocations.json              cumulative revocations
releases/<id>--v<v>/<id>-<v>.dpplugin  release assets (GitHub Releases, not Pages)
site/v1/keys/<n>.json(.minisig)        root-signed key sets (signature array)
site/v1/index/<seq>.json(.minisig)     signed index snapshots (newest plus 10)
site/v1/plugins/<id>/<sha256>.json     content-addressed details
site/v1/timestamp.json(.minisig)       the only mutable document

registry/plugins/<id>/listing.json     storefront listing (author source)
registry/plugins/<id>/media/*          its images: .png, .webp, .jpg
editorial/editorial.json               storefront placements (maintainers only)
editorial/media/*                      their images
site/v1/storefront/<seq>.json(.minisig)  signed storefront envelope (index role)
site/v1/storefront/listings/<id>/<sha256>.json  content-addressed listings
site/v1/storefront/editorial/<sha256>.json      content-addressed editorial
site/v1/media/<sha256>.<png|webp|jpg>  content-addressed images
```

Secret keys never belong in this directory.

## Commands

| Command | Does | Needs |
| --- | --- | --- |
| `keys generate --role root\|index\|timestamp\|publisher --out FILE` | minisign key pair: `FILE` (mode 0600, scrypt-encrypted unless `--no-password`) and `FILE.pub` | password |
| `keyset init --root PUB… [--root-threshold N] --index PUB --timestamp PUB [--seq-floor N] [--first-party]` | `roots.json` and an unsigned `keys/1.json`; `--first-party` adds `"first_party": {"namespaces": ["deckpilot"]}` ([first-party plugins](#first-party-plugins-deckpilot)) | public keys only |
| `keyset rotate [--root PUB…] [--root-threshold N] [--index PUB] [--timestamp PUB] [--seq-floor N] [--seq-floor-reset] [--first-party \| --no-first-party]` | unsigned `keys/<n+1>.json`; unchanged roles and the first-party authorization are carried over (`--first-party` adds it, `--no-first-party` withdraws it); `seq_floor` defaults to the live seq | public keys only |
| `keyset sign --key FILE [--version N]` | adds one root signature; prints how many of the previous and new root thresholds are met | a root key |
| `package --plugin-dir DIR --module FILE.wasm --out FILE.dpplugin [--first-party]` | builds a package: `manifest.json`, `LICENSE` and the files the manifest names from `DIR`, the module as `plugin.wasm` (`docs/marketplace/artifact-contract.md`), deterministic and checked by every reader rule; a `deckpilot.*` plugin needs `--first-party` (the first-party manifest rules) | — |
| `publisher --namespace NS --github-id N --login L [--display-name] [--verified] [--first-party]` | `registry/publishers/<login>.json`; the namespace `deckpilot` needs `--first-party`, every other reserved namespace is refused | — |
| `add-version --package F.dpplugin [--plugin-detail plugin.json] --source-repository O/N --tag T --commit SHA --builder V [--source-path P] [--attestation URL] [--first-party]` | verifies the package with `deckpilot-plugin-package` (`read_archive`, then `verify_package` against its own digests), takes sha256, size, manifest digest, permissions, engines and platforms from it, appends the version, stores the release asset; a `deckpilot.*` package needs `--first-party` | — |
| `yank --id ID --version V --reason R [--advisory URL]` | marks the version `yanked` in its detail and adds a `yank` revocation (the Settings notice) | — |
| `revoke --id ID --versions V,…\|* --action disable\|yank --reason R [--advisory URL]` | adds a revocation, or updates reason, advisory and date of an identical one; never removes or narrows | — |
| `lift-revocation --id ID [--versions …]` | removes revocations; refused unless a root-signed key set with `seq_floor_reset` has no index yet | — |
| `build-index --source-commit SHA [--min-app-version V] [--keep 10]` | details and an unsigned `index/<seq>.json`, `seq` = live seq + 1 (at least `seq_floor`); prunes to the newest 11 snapshots and the details they pin | — |
| `sign-index --key FILE [--seq N]` | `index/<seq>.json.minisig` with `deckpilot-registry v1 role:index seq:<seq> keys:<n>` | the index key |
| `timestamp --key FILE [--expires-in 7d]` | re-signs `timestamp.json` for the newest signed index | the timestamp key |
| `verify-site [--roots F] [--state DIR] [--min-remaining 3d] [--require-packages] [--require-storefront]` | runs the client end to end (see below) | — |
| `storefront check` | checks the [storefront](#the-storefront) sources: input schemas, text, links, markdown, every image (header rules and a full decode), the ids and versions they name | — |
| `storefront build [--seq N]` | for the signed `index/<N>.json` (default: newest): listing and editorial documents (published entries only), `media/<sha256>.<ext>`, and the unsigned `storefront/<N>.json` bound to that index; prunes storefronts whose index is gone and files no kept storefront references | — |
| `storefront sign --key FILE [--seq N]` | `storefront/<N>.json.minisig` with `deckpilot-registry v1 role:storefront seq:<N> keys:<n>`, after checking the envelope is bound to the signed `index/<N>.json` | the index key |
| `release sign --package F.dpplugin --id ID --version V (--key FILE \| --key-env NAME) [--out-dir DIR]` | the first-party publisher signature of a release artifact as `ID V` (from the release tag; the package must say the same) by **one** key, written to `F.dpplugin.<KEYID>.minisig` (KEYID: the key's minisign key id, 16 uppercase hex) next to the package or in `--out-dir`, never overwritten; after the verifier's own checks, which run before the key is loaded; the key is dropped before the new signature is verified ([publisher-signature.md](../../docs/marketplace/publisher-signature.md)); run once per key, only in `knittingcode/deckpilot-plugins`' release environment | one publisher key |
| `release verify --package F [--signatures-dir DIR] --id ID --version V --sha256 H --size N --app-version V --plugin-api M.N [--platform P] [--keys F] [--released-pins F] [--revocations F] [--audit F.jsonl] [--unpack DIR]` | reads every `F.*.minisig` next to the package (or in `--signatures-dir`), lists each file as counted (key id, fingerprint) or ignored (reason), and verifies the release artifact against the claim, the threshold of distinct pinned publisher keys (`--keys`: test keys, flagged in the output), revocations, the host and the rollback floor; audits the outcome; grants nothing | — |

`--now <RFC 3339>` replaces the system clock in every command that records
a time.

### What each command refuses

- **Key sets**: roles sharing a key; an index or timestamp threshold other
  than 1 (those documents carry a single signature, so a higher threshold
  could never be met); a rotation that changes nothing; a `seq_floor`
  lower than the current one without `--seq-floor-reset`; a signature from
  a key that is not a root of the previous or the new key set. Every later
  command first walks the whole key set chain with the client's own
  `apply_keyset`, so nothing is built on a key set clients would refuse.
- **Versions**: an invalid package; a version not strictly greater than
  the latest recorded (append-only, section 11.3); another package for a
  recorded version (re-adding the identical package is a no-op); an
  unknown publisher namespace; a detail over 256 KiB; a `deckpilot.*`
  package without `--first-party`, or with it while the current key set
  does not authorize `deckpilot` or no verified publisher owns it;
  `--first-party` on any other package.
- **Publishers**: a reserved namespace (`RESERVED_PUBLISHERS`), except
  `deckpilot` with `--first-party`.
- **Index**: an earlier index still unsigned; revocations of the live
  index that `registry/revocations.json` no longer covers at least as
  strongly (the client's `RevocationDropped` rule), except right after a
  `seq_floor_reset` key set; a document over the schema's limits.
- **Signatures**: a key that is not in the current key set's role; an
  index naming an older key set (a rotation needs a new index).
- **Timestamp**: validity over 14 days (the client's cap) or not positive;
  an index whose signature does not verify for the current key set; a seq
  below the live timestamp's (unless the key set resets the floor) or
  below `seq_floor`; a signing time earlier than the live timestamp's.

### `verify-site`

The CI gate before every deploy. It opens a `RegistryClient` with
`roots.json` (or `--roots`) and runs `refresh` over `site/v1` exactly as
the agent will: key set chain, timestamp, index. Then it requires `Fresh`
and at least `--min-remaining` before expiry, verifies every listed
detail with `verify_detail`, runs `resolve_install` for every offered
version (and checks withdrawn ones are refused), verifies each release
asset found under `releases/` with `verify_package` against the pins
(`--require-packages`: every one must be there), and checks that every
listed revocation is enforced by `installed_verdict`. A `deckpilot.*`
plugin offered while the key set does not authorize `deckpilot`, or
without a verified `deckpilot` publisher, fails it: `resolve_install`
refuses it as every client would.

Without `--state` it is a fresh client (trust on first use). With `--state
DIR` it uses a `FileTrustStore`: run it once against the live site to get
the state of an existing client, then against the new site with that
state, and it proves that existing clients accept the update: no rollback,
no equivocation, no dropped revocation, a continuous key set chain. A
refused site never changes the stored state.

Without `--require-storefront`, `verify-site` does not look at
`storefront/` or `media/` at all: its result and output are the same as
before storefronts existed, whatever those directories hold. With it, it
then fetches `storefront/<seq>.json` for the verified seq and has the same
client verify it (`verify_storefront`, `verify_listing`,
`verify_editorial`), then checks every image the documents pin with the
client's `verify_media` and decodes it in full; a missing or refused
storefront fails.

## The storefront

Editorial placements (banners, rows, spotlights, collections) and listing
content (promo text, description, screenshots, icon, links, release notes)
for Discover. Nothing in the documents above changes: every registry
document refuses unknown fields and DeckPilot 0.10 to 0.12 read the
production registry, so the storefront lives at **new paths** that older
clients never request, and the key set gains **no role**.

- **Signer**: the index role, with its own exact trusted comment
  `deckpilot-registry v1 role:storefront seq:<seq> keys:<kv>`
  (`sig::storefront_comment`). Comments are exact, so an index signature
  is never a storefront signature, nor the reverse. Same `sign` job, same
  `registry-release` environment, same admin-only tag as the index. The
  timestamp key and the publisher keys never sign it.
- **Binding**: `storefront/<seq>.json` carries `seq`, `keys_version` and the
  `{path, sha256, length}` of `index/<seq>.json`; a client accepts it only
  for its currently verified timestamp (so rollback and freeze protection
  come from the timestamp chain), and only if every listed id is in that
  index. Listings and the editorial document are pinned by sha256 and
  length and checked before parsing; images likewise, at
  `media/<sha256>.<ext>`, a path the client derives itself.
- **Equivocation**: the client records the seq and sha256 of the
  storefront it accepted in `registry-storefront.json`, beside (not in)
  `registry-trust.json`, whose fields stay exactly those older builds read.
  Another storefront for the same seq is refused.
- **Strictness**: the envelope and image references refuse unknown fields;
  listing and editorial content ignore them, and an editorial section of an
  unknown `kind` is skipped. An unknown `storefront_version`,
  `listing_version` or `editorial_version` refuses that document only.
- **Refusal**: a refused storefront means catalog-only Discover; it never
  changes the result of a refresh or an install.

Sources: `registry/plugins/<id>/listing.json`
([`schema/inputs/listing.schema.json`](schema/inputs/listing.schema.json),
written by the plugin's author, no placement fields) and
`editorial/editorial.json`
([`schema/inputs/editorial.schema.json`](schema/inputs/editorial.schema.json),
maintainers only; `draft`, `published` or `archived`, only `published`
reaches the site, without `status`). The signed documents are described by
`schema/registry-{storefront,listing,editorial}.schema.json`.

Images (`storefront check` and every client):

| Use | Formats | Size and aspect | Max bytes |
| --- | --- | --- | --- |
| icon | PNG, WebP | square, 256 to 1024 px | 256 KiB |
| screenshot (0 to 8, alt text required) | PNG, WebP, JPEG | 16:10 or 16:9 (±1%), 1280 to 2560 px wide | 1 MiB |
| promo art (alt text required) | PNG, WebP, JPEG | 2:1, 1600 to 3200 px wide | 1 MiB |
| banner art (alt text, or `decorative`) | PNG, WebP, JPEG | 3:1, 1200 to 3600 px wide | 1 MiB |

At most 8,388,608 pixels per image, 8 MiB per listing and 16 MiB for the
editorial document. 8-bit channels only, no animation (APNG, animated
WebP), no metadata (PNG text and `eXIf` chunks, JPEG APP1/APP13 and
comments, WebP `EXIF`/`XMP`), an ICC profile up to 64 KiB; PNG chunks
from an allow-list with every CRC checked and nothing after `IEND`; JPEG
baseline or progressive with 1 or 3 components and nothing after `EOI`;
WebP whose RIFF size is the file length. The format is sniffed from the
bytes and must match the extension. **SVG is refused.**

Text is Unicode NFC, without control characters (except line breaks in
descriptions and release notes), bidi controls or zero-width characters;
every localized map has `en` (and today only `en` and `pl`). Descriptions
and release notes use a safe markdown subset (paragraphs, one-level lists,
bold, emphasis, code, https links); raw HTML, headings, images, tables and
quotes are refused. Links are `https:` only, at most 512 characters, with
no userinfo, IP address, port or punycode host; a `deckpilot.*` listing
links only to `github.com/knittingcode/…` or `knittingcode.github.io`.
Listing text outside the `deckpilot` namespace that says "official",
"verified" or "DeckPilot team" is flagged for review.

The publishing order is `build-index`, `sign-index`, `storefront build`,
`storefront sign`, `timestamp`, `verify-site --require-storefront`.

## First-party plugins (`deckpilot.*`)

The publisher `deckpilot` is reserved: a manifest that names it is refused
everywhere (`Manifest::validate`) except for the plugins the application
bundles, verified against digests compiled into the agent. A registry may
serve `deckpilot.*` only when **all** of these hold, checked by the client
(`RegistryClient::resolve_install`) before any download:

1. The current key set, signed by the offline root role, carries
   `"first_party": {"namespaces": ["deckpilot"]}` (`keyset init
   --first-party` or `keyset rotate --first-party`). The field is covered
   by the root signatures like the rest of `keys/<n>.json`; a key set naming
   any other namespace there is invalid. Absent means no authorization; key
   sets without it are unchanged.
2. The verified index lists the publisher `deckpilot` with `verified: true`
   (`publisher --namespace deckpilot --first-party --verified`).
3. The package then passes `verify_package` with the first-party manifest
   rules (`ExpectedPackage::first_party`, set only by the client for such a
   plugin); every other reserved publisher stays refused.

The index key alone therefore cannot open the namespace: it needs a root
ceremony. The agent never takes the plugins it bundles
(`deckpilot.github`, `deckpilot.gitlab`, `deckpilot.jira`,
`deckpilot.notifications`, `deckpilot_plugin_bundled::BUNDLED_PLUGIN_IDS`)
from any registry, so their pinned-digest trust stays their only source;
listing them is pointless and their installs are refused.

**Withdrawing it.** `keyset rotate --no-first-party`, signed by the roots,
refuses new installs and updates of `deckpilot.*` from then on. Installed
copies keep running, as with a yank; stopping them takes a `disable`
revocation (`revoke --action disable`). As long as the index still offers
a `deckpilot.*` version, `verify-site` fails: revoke (or yank) them in the
same publish.

**Compatibility.** A client that predates the field refuses a key set
carrying it (unknown field) and with it the rest of the chain. No released
DeckPilot reads the registry yet, so the field belongs in the **first
production key set** (`keyset init --first-party` in the ceremony below),
never added after clients exist.

## Secrets

- Passwords come from `--password-stdin` (one line), `--password-env NAME`,
  or `DECKPILOT_REGISTRY_KEY_PASSWORD`. They are never printed, never put
  in error messages, and wiped from the tool's own memory after use.
- Secret keys are read from files only. Output names public key ids at
  most. `SigningKey` and `Password` redact themselves in `Debug`.
- Documents are deterministic (pretty JSON in struct order, final newline;
  same inputs, same bytes). Minisign signatures are randomized, so only
  the signature files differ between two runs.

## Production keys: key ceremony (U-10)

`keys generate` exists for local registries and tests. Production keys
are created in a key ceremony that the owner decides on (U-10; v1 plans
two root keys with threshold 1). At minimum:

1. On an offline machine, create the two root keys and the first index
   and timestamp keys, each with its own strong password.
2. `keyset init --first-party` with the public keys (the first-party
   authorization must be in this first key set,
   [above](#first-party-plugins-deckpilot)); `keyset sign` with one or both
   roots on that machine. Commit `roots.json` and `keys/1.json(.minisig)`.
3. Root secret keys stay offline (maintainer machine plus an offline
   backup) and never reach GitHub.
4. In `knittingcode/deckpilot-plugins`, the index key and its password
   become the `REGISTRY_INDEX_KEY(_PASSWORD)` secrets of the
   `registry-release` environment (tags `*--v*`), and the timestamp key the
   `REGISTRY_TIMESTAMP_KEY(_PASSWORD)` secrets of both `registry-release`
   and `registry-timestamp` (`main` only, the daily refresh). They are
   never secrets of `knittingcode/DeckPilot`.
5. `roots.json` becomes the DeckPilot repository variable
   `DECKPILOT_REGISTRY_ROOT_PUBKEYS` ([below](#embedding-the-roots-in-a-release-build)).

| Event (section 9.4) | Commands |
| --- | --- |
| Rotate the index or timestamp key | `keys generate` offline; `keyset rotate --index NEW.pub` (or `--timestamp`); `keyset sign` with a root; replace the environment secret; `build-index`, `sign-index` with the new key, `timestamp`; deploy key set, index and timestamp together |
| Index-key compromise | as above with `--seq-floor-reset` (`--seq-floor` the last legitimate seq); `lift-revocation` for revocations the attacker added; re-publish |
| Timestamp-key compromise | as above for `--timestamp`; impact was limited to freezing |
| Root rotation | `keyset rotate --root …`; `keyset sign` with the current roots and with the new roots |
| Root loss or compromise | app release with new embedded roots (not this tool): replace `DECKPILOT_REGISTRY_ROOT_PUBKEYS` and release |
| Withdraw the first-party authorization | `keyset rotate --no-first-party`; `keyset sign` with a root; `revoke` (or `yank`) the offered `deckpilot.*` versions; `build-index`, `sign-index`, `timestamp`; deploy together |

## Embedding the roots in a release build

The agent trusts the registry through a root role compiled into it:
`DECKPILOT_REGISTRY_ROOT_PUBKEYS` at **build time**, read by this crate's
`build.rs` (`src/roots.rs`, `roots::EMBEDDED`). Its value is the content
of `roots.json` exactly as `keyset init` writes it and
`knittingcode/deckpilot-plugins` commits it as `keys/roots.json`
(`{"keys": [...], "threshold": n}`), or the absolute path of that file.
The build checks it with the client's rules and fails on a malformed
value (bad JSON, unknown field, 0 or more than 4 keys, a key that is not
a minisign public key, a duplicate, a threshold of 0, above 4 or above
the number of keys, an unreadable or relative path): never partial
trust. Unset or empty, nothing is embedded and the agent uses no
registry unless it is given `--plugin-registry-roots`.

After the ceremony the owner:

1. Checks the committed file locally:
   `DECKPILOT_REGISTRY_ROOT_PUBKEYS="$(cat keys/roots.json)" cargo build --release -p deckpilot-agent --features plugins`.
2. Creates the **repository variable** (not a secret; the keys are
   public) `DECKPILOT_REGISTRY_ROOT_PUBKEYS` in `knittingcode/deckpilot`
   with that content: `gh variable set DECKPILOT_REGISTRY_ROOT_PUBKEYS
   --repo knittingcode/deckpilot < keys/roots.json`.
3. Releases. `release.yml` passes the variable to the agent build only
   when it is set; nothing else changes.

A release build with embedded roots ignores `--plugin-registry-roots`
and `DECKPILOT_PLUGIN_REGISTRY_ROOTS` (they override embedded roots in
debug and `test-hooks` builds only); a release build without embedded
roots still accepts them, for a self-hosted registry. The embedded roots
are the anchor of the key set chain, so index, timestamp and root
rotations signed as in the table above need no new app build. The
precedence rules and their tests:
[`docs/marketplace/agent-integration.md`](../../docs/marketplace/agent-integration.md#trusted-root-keys).

## Intended GitHub Actions flow (separate registry repository)

The design is section 11. The real workflows live in the registry
repository,
[`knittingcode/deckpilot-plugins`](https://github.com/knittingcode/deckpilot-plugins/tree/main/.github/workflows)
(`validate.yml`, `publish.yml`, `refresh-timestamp.yml`); they replace
the spike's `publish-workflow.yml.example` sketch, and where they differ
from the plan below, they are authoritative. Its environments and keys
wait on the owner's approval of the trust configuration. The tool is
built in each job from a pinned DeckPilot commit with `cargo build -p
deckpilot-plugin-registry --features publish --locked`.

### `validate.yml`: untrusted pull requests, no secrets

`pull_request` only (never `pull_request_target`), `permissions: {}`, no
environment, no secrets. Tooling from the base SHA, PR data read as data.
In a scratch copy of `main`'s `registry/` it replays the PR: the
reproducible build, then `add-version` with the built package (which
verifies it), then `build-index`. To run the client end to end without
any secret, it signs that scratch registry with throwaway keys made in
the job (`keys generate --no-password`, a fresh `keyset init`), runs
`sign-index`, `timestamp` and `verify-site --require-packages`, and
discards the keys and the output. The job summary shows the permission
diff and the computed sha256 and size. Its result is advisory (a PR can
edit the workflow); the `gate` job on `main` repeats it.

### `publish.yml`: release tags, protected environments

Runs for a release tag the owner creates (`<plugin-id>--v<version>`, or
`registry--v…` to re-sign the index only); tags are admin-only by ruleset.

| Job | Environment | Secrets | Tool |
| --- | --- | --- | --- |
| `resolve` | — | none | the tag names a reviewed commit on `main` and a recorded version |
| `build` (untrusted, `contents: read`) | — | none | rebuilds every recorded package from its pinned source; plugin build scripts run only here |
| `sign` | `registry-release` | index and timestamp keys | builds nothing; `tools/publish-site.sh index` re-checks each package's sha256 and size against the reviewed pins, signs the index and the timestamp, and runs `verify-site` as an existing client |
| `release` (`contents: write`) | — | none | uploads the verified package to the GitHub Release of the tag (never creates a tag) |
| `deploy` | `github-pages` | none | the landing page plus the signed `site/v1` |
| `verify-published` | — | none | fetch the live site anonymously, `verify-site` on it |

Signing happens only in the protected environments; their secrets are
released only to jobs on `main` after review, never to pull requests or
forks. No job that runs author code has a secret, an environment or
`id-token: write`.

With storefronts enabled (`REGISTRY_STOREFRONT_ENABLED`), the `sign` job
runs `storefront build` and `storefront sign` with the index key between
`sign-index` and `timestamp`, and `verify-site` adds
`--require-storefront`; an editorial-only change is published with a
`registry--v…` tag (a new seq with the same plugins). Off, the site is
exactly what it was before storefronts.

### `timestamp.yml`: daily

`schedule` plus `workflow_dispatch`, same concurrency group as
`publish.yml`: mirror the live site, `verify-site` (state of an existing
client), `timestamp --expires-in 7d` in `registry-timestamp`, `verify-site
--state … --min-remaining 6d`, deploy, `verify-published`. It never needs
the index key: the timestamp alone keeps clients fresh (this crate's
decision 1), and `index/<seq>.json` stays byte-for-byte the same. The
mirror must keep `storefront/` and `media/` byte for byte as well, or the
first refresh after a storefront publish would delete them.

## Not covered

- Fetching or deploying anything (mirroring the live site, GitHub
  Releases, Pages), `check-live`, `registry-keys.yml`.
- Validation of a PR's `registry/plugins/<id>/versions/<v>.json`
  submission against the built package: here the version record is
  derived from the package, so a submission file would be compared with
  `add-version`'s result by the workflow.
- Source archives (`source.archive`) and attestations are recorded only if
  given; nothing creates them.
- Publisher verification (`verified` is set by the maintainer).
