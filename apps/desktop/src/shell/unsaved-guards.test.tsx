// @vitest-environment jsdom
import { act, cleanup, renderHook, waitFor } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { DialogsContext } from "@/components/dialogs/use-dialogs"
import type { Dialogs } from "@/components/dialogs/dialog-service"
import { emitBackend, resetBackend } from "@/test/tauri"

import {
  CLOSE_REQUESTED_EVENT,
  QUIT_REQUESTED_EVENT,
  leavesEditor,
  useUnsavedGuards,
} from "./unsaved-guards"

const invoke = vi.hoisted(() =>
  vi.fn<(command: string, args?: unknown) => Promise<null>>(() => Promise.resolve(null)),
)
vi.mock("@tauri-apps/api/core", () => ({ invoke }))
vi.mock("@tauri-apps/api/event", async () => (await import("@/test/tauri")).eventModule)
const onCloseRequested = vi.hoisted(() => vi.fn(() => Promise.resolve(() => undefined)))
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ onCloseRequested }),
}))
vi.mock("sonner", () => ({ toast: { warning: vi.fn() } }))

const designer = { mode: "workspace", page: "designer" }

function setup(dirty: boolean, discard: boolean) {
  const destructive = vi.fn(() => Promise.resolve(discard))
  const dialogs = { destructive } as unknown as Dialogs
  const navigate = vi.fn()
  const setDirty = vi.fn()
  const dirtyRef = { current: dirty }
  const wrapper = ({ children }: { children: ReactNode }) => (
    <DialogsContext value={dialogs}>{children}</DialogsContext>
  )
  const hook = renderHook(() => useUnsavedGuards(dirtyRef, designer, navigate, setDirty), {
    wrapper,
  })
  return { hook, destructive, navigate, setDirty }
}

beforeEach(() => {
  resetBackend()
  invoke.mockClear()
  onCloseRequested.mockClear()
})
afterEach(() => {
  cleanup()
})

describe("unsaved edits are never discarded without asking", () => {
  it("knows when a page change leaves the editor", () => {
    expect(leavesEditor(designer, "workflows")).toBe(true)
    // The settings keep the editor mounted under them.
    expect(leavesEditor(designer, "settings:security")).toBe(false)
    expect(leavesEditor(designer, "designer")).toBe(false)
    expect(leavesEditor({ mode: "workspace", page: "workflows" }, "dashboard")).toBe(false)
  })

  it("leaves at once without unsaved edits", () => {
    const { hook, destructive, navigate } = setup(false, false)
    hook.result.current("workflows")
    expect(navigate).toHaveBeenCalledWith("workflows")
    expect(destructive).not.toHaveBeenCalled()
  })

  it("asks before leaving the editor, and stays when cancelled", async () => {
    const stay = setup(true, false)
    stay.hook.result.current("workflows")
    await waitFor(() => {
      expect(stay.destructive).toHaveBeenCalled()
    })
    expect(stay.navigate).not.toHaveBeenCalled()
    cleanup()
    const leave = setup(true, true)
    leave.hook.result.current("workflows")
    await waitFor(() => {
      expect(leave.navigate).toHaveBeenCalledWith("workflows")
    })
    expect(leave.setDirty).toHaveBeenCalledWith(false)
  })

  it("asks before quitting, and quits only when the edits are discarded", async () => {
    const stay = setup(true, false)
    await waitFor(() => {
      expect(emitBackend(QUIT_REQUESTED_EVENT, null)).toBe(1)
    })
    await waitFor(() => {
      expect(stay.destructive).toHaveBeenCalled()
    })
    expect(invoke).not.toHaveBeenCalledWith("quit_app")
    cleanup()
    resetBackend()
    setup(true, true)
    await waitFor(() => {
      expect(emitBackend(QUIT_REQUESTED_EVENT, null)).toBe(1)
    })
    await act(async () => {
      await Promise.resolve()
    })
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("quit_app")
    })
  })

  describe("closing the main window (the backend asks only over unsaved edits)", () => {
    it("leaves the close to the backend: no onCloseRequested listener", () => {
      // With one, Tauri holds back every close for the page's
      // window.destroy(), which the main window may not call.
      setup(false, false)
      setup(true, false)
      expect(onCloseRequested).not.toHaveBeenCalled()
    })

    it("asks over unsaved edits, and stays when cancelled", async () => {
      const stay = setup(true, false)
      await waitFor(() => {
        expect(emitBackend(CLOSE_REQUESTED_EVENT, null)).toBe(1)
      })
      await waitFor(() => {
        expect(stay.destructive).toHaveBeenCalled()
      })
      await act(async () => {
        await Promise.resolve()
      })
      expect(invoke).not.toHaveBeenCalledWith("close_main_window")
      expect(stay.setDirty).not.toHaveBeenCalled()
    })

    it("closes once the edits are discarded", async () => {
      const discard = setup(true, true)
      await waitFor(() => {
        expect(emitBackend(CLOSE_REQUESTED_EVENT, null)).toBe(1)
      })
      await waitFor(() => {
        expect(invoke).toHaveBeenCalledWith("close_main_window")
      })
      expect(discard.destructive).toHaveBeenCalledTimes(1)
      expect(discard.setDirty).toHaveBeenCalledWith(false)
    })

    it("closes at once when the edits were saved meanwhile", async () => {
      const clean = setup(false, false)
      await waitFor(() => {
        expect(emitBackend(CLOSE_REQUESTED_EVENT, null)).toBe(1)
      })
      await waitFor(() => {
        expect(invoke).toHaveBeenCalledWith("close_main_window")
      })
      expect(clean.destructive).not.toHaveBeenCalled()
    })
  })
})
