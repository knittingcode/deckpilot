#!/usr/bin/env bash
# Tests merge-group-paths.sh: which merge queue groups start a path-filtered
# workflow's jobs, by the workflow's own on.pull_request.paths.
set -euo pipefail
# The script writes to GITHUB_OUTPUT, which a runner sets: each case points
# it at its own file.

dir="$(cd "$(dirname "$0")" && pwd)"
script="$dir/merge-group-paths.sh"
workflows="$(cd "$dir/../../.github/workflows" && pwd)"
work=$(mktemp -d "${TMPDIR:-/tmp}/merge-group-paths-test.XXXXXX")
trap 'rm -rf "$work"' EXIT
failures=0

# expect true|false <workflow file> <changed file>...
expect() {
  local expected="$1" workflow="$2"
  shift 2
  printf '%s\n' "$@" > "$work/changed"
  : > "$work/out"
  GITHUB_OUTPUT="$work/out" "$script" "$workflow" "$work/changed" 2> /dev/null || true
  if [ "$(cat "$work/out")" = "run=$expected" ]; then
    echo "ok   $(basename "$workflow"): $*"
  else
    echo "FAIL $(basename "$workflow"): $*: got '$(cat "$work/out")', expected 'run=$expected'"
    failures=$((failures + 1))
  fi
}

# The translation, on a fixture: `**` crosses directories, `*` does not,
# `.` is literal, whole paths only, comments and other events' lists ignored.
cat > "$work/fixture.yml" <<'YAML'
on:
  push:
    branches: [main]
    paths:
      - pushed-only/**
  pull_request:
    paths:
      # A comment between the items.
      - src/**
      - crates/lib-*/**
      - Cargo.lock # a trailing comment
      - tools/build.sh
  merge_group:
    types: [checks_requested]
  workflow_dispatch:
YAML
f="$work/fixture.yml"
expect true "$f" src/main.rs
expect true "$f" src/a/b/c.rs
expect true "$f" docs/x.md crates/lib-core/src/lib.rs
expect false "$f" crates/lib-core
expect false "$f" crates/other/src/lib.rs
expect true "$f" Cargo.lock
expect false "$f" Cargo-lock
expect false "$f" sub/Cargo.lock
expect false "$f" Cargo.lock.bak
expect true "$f" tools/build.sh
expect false "$f" tools/build.shx
expect false "$f" pushed-only/x
expect false "$f" docs/x.md README.md
# When in doubt, run: no list, an untranslated pattern, no changed files.
printf 'on:\n  pull_request:\n  merge_group:\n' > "$work/nolist.yml"
expect true "$work/nolist.yml" docs/x.md
printf 'on:\n  pull_request:\n    paths:\n      - "src/**"\n' > "$work/quoted.yml"
expect true "$work/quoted.yml" docs/x.md
printf 'on:\n  pull_request:\n    paths:\n      - src/**\n      - !src/x\n' > "$work/negated.yml"
expect true "$work/negated.yml" docs/x.md
expect true "$f" ""
# Not a merge group, no list file: run.
: > "$work/out"
EVENT_NAME=pull_request GITHUB_OUTPUT="$work/out" "$script" "$f" 2> /dev/null
if [ "$(cat "$work/out")" = "run=true" ]; then echo "ok   pull_request event"; else
  echo "FAIL pull_request event: $(cat "$work/out")"; failures=$((failures + 1)); fi

# The real workflows that use it: each list is read, a docs-only group
# skips, and a file of the list runs.
while read -r name file; do
  set -- "$name" "$file"
  grep -q '^  merge_group:' "$workflows/$1" || { echo "FAIL $1: no merge_group trigger"; failures=$((failures + 1)); }
  grep -q "merge-group-paths.sh .github/workflows/$1" "$workflows/$1" \
    || { echo "FAIL $1: does not check its own file"; failures=$((failures + 1)); }
  expect false "$workflows/$1" docs/development.md CHANGELOG.md
  expect true "$workflows/$1" docs/development.md "$2"
  # The workflow's own file is always in its list.
  expect true "$workflows/$1" ".github/workflows/$1"
done << 'CASES'
desktop-e2e.yml apps/desktop/src/App.tsx
plugins-agent.yml crates/deckpilot-plugin-store/src/lib.rs
compatibility.yml tools/compat/run.sh
plugin-runtime.yml plugins/runtime/crates/host/src/main.rs
release-tests.yml .github/workflows/ci.yml
advisories.yml plugins/runtime/Cargo.lock
website.yml website/src/index.astro
CASES
expect false "$workflows/plugins-agent.yml" crates/deckpilot-core/src/lib.rs
expect false "$workflows/plugin-runtime.yml" plugins/first-party/bundled.json

if [ "$failures" -gt 0 ]; then
  echo "$failures failed"
  exit 1
fi
echo "all passed"
