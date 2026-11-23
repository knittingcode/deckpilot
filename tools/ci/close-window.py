#!/usr/bin/env python3
"""Finds or closes a process's visible top-level X11 window titled DeckPilot.

    close-window.py <pid> find    exit 0 and print the window once mapped
    close-window.py <pid> close   send it WM_DELETE_WINDOW, as a window
                                  manager does for the close button

A pid of 0 takes any process's window of that title (an isolated session
where the app's process is not the one started, e.g. an AppImage's runtime).

Used where no window manager runs (Xvfb in CI): the app's top-level windows
are then children of the root window. GTK answers WM_DELETE_WINDOW with its
delete-event, which Tauri reports as the window's CloseRequested, the same as
the title bar's close button. Needs python3-xlib.
"""

import sys

from Xlib import X, display, protocol

TITLE = "DeckPilot"


def main() -> int:
    pid, mode = int(sys.argv[1]), sys.argv[2]
    d = display.Display()
    root = d.screen().root
    net_wm_pid = d.intern_atom("_NET_WM_PID")
    candidates = []
    for window in root.query_tree().children:
        try:
            if window.get_attributes().map_state != X.IsViewable:
                continue
            owner = window.get_full_property(net_wm_pid, X.AnyPropertyType)
            name = window.get_wm_name()
        except Exception:  # a window that went away meanwhile
            continue
        if isinstance(name, bytes):
            name = name.decode("utf-8", "replace")
        if pid and owner is not None and owner.value[0] != pid:
            continue
        if name == TITLE:
            candidates.append((window, owner is not None))
    # The process's own window first (_NET_WM_PID); by title only else.
    candidates.sort(key=lambda c: not c[1])
    if not candidates:
        return 1
    window, by_pid = candidates[0]
    how = "_NET_WM_PID and title" if by_pid else "title"
    if mode == "close":
        event = protocol.event.ClientMessage(
            window=window,
            client_type=d.intern_atom("WM_PROTOCOLS"),
            data=(32, [d.intern_atom("WM_DELETE_WINDOW"), X.CurrentTime, 0, 0, 0]),
        )
        window.send_event(event, event_mask=X.NoEventMask)
        d.flush()
        print(f"sent WM_DELETE_WINDOW to window 0x{window.id:x} '{TITLE}' (found by {how})")
    else:
        print(f"window 0x{window.id:x} '{TITLE}' of pid {pid} (found by {how})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
