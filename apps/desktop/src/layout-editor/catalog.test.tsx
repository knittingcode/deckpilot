// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import type { DragEndEvent } from "@dnd-kit/core"
import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import type { ReactNode } from "react"
import { toast } from "sonner"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import type { ActionInput } from "@/lib/api"
import { i18n } from "@/i18n"
import { moduleInfo } from "@/lib/test-fixtures"
import type {
  ActionInstance,
  ActionType,
  DeviceLayout,
  DeviceModel,
  ModuleInfo,
  ProfileView,
} from "@/lib/types"

import {
  catalogDragId,
  catalogGroups,
  defaultParameters,
  existingDefault,
  filterCatalog,
  freeActionName,
  newAction,
} from "./catalog"
import { LayoutEditor } from "./editor"

Element.prototype.hasPointerCapture = () => false
Element.prototype.releasePointerCapture = () => undefined
Element.prototype.scrollIntoView = () => undefined

const wrap = {
  key: "wrapAround",
  label: "Wrap around",
  kind: { type: "boolean" as const },
  required: true,
  default: { type: "boolean" as const, value: true },
}

const nextPage: ActionType = {
  id: "deck.next-page",
  name: "Next page",
  description: "Shows the next page of the layout on the device.",
  category: "Navigation",
  requiresProject: false,
  parameters: [wrap],
  node: { icon: "chevron-right" } as ActionType["node"],
}
const indicator: ActionType = {
  id: "deck.page-indicator",
  name: "Page indicator",
  description: "Shows the page the device is on.",
  category: "Navigation",
  requiresProject: false,
  parameters: [],
}
const openUrl: ActionType = {
  id: "system.open-url",
  name: "Open web page",
  description: "Opens a web page.",
  category: "System",
  requiresProject: false,
  parameters: [
    { key: "url", label: "URL", kind: { type: "text", multiline: false }, required: true },
  ],
}
const lockScreen: ActionType = {
  id: "system.lock",
  name: "Lock screen",
  description: "Locks the computer.",
  category: "System",
  requiresProject: false,
  parameters: [],
}
const oldNext: ActionType = {
  ...nextPage,
  id: "projects.next",
  name: "Next project",
  category: "Projects",
  deprecated: true,
}
const wait: ActionType = {
  id: "general.wait",
  name: "Wait",
  description: "Waits a number of seconds.",
  category: "General",
  requiresProject: false,
  parameters: [
    {
      key: "seconds",
      label: "Seconds",
      kind: { type: "integer", min: 1, max: 300 },
      required: true,
      default: { type: "integer", value: 5 },
    },
  ],
}

const modules: ModuleInfo[] = [
  moduleInfo({ id: "deck", name: "Deck", actionTypes: [nextPage, indicator] }),
  moduleInfo({ id: "system", name: "System", actionTypes: [openUrl] }),
  moduleInfo({ id: "projects", name: "Projects", actionTypes: [oldNext] }),
  moduleInfo({ id: "general", name: "General", actionTypes: [wait], enabled: false }),
]

/** Modules a test adds to the editor's (the catalog's own tests use `modules`). */
let extraModules: ModuleInfo[] = []

const instance = (over: Partial<ActionInstance>): ActionInstance => ({
  id: "a",
  name: "Next page",
  actionType: nextPage.id,
  parameters: { wrapAround: { type: "boolean", value: true } },
  description: "",
  icon: null,
  label: null,
  ...over,
})

describe("the catalog of action types", () => {
  it("offers every type of enabled modules and plugins, by category, each once", () => {
    // Types that need a value are offered too (the Inspector asks for it);
    // deprecated ones (a legacy id routed to a plugin) and disabled modules'
    // are not.
    expect(catalogGroups(modules).map((g) => [g.label, g.entries.map((e) => e.type.id)])).toEqual([
      ["Navigation", ["deck.next-page", "deck.page-indicator"]],
      ["System", ["system.open-url"]],
    ])
    const enabled = modules.map((m) => ({ ...m, enabled: true }))
    expect(catalogGroups(enabled).map((g) => g.label)).toEqual(["Navigation", "System", "General"])
    // A plugin's types are listed, with why one cannot be placed now.
    const asPlugin = enabled.map((m) =>
      m.id === "general"
        ? { ...m, plugin: { unavailable: { "general.wait": "needs a permission" } } }
        : m,
    ) as unknown as ModuleInfo[]
    const general = catalogGroups(asPlugin).find((g) => g.label === "General")
    expect(general?.entries.map((e) => [e.type.id, e.unavailable])).toEqual([
      ["general.wait", "needs a permission"],
    ])
    // The same type from two providers: listed once.
    const twice = [...modules, moduleInfo({ id: "copy", actionTypes: [nextPage] })]
    expect(
      catalogGroups(twice)
        .flatMap((g) => g.entries)
        .filter((e) => e.type.id === nextPage.id),
    ).toHaveLength(1)
  })

  it("creates an action with the type's defaults and a name of its own", () => {
    expect(defaultParameters(nextPage)).toEqual({ wrapAround: { type: "boolean", value: true } })
    expect(defaultParameters(indicator)).toEqual({})
    expect(newAction([], nextPage)).toEqual({
      name: "Next page",
      actionType: "deck.next-page",
      parameters: { wrapAround: { type: "boolean", value: true } },
      description: "",
      icon: "chevron-right",
      label: null,
    })
    expect(freeActionName([instance({}), instance({ name: "next page 2" })], "Next page")).toBe(
      "Next page 3",
    )
  })

  it("reuses an action that is exactly a new one, and no other", () => {
    expect(existingDefault([instance({})], nextPage)?.id).toBe("a")
    // Renamed by the user: still the same action.
    expect(existingDefault([instance({ name: "Forward" })], nextPage)?.id).toBe("a")
    // Other settings, a label of its own, another type: not that action.
    const stops = { wrapAround: { type: "boolean" as const, value: false } }
    expect(existingDefault([instance({ parameters: stops })], nextPage)).toBeUndefined()
    expect(existingDefault([instance({ label: ">" })], nextPage)).toBeUndefined()
    expect(existingDefault([instance({ actionType: "deck.previous-page" })], nextPage)).toBe(
      undefined,
    )
  })

  it("never reuses an action that still needs configuration", () => {
    // A draft of "Open web page" (no URL yet) is exactly a new one, but
    // each drop gets its own so configuring one key leaves the others.
    const draft = instance({ actionType: openUrl.id, parameters: {} })
    expect(existingDefault([draft], openUrl)).toBeUndefined()
  })

  it("searches names, descriptions, categories and modules", () => {
    const groups = catalogGroups(modules)
    const ids = (query: string) =>
      filterCatalog(groups, query).flatMap((g) => g.entries.map((e) => e.type.id))
    expect(ids("")).toHaveLength(3)
    expect(ids("indicator")).toEqual(["deck.page-indicator"])
    expect(ids("NAVIGATION")).toHaveLength(2)
    expect(ids("deck")).toHaveLength(2)
    expect(ids("soundboard")).toEqual([])
  })
})

// ---------------------------------------------------------------------------
// In the editor

const createAction = vi.fn()
const refresh = vi.fn()

const dnd = vi.hoisted(() => ({
  onDragEnd: undefined as ((event: DragEndEvent) => void) | undefined,
  data: new Map<string, unknown>(),
}))

vi.mock("@dnd-kit/core", async (original) => {
  const actual = await original<typeof import("@dnd-kit/core")>()
  return {
    ...actual,
    DndContext: (props: { onDragEnd?: (event: DragEndEvent) => void; children?: ReactNode }) => {
      dnd.onDragEnd = props.onDragEnd
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
    saveLayout: (_profile: string, saved: DeviceLayout) =>
      Promise.resolve({ ...saved, agentReload: { state: "applied" } }),
    createAction: (...args: unknown[]) => createAction(...args) as unknown,
  },
  errorMessage: (error: unknown) => (error instanceof Error ? error.message : String(error)),
}))
vi.mock("@/lib/app-context", () => ({
  useActionModules: () => [...modules, ...extraModules],
  useAppContext: () => ({ profiles: { activeProfileId: null, profiles: [] }, refresh }),
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
const layout: DeviceLayout = {
  id: "layout",
  name: "Main",
  model: mk2.id,
  homePage: "home",
  pages: [{ id: "home", name: "Home", bindings: [] }],
}

function renderEditor(actions: ActionInstance[] = []) {
  const view = {
    activeProjectId: null,
    profile: { id: "p", name: "Work", actions, workflows: [], projects: [], layouts: [layout] },
  } as unknown as ProfileView
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
  return userEvent.setup()
}

const describes = (name: string) => screen.getByRole("button", { name }).getAttribute("title")

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
  dnd.data.clear()
  refresh.mockClear()
  vi.mocked(toast.error).mockClear()
  createAction.mockReset()
  let made = 0
  createAction.mockImplementation((_profile: string, input: ActionInput) => {
    made += 1
    return Promise.resolve({ ...input, id: `new-${made}` })
  })
})
afterEach(cleanup)

describe("the catalog in the layout editor", { timeout: 20_000 }, () => {
  it("puts a type on a key by creating its action once", async () => {
    renderEditor()
    const section = screen.getByRole("region", { name: "Navigation" })
    expect(within(section).getByText("Next page")).toBeInTheDocument()
    expect(within(section).getByText("Page indicator")).toBeInTheDocument()
    // Types that need settings are offered (the Inspector asks for them);
    // deprecated types and disabled modules' are not.
    expect(
      within(screen.getByRole("region", { name: "System" })).getByText("Open web page"),
    ).toBeInTheDocument()
    expect(screen.queryByText("Next project")).not.toBeInTheDocument()
    expect(screen.queryByText("Wait")).not.toBeInTheDocument()

    drop(catalogDragId("deck.next-page"), "key:0")
    await waitFor(() => {
      expect(describes("Key 1")).toBe("Key 1: Press: Next page")
    })
    expect(createAction).toHaveBeenCalledExactlyOnceWith("p", {
      name: "Next page",
      actionType: "deck.next-page",
      parameters: { wrapAround: { type: "boolean", value: true } },
      description: "",
      icon: "chevron-right",
      label: null,
    })
    // The profile is reloaded with the new action; the library still lists
    // types only, not the profile's actions.
    expect(refresh).toHaveBeenCalledTimes(1)
    expect(screen.queryByRole("region", { name: "My actions" })).not.toBeInTheDocument()

    // The same type on another key: the same action, nothing is created.
    drop(catalogDragId("deck.next-page"), "key:1")
    await waitFor(() => {
      expect(describes("Key 2")).toBe("Key 2: Press: Next page")
    })
    expect(createAction).toHaveBeenCalledTimes(1)
    // Another type: its own action.
    drop(catalogDragId("deck.page-indicator"), "key:2")
    await waitFor(() => {
      expect(describes("Key 3")).toBe("Key 3: Press: Page indicator")
    })
    expect(createAction).toHaveBeenCalledTimes(2)
  })

  it("reuses the profile's action and says when one cannot be created", async () => {
    renderEditor([instance({ id: "mine", name: "Forward" })])
    drop(catalogDragId("deck.next-page"), "key:4")
    await waitFor(() => {
      expect(describes("Key 5")).toBe("Key 5: Press: Forward")
    })
    expect(createAction).not.toHaveBeenCalled()

    createAction.mockRejectedValueOnce(new Error("the configuration is locked"))
    drop(catalogDragId("deck.page-indicator"), "key:5")
    await waitFor(() => {
      expect(toast.error).toHaveBeenCalledWith("“Page indicator” could not be added", {
        description: "the configuration is locked",
      })
    })
    // Nothing was put on the key.
    expect(describes("Key 6")).toBe("Key 6")
  })

  it("searches the whole library and folds its groups", async () => {
    const user = renderEditor([instance({ id: "mine", name: "Forward" })])
    // Saved actions are not listed: the library is the types.
    expect(screen.queryByText("Forward")).not.toBeInTheDocument()
    const search = screen.getByRole("searchbox", { name: "Search the library" })
    await user.type(search, "indicator")
    expect(screen.getByText("Page indicator")).toBeInTheDocument()
    expect(screen.queryByText("Next page")).not.toBeInTheDocument()
    expect(screen.queryByText("New folder")).not.toBeInTheDocument()
    await user.clear(search)
    await user.type(search, "zzz")
    expect(screen.getByText("Nothing matches.")).toBeInTheDocument()
    await user.clear(search)
    expect(screen.getByText("Next page")).toBeInTheDocument()
    expect(screen.getByText("New folder")).toBeInTheDocument()

    const fold = screen.getByRole("button", { name: /^Navigation/ })
    expect(fold).toHaveAttribute("aria-expanded", "true")
    await user.click(fold)
    expect(fold).toHaveAttribute("aria-expanded", "false")
    expect(screen.queryByText("Page indicator")).not.toBeInTheDocument()
    // A search shows what it finds, also in a folded group.
    await user.type(search, "indicator")
    expect(screen.getByText("Page indicator")).toBeInTheDocument()
  })
})

describe("the library's categories", { timeout: 20_000 }, () => {
  beforeEach(() => {
    extraModules = [moduleInfo({ id: "lock", name: "Lock", actionTypes: [lockScreen] })]
  })
  afterEach(() => {
    extraModules = []
  })
  const sections = () =>
    screen.getAllByRole("region").map((section) => section.getAttribute("aria-label"))
  const count = (name: string) =>
    within(screen.getByRole("region", { name }))
      .getByRole("button", { expanded: true })
      .querySelector("[aria-hidden].tabular-nums")?.textContent

  it("are named, not prefixed, with navigation in one section", () => {
    renderEditor()
    expect(document.body.textContent).not.toMatch(/New:/)
    // One Navigation section: the pages, New folder and the navigation types.
    expect(sections().filter((name) => name === "Navigation")).toHaveLength(1)
    const navigation = within(screen.getByRole("region", { name: "Navigation" }))
    for (const name of ["New folder", "Next page", "Page indicator"]) {
      expect(navigation.getByText(name)).toBeInTheDocument()
    }
    const entries = screen
      .getByRole("region", { name: "Navigation" })
      .querySelectorAll(":scope > ul > *").length
    expect(entries).toBeGreaterThanOrEqual(3)
    expect(count("Navigation")).toBe(String(entries))
    const system = within(screen.getByRole("region", { name: "System" }))
    expect(system.getByText("Lock screen")).toBeInTheDocument()
    expect(system.getByText("Open web page")).toBeInTheDocument()
    expect(count("System")).toBe("2")
  })

  it("are in Polish in a Polish window, and found by their Polish names", async () => {
    await act(async () => {
      await i18n.changeLanguage("pl")
    })
    try {
      const user = renderEditor()
      expect(document.body.textContent).not.toMatch(/Nowe:|New:/)
      expect(sections()).toContain("Nawigacja")
      expect(sections()).not.toContain("Navigation")
      expect(sections()).toContain("System")
      const search = screen.getByRole("searchbox")
      await user.type(search, "nawigacja")
      expect(screen.getByText("Page indicator")).toBeInTheDocument()
      expect(screen.queryByText("Lock screen")).not.toBeInTheDocument()
      // The action and its stable type are the same whatever the language.
      drop(catalogDragId("system.lock"), "key:0")
      await waitFor(() => {
        expect(createAction).toHaveBeenCalledWith(
          "p",
          expect.objectContaining({ actionType: "system.lock" }),
        )
      })
    } finally {
      await act(async () => {
        await i18n.changeLanguage("en")
      })
    }
  })
})
