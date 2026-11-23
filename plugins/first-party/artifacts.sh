# shellcheck shell=bash disable=SC2154
# The release-artifact half of plugins/first-party/build.sh, sourced by it
# (not run on its own): stages the bundled.json entries marked
# "source": "artifact" from the signed .dpplugin releases of
# knittingcode/deckpilot-plugins that artifacts.json locks
# (docs/marketplace/publisher-signature.md, "Wave B: consuming an artifact").
#
# For every such entry, in this order, each step refusing the build:
#   1. the reviewed lock entry: id and version equal bundled.json's, `url`
#      exactly the immutable release URL of that version
#      (https://github.com/knittingcode/deckpilot-plugins/releases/download/
#      <id>--v<version>/<id>-<version>.dpplugin; never `latest`), `keyIds`
#      distinct key ids;
#   2. the package and one <package>.<KEYID>.minisig per key id from the
#      cache directory, downloaded from that URL only when missing (never
#      with --offline);
#   3. the package's size and sha256 against the lock, before anything
#      parses it;
#   4. `deckpilot-registry release verify` with the pinned publisher keys,
#      a fresh directory holding exactly the package and its listed
#      signature files, released-pins.json (no downgrade, the same version
#      only with the same files), artifacts.json's revocations, the audit
#      log and --unpack into bundled/<id>/;
#   5. every listed key id counted; the privileged-kinds line of the
#      verification (`privileged: none`, or `privileged: reviewed: <id>
#      <version> …` when the compiled gate is open and the package is
#      exactly the reviewed privileged policy's entry: with the gate closed
#      `release verify` refuses every privileged package before this); and
#      the unpacked files: the manifest's id and version, and only the
#      files a bundled plugin may contain.
# Uses build.sh's variables and helpers (here, plugins, target_dir, out,
# bundled, lock, artifacts, the options, fail, jq, sha, size).

# The only place release artifacts come from. Trust comes from the claim
# and the signatures, never from this URL; it is fixed so that the lock
# names one immutable file and nothing else is ever fetched.
ARTIFACT_RELEASES="https://github.com/knittingcode/deckpilot-plugins/releases/download/"
# Largest signature file (publisher::MAX_SIGNATURE_BYTES).
MAX_SIGNATURE_BYTES=2048

# platform_name: macos, linux or windows, for released-pins.json and the
# host check.
platform_name() {
  case "$(uname -s)" in
    Darwin) echo macos ;;
    Linux) echo linux ;;
    MINGW* | MSYS* | CYGWIN*) echo windows ;;
    *) fail "unknown platform $(uname -s): pass --platform" ;;
  esac
}

# fetch <url> <file> <max bytes>: an HTTPS download (redirects to HTTPS
# only, at most <max bytes>) to <file>, atomically.
fetch() {
  local url=$1 file=$2 max=$3
  [ "$offline" = false ] || fail "$(basename "$file") is not in $artifacts_dir (--offline: nothing is downloaded)"
  command -v curl > /dev/null || fail "curl is required to download $url"
  echo "build.sh: downloading $url"
  rm -f "$file.part"
  curl --fail --silent --show-error --location --proto '=https' --proto-redir '=https' \
    --max-filesize "$max" --retry 3 --retry-delay 2 --output "$file.part" "$url" \
    || { rm -f "$file.part"; fail "could not download $url"; }
  mv "$file.part" "$file"
}

# registry: the deckpilot-registry binary (built once when not given).
registry() {
  if [ -z "$registry_tool" ]; then
    local root dir
    root=$(dirname "$plugins")
    dir="$target_dir/registry-tool"
    echo "build.sh: building deckpilot-registry (release verify)" >&2
    (cd "$root" && env -u RUSTFLAGS -u RUSTC_WRAPPER -u RUSTC_WORKSPACE_WRAPPER -u CARGO_BUILD_TARGET \
      cargo build --locked -p deckpilot-plugin-registry --features publish --bin deckpilot-registry \
      --target-dir "$dir" >&2) || fail "could not build deckpilot-registry"
    registry_tool="$dir/debug/deckpilot-registry"
    [ -x "$registry_tool" ] || registry_tool="$registry_tool.exe"
  fi
  [ -x "$registry_tool" ] || fail "$registry_tool is not an executable deckpilot-registry"
  printf '%s\n' "$registry_tool"
}

# allowed_file <path>: the files a bundled plugin may contain (as
# tools/release-pipeline/check-bundled-plugins.sh, which checks again).
allowed_file() {
  case "$1" in manifest.json | plugin.wasm | LICENSE) return 0 ;; esac
  [[ "$1" =~ ^locales/[a-z]{2,3}(-[A-Z][a-z]{3})?(-([A-Z]{2}|[0-9]{3}))?\.json$ ]] && return 0
  [[ "$1" =~ ^assets/([A-Za-z0-9_][A-Za-z0-9._-]*/)*[A-Za-z0-9_][A-Za-z0-9._-]*\.(png|jpg|jpeg)$ ]]
}

stage_artifacts() {
  local cache tool work audit entry id version locked url expected sha256 bytes
  local package key_id key_ids counted listing file admission
  cache=${artifacts_dir:-$target_dir/first-party-artifacts}
  mkdir -p "$cache"
  cache=$(cd "$cache" && pwd)
  [ -n "$platform" ] || platform=$(platform_name)
  if [ -z "$app_version" ]; then
    app_version=$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"$/\1/p' "$(dirname "$plugins")/Cargo.toml" | head -n 1)
    [ -n "$app_version" ] || fail "no workspace version in Cargo.toml: pass --app-version"
  fi
  if [ "$fetch_only" = false ]; then
    tool=$(registry)
    work="$target_dir/first-party-artifacts-verify"
    audit="$out/artifacts-audit.jsonl"
  fi

  for entry in "${artifacts[@]}"; do
    IFS=$'\t' read -r id version _ <<< "$entry"
    # 1. The reviewed entry.
    case "$id" in deckpilot.*) ;; *) fail "$id is not a first-party id (deckpilot.*)" ;; esac
    [ "$(jq --arg id "$id" '[.plugins[] | select(.id == $id)] | length' "$lock")" = 1 ] \
      || fail "bundled.json takes $id from a release artifact, but artifacts.json does not lock it exactly once"
    locked=$(jq -c --arg id "$id" '.plugins[] | select(.id == $id)' "$lock")
    [ "$(jq -r .version <<< "$locked")" = "$version" ] \
      || fail "bundled.json lists $id $version, artifacts.json locks $id $(jq -r .version <<< "$locked"): they must agree"
    [ "$(jq -r 'keys_unsorted | sort | join(",")' <<< "$locked")" = "id,keyIds,sha256,size,url,version" ] \
      || fail "artifacts.json: $id has the fields $(jq -r 'keys | join(", ")' <<< "$locked"); exactly id, version, url, sha256, size and keyIds"
    package="$id-$version.dpplugin"
    expected="$ARTIFACT_RELEASES$id--v$version/$package"
    url=$(jq -r .url <<< "$locked")
    [ "$url" = "$expected" ] \
      || fail "artifacts.json: $id $version has url $url; only the immutable release URL $expected is used"
    sha256=$(jq -r .sha256 <<< "$locked")
    [[ "$sha256" =~ ^[0-9a-f]{64}$ ]] || fail "artifacts.json: $id $version: sha256 is not 64 lowercase hex characters"
    bytes=$(jq -r '.size | select(type == "number" and . > 0 and . == floor)' <<< "$locked")
    [ -n "$bytes" ] || fail "artifacts.json: $id $version: size is not a positive whole number"
    key_ids=$(jq -r '.keyIds | select(type == "array") | .[]' <<< "$locked")
    [ -n "$key_ids" ] || fail "artifacts.json: $id $version names no keyIds"
    [ -z "$(LC_ALL=C sort <<< "$key_ids" | uniq -d)" ] || fail "artifacts.json: $id $version lists a key id twice"
    while IFS= read -r key_id; do
      [[ "$key_id" =~ ^[0-9A-F]{16}$ ]] || fail "artifacts.json: $id $version: $key_id is not a key id (16 characters 0-9A-F)"
    done <<< "$key_ids"

    # 2. The files, from the cache or that URL.
    [ -f "$cache/$package" ] || fetch "$url" "$cache/$package" "$bytes"
    while IFS= read -r key_id; do
      file="$package.$key_id.minisig"
      [ -f "$cache/$file" ] || fetch "$url.$key_id.minisig" "$cache/$file" "$MAX_SIGNATURE_BYTES"
      [ -f "$cache/$file" ] || fail "$id $version: the signature file $file is missing"
    done <<< "$key_ids"

    # 3. Size and sha256 first.
    [ "$(size "$cache/$package")" = "$bytes" ] \
      || fail "$id $version: $cache/$package has $(size "$cache/$package") bytes, artifacts.json says $bytes"
    [ "$(sha "$cache/$package")" = "$sha256" ] \
      || fail "$id $version: $cache/$package has sha256 $(sha "$cache/$package"), artifacts.json says $sha256"
    if [ "$fetch_only" = true ]; then
      echo "fetched $id $version: $cache/$package ($bytes bytes, sha256 $sha256) and $(wc -l <<< "$key_ids" | tr -d ' ') signature file(s)"
      continue
    fi

    # 4. The publisher signature, the floor, the revocations and the host,
    # on a copy holding only the package and its listed signature files.
    rm -rf "${work:?}/$id"
    mkdir -p "$work/$id"
    cp "$cache/$package" "$work/$id/$package"
    while IFS= read -r key_id; do
      cp "$cache/$package.$key_id.minisig" "$work/$id/"
    done <<< "$key_ids"
    local verify=(release verify --package "$work/$id/$package" --signatures-dir "$work/$id"
      --id "$id" --version "$version" --sha256 "$sha256" --size "$bytes"
      --app-version "$app_version" --plugin-api "$plugin_api" --platform "$platform"
      --released-pins "$here/released-pins.json" --revocations "$lock"
      --audit "$audit" --unpack "$bundled/$id")
    [ -z "$test_keys" ] || verify+=(--keys "$test_keys")
    listing=$("$tool" "${verify[@]}" 2>&1) || {
      rm -rf "${bundled:?}/$id"
      printf '%s\n' "$listing" >&2
      fail "$id $version: the release artifact does not verify"
    }
    printf '%s\n' "$listing"

    # 5. Every listed key counted; the privileged-kinds admission; the
    # unpacked files.
    counted=$(sed -n 's/^  counted  .*: key \([0-9A-F]\{16\}\) (fingerprint .*$/\1/p' <<< "$listing" | LC_ALL=C sort)
    [ "$counted" = "$(LC_ALL=C sort <<< "$key_ids")" ] \
      || fail "$id $version: the keys that counted ($(tr '\n' ' ' <<< "$counted")) are not the keyIds artifacts.json lists ($(tr '\n' ' ' <<< "$key_ids"))"
    admission=$(sed -n 's/^  privileged: //p' <<< "$listing")
    [ "$(grep -c '^  privileged: ' <<< "$listing")" = 1 ] \
      || fail "$id $version: release verify did not print exactly one privileged-kinds line (a deckpilot-registry older than the reviewed privileged policy?)"
    case "$admission" in
      none) ;;
      "reviewed: $id $version "*)
        echo "build.sh: $id $version declares privileged kinds; admitted by the reviewed privileged policy ($admission)" ;;
      *) fail "$id $version: unexpected privileged-kinds admission: $admission" ;;
    esac
    [ "$(jq -r .id "$bundled/$id/manifest.json")@$(jq -r .version "$bundled/$id/manifest.json")" = "$id@$version" ] \
      || fail "$id $version: the unpacked manifest is not $id $version"
    [ "$(jq 'has("assets")' "$bundled/$id/manifest.json")" = false ] \
      || fail "$id $version: a bundled plugin has no assets but its icons"
    while IFS= read -r file; do
      allowed_file "$file" \
        || fail "$id $version: the package contains $file, which a bundled plugin may not (manifest.json, plugin.wasm, LICENSE, images under assets/, locales/<tag>.json)"
    done < <(cd "$bundled/$id" && find . -type f | sed 's|^\./||' | LC_ALL=C sort)
  done
}
