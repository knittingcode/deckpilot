// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

// Action contexts in the device editor (lib/action-context.ts): what the
// library offers for keys, the Displays section, and saved bindings whose
// action does not work where they are, with what they can become.

import { cleanup, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { toast } from "sonner"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { moduleInfo } from "@/lib/test-fixtures"
import type { ActionType, DeviceLayout, DeviceModel, ModuleInfo, ProfileView } from "@/lib/types"

import { LayoutEditor } from "./editor"
import { resetDnd } from "@/test/dnd"

Element.prototype.hasPointerCapture = () => false
Element.prototype.releasePointerCapture = () => undefined
Element.prototype.scrollIntoView = () => undefined

const type = (
  id: string,
  name: string,
  contexts?: ActionType["contexts"],
  category = "General",
): ActionType => ({
  id,
  name,
  description: `${name}.`,
  category,
  requiresProject: false,
  parameters: [],
  ...(contexts ? { contexts } : {}),
})
/** Key and gesture only. */
const nextPage = type("deck.next-page", "Next page", ["key", "gesture"], "Navigation")
/** A step or a node, never a key. */
const log = type("general.log", "Log", ["sequence", "workflow"])
/** Shows something on a key; its press does nothing. */
const indicator = type("deck.page-indicator", "Page indicator", ["display"])
/** Named like the step-only Wait, but declares nothing: it goes everywhere. */
const wait = type("general.wait", "Wait")
/** As the agent declares Delay (and Wait) since #407: a step or a node. */
const delay = type("general.delay", "Delay", ["sequence", "workflow"])

const modules: ModuleInfo[] = [
  moduleInfo({ id: "deck", actionTypes: [nextPage, indicator] }),
  moduleInfo({ id: "general", actionTypes: [log, wait, delay] }),
]

vi.mock("@dnd-kit/core", async (original) =>
  (await import("@/test/dnd")).dndModule(await original()),
)

const api = vi.hoisted(() => ({
  previewLayout: vi.fn(),
  saveLayout: vi.fn(),
  createAction: vi.fn(),
  saveWorkflow: vi.fn(),
}))

vi.mock("@/lib/api", () => ({
  api: {
    previewLayout: (...args: unknown[]) => api.previewLayout(...args) as unknown,
    listIcons: () => Promise.resolve([]),
    pageTree: () => Promise.resolve({ pages: [], warnings: [] }),
    listActionTypes: () => Promise.resolve([nextPage, log, indicator, wait]),
    saveWorkflow: (...args: unknown[]) => api.saveWorkflow(...args) as unknown,
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
const actions = [
  { id: "next", name: "Forward", actionType: nextPage.id, parameters: {}, icon: null },
  { id: "log", name: "Write log", actionType: log.id, parameters: {}, icon: null },
  { id: "pages", name: "Pages shown", actionType: indicator.id, parameters: {}, icon: null },
  { id: "wait", name: "Pause", actionType: wait.id, parameters: {}, icon: null },
  { id: "short", name: "Short delay", actionType: delay.id, parameters: {}, icon: null },
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
  api.saveWorkflow.mockReset()
  api.saveWorkflow.mockImplementation((_profile: string, workflow: { id: string }) =>
    Promise.resolve({ id: workflow.id }),
  )
})
afterEach(cleanup)

describe("the library by context", { timeout: 20_000 }, () => {
  it("offers what goes on a key, lists display-only ones apart, and hides step and node actions", async () => {
    renderEditor(mk2)
    // Types only: no saved actions ("My actions") in the library.
    expect(screen.queryByRole("region", { name: "My actions" })).not.toBeInTheDocument()
    expect(screen.queryByText("Forward")).not.toBeInTheDocument()
    // Key and gesture; and a type that declares nothing, whatever its name.
    expect(screen.getByText("Next page")).toBeInTheDocument()
    expect(screen.getByText("Wait")).toBeInTheDocument()
    // A step-and-node type is not offered for keys at all.
    expect(screen.queryByText("Log")).not.toBeInTheDocument()
    // Display-only types: a section of their own; no saved actions in it.
    const displays = within(screen.getByRole("region", { name: "Displays" }))
    expect(displays.getByText("Page indicator")).toBeInTheDocument()
    expect(displays.queryByText("Pages shown")).not.toBeInTheDocument()
    // Search covers it too.
    await userEvent
      .setup()
      .type(screen.getByRole("searchbox", { name: "Search the library" }), "indicator")
    expect(screen.getByRole("region", { name: "Displays" })).toHaveTextContent("Page indicator")
  })
})

describe("saved bindings whose action does not work there", { timeout: 20_000 }, () => {
  const stored = () =>
    layoutFor(mk2, [
      {
        control: { kind: "key", index: 0 },
        trigger: { type: "press" },
        target: { type: "action", action: "log" },
      },
    ])

  it("are kept and marked with the reason in words, offering what they can become", async () => {
    const user = renderEditor(mk2, stored())
    await user.click(control("Key 1"))
    const note = await screen.findByText(
      /This action does not work here: Key\. It works as: Multi Action step, Workflow node\./,
    )
    expect(note).toHaveTextContent("Kept from the saved configuration; it will not run here.")
    const offer = within(screen.getByRole("group", { name: "Not available here" }))
    // It works as a step and as a node: both are offered.
    expect(offer.getByRole("button", { name: "Wrap in a Multi Action" })).toBeInTheDocument()
    expect(offer.getByRole("button", { name: "Move to a workflow" })).toBeInTheDocument()
    // Keep: nothing changes, nothing is saved.
    await user.click(offer.getByRole("button", { name: "Keep" }))
    expect(screen.queryByRole("group", { name: "Not available here" })).not.toBeInTheDocument()
    expect(api.saveWorkflow).not.toHaveBeenCalled()
    expect(screen.getByRole("button", { name: /Save/ })).toBeDisabled()
  })

  it("wrap in a Multi Action only after confirmation, through the workflow save", async () => {
    const user = renderEditor(mk2, stored())
    await user.click(control("Key 1"))
    const offer = within(await screen.findByRole("group", { name: "Not available here" }))
    await user.click(offer.getByRole("button", { name: "Wrap in a Multi Action" }))
    const confirm = await screen.findByRole("alertdialog", {
      name: "Wrap Write log in a Multi Action?",
    })
    expect(api.saveWorkflow).not.toHaveBeenCalled()
    await user.click(within(confirm).getByRole("button", { name: "Wrap in a Multi Action" }))
    await waitFor(() => {
      expect(api.saveWorkflow).toHaveBeenCalledTimes(1)
    })
    const workflow = api.saveWorkflow.mock.calls[0]?.[1] as {
      name: string
      steps: { action: string }[]
      graph?: unknown
    }
    expect(workflow.name).toBe("Write log")
    expect(workflow.steps.map((s) => s.action)).toEqual(["log"])
    expect(workflow.graph).toBeUndefined()
    expect(toast.success).toHaveBeenCalledWith("Write log now runs in a Multi Action")
    // The binding now runs the Multi Action; the layout is saved with Save.
    expect(screen.getByRole("button", { name: /Save/ })).toBeEnabled()
    expect(screen.queryByText(/This action does not work here/)).not.toBeInTheDocument()
  })

  const withOverrides = () =>
    layoutFor(mk2, [
      {
        control: { kind: "key", index: 0 },
        trigger: { type: "press" },
        target: {
          type: "action",
          action: "log",
          overrides: { message: { type: "text", value: "from the key" } },
        },
      },
    ])

  it("wraps with the binding's own settings as the step's", async () => {
    const user = renderEditor(mk2, withOverrides())
    await user.click(control("Key 1"))
    const offer = within(await screen.findByRole("group", { name: "Not available here" }))
    await user.click(offer.getByRole("button", { name: "Wrap in a Multi Action" }))
    const confirm = await screen.findByRole("alertdialog", {
      name: "Wrap Write log in a Multi Action?",
    })
    await user.click(within(confirm).getByRole("button", { name: "Wrap in a Multi Action" }))
    await waitFor(() => {
      expect(api.saveWorkflow).toHaveBeenCalledTimes(1)
    })
    const workflow = api.saveWorkflow.mock.calls[0]?.[1] as {
      steps: { action: string; overrides?: Record<string, unknown> }[]
    }
    expect(workflow.steps).toHaveLength(1)
    expect(workflow.steps[0]?.overrides).toEqual({
      message: { type: "text", value: "from the key" },
    })
  })

  it("moves with the binding's own settings as the node's", async () => {
    const user = renderEditor(mk2, withOverrides())
    await user.click(control("Key 1"))
    const offer = within(await screen.findByRole("group", { name: "Not available here" }))
    await user.click(offer.getByRole("button", { name: "Move to a workflow" }))
    const confirm = await screen.findByRole("alertdialog", {
      name: "Move Write log to a workflow?",
    })
    await user.click(within(confirm).getByRole("button", { name: "Move to a workflow" }))
    await waitFor(() => {
      expect(api.saveWorkflow).toHaveBeenCalledTimes(1)
    })
    const workflow = api.saveWorkflow.mock.calls[0]?.[1] as {
      graph?: { nodes: { type: string; overrides?: Record<string, unknown> }[] }
    }
    const node = workflow.graph?.nodes.find((n) => n.type === "action")
    expect(node?.overrides).toEqual({ message: { type: "text", value: "from the key" } })
  })

  it("keeps edits made while the new workflow is being saved", async () => {
    let finish: (value: { id: string }) => void = () => undefined
    api.saveWorkflow.mockImplementation(
      (_profile: string, workflow: { id: string }) =>
        new Promise((resolve) => {
          finish = () => {
            resolve({ id: workflow.id })
          }
        }),
    )
    const user = renderEditor(mk2, stored())
    await user.click(control("Key 1"))
    const offer = within(await screen.findByRole("group", { name: "Not available here" }))
    await user.click(offer.getByRole("button", { name: "Wrap in a Multi Action" }))
    const confirm = await screen.findByRole("alertdialog", {
      name: "Wrap Write log in a Multi Action?",
    })
    await user.click(within(confirm).getByRole("button", { name: "Wrap in a Multi Action" }))
    await waitFor(() => {
      expect(api.saveWorkflow).toHaveBeenCalledTimes(1)
    })
    // While the workflow is saved, the key gets a title.
    const title = screen.getByRole("textbox", { name: "Key title" })
    await user.clear(title)
    await user.type(title, "Logged")
    finish({ id: "" })
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith("Write log now runs in a Multi Action")
    })
    await user.click(screen.getByRole("button", { name: /Save/ }))
    await waitFor(() => {
      expect(api.saveLayout).toHaveBeenCalledTimes(1)
    })
    const saved = api.saveLayout.mock.calls[0]?.[1] as DeviceLayout
    const binding = saved.pages[0]?.bindings.find((b) => b.trigger.type === "press")
    // Both: the title typed meanwhile and the new target.
    expect(binding?.label).toBe("Logged")
    expect(binding?.target.type).toBe("workflow")
  })

  it("on a gesture card: a double click bound to a display-only action is kept and marked", async () => {
    const user = renderEditor(
      mk2,
      layoutFor(mk2, [
        {
          control: { kind: "key", index: 0 },
          trigger: { type: "click" },
          target: { type: "action", action: "next" },
        },
        {
          control: { kind: "key", index: 0 },
          trigger: { type: "doubleClick" },
          target: { type: "action", action: "pages" },
        },
      ]),
    )
    await user.click(control("Key 1"))
    expect(
      await screen.findByText(
        /This action does not work here: Gesture\. It works as: Key display\./,
      ),
    ).toBeInTheDocument()
    // Nothing it could become: only the reason, no offer.
    expect(screen.queryByRole("group", { name: "Not available here" })).not.toBeInTheDocument()
  })

  it("says that a key showing a display-only action does nothing when pressed", async () => {
    const user = renderEditor(
      mk2,
      layoutFor(mk2, [
        {
          control: { kind: "key", index: 0 },
          trigger: { type: "press" },
          target: { type: "action", action: "pages" },
        },
      ]),
    )
    await user.click(control("Key 1"))
    expect(await screen.findByText("Display only — pressing does nothing.")).toBeInTheDocument()
    expect(screen.queryByText(/This action does not work here/)).not.toBeInTheDocument()
  })
})

describe(
  "Delay as the agent declares it: a Multi Action step or a workflow node",
  {
    timeout: 20_000,
  },
  () => {
    it("is neither offered for a key nor found by searching", async () => {
      const user = renderEditor(mk2)
      expect(screen.queryByText("Delay")).not.toBeInTheDocument()
      expect(screen.queryByText("Short delay")).not.toBeInTheDocument()
      await user.type(screen.getByRole("searchbox", { name: "Search the library" }), "delay")
      expect(screen.queryByText("Delay")).not.toBeInTheDocument()
      expect(screen.queryByText("Short delay")).not.toBeInTheDocument()
    })

    it("saved on a key, is kept as saved and marked, and nothing changes until confirmed", async () => {
      const user = renderEditor(
        mk2,
        layoutFor(mk2, [
          {
            control: { kind: "key", index: 0 },
            trigger: { type: "press" },
            target: { type: "action", action: "short" },
          },
        ]),
      )
      await user.click(control("Key 1"))
      expect(
        await screen.findByText(
          /This action does not work here: Key\. It works as: Multi Action step, Workflow node\./,
        ),
      ).toBeInTheDocument()
      const offer = within(screen.getByRole("group", { name: "Not available here" }))
      expect(offer.getByRole("button", { name: "Wrap in a Multi Action" })).toBeInTheDocument()
      expect(api.saveLayout).not.toHaveBeenCalled()
      expect(api.saveWorkflow).not.toHaveBeenCalled()
      expect(screen.getByRole("button", { name: /Save/ })).toBeDisabled()
    })
  },
)
