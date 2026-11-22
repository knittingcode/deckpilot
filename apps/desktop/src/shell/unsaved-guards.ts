// Unsaved layout edits are never discarded without asking
// (docs/editor-navigation-and-simulation.md, section 3): leaving the
// device editor for another workspace page, closing the main window
// (Windows, Linux: it ends the editor; macOS hides it and keeps it) and
// quitting the app (⌘Q). The device's live preview ends with the editor,
// and the device shows the saved configuration again.

import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { type RefObject, useCallback, useContext, useEffect } from "react"
import { toast } from "sonner"

import { DialogsContext } from "@/components/dialogs/use-dialogs"
import { i18n } from "@/i18n"
import { locationOf } from "@/lib/navigation"

import { unsavedWarning } from "./editor-selection-context"

/** The backend's event asking to confirm quitting (`commands/unsaved.rs`). */
export const QUIT_REQUESTED_EVENT = "quit-requested"

/** The backend's event asking to confirm closing the main window (`commands/unsaved.rs`). */
export const CLOSE_REQUESTED_EVENT = "main-close-requested"

/** The page the device editor is. */
const DESIGNER = "designer"

/** Whether going to `page` from `from` leaves the device editor. */
export function leavesEditor(from: { mode: string; page: string }, page: string): boolean {
  const to = locationOf(page)
  return (
    from.mode === "workspace" &&
    from.page === DESIGNER &&
    to.mode === "workspace" &&
    to.page !== DESIGNER
  )
}

/** Tells the app's backend whether there are unsaved edits (quitting asks then). */
export function reportUnsaved(unsaved: boolean) {
  void invoke("set_unsaved_edits", { unsaved }).catch(() => undefined)
}

/**
 * `navigate` that asks before leaving the device editor with unsaved
 * edits, and the close and quit guards of the main window.
 */
export function useUnsavedGuards(
  dirtyRef: RefObject<boolean>,
  location: { mode: string; page: string },
  navigate: (page: string) => void,
  setDirty: (dirty: boolean) => void,
) {
  const dialogs = useContext(DialogsContext)
  const confirm = useCallback(async () => {
    if (!dirtyRef.current) return true
    if (!dialogs) {
      toast.warning(unsavedWarning())
      return false
    }
    const discard = await dialogs.destructive({
      title: i18n.t("editor:editing.discard.title"),
      description: i18n.t("editor:editing.discard.description"),
      confirmLabel: i18n.t("editor:editing.discard.confirm"),
    })
    if (discard) setDirty(false)
    return discard
  }, [dirtyRef, dialogs, setDirty])

  const guarded = useCallback(
    (page: string) => {
      if (!dirtyRef.current || !leavesEditor(location, page)) {
        navigate(page)
        return
      }
      void confirm().then((ok) => {
        if (ok) navigate(page)
      })
    },
    [confirm, dirtyRef, location, navigate],
  )

  // Closing the main window over unsaved edits (Windows, Linux: it ends the
  // editor; macOS hides it and keeps it). The backend decides
  // (`lifecycle::on_window_event`): without unsaved edits the window closes
  // at once, with them it stays and asks here. No `onCloseRequested`
  // listener: with one Tauri holds back every close for the page's
  // `window.destroy()`, which this window may not call (0.10.0 to 0.12.0
  // the close button did nothing).
  useEffect(() => {
    let stop: (() => void) | undefined
    let stopped = false
    listen(CLOSE_REQUESTED_EVENT, () => {
      void confirm().then((ok) => {
        if (ok) void invoke("close_main_window").catch(() => undefined)
      })
    }).then(
      (unlisten) => {
        if (stopped) unlisten()
        else stop = unlisten
      },
      () => undefined,
    )
    return () => {
      stopped = true
      stop?.()
    }
  }, [confirm])

  // ⌘Q (and quitting from the app menu) over unsaved edits.
  useEffect(() => {
    let stop: (() => void) | undefined
    let stopped = false
    listen(QUIT_REQUESTED_EVENT, () => {
      void confirm().then((ok) => {
        if (ok) void invoke("quit_app").catch(() => undefined)
      })
    }).then(
      (unlisten) => {
        if (stopped) unlisten()
        else stop = unlisten
      },
      () => undefined,
    )
    return () => {
      stopped = true
      stop?.()
    }
  }, [confirm])

  return guarded
}
