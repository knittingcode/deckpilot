#!/usr/bin/env bash
# Builds the first-party plugins the application bundles and stages them as
# app components, with the pins the agent is compiled with
# (docs/marketplace/module-migration.md §4.4, §4.5).
#
#   plugins/first-party/build.sh --out DIR [--test-fixtures] [--target-dir DIR]
#       [--artifacts-dir DIR] [--offline | --fetch-only] [--registry-tool FILE]
#       [--app-version V] [--plugin-api MAJOR.MINOR] [--platform macos|linux|windows]
#
# Writes DIR/bundled/<id>/manifest.json, plugin.wasm, LICENSE, the icons,
# the visual assets (contributes.visualAssets, PNG or JPEG under assets/)
# and the locale files (locales/<tag>.json) the manifest names, and
# DIR/bundled/pins.json: for every plugin its id,
# version, class and the size and sha256 of every file. The release build
# compiles the agent with DECKPILOT_BUNDLED_PINS=DIR/bundled/pins.json and
# ships DIR/bundled as a Tauri resource (tauri.plugins.conf.json); the
# agent trusts the files only if they match the pins compiled into it.
#
# The plugins are those of plugins/first-party/bundled.json.
# --test-fixtures adds plugins/first-party/test-fixtures/bundled.json (the
# test plugin deckpilot.example-default), for tests and dry runs only:
# a release bundle never contains a test fixture.
#
# Each entry is built from its source here ({"path": "<dir>", "class"},
# "source": "source", the default) or taken from a signed release artifact
# of knittingcode/deckpilot-plugins ({"id", "version", "class", "source":
# "artifact"}), never both. For an artifact the reviewed entry of
# artifacts.json (id, version, url, sha256, size, keyIds) is the claim:
# the package and one <package>.<KEYID>.minisig per key id are taken from
# --artifacts-dir (default <target dir>/first-party-artifacts) or, when
# missing there, downloaded from exactly that immutable release URL; size
# and sha256 are checked first; then `deckpilot-registry release verify`
# with the PINNED publisher keys, released-pins.json (no downgrade, the same
# version only with the same files), artifacts.json's revocations, the
# audit log DIR/artifacts-audit.jsonl, and --unpack into DIR/bundled/<id>/;
# every key id must have counted. Any mismatch refuses the build. No network
# is used for source entries, or at all with --offline (every file must be
# in --artifacts-dir). --fetch-only downloads into --artifacts-dir, checks
# size and sha256 and stops (pre-staging a cache for an offline build).
# --registry-tool names a deckpilot-registry binary (default: built with
# cargo, feature publish, only when an artifact is listed). The host the
# artifact is checked for: --app-version (default: the workspace version),
# --plugin-api (default 1.9, the agent's), --platform (default: this one).
#
# Reproducible on one platform, like plugins/examples/build.sh: the pinned
# toolchain (rust-toolchain.toml), the locked plugins/runtime workspace,
# the `guest` profile, remapped source and Cargo paths, and no flags from
# the calling environment. Independent of where the checkout is: the build
# runs in a copy of the workspace rooted at the plugins directory (below),
# so the same commit builds the same files on every machine of a platform.
# An artifact is unpacked byte for byte, so the same lock gives the same
# files and pins. No package (.dpplugin) and no signature is staged:
# bundled plugins are trusted by the pins, never by a key.
#
# Tests only: --test-only-publisher-keys FILE verifies artifacts against
# the throwaway keys in FILE instead of the pinned ones (`release verify
# --keys`). It needs DECKPILOT_TEST_ONLY_PUBLISHER_KEYS=1 and leaves
# DIR/bundled/NOT-FOR-RELEASE, which check-bundled-plugins.sh refuses, so
# such a bundle can never ship.
#
# Needs cargo (with the wasm32-unknown-unknown target) and jq; curl to
# download an artifact.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
plugins=$(dirname "$here")
runtime="$plugins/runtime"
target_dir="$runtime/target"
out=""
fixtures=false
artifacts_dir=""
offline=false
fetch_only=false
registry_tool=""
app_version=""
# The plugin API the agent hosts (PLUGIN_API, apps/agent/src/plugins/service.rs).
plugin_api="1.9"
platform=""
test_keys=""

while [ $# -gt 0 ]; do
  case "$1" in
    --out) out=$2; shift 2 ;;
    --target-dir) target_dir=$2; shift 2 ;;
    --test-fixtures) fixtures=true; shift ;;
    --artifacts-dir) artifacts_dir=$2; shift 2 ;;
    --offline) offline=true; shift ;;
    --fetch-only) fetch_only=true; shift ;;
    --registry-tool) registry_tool=$2; shift 2 ;;
    --app-version) app_version=$2; shift 2 ;;
    --plugin-api) plugin_api=$2; shift 2 ;;
    --platform) platform=$2; shift 2 ;;
    --test-only-publisher-keys) test_keys=$2; shift 2 ;;
    -h | --help) sed -n '2,64p' "$0"; exit 0 ;;
    *) echo "build.sh: unknown argument $1" >&2; exit 2 ;;
  esac
done
[ -n "$out" ] || [ "$fetch_only" = true ] || { echo "build.sh: --out DIR is required" >&2; exit 2; }
[ "$offline" = false ] || [ "$fetch_only" = false ] || { echo "build.sh: --offline and --fetch-only exclude each other" >&2; exit 2; }
[ "$offline" = false ] || [ -n "$artifacts_dir" ] || { echo "build.sh: --offline needs --artifacts-dir DIR" >&2; exit 2; }
[ "$fetch_only" = false ] || [ -n "$artifacts_dir" ] || { echo "build.sh: --fetch-only needs --artifacts-dir DIR" >&2; exit 2; }
if [ -n "$test_keys" ] && [ "${DECKPILOT_TEST_ONLY_PUBLISHER_KEYS:-}" != 1 ]; then
  echo "build.sh: --test-only-publisher-keys is for tests only and needs DECKPILOT_TEST_ONLY_PUBLISHER_KEYS=1; a release verifies against the pinned keys" >&2
  exit 2
fi
# Cargo runs in plugins/runtime: a relative target directory is the caller's.
mkdir -p "$target_dir"
target_dir=$(cd "$target_dir" && pwd)
fail() { echo "build.sh: $*" >&2; exit 1; }
command -v jq > /dev/null || fail "jq is required"
# jq on Windows ends its lines with CRLF; every value here is one line.
jq() { command jq "$@" | tr -d '\r'; }

# A build that ends with status 0 has staged every listed plugin and
# written their pins (checked against the lists at the end). Ending with
# status 0 before that, on any path, is a failure here: a dry run on
# Windows once ended after the first artifact with status 0, no error and
# no pins.json, and only a later diff noticed. An incomplete bundle is
# never left to pass for a smaller one. (--fetch-only stops early by design
# and stages nothing.)
complete=false
on_exit() {
  local status=$?
  if [ "$status" = 0 ] && [ "$fetch_only" = false ] && [ "$complete" = false ]; then
    rm -f "${bundled:-$out/bundled}/pins.json"
    echo "build.sh: ended before every listed plugin was staged and pinned; ${bundled:-$out/bundled} is incomplete" >&2
    exit 1
  fi
}
trap on_exit EXIT

if command -v sha256sum > /dev/null; then
  sha() { sha256sum < "$1" | cut -d' ' -f1; }
else
  sha() { shasum -a 256 < "$1" | cut -d' ' -f1; }
fi
size() { wc -c < "$1" | tr -d ' '; }

# The plugin lists: "<directory relative to plugins/first-party> <class>"
# for a source build, "<id> <version> <class>" for a release artifact.
lists=("$here/bundled.json")
[ "$fixtures" = false ] || lists+=("$here/test-fixtures/bundled.json")
lock="$here/artifacts.json"
[ -f "$lock" ] || fail "$lock is missing"
jq -e '.artifactsVersion == 1 and (.plugins | type == "array") and (.revoked | type == "array")' "$lock" > /dev/null \
  || fail "$lock is not artifacts format 1"
entries=()
artifacts=()
listed_ids=()
for list in "${lists[@]}"; do
  base=$(dirname "$list")
  # Fields joined by U+001F: a tab-separated empty field would be dropped.
  while IFS=$'\x1f' read -r source path id version class; do
    case "$class" in default | migration) ;; *) fail "$list: ${path:-$id} has class '$class' (default or migration)" ;; esac
    case "$source" in
      source)
        [ -n "$path" ] && [ -z "$id" ] && [ -z "$version" ] \
          || fail "$list: a source entry names its directory (path) and nothing else: ${path:-$id}"
        rel=${base#"$here"}
        rel=${rel#/}
        entries+=("${rel:+$rel/}$path"$'\t'"$class")
        ;;
      artifact)
        [ -z "$path" ] && [ -n "$id" ] && [ -n "$version" ] \
          || fail "$list: an artifact entry names id and version and no path: ${path:-$id}"
        artifacts+=("$id"$'\t'"$version"$'\t'"$class")
        listed_ids+=("$id")
        ;;
      *) fail "$list: ${path:-$id} has source '$source' (source or artifact)" ;;
    esac
  done < <(jq -r '.plugins[] | [(.source // "source"), (.path // ""), (.id // ""), (.version // ""), .class] | join("\u001f")' "$list")
done

# Never both: every locked artifact is listed as one, no artifact twice,
# and no source build of a locked id (checked again by id below, once the
# manifests are read).
while IFS=$'\t' read -r id version; do
  found=false
  for entry in ${artifacts[@]+"${artifacts[@]}"}; do
    IFS=$'\t' read -r aid aversion _ <<< "$entry"
    [ "$aid" = "$id" ] || continue
    [ "$aversion" = "$version" ] \
      || fail "artifacts.json locks $id $version, bundled.json lists $id $aversion: they must agree"
    found=true
  done
  [ "$found" = true ] || fail "artifacts.json locks $id $version, which bundled.json does not list with \"source\": \"artifact\""
done < <(jq -r '.plugins[] | [.id, .version] | @tsv' "$lock")
duplicates=$(printf '%s\n' ${listed_ids[@]+"${listed_ids[@]}"} | LC_ALL=C sort | uniq -d)
[ -z "$duplicates" ] || fail "$duplicates is listed twice"

if [ ${#artifacts[@]} -eq 0 ] && [ "$fetch_only" = true ]; then
  echo "build.sh: no plugin is taken from a release artifact; nothing to fetch"
  exit 0
fi

if [ "$fetch_only" = false ]; then
  bundled="$out/bundled"
  rm -rf "$bundled"
  mkdir -p "$bundled"
fi

crates=()
source_ids=()
for entry in ${entries[@]+"${entries[@]}"}; do
  dir="$here/${entry%%$'\t'*}"
  [ -f "$dir/Cargo.toml" ] || fail "$dir has no Cargo.toml"
  crates+=("$(sed -n 's/^name = "\(.*\)"$/\1/p' "$dir/Cargo.toml" | head -n 1)")
  # Never both, before anything is downloaded or built.
  id=$(jq -r .id "$dir/manifest.json")
  source_ids+=("$id")
  [ "$(jq --arg id "$id" '[.plugins[] | select(.id == $id)] | length' "$lock")" = 0 ] \
    || fail "$id is built from $dir and locked as a release artifact in artifacts.json: a plugin is one or the other"
  for listed in ${listed_ids[@]+"${listed_ids[@]}"}; do
    [ "$listed" != "$id" ] || fail "$id is listed as a release artifact and built from $dir: a plugin is one or the other"
  done
done

if [ ${#artifacts[@]} -gt 0 ]; then
  # shellcheck source=plugins/first-party/artifacts.sh
  source "$here/artifacts.sh"
  stage_artifacts
fi
[ "$fetch_only" = false ] || exit 0

if [ ${#crates[@]} -gt 0 ]; then
  # Cargo hashes a path package into its -C metadata (and so into symbol
  # hashes, type ids and the code layout of the module) by its path
  # relative to the workspace root, but by its ABSOLUTE path when it lies
  # outside that root, as plugins/first-party and plugins/examples do
  # (members "../first-party/..." of plugins/runtime). A checkout at
  # another path then builds other plugin.wasm files for the same
  # sources. So the build runs in a copy of the workspace whose root is
  # the plugins directory itself: runtime/, first-party/, examples/ and
  # schemas/ under one Cargo.toml (plugins/runtime's, with its paths
  # rebased), the same Cargo.lock and the pinned toolchain. Every member
  # is then inside the root and nothing depends on where the checkout is.
  stage="$target_dir/first-party-source"
  rm -rf "$stage"
  mkdir -p "$stage/plugins/runtime"
  cp "$(dirname "$plugins")/rust-toolchain.toml" "$stage/"
  cp -R "$runtime/crates" "$runtime/guest" "$stage/plugins/runtime/"
  cp -R "$plugins/first-party" "$plugins/examples" "$plugins/schemas" "$stage/plugins/"
  cp "$runtime/Cargo.lock" "$stage/plugins/Cargo.lock"
  # Member and dependency paths: "../x" becomes "x", "x" becomes "runtime/x".
  sed -e 's#^\( *\)"\.\./\([^"]*\)",#\1"\2",#' \
    -e 't' \
    -e 's#^\( *\)"\([^"]*\)",#\1"runtime/\2",#' \
    -e 's#path = "\([^"]*\)"#path = "runtime/\1"#' \
    "$runtime/Cargo.toml" | tr -d '\r' > "$stage/plugins/Cargo.toml"
  # The members name their workspace (`workspace = "../../runtime"`):
  # now the plugins directory.
  find "$stage/plugins/first-party" "$stage/plugins/examples" -name Cargo.toml -print0 \
    | while IFS= read -r -d '' manifest; do
      sed -e 's#^workspace = "\(.*\)/runtime"#workspace = "\1"#' "$manifest" | tr -d '\r' > "$manifest.tmp"
      mv "$manifest.tmp" "$manifest"
    done
  cargo_home=${CARGO_HOME:-$HOME/.cargo}
  native_stage=$stage/plugins
  if command -v cygpath > /dev/null; then
    native_stage=$(cygpath -w "$stage/plugins")
    cargo_home=$(cygpath -w "$cargo_home")
    target_dir=$(cygpath -u "$target_dir")
  fi
  sep=$'\x1f'
  flags="--remap-path-prefix=$native_stage=/deckpilot/plugins${sep}--remap-path-prefix=$cargo_home=/cargo"
  args=()
  for crate in "${crates[@]}"; do args+=(--package "$crate"); done
  (
    cd "$stage/plugins"
    env -u RUSTFLAGS -u RUSTC_WRAPPER -u RUSTC_WORKSPACE_WRAPPER -u CARGO_BUILD_TARGET \
      CARGO_ENCODED_RUSTFLAGS="$flags" \
      cargo build --locked "${args[@]}" \
      --target wasm32-unknown-unknown --profile guest --target-dir "$target_dir"
  )
fi

pins='[]'
# pin <id> <version> <class>: the size and sha256 of every file of
# bundled/<id>/, added to the pins.
pin() {
  local id=$1 version=$2 class=$3 target="$bundled/$1" files='{}' file
  while IFS= read -r file; do
    files=$(jq --arg f "$file" --arg s "$(sha "$target/$file")" --argjson n "$(size "$target/$file")" \
      '. + {($f): {sha256: $s, size: $n}}' <<< "$files")
  done < <(cd "$target" && find . -type f | sed 's|^\./||' | LC_ALL=C sort)
  pins=$(jq --arg id "$id" --arg v "$version" --arg c "$class" --argjson f "$files" \
    '. + [{id: $id, version: $v, class: $c, files: $f}]' <<< "$pins")
}
for entry in ${artifacts[@]+"${artifacts[@]}"}; do
  IFS=$'\t' read -r id version class <<< "$entry"
  pin "$id" "$version" "$class"
  printf '%s  %s %s (%s, release artifact)\n' "$(sha "$bundled/$id/plugin.wasm")" "$id" "$version" "$class"
done
index=0
for entry in ${entries[@]+"${entries[@]}"}; do
  dir="$here/${entry%%$'\t'*}"
  class="${entry##*$'\t'}"
  crate="${crates[$index]}"
  index=$((index + 1))
  manifest="$dir/manifest.json"
  id=$(jq -r .id "$manifest")
  version=$(jq -r .version "$manifest")
  case "$id" in deckpilot.*) ;; *) fail "$manifest: $id is not a first-party id (deckpilot.*)" ;; esac
  [ "$(jq 'has("assets")' "$manifest")" = false ] \
    || fail "$manifest: a bundled plugin has no assets but its icons"
  target="$bundled/$id"
  [ ! -e "$target" ] || fail "$id is listed twice"
  mkdir -p "$target"
  cp "$manifest" "$target/manifest.json"
  cp "$dir/LICENSE" "$target/LICENSE"
  cp "$target_dir/wasm32-unknown-unknown/guest/${crate//-/_}.wasm" "$target/plugin.wasm"
  while IFS= read -r icon; do
    case "$icon" in assets/*.png) ;; *) fail "$manifest: icon $icon is not assets/<name>.png" ;; esac
    mkdir -p "$target/assets"
    cp "$dir/$icon" "$target/$icon"
  done < <(jq -r '.icons // {} | .[]' "$manifest")
  # Plugin API 1.8: the icons and image backgrounds the plugin adds to the
  # libraries, checked against their use when the agent seeds them.
  while IFS= read -r asset; do
    [ -n "$asset" ] || continue
    [[ "$asset" =~ ^assets/([A-Za-z0-9_][A-Za-z0-9._-]*/)*[A-Za-z0-9_][A-Za-z0-9._-]*\.(png|jpg|jpeg)$ ]] \
      || fail "$manifest: visual asset $asset is not a .png, .jpg or .jpeg file under assets/"
    mkdir -p "$(dirname "$target/$asset")"
    cp "$dir/$asset" "$target/$asset"
  done < <(jq -r '.contributes.visualAssets // {} | ((.icons // [])[].asset), ((.backgrounds // [])[] | .asset // empty)' "$manifest")
  # Plugin API 1.6: the translations the manifest declares, pinned below
  # like every other file (docs/marketplace/sdk/localization.md).
  while IFS= read -r tag; do
    [[ "$tag" =~ ^[a-z]{2,3}(-[A-Z][a-z]{3})?(-([A-Z]{2}|[0-9]{3}))?$ ]] \
      || fail "$manifest: locale $tag is not a canonical tag"
    mkdir -p "$target/locales"
    cp "$dir/locales/$tag.json" "$target/locales/$tag.json"
  done < <(jq -r '.locales // [] | .[]' "$manifest")

  pin "$id" "$version" "$class"
  printf '%s  %s %s (%s)\n' "$(sha "$target/plugin.wasm")" "$id" "$version" "$class"
done

jq -S -n --argjson p "$pins" '{pinsVersion: 1, plugins: ($p | sort_by(.id))}' > "$bundled/pins.json"
# Complete or nothing: the pins name exactly the listed plugins, each staged
# with its module, and no other plugin directory is there.
want=$(printf '%s\n' ${listed_ids[@]+"${listed_ids[@]}"} ${source_ids[@]+"${source_ids[@]}"} | LC_ALL=C sort)
pinned=$(jq -r '.plugins[].id' "$bundled/pins.json" | LC_ALL=C sort)
staged=$(cd "$bundled" && find . -mindepth 1 -maxdepth 1 -type d | sed 's|^\./||' | LC_ALL=C sort)
[ "$pinned" = "$want" ] && [ "$staged" = "$want" ] || {
  rm -f "$bundled/pins.json"
  fail "the bundle is incomplete: listed $(tr '\n' ' ' <<< "$want"); pinned $(tr '\n' ' ' <<< "$pinned"); staged $(tr '\n' ' ' <<< "$staged")"
}
for id in ${want}; do
  [ -s "$bundled/$id/plugin.wasm" ] && [ -s "$bundled/$id/manifest.json" ] \
    || { rm -f "$bundled/pins.json"; fail "$id is pinned without its plugin.wasm and manifest.json"; }
done
echo "wrote $bundled/pins.json ($(jq '.plugins | length' "$bundled/pins.json") plugins)"
if [ -n "$test_keys" ]; then
  # Verified with throwaway keys: never shippable (check-bundled-plugins.sh
  # refuses anything but pins.json and the plugins at the top level).
  echo "verified with the test-only publisher keys $test_keys, not the pinned keys: NOT FOR RELEASE" > "$bundled/NOT-FOR-RELEASE"
  echo "build.sh: test-only publisher keys: $bundled is not for release" >&2
fi
complete=true
