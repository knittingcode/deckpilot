#!/usr/bin/env bash
# Tests plugins/first-party/build.sh's release-artifact path (artifacts.sh)
# in a copy of the plugins tree, with throwaway publisher keys generated
# here (never a production key) through the test-only keys hook
# (--test-only-publisher-keys, DECKPILOT_TEST_ONLY_PUBLISHER_KEYS=1), and a
# curl stand-in that serves the fixture release from a directory and
# records every URL asked for (nothing reaches the network).
#
#   plugins/first-party/test-build.sh <work dir> <deckpilot-registry>
#
# <deckpilot-registry>: the registry tool, built with
#   cargo build --locked -p deckpilot-plugin-registry --features publish --bin deckpilot-registry
#
# The fixture artifact is the test plugin deckpilot.example-default, built
# from source here first (so the cases also run a source build), packaged
# and signed by two throwaway keys. Cases: a valid two-signature artifact
# staged and pinned (offline cache, download, fetch-only then offline,
# reproducible); refusals for tampered bytes, one signature, a wrong key,
# a revoked key, a downgrade below released-pins.json and the same version
# with other files, a wrong id or version, a missing file, a mutable or
# foreign URL, the pinned keys (which are not the throwaway keys), the keys
# hook without its variable; never both a source and an artifact; and a
# mixed source and artifact build.
#
# Needs cargo with the wasm32-unknown-unknown target, jq and bash.
set -euo pipefail

work="${1:?usage: test-build.sh <work dir> <deckpilot-registry>}"
tool="${2:?usage: test-build.sh <work dir> <deckpilot-registry>}"
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
mkdir -p "$work"
work=$(cd "$work" && pwd)
tool=$(cd "$(dirname "$tool")" && pwd)/$(basename "$tool")
[ -x "$tool" ] || { echo "test-build.sh: $tool is not executable" >&2; exit 2; }
jq() { command jq "$@" | tr -d '\r'; }
if command -v sha256sum > /dev/null; then
  sha() { sha256sum < "$1" | cut -d' ' -f1; }
else
  sha() { shasum -a 256 < "$1" | cut -d' ' -f1; }
fi
size() { wc -c < "$1" | tr -d ' '; }

passed=0
pass() { passed=$((passed + 1)); echo "ok: $*"; }
bad() { echo "FAIL: $*" >&2; exit 1; }

case "$(uname -s)" in
  Darwin) platform=macos ;;
  Linux) platform=linux ;;
  *) platform=windows ;;
esac

# The plugins tree (and what build.sh reads beside it), without build
# outputs: build.sh runs from the copy, so the copy's lists are the inputs.
repo="$work/repo"
rm -rf "$repo"
mkdir -p "$repo"
(cd "$root" && tar --exclude=target -cf - rust-toolchain.toml Cargo.toml plugins) | (cd "$repo" && tar -xf -)
fp="$repo/plugins/first-party"
build="$fp/build.sh"
target="$work/target"
cp "$fp/released-pins.json" "$work/released-pins.orig.json"

# The curl stand-in: serves <prefix><tag>/<name> from $work/release/<tag>/<name>.
mkdir -p "$work/bin" "$work/release"
cat > "$work/bin/curl" << 'STUB'
#!/usr/bin/env bash
out="" url=""
while [ $# -gt 0 ]; do
  case "$1" in
    --output | -o) out=$2; shift 2 ;;
    --proto | --proto-redir | --max-filesize | --retry | --retry-delay) shift 2 ;;
    -*) shift ;;
    *) url=$1; shift ;;
  esac
done
echo "$url" >> "$STUB_DIR/curl.log"
prefix="https://github.com/knittingcode/deckpilot-plugins/releases/download/"
rest=${url#"$prefix"}
if [ "$rest" = "$url" ] || [ ! -f "$STUB_DIR/release/$rest" ]; then
  echo "curl: (22) The requested URL returned error: 404" >&2
  exit 22
fi
cp "$STUB_DIR/release/$rest" "$out"
STUB
chmod 755 "$work/bin/curl"
export STUB_DIR="$work"
export PATH="$work/bin:$PATH"
calls() { if [ -f "$work/curl.log" ]; then cat "$work/curl.log"; fi; }

# lists <bundled.json plugins> <fixtures plugins> <artifacts.json plugins> [revoked]
lists() {
  jq -n --argjson p "$1" '{plugins: $p}' > "$fp/bundled.json"
  jq -n --argjson p "$2" '{plugins: $p}' > "$fp/test-fixtures/bundled.json"
  jq -n --argjson p "$3" --argjson r "${4:-[]}" '{artifactsVersion: 1, plugins: $p, revoked: $r}' > "$fp/artifacts.json"
}

# run <name> <build.sh arguments...>: build.sh into $work/out-<name>, with
# the throwaway keys unless the arguments say otherwise; output in
# $work/<name>.log; returns build.sh's status.
run() {
  local name=$1
  shift
  rm -rf "$work/out-$name" "$work/curl.log"
  DECKPILOT_TEST_ONLY_PUBLISHER_KEYS=1 "$build" --out "$work/out-$name" --target-dir "$target" \
    --registry-tool "$tool" "$@" > "$work/$name.log" 2>&1
}
# refused <name> <expected text> <build.sh arguments...>
refused() {
  local name=$1 expected=$2
  shift 2
  if run "$name" "$@"; then
    cat "$work/$name.log" >&2
    bad "$name: build.sh accepted it"
  fi
  grep -F -q -- "$expected" "$work/$name.log" || { cat "$work/$name.log" >&2; bad "$name: no \"$expected\" in the output"; }
  [ ! -e "$work/out-$name/bundled/pins.json" ] || bad "$name: pins.json was written"
  pass "$name refused ($expected)"
}

# ---------------------------------------------------------------- fixtures

# A source build of the fixtures (no network, no registry tool needed).
lists '[]' '[{"path": "example-default", "class": "default"}, {"path": "legacy-fixture", "class": "migration"}]' '[]'
rm -f "$work/curl.log"
"$build" --test-fixtures --out "$work/source" --target-dir "$target" > "$work/source.log" 2>&1 \
  || { cat "$work/source.log" >&2; bad "the source build failed"; }
[ -z "$(calls)" ] || bad "a source build asked curl for $(calls)"
[ ! -e "$target/registry-tool" ] || bad "a source build built the registry tool"
pass "source entries build without any download"

id=deckpilot.example-default
version=$(jq -r .version "$work/source/bundled/$id/manifest.json")
package="$id-$version.dpplugin"
tag="$id--v$version"
url="https://github.com/knittingcode/deckpilot-plugins/releases/download/$tag/$package"

# The release: the source build's files as a package, signed by A and B;
# C is a key no keys file pins.
keys="$work/keys"
rm -rf "$keys" "$work/release"
mkdir -p "$keys" "$work/release/$tag"
for k in a b c; do
  "$tool" keys generate --role publisher --out "$keys/$k.key" --no-password > /dev/null
done
"$tool" package --plugin-dir "$work/source/bundled/$id" --module "$work/source/bundled/$id/plugin.wasm" \
  --out "$work/release/$tag/$package" --first-party > /dev/null
key_id() {
  local dir="$work/sig-$1"
  rm -rf "$dir" && mkdir -p "$dir"
  "$tool" release sign --package "$work/release/$tag/$package" --id "$id" --version "$version" \
    --key "$keys/$1.key" --out-dir "$dir" > /dev/null
  local file
  file=$(cd "$dir" && ls)
  file=${file#"$package."}
  echo "${file%.minisig}"
}
A=$(key_id a)
B=$(key_id b)
C=$(key_id c)
cp "$work/sig-a/$package.$A.minisig" "$work/sig-b/$package.$B.minisig" "$work/release/$tag/"
pub() { tr -d '\r\n' < "$keys/$1.key.pub"; }
jq -n --arg a "$(pub a)" --arg b "$(pub b)" \
  '{publisherKeysVersion: 1, publisher: "deckpilot", keys: [$a, $b], threshold: 2, revokedKeys: []}' > "$keys/two.json"
jq -n --arg a "$(pub a)" --arg b "$(pub b)" \
  '{publisherKeysVersion: 1, publisher: "deckpilot", keys: [$a], threshold: 1, revokedKeys: [$b]}' > "$keys/b-revoked.json"
sha256=$(sha "$work/release/$tag/$package")
bytes=$(size "$work/release/$tag/$package")

# entry [field value-json]...: the lock entry, with fields replaced.
entry() {
  local e
  e=$(jq -n --arg id "$id" --arg v "$version" --arg u "$url" --arg s "$sha256" --argjson n "$bytes" \
    --arg a "$A" --arg b "$B" '{id: $id, version: $v, url: $u, sha256: $s, size: $n, keyIds: [$a, $b]}')
  while [ $# -gt 0 ]; do
    e=$(jq --arg f "$1" --argjson v "$2" '.[$f] = $v' <<< "$e")
    shift 2
  done
  echo "[$e]"
}
listed() { jq -n --arg id "$id" --arg v "${1:-$version}" '[{id: $id, version: $v, class: "default", source: "artifact"}]'; }
# cache <dir> <files...>: a pre-downloaded directory with these release files.
cache() {
  local dir="$work/$1"
  shift
  rm -rf "$dir" && mkdir -p "$dir"
  for file in "$@"; do cp "$work/release/$tag/$file" "$dir/"; done
  echo "$dir"
}
two_keys=(--test-only-publisher-keys "$keys/two.json")

# ------------------------------------------------------------ accepted

lists "$(listed)" '[]' "$(entry)"
full=$(cache cache-full "$package" "$package.$A.minisig" "$package.$B.minisig")
run offline --artifacts-dir "$full" --offline "${two_keys[@]}" || { cat "$work/offline.log" >&2; bad "the valid artifact was refused"; }
[ -z "$(calls)" ] || bad "--offline asked curl for $(calls)"
out="$work/out-offline/bundled"
diff -r "$work/source/bundled/$id" "$out/$id" > /dev/null || bad "the unpacked files are not the released files"
[ "$(jq -c --arg id "$id" '.plugins[] | select(.id == $id)' "$out/pins.json")" \
  = "$(jq -c --arg id "$id" '.plugins[] | select(.id == $id)' "$work/source/bundled/pins.json")" ] \
  || bad "the artifact's pins are not those of its files"
[ "$(jq '.plugins | length' "$out/pins.json")" = 1 ] || bad "pins.json lists more than the artifact"
grep -F -q "counted" "$work/offline.log" || bad "release verify listed no counted key"
grep -F -q "not the pinned keys" "$work/offline.log" || bad "the test keys were not reported as such"
grep -F -x -q "  privileged: none" "$work/offline.log" || bad "release verify printed no privileged-kinds line"
[ "$(wc -l < "$work/out-offline/artifacts-audit.jsonl" | tr -d ' ')" = 1 ] || bad "not one audit record"
grep -F -q '"outcome":"verified"' "$work/out-offline/artifacts-audit.jsonl" || bad "no accepted audit record"
[ -f "$out/NOT-FOR-RELEASE" ] || bad "a bundle verified with test keys is not marked"
if "$root/tools/release-pipeline/check-bundled-plugins.sh" "$out" "$work/no-agent" > "$work/check.log" 2>&1; then
  bad "check-bundled-plugins.sh accepted a bundle verified with test keys"
fi
grep -F -q "NOT-FOR-RELEASE" "$work/check.log" || { cat "$work/check.log" >&2; bad "check-bundled-plugins.sh did not name NOT-FOR-RELEASE"; }
[ -z "$(find "$work/out-offline" -name '*.minisig' -o -name '*.dpplugin')" ] || bad "a package or signature was staged"
pass "a valid two-signature artifact from the offline cache is staged and pinned, and marked not for release"

run offline-again --artifacts-dir "$full" --offline "${two_keys[@]}" || bad "the second offline build failed"
diff -r "$out" "$work/out-offline-again/bundled" > /dev/null || bad "the same inputs gave other files or pins"
pass "the same lock and cache give the same bundled bytes and pins"

download=$(cache cache-download)
run download --artifacts-dir "$download" "${two_keys[@]}" || { cat "$work/download.log" >&2; bad "the download build failed"; }
[ "$(calls | LC_ALL=C sort)" = "$(printf '%s\n' "$url" "$url.$A.minisig" "$url.$B.minisig" | LC_ALL=C sort)" ] \
  || bad "the download asked for $(calls), not exactly the package and its two signature files"
diff -r "$out" "$work/out-download/bundled" > /dev/null || bad "the downloaded artifact staged other files"
pass "a download fetches exactly the immutable package URL and one signature file per key id"

fetched=$(cache cache-fetched)
rm -f "$work/curl.log"
DECKPILOT_TEST_ONLY_PUBLISHER_KEYS=1 "$build" --fetch-only --artifacts-dir "$fetched" > "$work/fetch.log" 2>&1 \
  || { cat "$work/fetch.log" >&2; bad "--fetch-only failed"; }
[ -f "$fetched/$package" ] && [ -f "$fetched/$package.$A.minisig" ] && [ -f "$fetched/$package.$B.minisig" ] \
  || bad "--fetch-only did not fill the cache"
run fetched --artifacts-dir "$fetched" --offline "${two_keys[@]}" || bad "the offline build from the fetched cache failed"
diff -r "$out" "$work/out-fetched/bundled" > /dev/null || bad "the fetched cache staged other files"
pass "--fetch-only pre-stages a cache that an --offline build verifies identically"

# ------------------------------------------------------------ refused

refused pinned-keys "refused" --artifacts-dir "$full" --offline
grep -F -q "the pinned keys" "$work/pinned-keys.log" || bad "the pinned-keys refusal does not name the pinned keys"
rm -rf "$work/out-unflagged"
if "$build" --out "$work/out-unflagged" --registry-tool "$tool" --artifacts-dir "$full" --offline \
  "${two_keys[@]}" > "$work/unflagged.log" 2>&1; then
  bad "the test keys hook ran without DECKPILOT_TEST_ONLY_PUBLISHER_KEYS=1"
fi
grep -F -q "tests only" "$work/unflagged.log" || bad "the unflagged hook was not refused as test-only"
pass "the throwaway keys are refused by the pinned keys, and the hook needs its variable"

tampered=$(cache cache-tampered "$package" "$package.$A.minisig" "$package.$B.minisig")
printf 'x' >> "$tampered/$package"
refused longer "bytes, artifacts.json says $bytes" --artifacts-dir "$tampered" --offline "${two_keys[@]}"
flipped=$(cache cache-flipped "$package" "$package.$A.minisig" "$package.$B.minisig")
head -c $((bytes - 1)) "$work/release/$tag/$package" > "$flipped/$package"
if [ "$(tail -c 1 "$work/release/$tag/$package" | od -An -tx1 | tr -d ' \n')" = 01 ]; then
  printf '\002' >> "$flipped/$package"
else
  printf '\001' >> "$flipped/$package"
fi
[ "$(size "$flipped/$package")" = "$bytes" ] || bad "the flipped package changed size"
refused tampered "artifacts.json says $sha256" --artifacts-dir "$flipped" --offline "${two_keys[@]}"
lists "$(listed)" '[]' "$(entry sha256 "\"$(sha "$flipped/$package")\"")"
refused tampered-claim "2 required" --artifacts-dir "$flipped" --offline "${two_keys[@]}"

lists "$(listed)" '[]' "$(entry keyIds "[\"$A\"]")"
refused one-signature "1 valid signature(s) from distinct pinned publisher keys, 2 required" \
  --artifacts-dir "$full" --offline "${two_keys[@]}"
lists "$(listed)" '[]' "$(entry)"
only_a=$(cache cache-only-a "$package" "$package.$A.minisig")
refused missing-signature-offline "$package.$B.minisig is not in" --artifacts-dir "$only_a" --offline "${two_keys[@]}"
rm -f "$work/release/$tag/$package.$B.minisig"
refused missing-signature-online "could not download $url.$B.minisig" --artifacts-dir "$(cache cache-only-a2 "$package" "$package.$A.minisig")" "${two_keys[@]}"
cp "$work/sig-b/$package.$B.minisig" "$work/release/$tag/"

wrong=$(cache cache-wrong-key "$package" "$package.$A.minisig")
cp "$work/sig-c/$package.$C.minisig" "$wrong/"
lists "$(listed)" '[]' "$(entry keyIds "[\"$A\", \"$C\"]")"
refused wrong-key "required" --artifacts-dir "$wrong" --offline "${two_keys[@]}"
grep -F -q "$C" "$work/wrong-key.log" || bad "the wrong key's file is not reported"

lists "$(listed)" '[]' "$(entry)"
refused revoked-key "signed by a revoked publisher key" --artifacts-dir "$full" --offline \
  --test-only-publisher-keys "$keys/b-revoked.json"
# A revoked release (artifacts.json `revoked`, read by release verify
# --revocations; ArtifactLock::from_json also refuses a locked release the
# lock revokes, in the agent and the lock's tests).
for versions in '"*"' "[\"$version\"]"; do
  lists "$(listed)" '[]' "$(entry)" "[{\"id\": \"$id\", \"versions\": $versions, \"reason\": \"compromised\"}]"
  refused revoked-release "is revoked: compromised" --artifacts-dir "$full" --offline "${two_keys[@]}"
done

# released-pins.json: a release shipped a higher version, or this version
# with other files.
floor() {
  jq --arg id "$id" --arg v "$1" --arg p "$platform" --argjson f "$2" \
    '.releases += [{release: "9.9.9", platform: $p, source: "test", plugins: [{id: $id, version: $v, files: $f}]}]' \
    "$work/released-pins.orig.json" > "$fp/released-pins.json"
}
lists "$(listed)" '[]' "$(entry)"
floor 9.0.0 "$(jq --arg id "$id" '.plugins[] | select(.id == $id) | .files' "$work/source/bundled/pins.json")"
refused downgrade "is below 9.0.0" --artifacts-dir "$full" --offline "${two_keys[@]}"
floor "$version" '{"LICENSE": {"sha256": "0000000000000000000000000000000000000000000000000000000000000000", "size": 1}}'
refused same-version-other-files "with other files" --artifacts-dir "$full" --offline "${two_keys[@]}"
floor "$version" "$(jq --arg id "$id" '.plugins[] | select(.id == $id) | .files' "$work/source/bundled/pins.json")"
run same-version-same-files --artifacts-dir "$full" --offline "${two_keys[@]}" || bad "the released version with its files was refused"
pass "the released version with the same files passes"
cp "$work/released-pins.orig.json" "$fp/released-pins.json"

# Wrong id or version: bundled.json and the lock disagree; the lock names
# another version than the package and its signatures are for.
lists "$(listed 9.9.9)" '[]' "$(entry)"
refused version-disagrees "they must agree" --artifacts-dir "$full" --offline "${two_keys[@]}"
lists "$(listed)" '[]' "$(entry id '"deckpilot.other"')"
refused id-disagrees "does not list" --artifacts-dir "$full" --offline "${two_keys[@]}"
other="$version-other"
other_package="$id-$other.dpplugin"
renamed=$(cache cache-renamed)
cp "$work/release/$tag/$package" "$renamed/$other_package"
cp "$work/release/$tag/$package.$A.minisig" "$renamed/$other_package.$A.minisig"
cp "$work/release/$tag/$package.$B.minisig" "$renamed/$other_package.$B.minisig"
lists "$(listed "$other")" '[]' \
  "$(version=$other url="https://github.com/knittingcode/deckpilot-plugins/releases/download/$id--v$other/$other_package" entry)"
refused version-relabelled "refused" --artifacts-dir "$renamed" --offline "${two_keys[@]}"

lists "$(listed)" '[]' "$(entry)"
refused missing-file "$package is not in" --artifacts-dir "$(cache cache-empty)" --offline "${two_keys[@]}"

for mutable in \
  "https://github.com/knittingcode/deckpilot-plugins/releases/latest/download/$package" \
  "https://github.com/someone/deckpilot-plugins/releases/download/$tag/$package" \
  "http://github.com/knittingcode/deckpilot-plugins/releases/download/$tag/$package"; do
  lists "$(listed)" '[]' "$(entry url "\"$mutable\"")"
  refused mutable-url "only the immutable release URL" --artifacts-dir "$(cache cache-mutable)" "${two_keys[@]}"
  [ -z "$(calls)" ] || bad "a refused URL was fetched: $(calls)"
done

# Never both.
lists "$(listed)" '[{"path": "example-default", "class": "default"}]' "$(entry)"
refused both "a plugin is one or the other" --test-fixtures --artifacts-dir "$full" --offline "${two_keys[@]}"
lists '[]' '[{"path": "example-default", "class": "default"}]' "$(entry)"
refused locked-not-listed "does not list" --test-fixtures --artifacts-dir "$full" --offline "${two_keys[@]}"
lists "$(listed)" '[]' '[]'
refused listed-not-locked "does not lock it" --artifacts-dir "$full" --offline "${two_keys[@]}"
lists "[{\"path\": \"x\", \"id\": \"$id\", \"version\": \"$version\", \"class\": \"default\", \"source\": \"artifact\"}]" '[]' "$(entry)"
refused artifact-with-path "names id and version and no path" --artifacts-dir "$full" --offline "${two_keys[@]}"

# ------------------------------------------------------------ mixed

lists "$(listed)" '[{"path": "legacy-fixture", "class": "migration"}]' "$(entry)"
run mixed --test-fixtures --artifacts-dir "$full" --offline "${two_keys[@]}" || { cat "$work/mixed.log" >&2; bad "the mixed build failed"; }
[ -z "$(calls)" ] || bad "the mixed offline build asked curl for $(calls)"
mixed="$work/out-mixed/bundled"
[ "$(jq -c '[.plugins[] | {id, class}]' "$mixed/pins.json")" \
  = '[{"id":"deckpilot.example-default","class":"default"},{"id":"deckpilot.legacy-fixture","class":"migration"}]' ] \
  || bad "the mixed build pins $(jq -c '[.plugins[].id]' "$mixed/pins.json")"
diff -r "$work/source/bundled/deckpilot.legacy-fixture" "$mixed/deckpilot.legacy-fixture" > /dev/null \
  || bad "the source plugin of the mixed build differs from the source build"
diff -r "$out/$id" "$mixed/$id" > /dev/null || bad "the artifact of the mixed build differs"
[ "$(jq -S . "$mixed/pins.json")" = "$(jq -S . "$work/source/bundled/pins.json")" ] \
  || bad "the mixed build's pins differ from the all-source build of the same files"
pass "a mixed build stages the artifact and the source plugin with the pins of both"

# ------------------------------------------------------------ incomplete

# Complete or a failure: a build that ends with status 0 after its first
# artifact (as a Windows dry run once did, with no error and no
# pins.json), or whose staged plugins are not the listed ones, fails and
# leaves no pins.json. The copy of build.sh is changed for each case and
# restored after.
cp "$build" "$work/build.sh.orig"
sed 's/^  stage_artifacts$/  stage_artifacts; exit 0/' "$work/build.sh.orig" > "$build"
grep -q '^  stage_artifacts; exit 0$' "$build" || bad "the early-end case could not change the copy of build.sh"
refused early-end "ended before every listed plugin was staged and pinned" \
  --test-fixtures --artifacts-dir "$full" --offline "${two_keys[@]}"
awk '/^# Complete or nothing:/ { print "rm -rf \"$bundled/deckpilot.legacy-fixture\"" } { print }' \
  "$work/build.sh.orig" > "$build"
# shellcheck disable=SC2016 # the literal line the case adds
grep -q '^rm -rf "$bundled/deckpilot.legacy-fixture"$' "$build" || bad "the missing-plugin case could not change the copy of build.sh"
refused missing-plugin "the bundle is incomplete" \
  --test-fixtures --artifacts-dir "$full" --offline "${two_keys[@]}"
cp "$work/build.sh.orig" "$build"
run complete-again --test-fixtures --artifacts-dir "$full" --offline "${two_keys[@]}" \
  || { cat "$work/complete-again.log" >&2; bad "the restored build.sh refused the mixed build"; }
pass "the restored build.sh builds the mixed bundle again"

echo "test-build.sh: $passed checks passed"
