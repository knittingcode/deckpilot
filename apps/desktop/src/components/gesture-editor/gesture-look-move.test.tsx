// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

// Removing a gesture whose look has labels, when the binding shown after
// has a look of its own: the dialog, and each choice, end to end.

import { DndContext } from "@dnd-kit/core"
import { act, cleanup, render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { type ReactNode, useEffect, useState } from "react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { decodeTarget, encodeTarget } from "@/layout-editor/targets"
import type { KeyLabel } from "@/lib/key-labels"
import type { ControlBinding, ControlId, DeviceLayout, KeyAppearance } from "@/lib/types"
import { installDomStubs } from "@/test/dom"
import { resetToasts, toast } from "@/test/toast"

import { GestureEditor, type GestureTargetGroup } from "./gesture-editor"

vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)

class FixedResizeObserver {
  callback: ResizeObserverCallback
  constructor(callback: ResizeObserverCallback) {
    this.callback = callback
  }
  observe() {
    const entry = { contentRect: { width: 1200, height: 200 } }
    this.callback([entry as unknown as ResizeObserverEntry], this)
  }
  unobserve() {}
  disconnect() {}
}

const KEY: ControlId = { kind: "key", index: 1 }
const OTHER: ControlId = { kind: "key", index: 4 }

const profile = {
  actions: [
    { id: "shop", name: "Shop", actionType: "general.log", parameters: {}, icon: null },
    { id: "mail", name: "Mail", actionType: "general.log", parameters: {}, icon: null },
  ],
  workflows: [],
  projects: [],
} as unknown as Parameters<typeof GestureEditor>[0]["profile"]

const groups: GestureTargetGroup[] = [
  {
    label: "Actions",
    options: [
      { value: "action:shop", label: "Shop", target: { type: "action", action: "shop" } },
      { value: "action:mail", label: "Mail", target: { type: "action", action: "mail" } },
    ],
  },
]

const label = (id: string): KeyLabel => ({ id, text: id.toUpperCase() })
const labels = (...ids: string[]) => ids.map(label)

/** Key 2: a single press with `clickLook`, a long press with `ownLook`. */
function layoutWith(clickLook: KeyAppearance, ownLook: KeyAppearance): DeviceLayout {
  const bindings: ControlBinding[] = [
    {
      control: KEY,
      trigger: { type: "click" },
      target: { type: "action", action: "shop" },
      label: "Shop",
      appearance: clickLook,
    },
    {
      control: KEY,
      trigger: { type: "longPress" },
      target: { type: "action", action: "mail" },
      label: null,
      appearance: ownLook,
    },
  ]
  return {
    id: "layout",
    name: "Main",
    model: "elgato.streamdeck.mk2",
    homePage: "home",
    pages: [{ id: "home", name: "Home", bindings }],
  }
}

const moving: KeyAppearance = { background: "#112233", labels: labels("a", "b") }
const own: KeyAppearance = { background: "#ffffff", labels: labels("c") }

/** The editor's draft, as the harness holds it, and a way to edit it from outside. */
const state = {
  draft: null as DeviceLayout | null,
  edit: null as ((l: DeviceLayout) => void) | null,
}

function Harness({ initial }: { initial: DeviceLayout }): ReactNode {
  const [draft, setDraft] = useState(initial)
  useEffect(() => {
    state.draft = draft
    state.edit = setDraft
  }, [draft])
  return (
    <DndContext>
      <GestureEditor
        draft={draft}
        pageId="home"
        control={KEY}
        profile={profile}
        groups={groups}
        encode={encodeTarget}
        decode={decodeTarget}
        types={[]}
        defaults={{ doubleClickMs: 300, longPressMs: 700 }}
        issues={[]}
        onChange={(next) => {
          setDraft(next)
        }}
        // As the editor's `commands.update`: the edit applies to the latest draft.
        onUpdate={(edit) => {
          setDraft((latest) => edit(latest))
        }}
      />
    </DndContext>
  )
}

const keyBindings = () =>
  state.draft?.pages[0]?.bindings.filter((b) => b.control.index === KEY.index) ?? []

async function removeSinglePress(initial: DeviceLayout) {
  const user = userEvent.setup()
  render(<Harness initial={initial} />)
  await user.click(screen.getByRole("button", { name: "Remove Single press" }))
  return user
}

beforeAll(() => {
  installDomStubs()
})
beforeEach(() => {
  state.draft = null
  state.edit = null
  resetToasts()
  vi.stubGlobal("ResizeObserver", FixedResizeObserver)
})
afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
})

describe("removing a gesture whose look moves onto one with a look of its own", () => {
  it("asks first and changes nothing until the user chooses, or on cancel", async () => {
    const initial = layoutWith(moving, own)
    const user = await removeSinglePress(initial)
    const dialog = screen.getByRole("alertdialog")
    expect(dialog).toHaveTextContent("has 2 labels, and Long press")
    expect(state.draft).toBe(initial)
    await user.click(screen.getByRole("button", { name: "Cancel" }))
    expect(screen.queryByRole("alertdialog")).toBeNull()
    expect(state.draft).toBe(initial)
  })

  it("keeps the long press's look, deleting the moving labels, and keeps the title", async () => {
    const user = await removeSinglePress(layoutWith(moving, own))
    await user.click(screen.getByRole("button", { name: "Keep its look and delete 2 labels" }))
    expect(keyBindings()).toEqual([
      expect.objectContaining({ trigger: { type: "longPress" }, label: "Shop", appearance: own }),
    ])
  })

  it("replaces the long press's look with the moving one", async () => {
    const user = await removeSinglePress(layoutWith(moving, own))
    await user.click(screen.getByRole("button", { name: "Replace it" }))
    expect(keyBindings()).toEqual([
      expect.objectContaining({ trigger: { type: "longPress" }, appearance: moving }),
    ])
  })

  it("merges the labels, keeping the long press's colours", async () => {
    const user = await removeSinglePress(layoutWith(moving, own))
    await user.click(screen.getByRole("button", { name: "Merge the labels" }))
    expect(keyBindings()).toEqual([
      expect.objectContaining({
        trigger: { type: "longPress" },
        appearance: { background: "#ffffff", labels: labels("c", "a", "b") },
      }),
    ])
  })

  it("says so when a merge no longer fits once chosen, and changes nothing", async () => {
    const user = await removeSinglePress(layoutWith(moving, own))
    // While the dialog is open the long press gets 6 more labels: 2 + 7 > 8.
    act(() => {
      state.edit?.(
        layoutWith(moving, { ...own, labels: labels("c", "d", "e", "f", "g", "h", "i") }),
      )
    })
    const before = state.draft
    await user.click(screen.getByRole("button", { name: "Merge the labels" }))
    expect(toast.warning).toHaveBeenCalledWith(
      "Nothing was changed: the key changed while you chose. Try again.",
    )
    expect(state.draft).toBe(before)
  })

  it("does not offer a merge past 8 labels (5 and 5)", async () => {
    await removeSinglePress(
      layoutWith(
        { labels: labels("a", "b", "c", "d", "e") },
        { labels: labels("f", "g", "h", "i", "j") },
      ),
    )
    expect(screen.getByRole("button", { name: "Merge the labels" })).toBeDisabled()
    expect(screen.getByRole("alertdialog")).toHaveTextContent("give the key 10 labels")
  })

  it("applies the choice to the draft as it is then: edits made meanwhile stay", async () => {
    const user = await removeSinglePress(layoutWith(moving, own))
    // While the dialog is open, another key gets an action.
    act(() => {
      state.edit?.({
        ...(state.draft as DeviceLayout),
        pages: (state.draft as DeviceLayout).pages.map((page) => ({
          ...page,
          bindings: [
            ...page.bindings,
            {
              control: OTHER,
              trigger: { type: "press" },
              target: { type: "action", action: "mail" },
              label: null,
            },
          ],
        })),
      })
    })
    await user.click(screen.getByRole("button", { name: "Keep its look and delete 2 labels" }))
    expect(state.draft?.pages[0]?.bindings).toContainEqual(
      expect.objectContaining({ control: OTHER, trigger: { type: "press" } }),
    )
    expect(keyBindings()).toEqual([
      expect.objectContaining({ trigger: { type: "longPress" }, appearance: own }),
    ])
  })

  it("moves the look without asking onto a binding with none, and says so", async () => {
    const initial = layoutWith(moving, {})
    const bindings = initial.pages[0]?.bindings.map((b) =>
      b.trigger.type === "longPress" ? { ...b, appearance: undefined } : b,
    )
    await removeSinglePress({
      ...initial,
      pages: [{ id: "home", name: "Home", bindings: bindings as ControlBinding[] }],
    })
    expect(screen.queryByRole("alertdialog")).toBeNull()
    expect(keyBindings()).toEqual([
      expect.objectContaining({ trigger: { type: "longPress" }, appearance: moving }),
    ])
    expect(toast.info).toHaveBeenCalledWith("The key's look (2 labels) moved to Long press.")
  })
})
