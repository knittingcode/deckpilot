#!/usr/bin/env bash
# Installs a freshly built Linux package on a clean system, with only the
# dependencies the package declares, and starts its agent:
#
#   tools/ci/check-linux-install.sh deb <package.deb>   (Ubuntu/Debian runner, sudo)
#   tools/ci/check-linux-install.sh rpm <package.rpm>   (any host with docker: Fedora container)
#   tools/ci/check-linux-install.sh appimage <file.AppImage>
#   tools/ci/check-linux-install.sh agent <agent> [<args>...]
#   tools/ci/check-linux-install.sh without-tray <agent>
#   tools/ci/check-linux-install.sh deb-lifecycle <package.deb>   (after `deb`, sudo)
#   tools/ci/check-linux-install.sh deb-container <package.deb>   (any host with docker: Ubuntu container)
#
# The agent's tray loads libayatana-appindicator3 at run time; a package
# that does not declare it installs fine and then has an agent that cannot
# start (DeckPilot 0.7.0's .deb and .rpm). So the check is not the
# installation alone but the installed agent running.
#
# `agent`: starts <agent> (with <args>) under Xvfb in a private D-Bus
# session with an isolated profile (HOME, XDG directories and
# DECKPILOT_DATA_DIR in a temporary directory), as a desktop session would,
# and fails unless it is still running after 15 s and has not reported a
# missing tray library. Used by the other commands.
#
# `appimage`: the agent of an AppImage, started as the app starts it
# (`<AppImage> --agent`, through FUSE). The AppImage bundles the tray
# library with the GLib it was built against: the system's copy cannot be
# loaded next to the AppImage's GLib (DeckPilot 0.7.0 on Ubuntu 24.04:
# "libayatana-ido3-0.4.so.0: undefined symbol: g_once_init_leave_pointer").
#
# `without-tray`: on a system without the tray library, the agent must keep
# running without a tray icon (and say so in its log).
#
# The udev rule and the maintainer scripts that reload udev
# (packaging/linux): `deb` and `rpm` check that the installed rule and
# scripts are the repository's; `deb-lifecycle` then checks that the
# installed postinst reloads udev on the runner, where udev runs, and
# reinstalls, removes and purges the .deb there; `rpm` reinstalls and removes the
# .rpm in a container with udevadm but no running udev, and
# `deb-container` installs, reinstalls, removes and purges the .deb in a
# container without udev. Each step must succeed, the rule must be there
# after the (re)install and gone after the removal.
#
# Uninstalling (`deb-lifecycle`, `deb-container`, `rpm`) also checks:
# - every file the package installed is gone after the removal (.rpm) or
#   the purge (.deb);
# - the removal's postrm (%postun) ran without error. A `udevadm` that
#   records its calls is put in front of the real one. With udev running
#   (the runner), the postrm must have reloaded udev's rules, and every
#   udevadm call must have succeeded. Without a running udev (containers),
#   it must not have called udevadm at all;
# - a user's DeckPilot files are untouched: a profile made beforehand in
#   the home directory (the data directory with a database, logs and a
#   plugin, and an autostart entry) is the same afterwards, file by file,
#   with the same modes and contents (docs/background-agent.md, "What
#   uninstalling leaves behind").
#
# Environment: FEDORA_IMAGE (rpm; default fedora:42), UBUNTU_IMAGE
# (deb-container; default ubuntu:24.04).
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
fail() { echo "::error::$*" >&2; exit 1; }

# Running, not a zombie.
alive() {
  local state
  state=$(sed -n 's/^State:[[:space:]]*\([A-Z]\).*/\1/p' "/proc/$1/status" 2> /dev/null || true)
  [ -n "$state" ] && [ "$state" != Z ]
}

agent() {
  local agent="$1" profile log pid expect_no_tray="${EXPECT_NO_TRAY:-}"
  shift
  profile=$(mktemp -d "${TMPDIR:-/tmp}/deckpilot-install-check.XXXXXX")
  mkdir -p "$profile"/{home,data,run,xdg-data,xdg-config,xdg-cache}
  chmod 700 "$profile/run" "$profile/data"
  log="$profile/agent.log"
  # The agent's own log is in the data directory; its stderr (panics,
  # library errors) in agent.log.
  env HOME="$profile/home" XDG_DATA_HOME="$profile/xdg-data" XDG_CONFIG_HOME="$profile/xdg-config" \
    XDG_CACHE_HOME="$profile/xdg-cache" XDG_RUNTIME_DIR="$profile/run" DECKPILOT_DATA_DIR="$profile/data" \
    xvfb-run -a dbus-run-session -- "$agent" "$@" > "$log" 2>&1 &
  local wrapper=$!
  pid=""
  for _ in $(seq 1 30); do
    sleep 0.5
    pid=$(pgrep -n -x deckpilot-agent || true)
    [ -n "$pid" ] && break
  done
  sleep 15
  local status=0
  if [ -z "$pid" ] || ! alive "$pid"; then
    echo "::error::the agent did not keep running; its output:" >&2
    status=1
  elif [ -n "$expect_no_tray" ]; then
    if ! cat "$log" "$profile"/data/logs/* 2> /dev/null | grep -q 'running without a tray icon'; then
      echo "::error::the agent did not report that it runs without a tray icon:" >&2
      status=1
    fi
  elif cat "$log" "$profile"/data/logs/* 2> /dev/null | grep -q -E 'Failed to load ayatana-appindicator3|undefined symbol|running without a tray icon'; then
    echo "::error::the agent could not load its tray library:" >&2
    status=1
  fi
  sed 's/^/  | /' "$log" >&2
  for file in "$profile"/data/logs/*; do
    [ -f "$file" ] && sed 's/^/  log | /' "$file" | tail -n 20 >&2
  done
  pkill -x deckpilot-agent 2> /dev/null || true
  kill "$wrapper" 2> /dev/null || true
  wait "$wrapper" 2> /dev/null || true
  rm -rf "$profile"
  [ "$status" -eq 0 ] || exit 1
  echo "the agent ($agent $*) started and kept running for 15 s${expect_no_tray:+, without a tray icon}"
}

rule=/usr/lib/udev/rules.d/70-deckpilot-streamdeck.rules

# The installed rule is the repository's; udevadm verify (systemd 254 and
# later) accepts it.
installed_rule() {
  [ -f "$rule" ] || fail "$rule is not installed"
  cmp -s "$rule" "$root/packaging/linux/70-deckpilot-streamdeck.rules" || fail "$rule is not packaging/linux's"
  if udevadm verify --help > /dev/null 2>&1; then
    udevadm verify --resolve-names=never "$rule" || fail "udevadm verify refuses $rule"
  fi
  echo "the udev rule is installed: $rule"
}

# The .deb's maintainer scripts as dpkg keeps them for package <name>.
deb_scripts() {
  local script
  for script in postinst postrm; do
    cmp -s "/var/lib/dpkg/info/$1.$script" "$root/packaging/linux/$script.sh" \
      || fail "/var/lib/dpkg/info/$1.$script is not packaging/linux/$script.sh"
  done
  echo "dpkg keeps the postinst and postrm of $1"
}

# fake_profile <home>: a user's DeckPilot files, as an installed app
# leaves them: the data directory (a database, backups, logs, a plugin)
# and a start-at-login entry. Refuses a home that has a data directory
# already: it never writes over real data.
data_id=io.github.knittingcode.deckpilot
fake_profile() {
  local home="$1" data="$1/.local/share/$data_id"
  [ ! -e "$data" ] || fail "$data exists already: not a test profile"
  [ ! -e "$home/.config/autostart/DeckPilot Agent.desktop" ] \
    || fail "$home/.config/autostart/DeckPilot Agent.desktop exists already: not a test profile"
  mkdir -p "$data/logs" "$data/backups" "$data/plugins/installed/example.plugin" "$home/.config/autostart"
  printf 'SQLite format 3\0 (a stand-in for the encrypted database)\n' > "$data/deckpilot.db"
  printf 'a backup\n' > "$data/backups/deckpilot.backup-1.db"
  printf 'INFO agent started\n' > "$data/logs/agent.2026-01-01.log"
  printf 'panic: thread="main"\n' > "$data/logs/output.log"
  printf '{"id":"example.plugin"}\n' > "$data/plugins/installed/example.plugin/manifest.json"
  printf '[Desktop Entry]\nType=Application\nName=DeckPilot Agent\nExec=/usr/bin/deckpilot-agent\n' \
    > "$home/.config/autostart/DeckPilot Agent.desktop"
  chmod 700 "$data"
  chmod 600 "$data/deckpilot.db" "$data/logs/output.log"
}

# profile_state <home>: every entry of the profile with its type, mode,
# size, modification time and, for files, contents' checksum.
profile_state() {
  (
    cd "$1"
    find ".local/share/$data_id" .config/autostart -printf '%y %m %s %T@ %p\n' | LC_ALL=C sort
    find ".local/share/$data_id" .config/autostart -type f -exec sha256sum {} + | LC_ALL=C sort -k 2
  )
}

# profile_untouched <home> <state before> <after what>
profile_untouched() {
  local now
  now=$(profile_state "$1")
  [ "$now" = "$2" ] || fail "the user's files in $1 changed $3: $(diff <(echo "$2") <(echo "$now") || true)"
  echo "the user's DeckPilot files in $1 are untouched $3"
}

# udevadm_recorder <real udevadm> <recorder> <log>: writes <recorder>, a
# udevadm that runs the real one and appends each call with its exit
# status to <log>.
udevadm_recorder() {
  local real="$1" recorder="$2" log="$3"
  mkdir -p "$(dirname "$recorder")"
  : > "$log"
  cat > "$recorder" << EOF
#!/bin/sh
"$real" "\$@"
status=\$?
echo "udevadm \$* -> \$status" >> "$log"
exit \$status
EOF
  chmod 755 "$recorder"
}

# postrm_checked <log> <what>: what the recorder saw during <what>.
postrm_checked() {
  local log="$1" what="$2"
  if grep -q -v -E -- '-> 0$' "$log"; then
    fail "a udevadm call failed during $what: $(cat "$log")"
  fi
  if [ -S /run/udev/control ]; then
    grep -q -x -F -- 'udevadm control --timeout=10 --reload-rules -> 0' "$log" \
      || fail "the postrm did not reload udev during $what: $(cat "$log")"
    echo "$what: the postrm reloaded udev's rules without error"
  else
    [ ! -s "$log" ] || fail "without a running udev, the postrm called udevadm during $what: $(cat "$log")"
    echo "$what: without a running udev, the postrm called nothing and succeeded"
  fi
}

# gone <list of files> <after what>: none of the package's files is left.
gone() {
  local path left=""
  while IFS= read -r path; do
    [ -n "$path" ] || continue
    if [ -f "$path" ] || [ -L "$path" ]; then left="$left $path"; fi
  done <<< "$1"
  [ -z "$left" ] || fail "files of the package are left $2:$left"
  echo "every file of the package is gone $2"
}

# deb_lifecycle <package.deb> [sudo]: reinstall, remove, purge, with a
# user's profile in this user's home ($HOME) that must stay as it is.
deb_lifecycle() {
  local package="$1" as="${2-}" name apt_get files before work log path
  name=$(dpkg-deb -f "$package" Package)
  work=$(mktemp -d "${TMPDIR:-/tmp}/deckpilot-uninstall.XXXXXX")
  log="$work/udevadm.log"
  path=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
  if command -v udevadm > /dev/null; then
    udevadm_recorder "$(command -v udevadm)" "$work/bin/udevadm" "$log"
    path="$work/bin:$path"
  else
    log=""
    echo "no udevadm: the postrm has nothing to call"
  fi
  # apt-get runs dpkg, and so the maintainer scripts, with DPkg::Path
  # (default /usr/sbin:/usr/bin:/sbin:/bin), not with its own PATH.
  apt_get="$as env PATH=$path DEBIAN_FRONTEND=noninteractive apt-get -y -q --no-install-recommends -o DPkg::Path=$path -o Acquire::Retries=3 -o Acquire::http::Timeout=30 -o Acquire::https::Timeout=30"
  $apt_get install --reinstall "$package" || fail "reinstalling $package failed"
  # With udev running the postinst reloads it: the recorder must have
  # seen that, or it is not in the scripts' way and proves nothing.
  if [ -n "$log" ] && [ -S /run/udev/control ]; then
    grep -q -F -- '--reload-rules' "$log" || fail "the udevadm recorder saw no call from the postinst: $(cat "$log")"
    echo "the maintainer scripts call the udevadm recorder"
  fi
  installed_rule
  deb_scripts "$name"
  files=$(dpkg-query -L "$name")
  fake_profile "$HOME"
  before=$(profile_state "$HOME")
  [ -z "$log" ] || : > "$log"
  $apt_get remove "$name" || fail "removing $name failed"
  [ ! -e "$rule" ] || fail "$rule is still there after removing $name"
  # The package has a postrm, so dpkg keeps it as config-files until the
  # purge, which runs `postrm purge`.
  [ "$(dpkg-query -W -f '${db:Status-Status}' "$name" 2> /dev/null || true)" = config-files ] \
    || fail "$name is not in the config-files state after its removal"
  [ -z "$log" ] || postrm_checked "$log" "the removal of $name"
  profile_untouched "$HOME" "$before" "after the removal"
  [ -z "$log" ] || : > "$log"
  $apt_get purge "$name" || fail "purging $name failed"
  case "$(dpkg-query -W -f '${db:Status-Status}' "$name" 2> /dev/null || true)" in
    "" | not-installed) ;;
    *) fail "$name is not purged" ;;
  esac
  [ -z "$log" ] || postrm_checked "$log" "the purge of $name"
  gone "$files" "after the purge"
  profile_untouched "$HOME" "$before" "after the purge"
  rm -rf "$HOME/.local/share/$data_id" "$HOME/.config/autostart/DeckPilot Agent.desktop" "$work"
  echo "$name: reinstalled, removed and purged; the udev rule and the package's files went with it, the user's files stayed"
}

case "${1-}" in
  deb-lifecycle)
    package="${2:?usage: check-linux-install.sh deb-lifecycle <package.deb>}"
    package="$(cd "$(dirname "$package")" && pwd)/$(basename "$package")"
    name=$(dpkg-deb -f "$package" Package)
    if [ -S /run/udev/control ]; then
      # With udev running, the installed postinst gets as far as the
      # reload (and the scoped trigger loop; this runner has no Elgato
      # device, test-udev-scripts.sh covers the trigger).
      trace=$(sudo sh -x "/var/lib/dpkg/info/$name.postinst" configure 2>&1) || fail "the postinst failed: $trace"
      grep -q -F -- '--reload-rules' <<< "$trace" || fail "the postinst did not reload udev with udev running: $trace"
      echo "with udev running, the postinst reloads its rules"
    else
      echo "::warning::udev does not run on this runner: the postinst's reload is not exercised"
    fi
    deb_lifecycle "$package" sudo
    ;;
  deb-container)
    package="${2:?usage: check-linux-install.sh deb-container <package.deb>}"
    dir="$(cd "$(dirname "$package")" && pwd)"
    name="$(basename "$package")"
    image="${UBUNTU_IMAGE:-ubuntu:24.04}"
    docker run --rm -v "$dir:/packages:ro" -v "$root:/repo:ro" "$image" bash -euo pipefail -c "
      export DEBIAN_FRONTEND=noninteractive
      apt-get update -q -o Acquire::http::Timeout=30 -o Acquire::https::Timeout=30 > /dev/null
      apt-get install -y -q --no-install-recommends -o Acquire::Retries=3 \"/packages/\$1\" > /dev/null
      # udevadm without a running udev, as on a system in a container:
      # the scripts must find that and call nothing.
      apt-get install -y -q --no-install-recommends -o Acquire::Retries=3 udev > /dev/null
      if command -v udevadm > /dev/null; then echo 'udevadm is installed, udev does not run'; else echo 'no udevadm'; fi
      /repo/tools/ci/check-linux-install.sh deb-installed \"/packages/\$1\"
    " _ "$name"
    ;;
  deb-installed)
    # Inside deb-container, as root.
    package="${2:?}"
    installed_rule
    deb_scripts "$(dpkg-deb -f "$package" Package)"
    deb_lifecycle "$package"
    ;;
  agent)
    shift
    [ "$#" -ge 1 ] || fail "usage: check-linux-install.sh agent <agent> [<args>...]"
    agent "$@"
    ;;
  appimage)
    file="${2:?usage: check-linux-install.sh appimage <file.AppImage>}"
    file="$(cd "$(dirname "$file")" && pwd)/$(basename "$file")"
    chmod +x "$file"
    # What the AppImage carries for the tray, and its GLib.
    extracted=$(mktemp -d)
    (cd "$extracted" && "$file" --appimage-extract > /dev/null)
    echo "the AppImage bundles: $(cd "$extracted/squashfs-root" && find usr/lib -maxdepth 1 \( -name 'libayatana-*' -o -name 'libdbusmenu-*' -o -name 'libglib-2.0.so*' \) | sort | tr '\n' ' ')"
    rm -rf "$extracted"
    if ldconfig -p | grep -q 'libayatana-appindicator3\.so\.1 '; then
      echo "the system has its own libayatana-appindicator3: $(ldconfig -p | grep 'libayatana-appindicator3\.so\.1 ' | head -n 1)"
    else
      echo "the system has no libayatana-appindicator3"
    fi
    agent "$file" --agent
    ;;
  without-tray)
    [ -n "${2-}" ] || fail "usage: check-linux-install.sh without-tray <agent>"
    ! ldconfig -p | grep -q -E 'lib(ayatana-)?appindicator3\.so' \
      || fail "an appindicator library is installed: this check needs a system without one"
    EXPECT_NO_TRAY=1 agent "$2"
    ;;
  deb)
    package="${2:?usage: check-linux-install.sh deb <package.deb>}"
    package="$(cd "$(dirname "$package")" && pwd)/$(basename "$package")"
    echo "Depends: $(dpkg-deb -f "$package" Depends)"
    if ldconfig -p | grep -q 'libayatana-appindicator3\.so\.1 '; then
      echo "::warning::libayatana-appindicator3 is installed already: this check does not show that the package pulls it in"
    fi
    sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -q --no-install-recommends -o Acquire::Retries=3 -o Acquire::http::Timeout=30 -o Acquire::https::Timeout=30 "$package"
    # The test's own tools (not dependencies of the package).
    sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -q --no-install-recommends -o Acquire::Retries=3 -o Acquire::http::Timeout=30 -o Acquire::https::Timeout=30 xvfb xauth dbus
    ldconfig -p | grep -q 'libayatana-appindicator3\.so\.1 ' \
      || fail "libayatana-appindicator3.so.1 is not installed after installing $package with its dependencies"
    installed_rule
    deb_scripts "$(dpkg-deb -f "$package" Package)"
    agent /usr/bin/deckpilot-agent
    ;;
  rpm)
    package="${2:?usage: check-linux-install.sh rpm <package.rpm>}"
    dir="$(cd "$(dirname "$package")" && pwd)"
    name="$(basename "$package")"
    image="${FEDORA_IMAGE:-fedora:42}"
    docker run --rm -v "$dir:/packages:ro" -v "$root:/repo:ro" "$image" bash -euo pipefail -c "
      rpm -qp --requires /packages/$name | sed 's/^/Requires: /'
      dnf -y -q install /packages/$name
      ldconfig -p | grep -q 'libayatana-appindicator3\.so\.1 ' \
        || { echo '::error::libayatana-appindicator3.so.1 is not installed after installing $name with its dependencies'; exit 1; }
      # The test's own tools (not dependencies of the package), and
      # udevadm without a running udev (a container): the reinstall's and
      # removal's scriptlets must find that and do nothing.
      dnf -y -q install xorg-x11-server-Xvfb xorg-x11-xauth dbus-daemon procps-ng which diffutils systemd-udev
      /repo/tools/ci/check-linux-install.sh rpm-installed \"/packages/\$1\"
    " _ "$name"
    ;;
  rpm-installed)
    # Inside the rpm container, as root.
    package="${2:?}"
    pkgname=$(rpm -qp --queryformat '%{NAME}' "$package")
    installed_rule
    # With this rpm, which knows every scriptlet tag: exactly the
    # repository's %post and %postun, nothing else.
    "$here/check-rpm-scriptlets.sh" "$package" "$root/packaging/linux/postinst.sh" "$root/packaging/linux/postrm.sh"
    "$here/check-linux-install.sh" agent /usr/bin/deckpilot-agent
    dnf -y -q reinstall "$package" || fail "reinstalling $package failed"
    installed_rule
    files=$(rpm -ql "$pkgname")
    # rpm runs scriptlets with a PATH of its own, so the recorder takes
    # the real udevadm's place (this container is thrown away). Without a
    # running udev the %postun stops before udevadm, so this shows that
    # it called nothing and succeeded; the reload itself is checked on
    # the .deb runner, where udev runs.
    real=$(command -v udevadm) || fail "no udevadm in the container"
    mv "$real" "$real.real"
    log=$(mktemp)
    udevadm_recorder "$real.real" "$real" "$log"
    # Root's home and a user's.
    mkdir -p /home/checker
    fake_profile /root
    fake_profile /home/checker
    before_root=$(profile_state /root)
    before_user=$(profile_state /home/checker)
    output=$(dnf -y remove "$pkgname" 2>&1) || fail "removing $pkgname failed: $output"
    echo "$output" | tail -n 5
    ! grep -q -i 'scriptlet failed' <<< "$output" || fail "a scriptlet failed while removing $pkgname: $output"
    [ ! -e "$rule" ] || fail "$rule is still there after removing $pkgname"
    ! rpm -q "$pkgname" > /dev/null 2>&1 || fail "$pkgname is still installed"
    postrm_checked "$log" "the removal of $pkgname"
    gone "$files" "after the removal"
    profile_untouched /root "$before_root" "after the removal"
    profile_untouched /home/checker "$before_user" "after the removal"
    echo "$pkgname: installed, reinstalled and removed; the udev rule and the package's files went with it, the user's files stayed"
    ;;
  *) fail "usage: check-linux-install.sh deb|rpm|deb-lifecycle|deb-container <package> | agent <agent> [<args>...]" ;;
esac
