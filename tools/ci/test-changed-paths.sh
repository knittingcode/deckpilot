#!/usr/bin/env bash
# Tests changed-paths.sh: which optional jobs a pull request's files turn on.
set -euo pipefail

script="$(cd "$(dirname "$0")" && pwd)/changed-paths.sh"
work=$(mktemp -d "${TMPDIR:-/tmp}/changed-paths-test.XXXXXX")
trap 'rm -rf "$work"' EXIT
failures=0

# expect "<expected outputs, sorted>" <changed file>...
expect() {
  local expected="$1"
  shift
  printf '%s\n' "$@" > "$work/changed"
  local actual
  : > "$work/out"
  GITHUB_OUTPUT="$work/out" "$script" "$work/changed" 2> /dev/null
  actual=$(sort "$work/out" | tr '\n' ' ')
  if [ "$actual" = "$expected " ]; then
    echo "ok   $*"
  else
    echo "FAIL $*: got '$actual', expected '$expected'"
    failures=$((failures + 1))
  fi
}

none="dev_tools=false keystore=false packaging=false release_manifest=false upgrade=false"
expect "$none" apps/desktop/src/App.tsx
expect "$none" crates/deckpilot-core/src/lib.rs docs/development.md
expect "$none" apps/agent/src/main.rs
expect "dev_tools=false keystore=false packaging=true release_manifest=false upgrade=false" apps/agent/build.rs
expect "dev_tools=false keystore=false packaging=true release_manifest=false upgrade=false" apps/agent/Info.plist
expect "dev_tools=false keystore=false packaging=true release_manifest=false upgrade=false" apps/desktop/src-tauri/tauri.release.conf.json
expect "dev_tools=false keystore=false packaging=true release_manifest=false upgrade=false" apps/desktop/src-tauri/Cargo.toml
expect "dev_tools=false keystore=false packaging=true release_manifest=false upgrade=false" apps/desktop/src-tauri/icons/icon.ico
expect "dev_tools=false keystore=false packaging=true release_manifest=false upgrade=false" apps/desktop/src-tauri/capabilities/default.json
expect "dev_tools=false keystore=false packaging=true release_manifest=false upgrade=false" apps/desktop/package.json
expect "dev_tools=false keystore=false packaging=true release_manifest=false upgrade=false" apps/desktop/src-tauri/tauri.conf.json
expect "dev_tools=false keystore=false packaging=true release_manifest=false upgrade=false" Cargo.lock
expect "dev_tools=false keystore=false packaging=true release_manifest=true upgrade=false" packaging/linux/70-deckpilot-streamdeck.rules
expect "dev_tools=false keystore=false packaging=true release_manifest=true upgrade=false" packaging/linux/postinst.sh
expect "dev_tools=false keystore=false packaging=false release_manifest=true upgrade=false" tools/ci/test-udev-scripts.sh
expect "dev_tools=false keystore=false packaging=true release_manifest=false upgrade=false" packaging/flatpak/udev.md
# The plugin host's entitlement and the check that enforces it.
expect "dev_tools=false keystore=false packaging=true release_manifest=false upgrade=false" apps/desktop/src-tauri/entitlements/plugin-host.entitlements
expect "dev_tools=false keystore=false packaging=true release_manifest=false upgrade=false" apps/desktop/src-tauri/tauri.plugins.macos.conf.json
expect "dev_tools=false keystore=false packaging=true release_manifest=false upgrade=false" .github/scripts/verify-macos-app.sh
# The plugin runtime the packages contain: host, bundled plugins, pins, check.
expect "dev_tools=false keystore=false packaging=true release_manifest=false upgrade=false" plugins/runtime/crates/host/src/main.rs
expect "dev_tools=false keystore=false packaging=true release_manifest=false upgrade=false" plugins/first-party/bundled.json
expect "dev_tools=false keystore=false packaging=true release_manifest=false upgrade=false" crates/deckpilot-plugin-bundled/build.rs
expect "dev_tools=false keystore=false packaging=true release_manifest=false upgrade=false" tools/release-pipeline/check-bundled-plugins.sh
expect "$none" plugins/examples/hello-world/src/lib.rs
# The distribution record the package checks read from the binaries.
expect "dev_tools=false keystore=false packaging=true release_manifest=false upgrade=false" crates/deckpilot-distribution/src/lib.rs
expect "dev_tools=false keystore=true packaging=false release_manifest=false upgrade=false" crates/deckpilot-keystore/src/lib.rs
expect "dev_tools=false keystore=true packaging=false release_manifest=true upgrade=false" tools/ci/with-keyring.sh
expect "dev_tools=false keystore=false packaging=true release_manifest=true upgrade=false" tools/ci/check-packages.sh
# The toolchain step the package checks run before their Rust cache.
expect "dev_tools=false keystore=false packaging=true release_manifest=true upgrade=false" tools/ci/rust-pinned-toolchain.sh
expect "dev_tools=false keystore=false packaging=true release_manifest=true upgrade=false" .github/workflows/release.yml
expect "dev_tools=false keystore=false packaging=false release_manifest=true upgrade=false" tools/release-pipeline/common.sh
expect "dev_tools=false keystore=false packaging=false release_manifest=true upgrade=true" crates/deckpilot-update/src/lib.rs
expect "dev_tools=true keystore=true packaging=false release_manifest=true upgrade=false" .github/workflows/ci.yml
expect "dev_tools=false keystore=false packaging=false release_manifest=false upgrade=false" .github/workflows/compatibility.yml
expect "dev_tools=false keystore=false packaging=true release_manifest=false upgrade=false" .github/workflows/packaging.yml
expect "dev_tools=false keystore=false packaging=true release_manifest=true upgrade=false" tools/ci/stage-plugin-runtime.sh
expect "dev_tools=false keystore=false packaging=true release_manifest=true upgrade=false" tools/ci/check-linux-install.sh
expect "dev_tools=false keystore=false packaging=false release_manifest=true upgrade=false" tools/ci/assemble-latest-json.sh
expect "dev_tools=true keystore=false packaging=false release_manifest=false upgrade=false" tools/dev/worktree-maintenance.sh
expect "dev_tools=true keystore=false packaging=false release_manifest=false upgrade=false" tools/dev/tests/target-slot.test.sh
expect "$none" tools/dev/src/main.rs tools/dev/Cargo.toml
# The upgrade from the latest release: storage and its migrations, the
# updater, the baseline release, the helpers and download it uses.
up="dev_tools=false keystore=false packaging=false release_manifest=false upgrade=true"
expect "$up" crates/deckpilot-storage/src/sqlite.rs
expect "$up" crates/deckpilot-storage/tests/fixtures/v0.14.0/deckpilot.db
expect "$up" apps/desktop/src-tauri/src/updates.rs
expect "$up" apps/desktop/src-tauri/src/updates_tests.rs
expect "$up" tools/release-pipeline/compat-baseline
expect "$up" tools/dev/src/validation.rs
expect "$up" tools/release-smoke/download.sh
expect "dev_tools=false keystore=false packaging=true release_manifest=true upgrade=false" tools/ci/check-upgrade.sh
expect "$none" tools/release-smoke/linux.sh crates/deckpilot-storage-extra/x.rs
# Look-alikes do not count.
expect "$none" crates/deckpilot-keystore-extra/x.rs tools/cix/a.sh xCargo.lock apps/desktop/src-tauri/src/tauri.rs

# Pushes run everything.
: > "$work/push"
GITHUB_OUTPUT="$work/push" EVENT_NAME=push "$script" 2> /dev/null
actual=$(sort "$work/push" | tr '\n' ' ')
if [ "$actual" = "dev_tools=true keystore=true packaging=true release_manifest=true upgrade=true " ]; then
  echo "ok   push to main runs every optional job"
else
  echo "FAIL push: got '$actual'"
  failures=$((failures + 1))
fi

# Merge queue groups: the files between the group's base and its head, here
# two queued pull requests on top of the base (a throwaway repository).
repo="$work/merge-group"
git init --quiet "$repo"
git -C "$repo" -c user.name=test -c user.email=test@example.invalid commit --quiet --allow-empty -m base
base=$(git -C "$repo" rev-parse HEAD)
mkdir -p "$repo/apps/desktop/src" "$repo/crates/deckpilot-keystore/src"
echo a > "$repo/apps/desktop/src/App.tsx"
git -C "$repo" add -A
git -C "$repo" -c user.name=test -c user.email=test@example.invalid commit --quiet -m "first queued pull request"
echo b > "$repo/crates/deckpilot-keystore/src/lib.rs"
git -C "$repo" add -A
git -C "$repo" -c user.name=test -c user.email=test@example.invalid commit --quiet -m "second queued pull request"
git -C "$repo" remote add origin "$repo"
# merge_group <base sha> <expected outputs>
merge_group() {
  : > "$work/group"
  (cd "$repo" && GITHUB_OUTPUT="$work/group" EVENT_NAME=merge_group BASE_SHA="$1" "$script" 2> /dev/null)
  actual=$(sort "$work/group" | tr '\n' ' ')
  if [ "$actual" = "$2 " ]; then
    echo "ok   merge group with base ${1:-(none)}: $2"
  else
    echo "FAIL merge group with base ${1:-(none)}: got '$actual', expected '$2'"
    failures=$((failures + 1))
  fi
}
merge_group "$base" "dev_tools=false keystore=true packaging=false release_manifest=false upgrade=false"
merge_group "" "dev_tools=true keystore=true packaging=true release_manifest=true upgrade=true"
merge_group 0000000000000000000000000000000000000000 "dev_tools=true keystore=true packaging=true release_manifest=true upgrade=true"

[ "$failures" -eq 0 ] || { echo "$failures case(s) failed"; exit 1; }
echo "changed paths: all cases passed"
