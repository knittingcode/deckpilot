#!/usr/bin/env bash
# Upgrades an installation of the latest published release to the packages
# of this commit, on an isolated profile, and checks that the user's data
# survives (docs/updates.md, "When rolling back is safe"):
#
#   tools/ci/check-upgrade.sh download <tag> <dir>
#   tools/ci/check-upgrade.sh deb      <tag> <release dir> <new .deb>      <deckpilot-dev>   (Ubuntu runner, sudo)
#   tools/ci/check-upgrade.sh appimage <tag> <release dir> <new .AppImage> <deckpilot-dev>
#   tools/ci/check-upgrade.sh agents   <old agent> <new agent> <deckpilot-dev>
#
# download: the release's .deb and AppImage with SHA256SUMS
#   (tools/release-smoke/download.sh), and their updater signatures
#   verified with minisign against the pinned updater key
#   (.github/updater.pub), each naming the release's version, as
#   tools/release-smoke/update.sh checks them; each .sig asset must also be
#   the signature latest.json announces for that package.
# deb: installs the release's .deb with apt, runs its agent on the data
#   below, then installs the new .deb over it with apt (the package
#   upgrade; a reinstall while this commit still has the release's version)
#   and runs the new agent on the same data.
# appimage: the same with AppImages, replaced in place as the updater
#   replaces them.
# agents: the same cycle with two agent executables (no package), e.g. to
#   try the checks on a development machine.
#
# The cycle, on a copy of the 0.14.0 fixture
# (crates/deckpilot-storage/tests/fixtures/v0.14.0: three profiles, the
# development password, update mode "install when idle"; schema 13, which
# 0.15.0 still uses):
#  1. the release's agent starts, is unlocked over IPC, and makes the
#     "Personal" profile the active one (it writes that to the database);
#  2. it is stopped (Shutdown) and the new version is installed;
#  3. the new agent starts, answers at this commit's protocol, unlocks:
#     the configuration has this commit's schema, the upgrade is recorded
#     with a backup, the backup (a copy) opens with the same key and holds
#     the profiles, and the profiles, the active profile and project and
#     the update mode are the ones from before;
#  4. downgrade protection: the release's agent, started again on the
#     upgraded data, refuses to open it and changes nothing (only when the
#     schema changed; going back is not promised, see docs/updates.md);
#  5. the new agent still opens it afterwards.
#
# Every agent runs headless with an isolated profile (HOME, XDG
# directories, DECKPILOT_DATA_DIR) in a private D-Bus session where
# dbus-run-session exists, and with the plugin registry pointed at an
# unreachable loopback URL: no real user data, credential store or
# production service is used. Throwaway data, development password only.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
fixture="$root/crates/deckpilot-storage/tests/fixtures/v0.14.0"
# Its schema, and its recorded history (created at that schema).
fixture_schema=13
fixture_history="0->13:ok"
# The profile the old agent activates; the fixture's active one is "Work".
profile_name=Personal
# Never contacted: the plugin registry the agents are pointed at.
registry=https://127.0.0.1:9/deckpilot-upgrade-check
fail() { echo "::error::$*" >&2; exit 1; }
step() { echo "== $*"; }

# One temporary folder for everything below, removed at the end.
work=$(mktemp -d "${TMPDIR:-/tmp}/deckpilot-upgrade.XXXXXX")
agent_pid=""
cleanup() {
  stop_agent 2> /dev/null || true
  rm -rf "${work:?}"
}
trap cleanup EXIT

# The isolated profile and its data directory, a copy of the fixture.
profile="$work/profile"
data="$profile/data"
new_profile() {
  rm -rf "${profile:?}"
  mkdir -p "$profile"/{home,run,xdg-data,xdg-config,xdg-cache}
  cp -R "$fixture" "$data"
  chmod 700 "$profile/run" "$data"
}

# in_profile <command>...: runs a command with the profile's environment
# (the agent's socket is in XDG_RUNTIME_DIR, so the client needs it too).
in_profile() {
  env HOME="$profile/home" XDG_DATA_HOME="$profile/xdg-data" XDG_CONFIG_HOME="$profile/xdg-config" \
    XDG_CACHE_HOME="$profile/xdg-cache" XDG_RUNTIME_DIR="$profile/run" DECKPILOT_DATA_DIR="$data" "$@"
}
# dev <validation command> [<args>]: deckpilot-dev validation on the data.
dev() {
  local command="$1"
  shift
  case "$dev_bin" in /*) ;; *) dev_bin="$PWD/$dev_bin" ;; esac
  in_profile "$dev_bin" validation "$command" "$data" "$@"
}

# start_agent <label> <command>...: runs the agent (with the flags below)
# and waits until it accepts connections.
start_agent() {
  local label="$1" log
  shift
  log="$work/agent-$label.log"
  rm -f "$data/agent.token"
  local session=()
  if command -v dbus-run-session > /dev/null; then session=(dbus-run-session --); fi
  in_profile ${session[@]+"${session[@]}"} "$@" --headless --no-usb --plugin-registry-url "$registry" \
    > "$log" 2>&1 &
  agent_pid=$!
  for _ in $(seq 1 120); do
    if [ -s "$data/agent.token" ] && dev agent > /dev/null 2>&1; then
      echo "$label agent started"
      return 0
    fi
    kill -0 "$agent_pid" 2> /dev/null || break
    sleep 0.5
  done
  sed 's/^/  | /' "$log" >&2
  fail "the $label agent did not start"
}

# stop_agent: Shutdown over IPC (as Quit does), then waits for the agent
# to end; killed after 20 s.
stop_agent() {
  [ -n "$agent_pid" ] || return 0
  dev shutdown > /dev/null 2>&1 || true
  for _ in $(seq 1 40); do
    kill -0 "$agent_pid" 2> /dev/null || break
    sleep 0.5
  done
  if kill -0 "$agent_pid" 2> /dev/null; then
    echo "::warning::the agent did not stop within 20 s after Shutdown; killing it"
    kill "$agent_pid" 2> /dev/null || true
    pkill -x deckpilot-agent 2> /dev/null || true
  fi
  wait "$agent_pid" 2> /dev/null || true
  agent_pid=""
  # No agent of this profile may be left (an AppImage's agent replaces the
  # process started here, a D-Bus session wraps it).
  for _ in $(seq 1 20); do
    dev agent > /dev/null 2>&1 || return 0
    sleep 0.5
  done
  fail "an agent still answers in $data after it was stopped"
}

# The fingerprint of the user's data that must survive: profiles, the
# active profile and project, protection (from `validation status`).
fingerprint() {
  dev status | tr ' ' '\n' | grep -E '^(security|protection|profiles|active_profile|active_project)=' | tr '\n' ' '
}

# files_digest: the database, its write-ahead log and the vault.
files_digest() {
  (cd "$data" && for f in deckpilot.db deckpilot.db-wal deckpilot.vault.json; do
    if [ -f "$f" ]; then printf '%s %s\n' "$f" "$(sha256_of "$f")"; fi
  done)
}
sha256_of() {
  if command -v sha256sum > /dev/null; then sha256sum < "$1" | cut -d' ' -f1; else shasum -a 256 < "$1" | cut -d' ' -f1; fi
}

# cycle <label>: the cycle above. Needs old_agent, new_agent (arrays: the
# commands that start each version) and install_new (a function that
# installs the new version, after which new_agent starts it).
cycle() {
  local label="$1" before status agent_line schema_line supported old_schema
  new_profile
  step "$label: the release's agent on the 0.14.0 fixture (schema 13)"
  start_agent "$label-old" "${old_agent[@]}"
  dev agent
  dev unlock
  dev activate "$profile_name"
  before=$(fingerprint)
  echo "before the upgrade: $before"
  case "$before" in
    *"active_profile=$profile_name "*) ;;
    *) fail "the release's agent did not activate $profile_name: $before" ;;
  esac
  case "$before" in *"profiles=3 "*) ;; *) fail "expected the fixture's 3 profiles: $before" ;; esac
  stop_agent

  step "$label: install this commit's version"
  install_new

  step "$label: this commit's agent on the same data"
  start_agent "$label-new" "${new_agent[@]}"
  agent_line=$(dev agent)
  echo "$agent_line"
  local client
  client=$(sed -n 's/.*client_protocol=\([0-9]*\).*/\1/p' <<< "$agent_line")
  case "$agent_line" in
    *"agent_protocol=$client negotiated_protocol=$client "*) ;;
    *) fail "the upgraded agent does not answer at this commit's protocol $client: $agent_line" ;;
  esac
  dev unlock
  status=$(fingerprint)
  echo "after the upgrade:  $status"
  [ "$status" = "$before" ] || fail "the data changed in the upgrade: before '$before', after '$status'"
  dev stored
  dev inventory
  schema_line=$(dev schema)
  echo "$schema_line"
  supported=$(sed -n 's/.*supported=\([0-9]*\).*/\1/p' <<< "$schema_line")
  old_schema=$fixture_schema
  # The release's agent must not have upgraded the fixture itself: then its
  # schema is newer than the fixture's, and the check would no longer
  # start from the release's own schema.
  local history
  history=$(sed -n 's/.* upgrades=\([^ ]*\) .*/\1/p' <<< "$schema_line")
  case "$history" in
    "$fixture_history" | "$fixture_history,$old_schema->$supported:ok") ;;
    *) fail "unexpected schema history '$history' (expected $fixture_history, then $old_schema->$supported): if the release's own schema is newer than $old_schema, add a fixture made by that release and use it here" ;;
  esac
  case "$schema_line" in
    "schema=$supported supported=$supported "*) ;;
    *) fail "the configuration does not have this commit's schema: $schema_line" ;;
  esac
  case "$schema_line" in
    *"updates=InstallWhenIdle "*) ;;
    *) fail "the update mode did not survive the upgrade: $schema_line" ;;
  esac
  if [ "$supported" -gt "$old_schema" ]; then
    case "$schema_line" in
      *" upgrades=$fixture_history,$old_schema->$supported:ok "*" backup=deckpilot-schema$old_schema-"*".db backup_profiles=3"*) ;;
      *) fail "the upgrade from schema $old_schema is not recorded with a backup that opens and holds the 3 profiles: $schema_line" ;;
    esac
    ls -l "$data/backups"
  else
    echo "this commit uses schema $old_schema as well: no upgrade, no backup expected"
  fi
  stop_agent

  if [ "$supported" -gt "$old_schema" ]; then
    step "$label: the release's agent refuses the upgraded data (downgrade protection)"
    local digest_before digest_after unlock
    digest_before=$(files_digest)
    start_agent "$label-old-again" "${old_agent[@]}"
    unlock=$(dev unlock 2>&1 || true)
    echo "unlock: $unlock"
    sleep 1
    stop_agent
    digest_after=$(files_digest)
    if grep -q "agent unlocked" <<< "$unlock"; then
      fail "the release's agent opened a configuration of schema $supported"
    fi
    [ "$digest_before" = "$digest_after" ] \
      || fail "the release's agent changed the upgraded data: $digest_before -> $digest_after"
    echo "refused, database and vault unchanged"

    step "$label: this commit's agent opens it afterwards"
    start_agent "$label-new-again" "${new_agent[@]}"
    dev unlock
    status=$(fingerprint)
    [ "$status" = "$before" ] || fail "the data changed after the refused downgrade: '$status'"
    stop_agent
  fi
  echo "$label: upgrade from the release passed"
}

# Checks the updater signature of <file> (its <file>.sig asset).
verify_signature() {
  local file="$1" version="$2" manifest="$3" platform="$4" pub="$work/updater.pub" sig="$work/sig.minisig" comment
  base64 -d < "$root/.github/updater.pub" > "$pub"
  base64 -d < "$file.sig" > "$sig"
  minisign -V -q -p "$pub" -m "$file" -x "$sig" > /dev/null || fail "$(basename "$file"): minisign rejects its signature"
  comment=$(sed -n 's/^trusted comment: //p' "$sig")
  grep -q -E "version:${version//./\\.}([[:space:]]|$)" <<< "$comment" \
    || fail "$(basename "$file"): the trusted comment does not name version $version: $comment"
  [ "$(jq -r --arg p "$platform" '.platforms[$p].signature' "$manifest" | tr -d '\r\n')" = "$(tr -d '\r\n' < "$file.sig")" ] \
    || fail "$(basename "$file").sig is not the signature latest.json announces for $platform"
  echo "$(basename "$file"): minisign ok with key $(sed -n 's/.*public key: //p' "$pub"), $comment, as latest.json $platform announces"
}

# An absolute path to deckpilot-dev (set below).
dev_bin=""
kind="${1:?usage: check-upgrade.sh download|deb|appimage|agents ...}"
case "$kind" in
  download)
    tag="${2:?usage: check-upgrade.sh download <tag> <dir>}"
    dir="${3:?usage: check-upgrade.sh download <tag> <dir>}"
    version="${tag#v}"
    SMOKE_OUT="$work/smoke" SMOKE_PLATFORM=upgrade "$root/tools/release-smoke/download.sh" "$tag" "$dir" \
      "DeckPilot_${version}_amd64.deb" "DeckPilot_${version}_amd64.deb.sig" \
      "DeckPilot_${version}_amd64.AppImage" "DeckPilot_${version}_amd64.AppImage.sig" latest.json
    cut -f2- "$work/smoke/results.tsv"
    ! cut -f3 "$work/smoke/results.tsv" | grep -q -v -x pass || fail "the download of $tag failed its checks"
    [ "$(jq -r .version "$dir/latest.json")" = "$version" ] || fail "latest.json of $tag does not announce $version"
    verify_signature "$dir/DeckPilot_${version}_amd64.deb" "$version" "$dir/latest.json" linux-x86_64-deb
    verify_signature "$dir/DeckPilot_${version}_amd64.AppImage" "$version" "$dir/latest.json" linux-x86_64-appimage
    ;;
  deb)
    tag="${2:?}" dir="${3:?}" new="${4:?}" dev_bin="${5:?}"
    version="${tag#v}"
    old="$(cd "$dir" && pwd)/DeckPilot_${version}_amd64.deb"
    new="$(cd "$(dirname "$new")" && pwd)/$(basename "$new")"
    [ -f "$old" ] || fail "$old is missing (run download first)"
    package=$(dpkg-deb -f "$new" Package)
    new_version=$(dpkg-deb -f "$new" Version)
    [ "$(dpkg-deb -f "$old" Package)" = "$package" ] || fail "the release's package is not $package"
    # The package manager's upgrade needs a newer version; while this
    # commit still has the release's version, the same version is
    # reinstalled (the package's files replaced). Older is a mistake.
    dpkg --compare-versions "$new_version" ge "$version" \
      || fail "this commit's package version $new_version is older than the release's $version"
    step "install the release's .deb ($version)"
    timeout 900 sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -q "$old" > "$work/apt-old.log" 2>&1 \
      || { tail -n 20 "$work/apt-old.log" >&2; fail "apt-get cannot install $old"; }
    [ "$(dpkg-query -W -f '${Version}' "$package")" = "$version" ] || fail "$package is not at $version"
    /usr/bin/deckpilot-agent --version
    old_agent=(/usr/bin/deckpilot-agent)
    new_agent=(/usr/bin/deckpilot-agent)
    install_new() {
      local reinstall=()
      if dpkg --compare-versions "$new_version" eq "$version"; then
        reinstall=(--reinstall)
        echo "this commit's package has the release's version $version: reinstalled over it"
      fi
      timeout 900 sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -q ${reinstall[@]+"${reinstall[@]}"} "$new" \
        > "$work/apt-new.log" 2>&1 || { tail -n 20 "$work/apt-new.log" >&2; fail "apt-get cannot install $new over $version"; }
      grep -E 'Unpacking|Setting up|Preparing' "$work/apt-new.log" || true
      [ "$(dpkg-query -W -f '${Version} ${Status}' "$package")" = "$new_version install ok installed" ] \
        || fail "$package is not installed at $new_version: $(dpkg-query -W -f '${Version} ${Status}' "$package")"
      # The installed agent is this commit's.
      dpkg-deb --fsys-tarfile "$new" | tar -xO ./usr/bin/deckpilot-agent > "$work/new-agent"
      cmp -s "$work/new-agent" /usr/bin/deckpilot-agent || fail "/usr/bin/deckpilot-agent is not the new package's"
      echo "installed: $(dpkg-query -W -f '${Package} ${Version}' "$package"), /usr/bin/deckpilot-agent is the new package's"
      # The release's agent, for the downgrade check: from its package,
      # not installed (the system keeps this commit's).
      dpkg-deb -x "$old" "$work/old-deb"
      old_agent=("$work/old-deb/usr/bin/deckpilot-agent")
    }
    cycle deb
    ;;
  appimage)
    tag="${2:?}" dir="${3:?}" new="${4:?}" dev_bin="${5:?}"
    version="${tag#v}"
    old="$(cd "$dir" && pwd)/DeckPilot_${version}_amd64.AppImage"
    new="$(cd "$(dirname "$new")" && pwd)/$(basename "$new")"
    [ -f "$old" ] || fail "$old is missing (run download first)"
    # Where the user keeps it; the updater replaces this file.
    installed="$work/Applications/DeckPilot.AppImage"
    mkdir -p "$work/Applications"
    cp "$old" "$installed" && chmod +x "$installed"
    old_agent=("$installed" --agent)
    new_agent=("$installed" --agent)
    install_new() {
      cp "$new" "$installed.new" && chmod +x "$installed.new" && mv -f "$installed.new" "$installed"
      cmp -s "$new" "$installed" || fail "$installed was not replaced"
      echo "replaced $installed with $(basename "$new")"
      # The release's AppImage, for the downgrade check.
      cp "$old" "$work/old.AppImage" && chmod +x "$work/old.AppImage"
      old_agent=("$work/old.AppImage" --agent)
    }
    cycle AppImage
    ;;
  agents)
    old_agent=("${2:?usage: check-upgrade.sh agents <old agent> <new agent> <deckpilot-dev>}")
    new_agent=("${3:?}")
    dev_bin="${4:?}"
    install_new() { echo "(no package: the new agent is ${new_agent[*]})"; }
    cycle agents
    ;;
  *) fail "unknown kind $kind" ;;
esac
