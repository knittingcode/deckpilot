// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

// Compatibility in the device editor (docs/device-controls.md): the library
// for the selected control, drops, the inspector and saving.

import type { DragEndEvent } from "@dnd-kit/core"
import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { toast } from "sonner"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { moduleInfo } from "@/lib/test-fixtures"
import type {
  ActionType,
  ControlFace,
  DeviceLayout,
  DeviceModel,
  ModuleInfo,
  ProfileView,
} from "@/lib/types"

import { catalogDragId } from "./catalog"
import { LayoutEditor } from "./editor"
import { dnd, drop as dropOnSlot, resetDnd } from "@/test/dnd"

Element.prototype.hasPointerCapture = () => false
Element.prototype.releasePointerCapture = () => undefined
Element.prototype.scrollIntoView = () => undefined

const KEY_INPUTS = ["press", "release", "click", "double-click", "long-press"] as const

const type = (id: string, name: string, controls?: ActionType["controls"]): ActionType => ({
  id,
  name,
  description: `${name}.`,
  category: "Navigation",
  requiresProject: false,
  parameters: [],
  ...(controls ? { controls } : {}),
})

/** Encoder only: a volume knob. */
const volume = type("audio.volume", "Volume", {
  encoder: ["rotate-clockwise", "rotate-counter-clockwise"],
})
/** Keys and, once per turn or press, dials. */
const nextPage = type("deck.next-page", "Next page", {
  key: [...KEY_INPUTS],
  encoder: ["rotate-clockwise", "rotate-counter-clockwise", "press", "click"],
})
/** Keys only, declared. */
const mute = type("audio.mute", "Mute", { key: [...KEY_INPUTS] })
/** No declaration: keys only. */
const log = type("general.log", "Log")

const modules: ModuleInfo[] = [
  moduleInfo({ id: "audio", actionTypes: [volume, mute] }),
  moduleInfo({ id: "deck", actionTypes: [nextPage, log] }),
]

vi.mock("@dnd-kit/core", async (original) =>
  (await import("@/test/dnd")).dndModule(await original()),
)

const api = vi.hoisted(() => ({
  previewLayout: vi.fn(),
  saveLayout: vi.fn(),
  createAction: vi.fn(),
}))

vi.mock("@/lib/api", () => ({
  api: {
    previewLayout: (...args: unknown[]) => api.previewLayout(...args) as unknown,
    listIcons: () => Promise.resolve([]),
    pageTree: () => Promise.resolve({ pages: [], warnings: [] }),
    listActionTypes: () => Promise.resolve([volume, nextPage, mute, log]),
    getGestureDefaults: () => Promise.resolve({ doubleClickMs: 300, longPressMs: 700 }),
    saveLayout: (...args: unknown[]) => api.saveLayout(...args) as unknown,
    createAction: (...args: unknown[]) => api.createAction(...args) as unknown,
  },
  errorMessage: (error: unknown) => (error instanceof Error ? error.message : String(error)),
}))
vi.mock("@/lib/app-context", () => ({
  useActionModules: () => modules,
  useAppContext: () => ({ profiles: { activeProfileId: null, profiles: [] }, refresh: vi.fn() }),
}))
vi.mock("sonner", () => ({
  toast: { success: vi.fn(), error: vi.fn(), warning: vi.fn() },
}))

const mk2: DeviceModel = {
  id: "elgato.streamdeck.mk2",
  vendor: "Elgato",
  name: "Stream Deck MK.2",
  usb: null,
  capabilities: { keys: { rows: 3, columns: 5, image: null }, encoders: null, touch: null },
  support: "unverified",
  verification: [],
}
const plus: DeviceModel = {
  id: "elgato.streamdeck.plus",
  vendor: "Elgato",
  name: "Stream Deck +",
  usb: null,
  capabilities: {
    keys: { rows: 2, columns: 4, image: null },
    encoders: { count: 4, pressable: true },
    touch: { zones: 4, image: null, gestures: ["tap"] },
  },
  support: "unverified",
  verification: [],
}

const actions = [
  { id: "vol", name: "Room volume", actionType: volume.id, parameters: {}, icon: null },
  { id: "mute", name: "Mute mic", actionType: mute.id, parameters: {}, icon: null },
  { id: "log", name: "Write log", actionType: log.id, parameters: {}, icon: null },
  { id: "next", name: "Forward", actionType: nextPage.id, parameters: {}, icon: null },
]

function layoutFor(model: DeviceModel, bindings: DeviceLayout["pages"][number]["bindings"] = []) {
  return {
    id: "layout",
    name: "Main",
    model: model.id,
    homePage: "home",
    pages: [{ id: "home", name: "Home", bindings }],
  } satisfies DeviceLayout
}

function renderEditor(model: DeviceModel, saved: DeviceLayout = layoutFor(model)) {
  const view = {
    activeProjectId: null,
    profile: {
      id: "p",
      name: "Work",
      actions,
      workflows: [],
      projects: [],
      layouts: [saved],
    },
  } as unknown as ProfileView
  render(
    <LayoutEditor
      profileId="p"
      view={view}
      model={model}
      saved={saved}
      onDirtyChange={vi.fn()}
      onSaved={vi.fn()}
    />,
  )
  return userEvent.setup()
}

const control = (name: string) => screen.getByRole("button", { name })
const describes = (name: string) => control(name).getAttribute("title")

function drop(id: string, over: string) {
  expect(dnd.data.has(id), `a draggable ${id}`).toBe(true)
  const onDragEnd = dnd.onDragEnd
  if (!onDragEnd) throw new Error("the editor has no drop handler")
  act(() => {
    onDragEnd({
      active: { id, data: { current: dnd.data.get(id) } },
      over: { id: over },
    } as unknown as DragEndEvent)
  })
}

beforeEach(() => {
  localStorage.clear()
  resetDnd()
  vi.mocked(toast.error).mockClear()
  vi.mocked(toast.warning).mockClear()
  api.previewLayout.mockReset()
  api.previewLayout.mockResolvedValue([])
  api.saveLayout.mockReset()
  api.saveLayout.mockImplementation((_profile: string, saved: DeviceLayout) =>
    Promise.resolve({ ...saved, agentReload: { state: "applied" } }),
  )
  api.createAction.mockReset()
})
afterEach(cleanup)

describe("the library for the selected control", { timeout: 20_000 }, () => {
  it("disables what cannot go on a Stream Deck MK.2 key, with the reason", async () => {
    const user = renderEditor(mk2)
    // Nothing is selected: everything can be dragged (no helper text above the list).
    expect(screen.queryByText("Select a control to see what fits it.")).not.toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Drag Volume onto a control" })).toBeInTheDocument()

    await user.click(control("Key 1"))
    const reason = "Requires a dial. The selected Stream Deck MK.2 key supports press actions only."
    // An encoder-only type: listed, not draggable, with the reason.
    const type = screen.getByLabelText(`Volume: not for the selected control. ${reason}`)
    expect(type).toHaveAttribute("aria-disabled", "true")
    expect(within(type).getByText(reason)).toBeInTheDocument()
    // Key types stay draggable, declared or not.
    expect(screen.getByRole("button", { name: "Drag Mute onto a control" })).toBeEnabled()
    expect(screen.getByRole("button", { name: "Drag Log onto a control" })).toBeEnabled()
    expect(screen.getByRole("button", { name: "Drag Next page onto a control" })).toBeEnabled()
  })

  it("disables key actions for a dial and explains a legacy action", async () => {
    const user = renderEditor(plus)
    await user.click(control("Dial 1"))
    const reason =
      "Requires a key. The selected Stream Deck + dial supports turn and press actions."
    expect(screen.getByLabelText(`Mute: not for the selected control. ${reason}`)).toHaveAttribute(
      "aria-disabled",
      "true",
    )
    // A type without a declaration is keys only.
    expect(
      screen.getByLabelText(`Log: not for the selected control. ${reason}`),
    ).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Drag Volume onto a control" })).toBeEnabled()
    expect(screen.getByRole("button", { name: "Drag Next page onto a control" })).toBeEnabled()

    await user.click(control("Touch zone 1"))
    expect(
      screen.getByLabelText(
        "Log: not for the selected control. Requires a key. The selected Stream Deck + touch zone supports tap actions only.",
      ),
    ).toBeInTheDocument()
  })

  it("lists what does not fit the selected control too, disabled, with no switch to hide it", async () => {
    // An earlier version's "Compatible only" preference changes nothing.
    localStorage.setItem("deckpilot.editor.compatibleOnly", "true")
    const user = renderEditor(mk2)
    expect(screen.queryByRole("switch", { name: "Compatible only" })).not.toBeInTheDocument()
    expect(screen.getByText("Volume")).toBeInTheDocument()
    await user.click(control("Key 1"))
    // Still listed after selecting a key, and still refused there.
    expect(screen.getByText("Volume")).toBeInTheDocument()
    expect(screen.getByLabelText(/^Volume: not for the selected control/)).toHaveAttribute(
      "aria-disabled",
      "true",
    )
    expect(screen.getByText("Mute")).toBeInTheDocument()
  })
})

describe("dropping", { timeout: 20_000 }, () => {
  it("refuses an incompatible action: nothing is bound, the drop is shown and announced", async () => {
    renderEditor(plus)
    drop(catalogDragId(mute.id), "encoder:0")
    expect(describes("Dial 1")).toBe("Dial 1")
    expect(api.createAction).not.toHaveBeenCalled()
    const reason =
      "Requires a key. The selected Stream Deck + dial supports turn and press actions."
    expect(toast.error).toHaveBeenCalledWith("Mute was not put on Dial 1", {
      description: reason,
    })
    expect(control("Dial 1")).toHaveAttribute("data-drop", "rejected")
    const announcement = document.querySelector("[data-slot='drop-announcement']")
    expect(announcement).toHaveTextContent(`Mute cannot go on Dial 1. ${reason}`)
    expect(announcement).toHaveAttribute("aria-live", "assertive")
    // The state goes away after a moment.
    await waitFor(
      () => {
        expect(control("Dial 1")).not.toHaveAttribute("data-drop")
      },
      { timeout: 4000 },
    )
  })

  it("refuses an encoder-only type from the catalog on a key, and creates nothing", () => {
    renderEditor(plus)
    drop(catalogDragId(volume.id), "key:0")
    expect(api.createAction).not.toHaveBeenCalled()
    expect(describes("Key 1")).toBe("Key 1")
    expect(toast.error).toHaveBeenCalledWith("Volume was not put on Key 1", {
      description: "Requires a dial. The selected Stream Deck + key supports press actions only.",
    })
  })

  it("refuses to move a key's binding onto a dial", () => {
    renderEditor(
      plus,
      layoutFor(plus, [
        {
          control: { kind: "key", index: 0 },
          trigger: { type: "press" },
          target: { type: "action", action: "mute" },
        },
      ]),
    )
    drop("control:key:0", "encoder:1")
    expect(describes("Key 1")).toBe("Key 1: Press: Mute mic")
    expect(describes("Dial 2")).toBe("Dial 2")
    expect(toast.error).toHaveBeenCalledWith("Key 1 was not put on Dial 2", expect.anything())
  })

  it("binds a compatible type to a trigger it can run from, reusing the action that is exactly it", async () => {
    renderEditor(plus)
    drop(catalogDragId(nextPage.id), "encoder:0")
    // "Forward" is exactly a new Next page action: it is used, none is made.
    await waitFor(() => {
      expect(describes("Dial 1")).toBe("Dial 1: Turn right: Forward")
    })
    expect(api.createAction).not.toHaveBeenCalled()
    expect(toast.error).not.toHaveBeenCalled()
  })
})

describe("the inspector", { timeout: 20_000 }, () => {
  const stored = layoutFor(plus, [
    {
      control: { kind: "encoder", index: 0 },
      trigger: { type: "rotateClockwise" },
      target: { type: "action", action: "log" },
    },
  ])
  const preview: ControlFace[] = [
    {
      control: { kind: "encoder", index: 0 },
      face: { title: "Write log" } as ControlFace["face"],
      incompatible: [
        {
          trigger: { type: "rotateClockwise" },
          reason: { type: "controlKindUnsupported", kind: "encoder", supported: ["key"] },
          message: "the action cannot be bound to an encoder; it works on keys only",
        },
      ],
    },
  ]

  it("says a stored incompatible binding is kept but not run, and why", async () => {
    api.previewLayout.mockResolvedValue(preview)
    const user = renderEditor(plus, stored)
    await user.click(control("Dial 1"))
    expect(
      await screen.findByText(
        "Not run from this control: Requires a key. The selected Stream Deck + dial supports turn and press actions. It stays assigned but is not run.",
      ),
    ).toBeInTheDocument()
  })

  it("offers each trigger only what can run from it", async () => {
    const user = renderEditor(plus)
    await user.click(control("Dial 1"))
    await user.click(screen.getByRole("combobox", { name: "Turn right" }))
    const listbox = screen.getByRole("listbox")
    expect(within(listbox).getByRole("option", { name: "Forward" })).not.toHaveAttribute(
      "data-disabled",
    )
    expect(within(listbox).getByRole("option", { name: "Room volume" })).not.toHaveAttribute(
      "data-disabled",
    )
    // What cannot run from it is not offered (no disabled entries).
    expect(within(listbox).queryByRole("option", { name: /Mute mic/ })).toBeNull()
    expect(within(listbox).queryByRole("option", { name: /Write log/ })).toBeNull()
  })

  it("offers a dial's press gestures only to actions that react to them", async () => {
    const user = renderEditor(plus)
    await user.click(control("Dial 1"))
    // Next page reacts to a press of a dial but not to a double click: a
    // drop of it on the double click is refused with the reason.
    const before = describes("Dial 1")
    const slot = [...dnd.droppables].find((id) => id.endsWith(":doubleClick"))
    dropOnSlot(catalogDragId(nextPage.id), slot ?? "no double click slot")
    await waitFor(() => {
      expect(toast.error).toHaveBeenCalledWith(
        "Not put on the double click of Dial 1",
        expect.anything(),
      )
    })
    expect(describes("Dial 1")).toBe(before)
    expect(screen.queryByRole("combobox", { name: /Double click/ })).toBeNull()
  })
})

describe("saving", { timeout: 20_000 }, () => {
  it("shows the reason when the backend refuses a new incompatible binding", async () => {
    api.saveLayout.mockRejectedValue(
      new Error(
        "page `Home`: encoder 0 (RotateClockwise): the action cannot be bound to an encoder; it works on keys only",
      ),
    )
    const user = renderEditor(plus)
    // Any change: a new page, then back to the first.
    await user.click(screen.getByRole("button", { name: "Add page" }))
    await user.click(screen.getByRole("button", { name: "Previous page" }))
    await user.click(screen.getByRole("button", { name: "Save" }))
    expect(
      await screen.findByText(
        "page `Home`: encoder 0 (Turn right): the action cannot be bound to an encoder; it works on keys only",
      ),
    ).toBeInTheDocument()
    // At the dial too, named as in the editor.
    await user.click(control("Dial 1"))
    expect(
      screen.getByText(
        "Dial 1 (Turn right): the action cannot be bound to an encoder; it works on keys only",
      ),
    ).toBeInTheDocument()
  })

  it("says which stored incompatible bindings it kept", async () => {
    api.saveLayout.mockImplementation((_profile: string, saved: DeviceLayout) =>
      Promise.resolve({
        ...saved,
        agentReload: { state: "applied" },
        incompatible: [
          "page `Home`: touch zone 0 (Touch { gesture: Tap }): the action cannot be bound to a touch zone; it works on keys only",
        ],
      }),
    )
    const user = renderEditor(plus)
    // Any change: a new page, then back to the first.
    await user.click(screen.getByRole("button", { name: "Add page" }))
    await user.click(screen.getByRole("button", { name: "Previous page" }))
    await user.click(screen.getByRole("button", { name: "Save" }))
    await waitFor(() => {
      expect(toast.warning).toHaveBeenCalledWith("1 binding is kept but not run", {
        description:
          "Their action cannot run from their control: page `Home`: touch zone 0 (Tap): the action cannot be bound to a touch zone; it works on keys only",
      })
    })
  })
})
