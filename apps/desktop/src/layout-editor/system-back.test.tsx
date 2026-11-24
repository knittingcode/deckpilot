// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

// The folder's protected system Back key and Auto Exit in the layout
// editor: shown with a lock, selected into a read-only inspector, going
// back by double-click and menu, refusing drops and edits, and the folder
// settings in English and Polish.

import type { DragEndEvent } from "@dnd-kit/core"
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import type { ReactNode } from "react"
import { toast } from "sonner"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { i18n } from "@/i18n"
import type { ControlBinding, DeviceLayout, DeviceModel, ProfileView } from "@/lib/types"
import { installDomStubs } from "@/test/dom"

import { LayoutEditor } from "./editor"

const saveLayout = vi.fn()

/** The editor's drag and drop as wired (see folders.test.tsx). */
const dnd = vi.hoisted(() => ({
  onDragEnd: undefined as ((event: DragEndEvent) => void) | undefined,
  data: new Map<string, unknown>(),
}))

vi.mock("@dnd-kit/core", async (original) => {
  const actual = await original<typeof import("@dnd-kit/core")>()
  return {
    ...actual,
    DndContext: (props: {
      id?: string
      onDragEnd?: (event: DragEndEvent) => void
      children?: ReactNode
    }) => {
      if (props.id !== "page-tree") dnd.onDragEnd = props.onDragEnd
      return <actual.DndContext {...props} />
    },
    useDraggable: (args: Parameters<typeof actual.useDraggable>[0]) => {
      dnd.data.set(String(args.id), args.data)
      return actual.useDraggable(args)
    },
  }
})
vi.mock("@/lib/api", () => ({
  api: {
    previewLayout: () => Promise.resolve([]),
    listIcons: () => Promise.resolve([]),
    pageTree: () => Promise.resolve({ pages: [], warnings: [] }),
    listActionTypes: () => Promise.resolve([]),
    getGestureDefaults: () => Promise.resolve({ doubleClickMs: 300, longPressMs: 700 }),
    saveLayout: (...args: unknown[]) => saveLayout(...args) as unknown,
  },
  errorMessage: (error: unknown) => String(error),
}))
vi.mock("@/lib/app-context", () => ({
  useActionModules: () => [],
  useAppContext: () => ({
    profiles: { activeProfileId: null, profiles: [] },
    refresh: vi.fn(),
  }),
}))
vi.mock("sonner", () => ({
  toast: { success: vi.fn(), error: vi.fn(), warning: vi.fn() },
}))

const mk2: DeviceModel = {
  id: "elgato.streamdeck.mk2",
  vendor: "Elgato",
  name: "Stream Deck MK.2",
  usb: null,
  capabilities: {
    keys: { rows: 3, columns: 5, image: { width: 72, height: 72 } as never },
    encoders: null,
    touch: null,
  },
  support: "verified",
  verification: [],
}

const key = (index: number) => ({ kind: "key" as const, index })
const run = (index: number, action: string): ControlBinding => ({
  control: key(index),
  trigger: { type: "press" },
  target: { type: "action", action },
  label: null,
})
const opens = (index: number, page: string): ControlBinding => ({
  ...run(index, "x"),
  target: { type: "openPage", page },
})

function makeLayout(old: ControlBinding[] = [run(0, "shop")]): DeviceLayout {
  return {
    id: "layout",
    name: "Main",
    model: mk2.id,
    homePage: "home",
    explicitTree: true,
    pages: [
      { id: "home", name: "Home", bindings: [opens(1, "tools"), opens(2, "old")] },
      {
        id: "tools",
        name: "Tools",
        parent: "home",
        bindings: [run(2, "shop")],
        back: { position: "reserved" },
        autoExit: { enabled: false, seconds: 10 },
      },
      // Migrated as legacy: its first key already ran something.
      { id: "old", name: "Old", parent: "home", bindings: old, back: { position: "legacy" } },
    ],
  }
}

function renderEditor(layout = makeLayout()) {
  const view = {
    activeProjectId: null,
    profile: {
      id: "p",
      name: "Work",
      description: "",
      actions: [{ id: "shop", name: "Shop", icon: null }],
      workflows: [],
      projects: [],
      layouts: [layout],
    },
  } as unknown as ProfileView
  const user = userEvent.setup()
  render(
    <LayoutEditor
      profileId="p"
      view={view}
      model={mk2}
      saved={layout}
      onDirtyChange={vi.fn()}
      onSaved={vi.fn()}
    />,
  )
  return user
}

const control = (name: string) => screen.getByRole("button", { name })
function inspector(): HTMLElement {
  const found = document.querySelector<HTMLElement>(
    "[data-slot=device-editor] [data-slot=inspector]",
  )
  if (!found) throw new Error("no inspector")
  return found
}

function drop(id: string, over: string) {
  expect(dnd.data.has(id), `a draggable ${id}`).toBe(true)
  act(() => {
    dnd.onDragEnd?.({
      active: { id, data: { current: dnd.data.get(id) } },
      over: { id: over },
    } as unknown as DragEndEvent)
  })
}

async function save(user: ReturnType<typeof userEvent.setup>): Promise<DeviceLayout> {
  await user.click(screen.getByRole("button", { name: /Save/ }))
  const [, saved] = saveLayout.mock.lastCall as [string, DeviceLayout]
  return saved
}

beforeAll(installDomStubs)
beforeEach(() => {
  dnd.data.clear()
  saveLayout.mockReset()
  saveLayout.mockImplementation((_profile: string, saved: DeviceLayout) =>
    Promise.resolve({ ...saved, agentReload: { state: "applied" } }),
  )
  vi.mocked(toast.error).mockClear()
})
afterEach(cleanup)

describe("the folder's system Back in the editor", { timeout: 20_000 }, () => {
  it("is drawn protected on the folder's first key only, and returns by double-click", async () => {
    const user = renderEditor()
    // The root page has none.
    expect(control("Key 1")).not.toHaveAttribute("data-system")
    await user.dblClick(control("Key 2"))
    const back = control("Key 1")
    expect(back).toHaveAttribute("data-system")
    expect(back).toHaveAttribute(
      "aria-description",
      "System control: Back, returns to the parent folder",
    )
    // It looks like every other key: no badge, no border of its own.
    expect(back.querySelector("[data-slot=system-mark]")).toBeNull()
    expect(back.querySelector("svg")).toBeNull()
    expect(back.className).toBe(control("Key 4").className)
    expect(control("Key 3")).not.toHaveAttribute("data-system")
    // A double-click goes back to Home, which has none.
    await user.dblClick(back)
    expect(control("Key 1")).not.toHaveAttribute("data-system")
  })

  it("selects into an empty, named inspector; nothing selected is empty too", async () => {
    const user = renderEditor()
    await user.dblClick(control("Key 2"))
    // Nothing selected: an empty inspector, no instructions or folder settings.
    expect(inspector().textContent).toBe("")
    const before = inspector().className
    await user.click(control("Key 1"))
    expect(control("Key 1")).toHaveAttribute("aria-pressed", "true")
    const region = screen.getByRole("region", { name: "Back – system control" })
    expect(region).toBe(inspector())
    // Nothing at all inside: no heading, text, controls or folder settings.
    expect(region.textContent).toBe("")
    expect(region.children).toHaveLength(0)
    // No deselect button: Escape deselects.
    expect(screen.queryByRole("button", { name: /^Deselect/ })).not.toBeInTheDocument()
    // The inspector keeps its frame (no collapse, no extra padding).
    expect(region.className).toBe(before)
    // An action key shows its settings, and no explanation without controls.
    await user.click(control("Key 3"))
    expect(inspector().querySelectorAll("[data-slot=gesture-card]").length).toBeGreaterThan(0)
    expect(within(inspector()).queryByText(/Only events this model can produce/)).toBeNull()
    expect(inspector().querySelector("[data-slot=folder-settings]")).toBeNull()
  })

  it("keeps the canvas and the inspector in place when the selection changes", async () => {
    const user = renderEditor()
    const viewport = () =>
      document.querySelector<HTMLElement>("[data-slot=device-editor-main]")?.firstElementChild
        ?.nextElementSibling?.nextElementSibling as HTMLElement | null
    const frame = () => ({
      viewport: viewport()?.getAttribute("style") ?? null,
      viewportClass: viewport()?.className ?? null,
      scroll: document.querySelector("[data-slot=inspector-scroll]")?.className,
      rect: JSON.stringify(inspector().getBoundingClientRect()),
      inspector: inspector().className,
    })
    const none = frame()
    await user.click(control("Key 1"))
    expect(frame()).toEqual(none)
    await user.dblClick(control("Key 2"))
    await user.click(control("Key 1"))
    expect(frame()).toEqual(none)
  })

  it("puts the folder's settings on the key that opens it, with Auto Exit validated", async () => {
    const user = renderEditor()
    await user.click(control("Key 2"))
    const region = inspector()
    const settings = within(region).getByRole("region", { name: "Folder settings" })
    // Only settings: no heading, no explanation.
    expect(within(settings).queryByRole("heading")).toBeNull()
    // The folder's name is the key header's title, not a setting here.
    expect(within(settings).queryByLabelText("Folder name")).toBeNull()
    expect(within(region).getByRole("textbox", { name: "Folder name" })).toHaveValue("Tools")
    // The time is offered once Auto Exit is on.
    expect(within(settings).queryByLabelText("Automatic exit time in seconds")).toBeNull()
    await user.click(within(settings).getByRole("switch", { name: "Automatic exit" }))
    const seconds = within(settings).getByLabelText("Automatic exit time in seconds")
    await user.clear(seconds)
    await user.type(seconds, "0")
    expect(within(settings).getByRole("alert")).toHaveTextContent(
      "Enter between 1 and 3600 seconds.",
    )
    await user.clear(seconds)
    await user.type(seconds, "1.5")
    expect(within(settings).getByRole("alert")).toHaveTextContent(
      "Enter a whole number of seconds.",
    )
    await user.clear(seconds)
    await user.type(seconds, "25")
    expect(within(settings).queryByRole("alert")).toBeNull()
    expect(settings.querySelector("[data-slot=auto-exit]")).toHaveTextContent(
      "Automatic exit after seconds",
    )
    // A reserved folder offers no move.
    expect(within(settings).queryByRole("button", { name: /Move Back/ })).toBeNull()
    const saved = await save(user)
    expect(saved.pages.find((p) => p.id === "tools")?.autoExit).toEqual({
      enabled: true,
      seconds: 25,
    })
    // Undo steps back to the stored setting.
    await user.click(screen.getByRole("button", { name: "Undo" }))
    await user.click(screen.getByRole("button", { name: "Undo" }))
    expect(within(inspector()).getByRole("switch", { name: "Automatic exit" })).not.toBeChecked()
  })

  it("offers Exit after action completes as a switch of its own, undone, reverted and saved", async () => {
    const user = renderEditor()
    await user.click(control("Key 2"))
    const settings = () => within(inspector()).getByRole("region", { name: "Folder settings" })
    const completion = () =>
      within(settings()).getByRole("switch", { name: "Exit after action completes" })
    const autoExit = () => within(settings()).getByRole("switch", { name: "Automatic exit" })
    // Off for a folder that never had it, and independent of Auto Exit:
    // no extra panel, the time only with Auto Exit.
    expect(completion()).not.toBeChecked()
    await user.click(completion())
    expect(completion()).toBeChecked()
    expect(autoExit()).not.toBeChecked()
    expect(within(settings()).queryByLabelText("Automatic exit time in seconds")).toBeNull()
    await user.click(autoExit())
    expect(within(settings()).getByLabelText("Automatic exit time in seconds")).toHaveValue("10")
    await user.click(completion())
    expect(completion()).not.toBeChecked()
    expect(autoExit()).toBeChecked()
    // Undo and Redo step through each switch.
    await user.click(screen.getByRole("button", { name: "Undo" }))
    expect(completion()).toBeChecked()
    await user.click(screen.getByRole("button", { name: "Undo" }))
    expect(autoExit()).not.toBeChecked()
    expect(completion()).toBeChecked()
    await user.click(screen.getByRole("button", { name: "Redo" }))
    expect(autoExit()).toBeChecked()
    // Revert: back to what is stored (both off).
    await user.click(screen.getByRole("button", { name: "Revert" }))
    expect(completion()).not.toBeChecked()
    expect(autoExit()).not.toBeChecked()
    // Saved as the folder's `autoReturn`, beside its unchanged Auto Exit.
    await user.click(completion())
    const saved = await save(user)
    const tools = saved.pages.find((p) => p.id === "tools")
    expect(tools?.autoReturn).toBe(true)
    expect(tools?.autoExit).toEqual({ enabled: false, seconds: 10 })
    expect(saved.pages.filter((p) => "autoReturn" in p).map((p) => p.id)).toEqual(["tools"])
  })

  it("keeps a folder's stored settings and shows them", async () => {
    const stored = makeLayout()
    const tools = stored.pages[1]
    if (!tools) throw new Error("no folder")
    stored.pages[1] = { ...tools, autoReturn: true, autoExit: { enabled: true, seconds: 25 } }
    const user = renderEditor(stored)
    await user.click(control("Key 2"))
    const settings = within(inspector()).getByRole("region", { name: "Folder settings" })
    expect(within(settings).getByRole("switch", { name: "Automatic exit" })).toBeChecked()
    expect(within(settings).getByLabelText("Automatic exit time in seconds")).toHaveValue("25")
    expect(
      within(settings).getByRole("switch", { name: "Exit after action completes" }),
    ).toBeChecked()
  })

  it("shows Automatic exit in one row with its time inline, edited in place", async () => {
    const user = renderEditor()
    await user.click(control("Key 2"))
    const settings = () => within(inspector()).getByRole("region", { name: "Folder settings" })
    const row = () => settings().querySelector<HTMLElement>("[data-slot=auto-exit]")
    const field = () => within(settings()).getByLabelText("Automatic exit time in seconds")
    const toggle = () => within(settings()).getByRole("switch", { name: "Automatic exit" })
    // Off: the short label only; no separate field or label for the time.
    expect(row()?.textContent.replace(/\s+/g, " ")).toBe("Automatic exit")
    expect(within(settings()).queryByRole("textbox")).toBeNull()
    expect(within(settings()).queryByText("Exit after")).toBeNull()
    const height = row()?.className
    // On: "Automatic exit after [10] seconds", in the same row.
    await user.click(toggle())
    expect(row()?.textContent.replace(/\s+/g, " ")).toBe("Automatic exit after seconds")
    expect(row()?.contains(field())).toBe(true)
    expect(field()).toHaveValue("10")
    expect(field()).toHaveAttribute("inputmode", "numeric")
    expect(field()).toHaveAttribute("type", "text")
    // Same row, same height class: no shift when toggling.
    expect(row()?.className).toBe(height)
    expect(row()?.className).toContain("min-h-8")
    // Focus selects it all; typing replaces it; one second is singular.
    act(() => {
      field().focus()
    })
    const input = field() as HTMLInputElement
    expect([input.selectionStart, input.selectionEnd]).toEqual([0, 2])
    await user.keyboard("1{Enter}")
    expect(field()).toHaveValue("1")
    expect(row()?.textContent.replace(/\s+/g, " ")).toBe("Automatic exit after second")
    // Invalid values are shown with why and never stored.
    for (const typed of ["0", "-5", "3601", "abc", ""]) {
      await user.clear(field())
      if (typed) await user.type(field(), typed)
      expect(within(settings()).getByRole("alert")).toBeInTheDocument()
    }
    // Escape brings back the stored time (1 s, the last committed one).
    await user.keyboard("{Escape}")
    expect(field()).toHaveValue("1")
    expect(within(settings()).queryByRole("alert")).toBeNull()
    // An invalid value left by blur is dropped too.
    await user.clear(field())
    await user.type(field(), "99999")
    await user.tab()
    expect(field()).toHaveValue("1")
    // Off keeps the time, on shows it again.
    await user.clear(field())
    await user.keyboard("45{Enter}")
    expect(field()).toHaveValue("45")
    await user.click(toggle())
    expect(row()?.textContent.replace(/\s+/g, " ")).toBe("Automatic exit")
    await user.click(toggle())
    expect(field()).toHaveValue("45")
    const saved = await save(user)
    expect(saved.pages.find((p) => p.id === "tools")?.autoExit).toEqual({
      enabled: true,
      seconds: 45,
    })
    // One undo step per committed edit: off/on, 45, 1, on.
    await user.click(screen.getByRole("button", { name: "Undo" }))
    await user.click(screen.getByRole("button", { name: "Undo" }))
    expect(toggle()).toBeChecked()
    await user.click(screen.getByRole("button", { name: "Undo" }))
    expect(field()).toHaveValue("1")
    await user.click(screen.getByRole("button", { name: "Redo" }))
    expect(field()).toHaveValue("45")
    // Revert: what the editor was opened with (off, 10 s).
    await user.click(screen.getByRole("button", { name: "Revert" }))
    expect(toggle()).not.toBeChecked()
    await user.click(toggle())
    expect(field()).toHaveValue("10")
    // Exit after action completes stays its own switch, right below.
    const completion = within(settings()).getByRole("switch", {
      name: "Exit after action completes",
    })
    expect(row()?.parentElement?.nextElementSibling?.contains(completion)).toBe(true)
  })

  it("offers only Go back and Test in its menu", async () => {
    const user = renderEditor()
    await user.dblClick(control("Key 2"))
    fireEvent.contextMenu(control("Key 1"))
    const menu = await screen.findByRole("menu")
    const items = within(menu)
      .getAllByRole("menuitem")
      .map((item) => item.textContent)
    expect(items.some((text) => text.includes("Go back"))).toBe(true)
    for (const absent of [/Remove/, /Copy/, /Cut/, /Paste/, /Duplicate/, /New folder/]) {
      expect(within(menu).queryByRole("menuitem", { name: absent })).toBeNull()
    }
    expect(within(menu).queryByText("Assign")).toBeNull()
    await user.click(within(menu).getByRole("menuitem", { name: /Go back/ }))
    expect(control("Key 1")).not.toHaveAttribute("data-system")
  })

  it("refuses drops, moves and bulk edits", async () => {
    const user = renderEditor()
    await user.dblClick(control("Key 2"))
    // Something from the library dropped on it, and the key dropped elsewhere.
    drop("library:back", "key:0")
    expect(toast.error).toHaveBeenCalled()
    drop("control:key:2", "key:0")
    expect(toast.error).toHaveBeenCalledTimes(2)
    expect(control("Key 1").getAttribute("title")).toBe(
      "Key 1: System control: Back, returns to the parent folder",
    )
    expect(control("Key 3").getAttribute("title")).toMatch(/Shop/)
    // Select every key and remove: the system Back stays, the rest goes.
    await user.click(control("Key 3"))
    fireEvent.keyDown(control("Key 3"), { key: "a", ctrlKey: true, metaKey: true })
    fireEvent.keyDown(control("Key 3"), { key: "Delete" })
    const saved = await save(user)
    const tools = saved.pages.find((p) => p.id === "tools")
    expect(tools?.bindings).toEqual([])
    expect(tools?.back).toEqual({ position: "reserved" })
  })

  it("moves a legacy folder's Back to the first key in one click", async () => {
    const user = renderEditor()
    // The key that opens the legacy folder offers the move.
    await user.click(control("Key 3"))
    await user.click(
      within(inspector()).getByRole("button", { name: "Move Back to the first key" }),
    )
    expect(within(inspector()).queryByRole("button", { name: /Move Back/ })).toBeNull()
    await user.dblClick(control("Key 3"))
    expect(control("Key 1")).toHaveAttribute("data-system")
    expect(control("Key 2").getAttribute("title")).toMatch(/Shop/)
    const saved = await save(user)
    expect(saved.pages.find((p) => p.id === "old")).toMatchObject({
      back: { position: "reserved" },
      bindings: [run(1, "shop")],
    })
  })

  it("says so when no key is free for the move", async () => {
    const full = Array.from({ length: 15 }, (_, i) => run(i, "shop"))
    const user = renderEditor(makeLayout(full))
    await user.click(control("Key 3"))
    await user.click(
      within(inspector()).getByRole("button", { name: "Move Back to the first key" }),
    )
    expect(within(inspector()).getByRole("alert")).toHaveTextContent(
      "Every key of this page runs something.",
    )
    await user.dblClick(control("Key 3"))
    expect(control("Key 1")).not.toHaveAttribute("data-system")
  })

  it("speaks Polish", async () => {
    await act(async () => {
      await i18n.changeLanguage("pl")
    })
    try {
      const user = renderEditor()
      await user.click(control("Klawisz 2"))
      const settings = within(inspector()).getByRole("region", { name: "Ustawienia folderu" })
      const row = () => settings.querySelector("[data-slot=auto-exit]")
      expect(row()?.textContent.replace(/\s+/g, " ")).toBe("Automatyczne wyjście")
      await user.click(within(settings).getByRole("switch", { name: "Automatyczne wyjście" }))
      const seconds = within(settings).getByLabelText("Czas automatycznego wyjścia w sekundach")
      expect(row()?.textContent.replace(/\s+/g, " ")).toBe("Automatyczne wyjście po sekundach")
      for (const [typed, unit] of [
        ["1", "sekundzie"],
        ["2", "sekundach"],
        ["5", "sekundach"],
        ["21", "sekundach"],
      ] as const) {
        await user.clear(seconds)
        await user.type(seconds, typed)
        expect(row()?.textContent.replace(/\s+/g, " ")).toBe(`Automatyczne wyjście po ${unit}`)
      }
      expect(within(settings).queryByLabelText("Wyjdź po")).toBeNull()
      await user.clear(seconds)
      await user.type(seconds, "4000")
      expect(within(settings).getByRole("alert")).toHaveTextContent("Wpisz od 1 do 3600 sekund.")
      expect(
        within(settings).getByRole("switch", { name: "Wyjdź po zakończeniu akcji" }),
      ).not.toBeChecked()
      // The folder's Back: an empty inspector named in Polish.
      await user.dblClick(control("Klawisz 2"))
      await user.click(control("Klawisz 1"))
      const back = screen.getByRole("region", { name: "Wstecz – element systemowy" })
      expect(back.textContent).toBe("")
    } finally {
      await act(async () => {
        await i18n.changeLanguage("en")
      })
    }
  })
})
