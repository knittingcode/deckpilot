# Stream Deck behind a USB hub: resets and "USB present, HID missing"

The case this page is about: a Stream Deck MK.2 (`0fd9:0080`)
connected through a monitor's or dock's USB hub on Linux. The hub resets
the device, the kernel logs errors such as `-28`, and afterwards the
device shows up in `lsusb` while DeckPilot does not see it. Only
power-cycling the monitor brings it back.

The page separates what DeckPilot's code is known to do from what is only
suspected, and shows how to tell the explanations apart on your own
machine. Nothing here has been reproduced on the hardware of the original
report yet.

## Confirmed: what DeckPilot does with the device

These follow from the code (`crates/deckpilot-hid`, `apps/agent/src/devices.rs`,
`apps/agent/src/service.rs` and the `elgato-streamdeck` and `hidapi` crates)
as of 0.15.0.

1. **Discovery does no USB I/O.** Every 2 seconds the agent lists HID
   devices through `hidapi`'s Linux backend, which reads udev and sysfs.
   Listing a device opens nothing and sends no request. Only devices that
   have a `hidraw` node and a serial number are listed.
2. **Without a `hidraw` node, DeckPilot does nothing.** If the USB device
   is there but its HID interface was not bound, nothing is listed, opened
   or sent. Today DeckPilot also cannot tell you so: it has no state for
   "USB present, HID missing".
3. **DeckPilot does not reset USB devices.** It has no code that resets,
   re-binds or power-cycles a USB device. It does not use libusb or
   `USBDEVFS_RESET`. Its only USB traffic is HID reports through
   `/dev/hidrawN`: feature reports (such as brightness) go out as HID
   SET_REPORT/GET_REPORT control requests, key images as output reports.
4. **Opening itself sends nothing.** Opening the device is an `open()` of
   `/dev/hidrawN`. Right after a successful open the agent sends one
   brightness report, then images for all 15 keys.
5. **Reading does not busy-poll.** Each device has one thread that waits in
   `hid_read_timeout` with a 100 ms timeout (30 ms while key titles
   scroll).
6. **Any HID error counts as a disconnect.** The device is closed, and the
   deck keeps showing the last images.
7. **Reconnecting has no back-off.** The next rescan, within 2 seconds,
   opens the device again if it is still listed. There is no limit on the
   number of attempts and no flap detection.
   - A device that cannot be opened is tried every 2 seconds, and each
     distinct error is logged once.
   - A device that opens and then fails at once goes through open,
     brightness, a 15-key redraw, error and close, up to about once every
     2 seconds. Each round logs "device connected" and "device connection
     lost".
8. **A hung read cannot stall the agent.** Closing waits at most 3 seconds.

## Hypotheses (not proven)

- **H1, the most likely: DeckPilot is not the cause.**
  - Kernel error `-28` is `ENOSPC`. On xHCI controllers it usually means
    there is not enough bandwidth or controller resources when the device's
    configuration is set ("can't set config #1, error -28").
  - When that fails, the USB device exists but none of its interfaces is
    bound. No `usbhid` means no `hidraw`: "USB present, HID missing".
  - All of this happens while the kernel enumerates the device, before any
    program can use it. Power-cycling the monitor resets its internal hub,
    which fits.
  - A monitor hub that also carries a webcam, Ethernet and audio reserves
    periodic bandwidth for them on the same upstream link.
- **H2, a possible contributor (low): DeckPilot's reconnect loop.**
  - This applies only if the HID interface does come up but is marginal,
    for example when the first writes fail. Then the no-back-off loop
    (fact 7) adds a `usbhid` open and close, and a burst of 15 images,
    every 2 seconds.
  - On a hub that is already resetting, that could make resets more
    frequent. It cannot cause `-28` while the configuration is set.
- **H3 (low): USB autosuspend.** With autosuspend enabled for HID devices
  (for example by TLP or powertop), a reconnect loop lets the device
  suspend between close and reopen. Every resume then goes through the
  marginal hub. In normal use DeckPilot keeps the device open, so this
  applies only during such a loop.
- **H4 (to rule out): another program uses the deck.** The Elgato software
  under Wine, streamdeck-ui or OpenDeck may open and reset it themselves.
  DeckPilot does not detect other users of the device.

## Telling them apart

You need the agent's log and the kernel log for the same period, with
their timestamps.

1. **The agent's log** is in `~/.local/share/io.github.knittingcode.deckpilot/logs/agent.<date>.log`
   (or `$DECKPILOT_DATA_DIR/logs/`). Look for "device connected",
   "device connection lost" and "device disconnected".
2. **The kernel log:**

   ```sh
   sudo dmesg -T | grep -iE 'usb|hid|-28'
   # or, across reboots:
   journalctl -k --since "1 hour ago" | grep -iE 'usb|hid|-28'
   ```

3. **Compare the two:**
   - **H1:** the resets or `-28` errors start before any "device connected"
     line, or happen while the log shows no reconnects.
   - **H2:** "device connected" and "device connection lost" pairs about
     2 seconds apart, which line up with the kernel's resets.
   - H4 is checked in step 6, H3 in step 7.
4. **Run with the agent stopped.** Quit the agent from the tray (or with
   **Stop agent** in Settings), check that it is gone with
   `pgrep -x deckpilot-agent`, and use the computer normally for a while.
   - If the resets and `-28` errors continue, the cause is outside
     DeckPilot (H1 or H4).
   - If they stop and come back when the agent runs, H2 or H3 is likely.
5. **Check for "USB present, HID missing":**

   ```sh
   lsusb -d 0fd9:                  # the USB device is there
   ls /dev/hidraw*                 # is there a node for it?
   for d in /sys/class/hidraw/*; do
     echo "$d: $(grep HID_ID "$d/device/uevent")"
   done                            # 0FD9 means an Elgato HID interface
   lsusb -t                        # which driver each interface has (usbhid?)
   ```

   If `lsusb` lists the deck but no `hidraw` node has `HID_ID=0003:00000FD9:…`,
   the HID interface is missing.
6. **Look for other users of the deck** (H4):
   `sudo fuser -v /dev/hidraw*` while DeckPilot is stopped.
7. **Check autosuspend** (H3):
   ```sh
   for d in /sys/bus/usb/devices/*; do
     [ "$(cat "$d/idVendor" 2>/dev/null)" = 0fd9 ] && echo "$d: $(cat "$d/power/control")"
   done
   ```

   `auto` means autosuspend is on. Check the hub the deck is attached to
   as well (the parent entry of the deck's path, e.g. `1-2` for `1-2.3`).

## What helps

- Connect the Stream Deck directly to the computer, or to a hub with its
  own power supply, instead of through the monitor.
- Move the webcam or other high-bandwidth devices off the monitor's hub.
- After a reset that leaves the HID interface missing, unplug and replug
  the Stream Deck; power-cycle the monitor if that does not help.

When you report the problem, please include the agent's log and the
`dmesg` output for the same period, and say whether the resets continued
with the agent stopped.
