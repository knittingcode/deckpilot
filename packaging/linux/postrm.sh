#!/bin/sh
# DeckPilot .deb postrm and .rpm %postun (apps/desktop/src-tauri/tauri.conf.json).
#
# Reloads udev's rules after the package's rule
# (/usr/lib/udev/rules.d/70-deckpilot-streamdeck.rules) was removed or
# replaced. Triggers nothing: a connected Stream Deck keeps its current
# access until it is unplugged.
#
# Runs as root from dpkg or rpm, with any arguments (deb: remove, purge,
# upgrade, ...; rpm: the instance count); they do not change what it does,
# and running it again does the same again. It never fails the removal:
# without udevadm, without a running udev (containers, image builds) or in
# a chroot it does nothing, and a failing udevadm is ignored. It removes
# nothing: user data and autostart entries are not touched.

# Overridden only by tools/ci/test-udev-scripts.sh, on a copy of this file.
udev_control=/run/udev/control

command -v udevadm > /dev/null 2>&1 || exit 0
if command -v systemd-detect-virt > /dev/null 2>&1 && systemd-detect-virt --quiet --chroot; then
  exit 0
fi
[ -S "$udev_control" ] || exit 0
udevadm control --reload-rules > /dev/null 2>&1 || true
exit 0
