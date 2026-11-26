#!/bin/sh
# DeckPilot .deb postinst and .rpm %post (apps/desktop/src-tauri/tauri.conf.json).
#
# Makes the package's udev rule (/usr/lib/udev/rules.d/70-deckpilot-streamdeck.rules)
# take effect without unplugging the Stream Deck: reloads udev's rules,
# then re-runs them for USB devices of Elgato's vendor ID (0fd9) and their
# hidraw nodes only. Never a global trigger: no other device sees an event.
#
# Runs as root from dpkg or rpm, with any arguments (deb: configure,
# abort-upgrade, ...; rpm: the instance count); they do not change what it
# does, and running it again does the same again. It never fails the
# installation: without udevadm, without a running udev (containers,
# image builds) or in a chroot it does nothing, and a failing udevadm is
# ignored. Writes nothing and starts nothing; the agent is never run here.

# Overridden only by tools/ci/test-udev-scripts.sh, on a copy of this file.
usb_devices=/sys/bus/usb/devices
udev_control=/run/udev/control

command -v udevadm > /dev/null 2>&1 || exit 0
if command -v systemd-detect-virt > /dev/null 2>&1 && systemd-detect-virt --quiet --chroot; then
  exit 0
fi
[ -S "$udev_control" ] || exit 0
udevadm control --reload-rules > /dev/null 2>&1 || exit 0

for device in "$usb_devices"/*; do
  [ -r "$device/idVendor" ] || continue
  vendor=$(cat "$device/idVendor" 2> /dev/null) || continue
  [ "$vendor" = 0fd9 ] || continue
  # The device and the nodes below it (its interfaces, their hidraw
  # nodes), restricted to the two subsystems the rule matches.
  udevadm trigger --action=change --subsystem-match=usb --subsystem-match=hidraw \
    --parent-match="$device" > /dev/null 2>&1 || true
done
exit 0
