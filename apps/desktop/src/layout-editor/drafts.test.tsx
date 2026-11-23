// @vitest-environment jsdom
// Draft actions in the Device Editor (`lib/action-completeness.ts`): a type
// whose required settings have no defaults is put on a key or gesture as a
// draft of its own, saved without those settings (never blank), marked
// "Needs configuration" on the key and in the Inspector, where its first
// missing setting takes focus; filling them in clears the marks. The
// backend's answers are mocked as the agent gives them.
import "@testing-library/jest-dom/vitest"

import type { DragEndEvent } from "@dnd-kit/core"
import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import type { ReactNode } from "react"
import { toast } from "sonner"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import type { SavedAction } from "@/lib/action-completeness"
import { missingParameters } from "@/lib/action-completeness"
import type { ActionInput } from "@/lib/api"
import { moduleInfo } from "@/lib/test-fixtures"
import type {
  ActionInstance,
  ActionType,
  DeviceLayout,
  DeviceModel,
  ParameterSpec,
  ProfileView,
} from "@/lib/types"

import { catalogDragId } from "./catalog"
import { LayoutEditor } from "./editor"

Element.prototype.hasPointerCapture = () => false
Element.prototype.releasePointerCapture = () => undefined
Element.prototype.scrollIntoView = () => undefined

const text = (key: string, label: string, required: boolean): ParameterSpec => ({
  key,
  label,
  kind: { type: "text", multiline: false },
  required,
})
const type = (id: string, name: string, parameters: ParameterSpec[]): ActionType => ({
  id,
  name,
  description: "",
  category: "System",
  requiresProject: false,
  parameters,
  contexts: ["key", "gesture", "sequence", "workflow"],
})
// As the System module declares them (modules/system/src/actions.rs).
const openApp = type("system.open-app", "Open application", [
  { key: "application", label: "Application", kind: { type: "resource" }, required: true },
  text("arguments", "Arguments", false),
])
const openUrl = type("system.open-url", "Open web page", [text("url", "URL", true)])
const openFile = type("system.open-file", "Open file", [text("path", "File", true)])
const openFolder = type("system.open-directory", "Open folder", [text("path", "Folder", true)])
const nextPage = type("deck.next-page", "Next page", [
  {
    key: "wrapAround",
    label: "Wrap around",
    kind: { type: "boolean" },
    required: true,
    default: { type: "boolean", value: true },
  },
])
const modules = [
  moduleInfo({
    id: "system",
    name: "System",
    actionTypes: [openApp, openUrl, openFile, openFolder],
  }),
  moduleInfo({ id: "deck", name: "Deck", actionTypes: [nextPage] }),
]

const createAction = vi.fn()
const updateAction = vi.fn()
const refresh = vi.fn()

const dnd = vi.hoisted(() => ({
  onDragEnd: undefined as ((event: DragEndEvent) => void) | undefined,
  data: new Map<string, unknown>(),
  // The drag monitors (a gesture's slots listen with `useDndMonitor`).
  monitors: new Set<(event: DragEndEvent) => void>(),
}))

vi.mock("@dnd-kit/core", async (original) => {
  const actual = await original<typeof import("@dnd-kit/core")>()
  const react = await import("react")
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
    useDndMonitor: (handlers: { onDragEnd?: (event: DragEndEvent) => void }) => {
      // Only the mounted monitors' current handlers, as dnd-kit calls them.
      react.useEffect(() => {
        const onDragEnd = handlers.onDragEnd
        if (!onDragEnd) return
        dnd.monitors.add(onDragEnd)
        return () => {
          dnd.monitors.delete(onDragEnd)
        }
      })
      actual.useDndMonitor(handlers)
    },
  }
})
vi.mock("@/lib/api", () => ({
  api: {
    previewLayout: () => Promise.resolve([]),
    listIcons: () => Promise.resolve([]),
    pageTree: () => Promise.resolve({ pages: [], warnings: [] }),
    listActionTypes: () => Promise.resolve(modules.flatMap((m) => m.actionTypes)),
    getGestureDefaults: () => Promise.resolve({ doubleClickMs: 300, longPressMs: 700 }),
    saveLayout: (_profile: string, saved: DeviceLayout) =>
      Promise.resolve({ ...saved, agentReload: { state: "applied" } }),
    createAction: (...args: unknown[]) => createAction(...args) as unknown,
    updateAction: (...args: unknown[]) => updateAction(...args) as unknown,
  },
  errorMessage: (error: unknown) => (error instanceof Error ? error.message : String(error)),
}))
vi.mock("@/lib/app-context", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/app-context")>()),
  useActionModules: () => modules,
  useAppContext: () => ({
    profiles: { activeProfileId: null, profiles: [] },
    refresh,
    navigate: vi.fn(),
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

/** The profile as `get_profile` answers it, with what is incomplete. */
function viewOf(actions: ActionInstance[]): ProfileView {
  const byId = new Map(modules.flatMap((m) => m.actionTypes).map((t) => [t.id, t]))
  const incompleteActions = Object.fromEntries(
    actions.flatMap((a) => {
      const t = byId.get(a.actionType)
      const missing = t ? missingParameters(t, a.parameters) : []
      return missing.length > 0 ? [[a.id, missing]] : []
    }),
  )
  return {
    activeProjectId: null,
    incompleteActions,
    profile: { id: "p", name: "Work", actions, workflows: [], projects: [], layouts: [layout] },
  } as unknown as ProfileView
}

let actions: ActionInstance[] = []
function renderEditor(initial: ActionInstance[] = []) {
  actions = [...initial]
  const ui = (view: ProfileView) => (
    <LayoutEditor
      profileId="p"
      view={view}
      model={mk2}
      saved={layout}
      onDirtyChange={vi.fn()}
      onSaved={vi.fn()}
    />
  )
  const result = render(ui(viewOf(actions)))
  // The profile reloaded, as after `refresh` (the agent's answer).
  const reload = () => {
    result.rerender(ui(viewOf(actions)))
  }
  return { user: userEvent.setup(), reload }
}

function drop(id: string, over: string) {
  expect(dnd.data.has(id), `a draggable ${id}`).toBe(true)
  const onDragEnd = dnd.onDragEnd
  if (!onDragEnd) throw new Error("the editor has no drop handler")
  const event = {
    active: { id, data: { current: dnd.data.get(id) } },
    over: { id: over },
  } as unknown as DragEndEvent
  act(() => {
    onDragEnd(event)
    for (const monitor of dnd.monitors) monitor(event)
  })
}

const key = (n: number) => screen.getByRole("button", { name: `Key ${n}` })
const keyMark = (n: number) => within(key(n)).queryByRole("img", { name: /needs configuration/ })

beforeEach(() => {
  dnd.data.clear()
  dnd.monitors.clear()
  refresh.mockClear()
  vi.mocked(toast.error).mockClear()
  createAction.mockReset()
  updateAction.mockReset()
  let made = 0
  // `create_action` saves a draft and says what it lacks.
  createAction.mockImplementation((_profile: string, input: ActionInput) => {
    made += 1
    const t = modules.flatMap((m) => m.actionTypes).find((c) => c.id === input.actionType)
    const missing = t ? missingParameters(t, input.parameters) : []
    const saved: SavedAction = {
      ...input,
      id: `new-${made}`,
      ...(missing.length > 0 ? { incomplete: missing } : {}),
    }
    actions.push(saved)
    return Promise.resolve(saved)
  })
  updateAction.mockImplementation((_profile: string, action: ActionInstance) => {
    actions = actions.map((a) => (a.id === action.id ? action : a))
    return Promise.resolve(action)
  })
})
afterEach(cleanup)

describe("dropping a type whose required settings are not filled in", { timeout: 20_000 }, () => {
  it.each([
    [openApp, "application", "Application"],
    [openUrl, "url", "URL"],
    [openFile, "path", "File"],
    [openFolder, "path", "Folder"],
  ])(
    "puts %s on the key as a draft, focusing what it needs",
    async (dropped, missingKey, label) => {
      const { reload } = renderEditor()
      drop(catalogDragId(dropped.id), "key:0")
      await waitFor(() => {
        expect(createAction).toHaveBeenCalledTimes(1)
      })
      // Saved without the required setting: absent, never blank.
      const input = createAction.mock.calls[0]?.[1] as ActionInput
      expect(input.actionType).toBe(dropped.id)
      expect(input.parameters).not.toHaveProperty(missingKey)
      expect(Object.values(input.parameters).every((v) => v.value !== "")).toBe(true)
      // No error: it is placed, the key is selected.
      expect(toast.error).not.toHaveBeenCalled()
      await waitFor(() => {
        expect(key(1)).toHaveAttribute("aria-pressed", "true")
      })
      reload()
      // Marked on the key and in the Inspector, its first missing field
      // focused, marked required and invalid, the error next to it.
      await waitFor(() => {
        expect(keyMark(1)).toBeInTheDocument()
      })
      const setup = await screen.findByRole("group", {
        name: `${dropped.name} needs configuration`,
      })
      expect(within(setup).getByText("Needs configuration")).toBeInTheDocument()
      const field = document.getElementById(`setup-new-1-${missingKey}`)
      expect(field).not.toBeNull()
      await waitFor(() => {
        expect(field).toHaveFocus()
      })
      expect(field).toHaveAttribute("aria-required", "true")
      expect(field).toHaveAttribute("aria-invalid", "true")
      expect(field).toHaveAccessibleDescription("Required: fill this in before the action can run.")
      expect(within(setup).getByText(label, { exact: false })).toBeInTheDocument()
    },
  )

  it("gives every drop its own draft: configuring one key never changes another", async () => {
    renderEditor()
    drop(catalogDragId(openUrl.id), "key:0")
    await waitFor(() => {
      expect(createAction).toHaveBeenCalledTimes(1)
    })
    drop(catalogDragId(openUrl.id), "key:1")
    await waitFor(() => {
      expect(createAction).toHaveBeenCalledTimes(2)
    })
    const names = createAction.mock.calls.map((call) => (call[1] as ActionInput).name)
    expect(new Set(names).size).toBe(2)
  })

  it("keeps sharing what is complete: a type with defaults reuses its action", async () => {
    const existing: ActionInstance = {
      id: "next",
      name: "Next page",
      actionType: nextPage.id,
      parameters: { wrapAround: { type: "boolean", value: true } },
      description: "",
      icon: null,
      label: null,
    }
    renderEditor([existing])
    drop(catalogDragId(nextPage.id), "key:0")
    drop(catalogDragId(nextPage.id), "key:1")
    await waitFor(() => {
      expect(key(2)).toHaveAttribute("title", "Key 2: Press: Next page")
    })
    expect(createAction).not.toHaveBeenCalled()
    expect(keyMark(1)).toBeNull()
  })

  it("saves what is typed, and clears the marks once it is complete", async () => {
    const { user, reload } = renderEditor()
    drop(catalogDragId(openUrl.id), "key:0")
    await waitFor(() => {
      expect(createAction).toHaveBeenCalledTimes(1)
    })
    reload()
    const field = await waitFor(() => {
      const found = document.getElementById("setup-new-1-url")
      expect(found).not.toBeNull()
      return found as HTMLElement
    })
    await user.type(field, "https://example.com")
    await waitFor(
      () => {
        expect(updateAction).toHaveBeenCalled()
      },
      { timeout: 2000 },
    )
    const saved = updateAction.mock.lastCall?.[1] as ActionInstance
    expect(saved.id).toBe("new-1")
    expect(saved.parameters).toEqual({ url: { type: "text", value: "https://example.com" } })
    expect(refresh).toHaveBeenCalled()
    reload()
    // Complete: no mark on the key, none in the Inspector; the field stays
    // (under the cursor) without its error.
    await waitFor(() => {
      expect(keyMark(1)).toBeNull()
    })
    expect(screen.queryByText("Needs configuration")).not.toBeInTheDocument()
    expect(document.getElementById("setup-new-1-url")).not.toHaveAttribute("aria-invalid")
  })

  it("shows a draft that was saved earlier as such, and leaves complete keys alone", async () => {
    const draft: ActionInstance = {
      id: "draft",
      name: "Open application",
      actionType: openApp.id,
      parameters: {},
      description: "",
      icon: null,
      label: null,
    }
    const done: ActionInstance = {
      id: "done",
      name: "Open docs",
      actionType: openUrl.id,
      parameters: { url: { type: "text", value: "https://docs.example.com" } },
      description: "",
      icon: null,
      label: null,
    }
    actions = [draft, done]
    const withBindings: DeviceLayout = {
      ...layout,
      pages: [
        {
          id: "home",
          name: "Home",
          bindings: [
            {
              control: { kind: "key", index: 0 },
              trigger: { type: "press" },
              target: { type: "action", action: "draft" },
              label: null,
            },
            {
              control: { kind: "key", index: 1 },
              trigger: { type: "press" },
              target: { type: "action", action: "done" },
              label: null,
            },
          ],
        },
      ],
    }
    render(
      <LayoutEditor
        profileId="p"
        view={{
          ...viewOf(actions),
          profile: { ...viewOf(actions).profile, layouts: [withBindings] },
        }}
        model={mk2}
        saved={withBindings}
        onDirtyChange={vi.fn()}
        onSaved={vi.fn()}
      />,
    )
    await waitFor(() => {
      expect(keyMark(1)).toBeInTheDocument()
    })
    expect(keyMark(2)).toBeNull()
  })
})

describe("a draft on a gesture", { timeout: 20_000 }, () => {
  it("is a draft of its own per gesture, marked on its card", async () => {
    const { user, reload } = renderEditor()
    await user.click(key(1))
    // The double click's slot of Key 1, in the Inspector.
    drop(catalogDragId(openApp.id), "gesture-slot:key:0:doubleClick")
    await waitFor(() => {
      expect(createAction).toHaveBeenCalledTimes(1)
    })
    drop(catalogDragId(openApp.id), "gesture-slot:key:0:longPress")
    await waitFor(() => {
      expect(createAction).toHaveBeenCalledTimes(2)
    })
    expect(toast.error).not.toHaveBeenCalled()
    reload()
    await waitFor(() => {
      const cards = [...document.querySelectorAll<HTMLElement>("[data-slot=gesture-card-action]")]
      expect(cards).toHaveLength(2)
      for (const card of cards) {
        expect(within(card).getByText("Needs configuration")).toBeInTheDocument()
      }
    })
    // Neither is sent with a blank application.
    for (const call of createAction.mock.calls) {
      expect((call[1] as ActionInput).parameters).not.toHaveProperty("application")
    }
    expect(keyMark(1)).toBeInTheDocument()
  })
})

describe("saving a drop", { timeout: 20_000 }, () => {
  it("is all saved once the agent's copy comes back without the empty label", async () => {
    const onDirtyChange = vi.fn()
    const existing: ActionInstance = {
      id: "next",
      name: "Next page",
      actionType: nextPage.id,
      parameters: { wrapAround: { type: "boolean", value: true } },
      description: "",
      icon: null,
      label: null,
    }
    const ui = (saved: DeviceLayout) => (
      <LayoutEditor
        profileId="p"
        view={viewOf([existing])}
        model={mk2}
        saved={saved}
        onDirtyChange={onDirtyChange}
        onSaved={vi.fn()}
      />
    )
    const { rerender } = render(ui(layout))
    const user = userEvent.setup()
    drop(catalogDragId(nextPage.id), "key:0")
    await waitFor(() => {
      expect(screen.getByText("Unsaved")).toBeInTheDocument()
    })
    // The draft's binding has `label: null`; the agent stores none.
    await user.click(screen.getByRole("button", { name: "Save" }))
    const stored: DeviceLayout = {
      ...layout,
      pages: [
        {
          id: "home",
          name: "Home",
          bindings: [
            {
              control: { kind: "key", index: 0 },
              trigger: { type: "press" },
              target: { type: "action", action: "next" },
            },
          ],
        },
      ],
    }
    rerender(ui(stored))
    await waitFor(() => {
      expect(screen.getByText("All changes saved")).toBeInTheDocument()
    })
    expect(onDirtyChange).toHaveBeenLastCalledWith(false)
  })
})
