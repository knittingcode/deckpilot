#!/usr/bin/env bash
# Tests the Linux packages' maintainer scripts (packaging/linux/postinst.sh
# and postrm.sh, the .deb postinst/postrm and the .rpm %post/%postun) without
# root, udev or a device: each runs as a copy whose sysfs and udev control
# paths point into a fixture tree, with a stand-in udevadm (and
# systemd-detect-virt) on PATH that records its arguments.
#
#   tools/ci/test-udev-scripts.sh [<work dir>]
#
# Checks: they always exit 0 (no udevadm, no running udev, a chroot, a
# failing udevadm); postinst reloads the rules, then triggers only the USB
# devices of vendor 0fd9 (and their hidraw nodes, through --parent-match
# and the usb/hidraw subsystem filter), never a global trigger; running it
# twice does the same twice; postrm only reloads. Needs python3 (to create
# the stand-in control socket).
# ok and bad never fail, so `check && ok || bad` is an if-then-else here.
# shellcheck disable=SC2015
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
work="${1:-$(mktemp -d "${TMPDIR:-/tmp}/udev-scripts-test.XXXXXX")}"
mkdir -p "$work"
work="$(cd "$work" && pwd)"
shell=$(command -v sh)
failures=0
ok() { echo "ok   $*"; }
bad() {
  echo "FAIL $*"
  failures=$((failures + 1))
}

# A fresh fixture: sysfs with an Elgato device (0fd9), another vendor's
# device and an interface (no idVendor); a control socket; stand-in tools.
fixture() {
  rm -rf "$work/case"
  mkdir -p "$work/case/sys/1-1" "$work/case/sys/1-2" "$work/case/sys/1-1:1.0" "$work/case/run" "$work/case/bin"
  echo 0fd9 > "$work/case/sys/1-1/idVendor"
  echo 0080 > "$work/case/sys/1-1/idProduct"
  echo 046d > "$work/case/sys/1-2/idVendor"
  python3 -c 'import socket, sys; socket.socket(socket.AF_UNIX).bind(sys.argv[1])' "$work/case/run/control"
  cat > "$work/case/bin/udevadm" << EOF
#!/bin/sh
echo "udevadm \$*" >> "$work/case/calls"
exit \${UDEVADM_STATUS:-0}
EOF
  cat > "$work/case/bin/systemd-detect-virt" << EOF
#!/bin/sh
exit \${CHROOT_STATUS:-1}
EOF
  chmod +x "$work/case/bin/udevadm" "$work/case/bin/systemd-detect-virt"
  # The one other tool the scripts use; the system's udevadm stays out of reach.
  ln -s "$(command -v cat)" "$work/case/bin/cat"
  : > "$work/case/calls"
}

# run <postinst|postrm> [<args>...]: the script with the fixture's paths,
# under a PATH of only the fixture's tools.
run() {
  local script="$1" copy="$work/case/$1.sh"
  shift
  sed -e "s#^usb_devices=.*#usb_devices=$work/case/sys#" \
    -e "s#^udev_control=.*#udev_control=$work/case/run/control#" \
    "$root/packaging/linux/$script.sh" > "$copy"
  grep -q "$work/case/run/control" "$copy" || {
    bad "$script.sh: no udev_control= line to point at the fixture"
    return 1
  }
  PATH="$work/case/bin" "$shell" "$copy" "$@"
}

calls() { cat "$work/case/calls"; }

expected_postinst="udevadm control --timeout=10 --reload-rules
udevadm trigger --action=change --subsystem-match=usb --subsystem-match=hidraw --parent-match=$work/case/sys/1-1"

echo "== postinst"
for args in configure "abort-upgrade 0.15.0" 1 2; do
  fixture
  # shellcheck disable=SC2086 # one word per maintainer script argument
  run postinst $args && status=0 || status=$?
  [ "$status" -eq 0 ] && [ "$(calls)" = "$expected_postinst" ] \
    && ok "postinst $args: reloads, then triggers only the 0fd9 device and its hidraw nodes" \
    || bad "postinst $args: status $status, calls: $(calls)"
done

fixture
run postinst configure && run postinst configure
[ "$(calls)" = "$expected_postinst
$expected_postinst" ] && ok "postinst twice: the same calls twice" || bad "postinst twice: $(calls)"

fixture
echo 0fd9 > "$work/case/sys/1-2/idVendor"
run postinst configure
triggers=$(grep '^udevadm trigger' "$work/case/calls" || true)
unscoped=$(echo "$triggers" | grep -v -E -e '--subsystem-match=usb --subsystem-match=hidraw --parent-match=[^ ]+/sys/1-[12]$' || true)
[ "$(echo "$triggers" | wc -l | tr -d ' ')" = 2 ] && [ -z "$unscoped" ] \
  && ok "postinst with two Elgato devices: one scoped trigger each, never a global one" \
  || bad "postinst with two Elgato devices: $(calls)"

fixture
rm "$work/case/sys/1-1/idVendor"
run postinst configure
[ "$(calls)" = "udevadm control --timeout=10 --reload-rules" ] && ok "postinst without an Elgato device: reload only" || bad "postinst without an Elgato device: $(calls)"

fixture
rm "$work/case/run/control"
run postinst configure && status=0 || status=$?
[ "$status" -eq 0 ] && [ -z "$(calls)" ] && ok "postinst without a running udev: nothing, status 0" || bad "postinst without a running udev: status $status, $(calls)"

fixture
CHROOT_STATUS=0 run postinst configure && status=0 || status=$?
[ "$status" -eq 0 ] && [ -z "$(calls)" ] && ok "postinst in a chroot: nothing, status 0" || bad "postinst in a chroot: status $status, $(calls)"

fixture
UDEVADM_STATUS=1 run postinst configure && status=0 || status=$?
[ "$status" -eq 0 ] && [ "$(calls)" = "udevadm control --timeout=10 --reload-rules" ] \
  && ok "postinst with a failing reload: no trigger, status 0" || bad "postinst with a failing reload: status $status, $(calls)"

fixture
rm "$work/case/bin/udevadm"
run postinst configure && status=0 || status=$?
[ "$status" -eq 0 ] && ok "postinst without udevadm: status 0" || bad "postinst without udevadm: status $status"

echo "== postrm"
for args in remove purge upgrade 0 1; do
  fixture
  run postrm "$args" && status=0 || status=$?
  [ "$status" -eq 0 ] && [ "$(calls)" = "udevadm control --timeout=10 --reload-rules" ] \
    && ok "postrm $args: reload only" || bad "postrm $args: status $status, calls: $(calls)"
done

fixture
UDEVADM_STATUS=1 run postrm remove && status=0 || status=$?
[ "$status" -eq 0 ] && ok "postrm with a failing reload: status 0" || bad "postrm with a failing reload: status $status"

fixture
rm "$work/case/run/control"
run postrm remove && status=0 || status=$?
[ "$status" -eq 0 ] && [ -z "$(calls)" ] && ok "postrm without a running udev: nothing, status 0" || bad "postrm without a running udev: status $status, $(calls)"

fixture
rm "$work/case/bin/udevadm"
run postrm remove && status=0 || status=$?
[ "$status" -eq 0 ] && ok "postrm without udevadm: status 0" || bad "postrm without udevadm: status $status"

rm -rf "$work/case"
if [ "$failures" -ne 0 ]; then
  echo "$failures check(s) failed"
  exit 1
fi
echo "all maintainer script checks passed"
