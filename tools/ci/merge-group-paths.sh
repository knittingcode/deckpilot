#!/usr/bin/env bash
# Whether a merge queue group needs a path-filtered workflow. Writes
# `run=true|false` to GITHUB_OUTPUT (or stdout).
#
#   tools/ci/merge-group-paths.sh <workflow file> [<changed files list>]
#
# GitHub applies `paths` filters to push and pull_request events only; a
# merge_group event always starts the workflow. This applies the workflow's
# own `on.pull_request.paths` list (read from the file, so there is one list
# to maintain) to the files of the merge group: those between the group's
# base commit (BASE_SHA, fetched here) and its head (HEAD), i.e. every pull
# request in the group. With a list file, those files are used instead
# (tests).
#
# When in doubt, run: any other event, a missing or unreadable list, a
# pattern this script does not translate (negation, character classes,
# braces) or a diff that cannot be computed gives run=true.
set -euo pipefail

out="${GITHUB_OUTPUT:-/dev/stdout}"
run() {
  echo "$1; running the workflow" >&2
  echo "run=true" >> "$out"
  exit 0
}

workflow="${1:-}"
[ -n "$workflow" ] && [ -r "$workflow" ] || run "workflow file '${workflow}' not readable"

if [ "$#" -gt 1 ]; then
  changed=$(cat "$2")
elif [ "${EVENT_NAME:-}" != merge_group ]; then
  run "event '${EVENT_NAME:-}' is not a merge group"
else
  # --no-renames: a moved file counts at its old and its new path.
  base="${BASE_SHA:-}"
  if [ -z "$base" ] || ! git fetch --quiet --no-tags --depth=1 origin "$base" 2> /dev/null \
    || ! changed=$(git diff --no-renames --name-only "$base" HEAD 2> /dev/null); then
    run "the changed files of the merge group cannot be determined"
  fi
fi
[ -n "$changed" ] || run "no changed files"

# The items of on.pull_request.paths: the block from a two-space
# `pull_request:` key to the next two-space key, its four-space `paths:`
# key, and the six-space `- item` lines under it (comments skipped).
patterns=$(awk '
  /^  pull_request:[[:space:]]*$/ { in_pr = 1; next }
  in_pr && /^  [^ #]/ { in_pr = 0; in_paths = 0 }
  !in_pr { next }
  /^    paths:[[:space:]]*$/ { in_paths = 1; next }
  in_paths && /^    [^ #]/ { in_paths = 0 }
  in_paths && /^      - / { sub(/^      - /, ""); sub(/[[:space:]]+#.*$/, ""); print }
' "$workflow")
[ -n "$patterns" ] || run "no on.pull_request.paths in $workflow"

# A GitHub filter pattern as an extended regular expression on whole paths:
# `**` any characters, `*` any characters but `/`, `.` literal. Anything
# else outside [A-Za-z0-9._/-] (quotes included) is not translated.
regex=""
while IFS= read -r pattern; do
  case "$pattern" in
    *[!A-Za-z0-9._/*-]* | "") run "pattern '$pattern' is not translated" ;;
  esac
  part=$(printf '%s' "$pattern" | sed -e 's/\./\\./g' -e 's/\*\*/@@/g' -e 's/\*/[^\/]*/g' -e 's/@@/.*/g')
  regex="${regex:+$regex|}^${part}\$"
done <<< "$patterns"

echo "paths of $workflow: $regex" >&2
echo "changed files:" >&2
while IFS= read -r file; do echo "  $file"; done <<< "$changed" >&2
if grep -q -E "$regex" <<< "$changed"; then
  echo "run=true" >> "$out"
else
  echo "no changed file matches; the workflow's jobs report as skipped" >&2
  echo "run=false" >> "$out"
fi
