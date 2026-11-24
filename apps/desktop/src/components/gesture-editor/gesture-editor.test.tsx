// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { LayoutEditor } from "@/layout-editor/editor"
import type { ActionType, DeviceLayout, DeviceModel, ProfileView } from "@/lib/types"
import { api, resetApi } from "@/test/api-mock"
import { drop, dnd, resetDnd } from "@/test/dnd"
import { savedByAgent } from "@/test/fixtures"

vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)
// Actions are chosen by dragging them from the Action Library.
vi.mock("@dnd-kit/core", async (original) =>
  (await import("@/test/dnd")).dndModule(await original()),
)
vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)
// The real module (with `AppContext`, which the variable picker reads), but a
// fixed `useAppContext` for the editor.
// The Work Session module is enabled unless a test turns it off.
// `noPlugins`: a build with plugins where none is installed (no Workflows plugin).
const enabledModules = vi.hoisted(() => ({ workSession: true, noPlugins: false }))
vi.mock("@/lib/app-context", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/app-context")>()),
  // The library lists types: Log message (whose action "Mail" is exactly a
  // new one, complete with the declared default, so placing the type uses
  // it; an incomplete one would be a draft of its own on every drop).
  useActionModules: () => [
    {
      id: "general",
      name: "General",
      enabled: true,
      supported: true,
      actionTypes: [
        {
          id: "general.log",
          name: "Log message",
          description: "",
          category: "general",
          requiresProject: false,
          parameters: [
            {
              key: "message",
              label: "Message",
              kind: { type: "text", multiline: false },
              required: true,
              default: { type: "text", value: "mail" },
            },
          ],
        },
      ],
    },
    ...(enabledModules.workSession ? [{ id: "work-session", enabled: true, actionTypes: [] }] : []),
  ],
  useAppContext: () => ({
    profiles: { activeProfileId: null, profiles: [] },
    refresh: vi.fn(),
    ...(enabledModules.noPlugins ? { modules: [], pluginModules: [] } : {}),
  }),
}))

const mk2: DeviceModel = {
  id: "elgato.streamdeck.mk2",
  vendor: "Elgato",
  name: "Stream Deck MK.2",
  usb: null,
  capabilities: { keys: { rows: 3, columns: 5, image: null }, encoders: null, touch: null },
  support: "verified",
  verification: [],
}

const logType: ActionType = {
  id: "general.log",
  name: "Log message",
  description: "",
  category: "general",
  requiresProject: false,
  parameters: [
    { key: "message", label: "Message", kind: { type: "text", multiline: false }, required: true },
    {
      key: "level",
      label: "Level",
      kind: { type: "choice", options: ["info", "warn"] },
      required: false,
    },
  ],
}

const teamName = { type: "reference", value: "shared.team.name" } as const

/**
 * Key 1: a raw press (0.6.0 style). Key 2: three gestures, one with a
 * shared-variable override, and its own long-press time. The Tools page has
 * gestures too, so a save must carry every page as it was.
 */
const layout: DeviceLayout = {
  id: "layout",
  name: "Main",
  model: mk2.id,
  homePage: "home",
  explicitTree: true,
  pages: [
    {
      id: "home",
      name: "Home",
      bindings: [
        {
          control: { kind: "key", index: 0 },
          trigger: { type: "press" },
          target: { type: "action", action: "shop" },
          label: "Shop",
        },
        {
          control: { kind: "key", index: 1 },
          trigger: { type: "click" },
          target: { type: "action", action: "shop", overrides: { message: teamName } },
          label: null,
          appearance: { background: "#204060" },
        },
        {
          control: { kind: "key", index: 1 },
          trigger: { type: "doubleClick" },
          target: { type: "workflow", workflow: "deploy" },
        },
        {
          control: { kind: "key", index: 1 },
          trigger: { type: "longPress" },
          target: { type: "openPage", page: "tools" },
        },
      ],
      gestures: [{ control: { kind: "key", index: 1 }, longPressMs: 1500 }],
    },
    {
      // A top-level page: Key 2 opens it with a long press next to its
      // other gestures, an ordinary page key. A key that opened a folder of
      // its page would be a folder key, which takes no gestures
      // (lib/folder-control.ts, layout-editor/folder-control.test.tsx).
      id: "tools",
      name: "Tools",
      bindings: [
        {
          control: { kind: "key", index: 0 },
          trigger: { type: "press" },
          target: { type: "back" },
        },
        {
          control: { kind: "key", index: 4 },
          trigger: { type: "longPress" },
          target: { type: "action", action: "mail", overrides: { level: teamName } },
        },
      ],
      gestures: [{ control: { kind: "key", index: 4 }, doubleClickMs: 500 }],
    },
  ],
}

const view = {
  activeProjectId: null,
  issues: [],
  actionUsages: {},
  workflowUsages: {},
  profile: {
    id: "p",
    name: "Work",
    description: "",
    actions: [
      {
        id: "shop",
        name: "Shop",
        actionType: logType.id,
        parameters: { message: { type: "text", value: "shop" } },
      },
      {
        id: "mail",
        name: "Mail",
        actionType: logType.id,
        parameters: { message: { type: "text", value: "mail" } },
      },
    ],
    workflows: [{ id: "deploy", name: "Deploy", steps: [] }],
    projects: [],
    layouts: [layout],
  },
} as unknown as ProfileView

function renderEditor(saved: DeviceLayout = layout) {
  const user = userEvent.setup()
  render(
    <LayoutEditor
      profileId="p"
      view={view}
      model={mk2}
      saved={saved}
      onDirtyChange={vi.fn()}
      onSaved={vi.fn()}
    />,
  )
  return user
}

const key = (name: string) => screen.getByRole("button", { name })
const describes = (name: string) => key(name).getAttribute("title")
/** What a gesture's card shows it does: its action's name, never a chooser. */
const row = (name: string) => {
  const card = screen.getByRole("group", { name })
  const shown = card.querySelector<HTMLElement>(
    "[data-slot=gesture-card-action], [data-slot=gesture-card-empty]",
  )
  if (!shown) throw new Error(`no card for ${name}`)
  return shown
}

const SLOTS: Record<string, string> = {
  "Single press": "click",
  "Double click": "doubleClick",
  "Long press": "longPress",
  Release: "release",
}

/** The library's drag id of what is named `option`. */
const LIBRARY: Record<string, string> = {
  // A type: its action that is exactly a new one ("Mail") is used.
  Mail: "library:type:general.log",
  Back: "library:back",
  Deploy: "library:workflow:deploy",
  Multi: "library:workflow:multi",
  "Work Session controller": "library:stateful:workSession",
}

/**
 * Assigns `option` to `gesture` of the selected control as the user does:
 * dragged from the Action Library onto the gesture's card ("Nothing": the
 * card's X).
 */
async function choose(user: ReturnType<typeof userEvent.setup>, gesture: string, option: string) {
  if (option === "Nothing") {
    await user.click(screen.getByRole("button", { name: `Remove ${gesture}` }))
    return
  }
  const slot = [...dnd.droppables].find(
    (id) => id.startsWith("gesture-slot:") && id.endsWith(`:${SLOTS[gesture] ?? gesture}`),
  )
  drop(LIBRARY[option] ?? option, slot ?? `no slot for ${gesture}`)
  // The drop resolves what was dragged before it assigns it.
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0))
  })
}

async function save(user: ReturnType<typeof userEvent.setup>): Promise<DeviceLayout> {
  await user.click(screen.getByRole("button", { name: /Save/ }))
  await waitFor(() => {
    expect(api.saveLayout).toHaveBeenCalled()
  })
  return api.saveLayout.mock.calls.at(-1)?.[1] as DeviceLayout
}

beforeAll(() => {
  // Radix selects rely on browser APIs that jsdom does not implement.
  Element.prototype.hasPointerCapture = () => false
  Element.prototype.releasePointerCapture = () => undefined
  Element.prototype.scrollIntoView = () => undefined
})
beforeEach(() => {
  resetDnd()
  resetApi()
  api.previewLayout.mockResolvedValue([])
  api.listIcons.mockResolvedValue([])
  api.listActionTypes.mockResolvedValue([logType])
  api.getGestureDefaults.mockResolvedValue({ doubleClickMs: 350, longPressMs: 800 })
  api.saveLayout.mockImplementation((_, saved) => Promise.resolve(savedByAgent(saved)))
})
afterEach(() => {
  cleanup()
  enabledModules.noPlugins = false
  vi.unstubAllEnvs()
})

describe("gesture editor without the Workflows plugin", () => {
  it("keeps a graph workflow on its gesture, read-only, and offers it nowhere else", async () => {
    vi.stubEnv("VITE_DECKPILOT_PLUGINS", "1")
    enabledModules.noPlugins = true
    const graph = { version: 1, nodes: [], edges: [] }
    const withGraph = {
      ...view,
      profile: {
        ...view.profile,
        workflows: [
          { id: "deploy", name: "Deploy", steps: [], graph },
          // A step workflow is a Multi Action, the key editor's own.
          { id: "multi", name: "Multi", steps: [] },
        ],
      },
    } as unknown as ProfileView
    const user = userEvent.setup()
    render(
      <LayoutEditor
        profileId="p"
        view={withGraph}
        model={mk2}
        saved={layout}
        onDirtyChange={vi.fn()}
        onSaved={vi.fn()}
      />,
    )
    await user.click(key("Key 2"))
    // Still assigned, not run while the plugin is off, and not editable here.
    expect(describes("Key 2")).toContain("Double click: Deploy")
    expect(
      screen.getByText(
        /^Read-only: the Workflows plugin is off\. Keys keep this workflow but do not run it/,
      ),
    ).toBeInTheDocument()
    // Not offered for another gesture (the library does not let it be
    // dragged, and a drop is refused); the Multi Action is.
    await user.click(key("Key 3"))
    expect(screen.getByRole("button", { name: "Drag Multi onto a control" })).toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "Drag Deploy onto a control" })).toBeNull()
    await choose(user, "Double click", "Deploy")
    expect(describes("Key 3")).toBe("Key 3")
    await choose(user, "Double click", "Multi")
    await waitFor(() => {
      expect(describes("Key 3")).toBe("Key 3: Double click: Multi")
    })
  })
})

describe("gesture editor", () => {
  // Adding and removing are separate tests: each select round trip
  // re-renders the whole editor, and eight of them plus a save in one test
  // made it the slowest of the suite, close to the 5 s limit under load.
  it("adds single click, double click and long press", async () => {
    const user = renderEditor()
    await user.click(key("Key 3"))
    expect(screen.getByRole("heading", { name: "Gestures" })).toBeInTheDocument()

    await choose(user, "Double click", "Deploy")
    expect(describes("Key 3")).toBe("Key 3: Double click: Deploy")
    await choose(user, "Long press", "Mail")
    await choose(user, "Single press", "Back")
    expect(describes("Key 3")).toBe("Key 3: Double click: Deploy · Long press: Mail · Click: Back")
    expect(screen.getByRole("note")).toHaveTextContent("after the double-click window (350 ms)")
    expect(screen.getByLabelText("Double-click window (ms)")).toHaveAttribute(
      "placeholder",
      "Default (350)",
    )
  })

  it("removes gestures until a single click is the immediate press again", async () => {
    const keyThree = { kind: "key", index: 2 } as const
    const withGestures: DeviceLayout = {
      ...layout,
      pages: layout.pages.map((page) => {
        if (page.id !== "home") return page
        return {
          ...page,
          bindings: [
            ...page.bindings,
            {
              control: keyThree,
              trigger: { type: "click" },
              target: { type: "action", action: "shop" },
            },
            {
              control: keyThree,
              trigger: { type: "doubleClick" },
              target: { type: "workflow", workflow: "deploy" },
            },
            {
              control: keyThree,
              trigger: { type: "longPress" },
              target: { type: "action", action: "mail" },
            },
          ],
        }
      }),
    }
    const user = renderEditor(withGestures)
    await user.click(key("Key 3"))
    expect(describes("Key 3")).toBe("Key 3: Click: Shop · Double click: Deploy · Long press: Mail")

    await choose(user, "Double click", "Nothing")
    expect(screen.getByRole("note")).toHaveTextContent("when the key is released")
    await choose(user, "Long press", "Nothing")
    expect(screen.getByRole("note")).toHaveTextContent("as soon as the key is pressed")
    await choose(user, "Single press", "Nothing")
    expect(describes("Key 3")).toBe("Key 3")

    // An empty key's single click alone is the immediate press (D2).
    await choose(user, "Single press", "Mail")
    expect(describes("Key 3")).toBe("Key 3: Press: Mail")
    const saved = await save(user)
    expect(saved.pages[0]?.bindings.filter((b) => b.control.index === 2)).toEqual([
      {
        control: keyThree,
        trigger: { type: "press" },
        target: { type: "action", action: "mail" },
        label: null,
      },
    ])
  })

  it("removes one gesture with its X, keeping the others, the title and the look", async () => {
    const keyThree = { kind: "key", index: 2 } as const
    const look = { background: "#112233", iconId: "icon-1", showTitle: true }
    const withGestures: DeviceLayout = {
      ...layout,
      pages: layout.pages.map((page) =>
        page.id !== "home"
          ? page
          : {
              ...page,
              bindings: [
                ...page.bindings,
                {
                  control: keyThree,
                  trigger: { type: "click" },
                  target: { type: "action", action: "shop" },
                  label: "Shop now",
                  appearance: look,
                },
                {
                  control: keyThree,
                  trigger: { type: "doubleClick" },
                  target: { type: "workflow", workflow: "deploy" },
                },
                {
                  control: keyThree,
                  trigger: { type: "longPress" },
                  target: { type: "action", action: "mail" },
                },
              ],
            },
      ),
    }
    const user = renderEditor(withGestures)
    await user.click(key("Key 3"))
    // No "Clear control" in the Inspector: each gesture's X removes its action.
    expect(screen.queryByRole("button", { name: /Clear control/ })).toBeNull()

    await user.click(screen.getByRole("button", { name: "Remove Single press" }))
    // The key keeps its title: the double click, now the shown binding, carries it.
    expect(describes("Key 3")).toBe("Key 3: Double click: Shop now · Long press: Mail")
    // The emptied card is a drop target again.
    expect(row("Single press")).toHaveAccessibleName("Nothing assigned to Single press")
    expect(row("Single press")).toHaveTextContent("Drag an action here")

    await user.click(screen.getByRole("button", { name: "Remove Long press" }))
    // Undo brings the long press back; redo removes it again.
    await user.click(screen.getByRole("button", { name: "Undo" }))
    expect(describes("Key 3")).toBe("Key 3: Double click: Shop now · Long press: Mail")
    await user.click(screen.getByRole("button", { name: "Redo" }))
    expect(describes("Key 3")).toBe("Key 3: Double click: Shop now")

    const saved = await save(user)
    // The double click now carries the key's title and look.
    expect(saved.pages[0]?.bindings.filter((b) => b.control.index === 2)).toEqual([
      {
        control: keyThree,
        trigger: { type: "doubleClick" },
        target: { type: "workflow", workflow: "deploy" },
        label: "Shop now",
        appearance: look,
      },
    ])
  })

  it("replaces one gesture's action by a drop, previews it, and saves or reverts it", async () => {
    const user = renderEditor()
    await user.click(key("Key 2"))
    expect(row("Single press")).toHaveTextContent("Shop")
    await choose(user, "Single press", "Mail")
    expect(describes("Key 2")).toBe(
      "Key 2: Click: Mail · Double click: Deploy · Long press: Open “Tools”",
    )
    expect(row("Single press")).toHaveTextContent("Mail")
    expect(row("Double click")).toHaveTextContent("Deploy")
    // The preview is asked for the replaced action.
    await waitFor(() => {
      const draft = api.previewLayout.mock.calls.at(-1)?.[1]
      expect(draft?.pages[0]?.bindings.find((b) => b.control.index === 1)?.target).toEqual({
        type: "action",
        action: "mail",
      })
    })
    // Revert brings the saved one back; nothing is saved.
    await user.click(screen.getByRole("button", { name: /Revert/ }))
    await waitFor(() => {
      expect(row("Single press")).toHaveTextContent("Shop")
    })
    expect(api.saveLayout).not.toHaveBeenCalled()

    await choose(user, "Single press", "Mail")
    const saved = await save(user)
    // Only the single press changed: its old overrides go with its old
    // action, the key's look stays, the other gestures are as they were.
    expect(saved.pages[0]?.bindings.filter((b) => b.control.index === 1)).toEqual([
      {
        control: { kind: "key", index: 1 },
        trigger: { type: "click" },
        target: { type: "action", action: "mail" },
        label: null,
        appearance: { background: "#204060" },
      },
      {
        control: { kind: "key", index: 1 },
        trigger: { type: "doubleClick" },
        target: { type: "workflow", workflow: "deploy" },
      },
      {
        control: { kind: "key", index: 1 },
        trigger: { type: "longPress" },
        target: { type: "openPage", page: "tools" },
      },
    ])
  })

  it("asks before turning a press into a click and changes nothing on cancel", async () => {
    const user = renderEditor()
    await user.click(key("Key 1"))
    expect(row("Single press")).toHaveTextContent("Shop")

    await choose(user, "Long press", "Mail")
    const dialog = await screen.findByRole("alertdialog", { name: "Switch Key 1 to gestures?" })
    expect(dialog).toHaveTextContent("acts as soon as it is pressed")
    expect(dialog).toHaveTextContent("before the long-press time (800 ms)")
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }))
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument()
    expect(describes("Key 1")).toBe("Key 1: Press: Shop")
    expect(screen.queryByText("Unsaved")).not.toBeInTheDocument()

    await choose(user, "Double click", "Deploy")
    const confirm = await screen.findByRole("alertdialog")
    expect(confirm).toHaveTextContent("after the double-click window (350 ms)")
    await user.click(within(confirm).getByRole("button", { name: "Switch to gestures" }))
    expect(describes("Key 1")).toBe("Key 1: Click: Shop · Double click: Deploy")
    expect(row("Single press")).toHaveTextContent("Shop")

    const saved = await save(user)
    const keyOne = saved.pages[0]?.bindings.filter((b) => b.control.index === 0)
    expect(keyOne).toEqual([
      {
        control: { kind: "key", index: 0 },
        trigger: { type: "click" },
        target: { type: "action", action: "shop" },
        label: "Shop",
      },
      {
        control: { kind: "key", index: 0 },
        trigger: { type: "doubleClick" },
        target: { type: "workflow", workflow: "deploy" },
        label: null,
      },
    ])
  })

  it("offers the Work Session controller on gestures and saves a stateful target", async () => {
    const workSession = { type: "stateful", machine: "workSession" } as const
    const user = renderEditor()
    await user.click(key("Key 3"))

    // Not on the raw release: a stateful key needs gestures.
    await choose(user, "Release", "Work Session controller")
    expect(describes("Key 3")).toBe("Key 3")

    await choose(user, "Single press", "Work Session controller")
    expect(describes("Key 3")).toBe("Key 3: Click: Work Session")
    await choose(user, "Long press", "Work Session controller")
    expect(row("Long press")).toHaveTextContent("Work Session controller")

    const saved = await save(user)
    expect(saved.pages[0]?.bindings.filter((b) => b.control.index === 2)).toEqual([
      {
        control: { kind: "key", index: 2 },
        trigger: { type: "click" },
        target: workSession,
        label: null,
      },
      {
        control: { kind: "key", index: 2 },
        trigger: { type: "longPress" },
        target: workSession,
        label: null,
      },
    ])
    // No action instance stands in for the session.
    expect(api.createAction).not.toHaveBeenCalled()
    expect(api.updateAction).not.toHaveBeenCalled()
  })

  it("offers no Work Session controller without the Work Session module", async () => {
    enabledModules.workSession = false
    try {
      const user = renderEditor()
      await user.click(key("Key 3"))
      expect(
        screen.getByRole("button", { name: "Drag Log message onto a control" }),
      ).toBeInTheDocument()
      expect(
        screen.queryByRole("button", { name: "Drag Work Session controller onto a control" }),
      ).toBeNull()
    } finally {
      enabledModules.workSession = true
    }
  })

  it("asks before a pressed key becomes a Work Session controller", async () => {
    const user = renderEditor()
    await user.click(key("Key 1"))
    await choose(user, "Single press", "Work Session controller")
    const dialog = await screen.findByRole("alertdialog", { name: "Switch Key 1 to gestures?" })
    expect(dialog).toHaveTextContent("still fires on the press")
    await user.click(within(dialog).getByRole("button", { name: "Switch to gestures" }))

    const saved = await save(user)
    expect(saved.pages[0]?.bindings.filter((b) => b.control.index === 0)).toEqual([
      {
        control: { kind: "key", index: 0 },
        trigger: { type: "click" },
        target: { type: "stateful", machine: "workSession" },
        label: "Shop",
      },
    ])
  })

  it("keeps raw release bindings and says the conversion removes them", async () => {
    const user = renderEditor()
    await user.click(key("Key 1"))
    await choose(user, "Release", "Mail")
    expect(describes("Key 1")).toBe("Key 1: Press: Shop · Release: Mail")

    await choose(user, "Long press", "Deploy")
    const dialog = await screen.findByRole("alertdialog")
    expect(dialog).toHaveTextContent("Its release binding (“Mail”) is removed")
    await user.click(within(dialog).getByRole("button", { name: "Switch to gestures" }))
    expect(describes("Key 1")).toBe("Key 1: Click: Shop · Long press: Deploy")
    // A gesture key offers no raw release.
    expect(screen.queryByRole("combobox", { name: "Release" })).not.toBeInTheDocument()
  })

  it("overrides parameters per gesture and saves a variable reference as a reference", async () => {
    const user = renderEditor()
    await user.click(key("Key 2"))
    // Overrides are this key's or gesture's only, apart from the action's definition.
    const toggle = screen.getByRole("button", {
      name: /Parameters for this key\/gesture only \(1 overridden\)/,
    })
    expect(toggle).toHaveAttribute("aria-expanded", "false")
    await user.click(toggle)
    expect(toggle).toHaveAttribute("aria-expanded", "true")
    expect(screen.getByRole("checkbox", { name: "Override “Message”" })).toBeChecked()

    // A second override next to the reference, from the shared editors.
    await user.click(screen.getByRole("checkbox", { name: "Override “Level”" }))
    await user.click(screen.getByRole("combobox", { name: "Level" }))
    await user.click(await screen.findByRole("option", { name: "warn" }))

    const saved = await save(user)
    const click = saved.pages[0]?.bindings.find(
      (b) => b.control.index === 1 && b.trigger.type === "click",
    )
    expect(click?.target).toEqual({
      type: "action",
      action: "shop",
      overrides: { message: teamName, level: { type: "choice", value: "warn" } },
    })
    // The saved action itself is not touched.
    expect(api.updateAction).not.toHaveBeenCalled()
    expect(api.createAction).not.toHaveBeenCalled()

    // Removing every override leaves no empty `overrides`.
    await user.click(screen.getByRole("checkbox", { name: "Override “Message”" }))
    await user.click(screen.getByRole("checkbox", { name: "Override “Level”" }))
    const cleared = await save(user)
    expect(
      cleared.pages[0]?.bindings.find((b) => b.control.index === 1 && b.trigger.type === "click")
        ?.target,
    ).toEqual({ type: "action", action: "shop" })
  })

  it("validates per-key timings and stores only overrides", async () => {
    const user = renderEditor()
    await user.click(key("Key 2"))
    const long = screen.getByLabelText("Long-press time (ms)")
    expect(long).toHaveValue(1500)
    const double = screen.getByLabelText("Double-click window (ms)")
    expect(double).toHaveValue(null)
    expect(double).toHaveAttribute("placeholder", "Default (350)")

    await user.type(double, "90")
    expect(double).toHaveAttribute("aria-invalid", "true")
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Double-click window must be between 150 and 1000 ms.",
    )
    expect(double).toHaveAccessibleDescription(/between 150 and 1000 ms/)
    await user.clear(double)
    await user.type(double, "450")
    expect(double).not.toHaveAttribute("aria-invalid")

    await user.click(screen.getByRole("button", { name: "Use the default long-press time" }))
    expect(long).toHaveValue(null)

    const saved = await save(user)
    expect(saved.pages[0]?.gestures).toEqual([
      { control: { kind: "key", index: 1 }, doubleClickMs: 450 },
    ])
  })

  it("saves every gesture, override, timing and page it did not edit", async () => {
    const user = renderEditor()
    await user.click(key("Key 2"))
    await user.type(screen.getByRole("textbox", { name: "Label for Double click" }), "Ship")
    const saved = await save(user)
    const expected = structuredClone(layout)
    const double = expected.pages[0]?.bindings[2]
    if (double) double.label = "Ship"
    expect(saved).toEqual(expected)
  })

  it("shows the backend's validation messages at the control", async () => {
    api.saveLayout.mockRejectedValue(
      new Error(
        "page `Home`: key 1: the long-press time must be between 300 and 5000 ms, not 9000 ms\npage `Home`: key 12 shows an icon that is not in the profile",
      ),
    )
    const user = renderEditor()
    await user.click(key("Key 2"))
    await user.type(screen.getByRole("textbox", { name: "Label for Long press" }), "x")
    await user.click(screen.getByRole("button", { name: /Save/ }))
    // Inline at the control, named as in the editor (the backend counts from 0).
    const inline = await screen.findByText(
      "Key 2: the long-press time must be between 300 and 5000 ms, not 9000 ms",
      { selector: "li" },
    )
    const list = inline.closest("ul")
    expect(list).toHaveAttribute("role", "alert")
    expect(list).not.toHaveTextContent("icon")
  })
})
