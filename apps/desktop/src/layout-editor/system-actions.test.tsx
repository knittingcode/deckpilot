// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { MachineResourceField } from "@/components/machine-resource-field"
import type {
  ActionInstance,
  ActionType,
  ControlId,
  DeviceLayout,
  MachineResourceBinding,
  ParameterSpec,
} from "@/lib/types"
import { installDomStubs } from "@/test/dom"

import { SystemActionPanels } from "./system-action-inspector"
import { longPressOf, withLongPress } from "./system-actions-model"

const api = vi.hoisted(() => ({
  getMachineConfig: vi.fn(),
  setResourceBinding: vi.fn(),
  listApplications: vi.fn(),
  describeApplication: vi.fn(),
  createAction: vi.fn(),
  updateAction: vi.fn(),
  runAction: vi.fn(),
}))
const dialog = vi.hoisted(() => ({ open: vi.fn() }))

vi.mock("@/lib/api", () => ({ api }))
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: dialog.open }))

beforeAll(() => {
  installDomStubs()
})

let bindings: MachineResourceBinding[] = []

beforeEach(() => {
  bindings = [{ key: "app.safari", kind: "application", value: "com.apple.Safari" }]
  api.getMachineConfig.mockImplementation(() => Promise.resolve({ os: "macos", bindings }))
  api.setResourceBinding.mockImplementation((key: string, kind: string, value: string) => {
    bindings = [...bindings.filter((b) => b.key !== key), { key, kind, value } as never]
    return Promise.resolve({ os: "macos", bindings })
  })
  api.listApplications.mockResolvedValue([
    { name: "Firefox", id: "org.mozilla.firefox", kind: "bundle", browser: true },
    { name: "Safari", id: "com.apple.Safari", kind: "bundle", browser: true },
    { name: "TextEdit", id: "com.apple.TextEdit", kind: "bundle", browser: false },
  ])
})

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

const key: ControlId = { kind: "key", index: 0 }
const application: ParameterSpec = {
  key: "application",
  label: "Application",
  kind: { type: "resource", kind: "application" },
  required: true,
}
const openApp: ActionType = {
  id: "system.open-app",
  name: "Open Application",
  description: "Starts an application.",
  category: "System",
  requiresProject: false,
  parameters: [application],
  node: { icon: "app-window" } as ActionType["node"],
}
const closeApp: ActionType = {
  ...openApp,
  id: "system.close-app",
  name: "Close Application",
  parameters: [
    application,
    {
      key: "force",
      label: "Force quit if it does not close",
      kind: { type: "boolean" },
      required: true,
      default: { type: "boolean", value: false },
    },
  ],
}
const safari = { type: "resource" as const, value: { type: "fixed" as const, key: "app.safari" } }
const opens: ActionInstance = {
  id: "open-safari",
  name: "Safari",
  actionType: "system.open-app",
  parameters: { application: safari },
}
const quits: ActionInstance = {
  id: "quit-safari",
  name: "Quit Safari",
  actionType: "system.close-app",
  parameters: { application: safari },
}

function layout(bindings: DeviceLayout["pages"][number]["bindings"]): DeviceLayout {
  return {
    id: "l",
    name: "Main",
    model: "mk2",
    homePage: "home",
    pages: [{ id: "home", name: "Home", bindings }],
  }
}

const pressOpens = layout([
  { control: key, trigger: { type: "press" }, target: { type: "action", action: opens.id } },
])

describe("quit on long press", () => {
  it("is an ordinary long-press binding, made the gesture editor's way", () => {
    expect(longPressOf(pressOpens, "home", key, [opens], safari)).toBe("none")
    const { layout: next, removed } = withLongPress(pressOpens, "home", key, quits)
    expect(removed).toEqual([])
    // The raw press became the click; the long press quits.
    expect(next.pages[0]?.bindings).toEqual([
      { control: key, trigger: { type: "click" }, target: { type: "action", action: opens.id } },
      {
        control: key,
        trigger: { type: "longPress" },
        target: { type: "action", action: quits.id },
        label: null,
      },
    ])
    expect(longPressOf(next, "home", key, [opens, quits], safari)).toBe("quit")
    // Another application's quit, or another action, is something else.
    const other = {
      ...quits,
      parameters: {
        application: { ...safari, value: { type: "fixed" as const, key: "app.other" } },
      },
    }
    expect(longPressOf(next, "home", key, [opens, other], safari)).toBe("other")
    // None removes only the long press.
    const back = withLongPress(next, "home", key, null).layout
    expect(back.pages[0]?.bindings.map((b) => b.trigger.type)).toEqual(["click"])
  })

  it("drops a release that cannot stay next to a long press", () => {
    const raw = layout([
      ...(pressOpens.pages[0]?.bindings ?? []),
      { control: key, trigger: { type: "release" }, target: { type: "action", action: "x" } },
    ])
    const { removed } = withLongPress(raw, "home", key, quits)
    expect(removed.map((b) => b.trigger.type)).toEqual(["release"])
  })
})

function renderPanel(draft: DeviceLayout, actions: ActionInstance[]) {
  const onChange = vi.fn()
  const onActionSaved = vi.fn()
  render(
    <SystemActionPanels
      draft={draft}
      pageId="home"
      control={key}
      actions={actions}
      types={[openApp, closeApp]}
      gestures
      context={{
        profileId: "p",
        modelName: "Stream Deck MK.2",
        layouts: [draft],
        defaultLayout: draft.id,
        usages: {},
        device: null,
        onActionSaved,
      }}
      onChange={onChange}
    />,
  )
  return { onChange, onActionSaved, user: userEvent.setup() }
}

describe("the Open Application panel", () => {
  it("binds Quit application to the long press with a new Close Application action", async () => {
    api.createAction.mockResolvedValue(quits)
    const { onChange, onActionSaved, user } = renderPanel(pressOpens, [opens])
    const panel = screen.getByRole("region", { name: "Open Application" })
    expect(await within(panel).findByText("com.apple.Safari")).toBeInTheDocument()
    expect(within(panel).getByText("This computer only")).toBeInTheDocument()
    await user.click(within(panel).getByRole("combobox", { name: "On long press" }))
    await user.click(await screen.findByRole("option", { name: "Quit the application" }))
    await waitFor(() => {
      expect(onChange).toHaveBeenCalled()
    })
    expect(api.createAction).toHaveBeenCalledWith("p", {
      name: "Quit Safari",
      actionType: "system.close-app",
      parameters: { application: safari },
      description: "",
      icon: null,
      label: null,
    })
    expect(onActionSaved).toHaveBeenCalledWith(quits)
    const next = onChange.mock.calls[0]?.[0] as DeviceLayout
    expect(longPressOf(next, "home", key, [opens, quits], safari)).toBe("quit")
  })

  it("reuses a graceful Close Application of the same application", async () => {
    const { onChange, user } = renderPanel(pressOpens, [opens, quits])
    const panel = screen.getByRole("region", { name: "Open Application" })
    await user.click(within(panel).getByRole("combobox", { name: "On long press" }))
    await user.click(await screen.findByRole("option", { name: "Quit the application" }))
    await waitFor(() => {
      expect(onChange).toHaveBeenCalled()
    })
    expect(api.createAction).not.toHaveBeenCalled()
  })

  it("says what a Close Application key does, gracefully or by force", () => {
    const pressCloses = layout([
      { control: key, trigger: { type: "press" }, target: { type: "action", action: quits.id } },
    ])
    renderPanel(pressCloses, [quits])
    const panel = screen.getByRole("region", { name: "Close Application" })
    expect(
      within(panel).getByText(/Graceful: the application is asked to quit/),
    ).toBeInTheDocument()
  })

  it("shows a failed test run in words", async () => {
    api.runAction.mockResolvedValue({
      status: "failed",
      message: "Close Application runs only when you start it",
      code: "not-user-initiated",
    })
    const { user } = renderPanel(pressOpens, [opens])
    const panel = screen.getByRole("region", { name: "Open Application" })
    await user.click(within(panel).getByRole("button", { name: "Test" }))
    expect(await within(panel).findByText(/never from a schedule or an event/)).toBeInTheDocument()
  })
})

describe("the machine-local pickers", () => {
  it("bind a listed application on this computer under a key of its own", async () => {
    const onChange = vi.fn()
    render(
      <MachineResourceField
        id="f"
        pick="application"
        value={null}
        optional={false}
        onChange={onChange}
      />,
    )
    const user = userEvent.setup()
    await user.click(screen.getByRole("combobox", { name: "Choose application…" }))
    await user.click(await screen.findByRole("option", { name: /TextEdit/ }))
    await waitFor(() => {
      expect(onChange).toHaveBeenCalledWith({
        type: "fixed",
        key: expect.stringMatching(/^app\.textedit-[a-z0-9]{6}$/) as unknown,
      })
    })
    expect(api.setResourceBinding).toHaveBeenCalledWith(
      expect.stringMatching(/^app\.textedit-[a-z0-9]{6}$/),
      "application",
      "com.apple.TextEdit",
    )
  })

  it("list browsers first and offer the default browser", async () => {
    const onChange = vi.fn()
    render(
      <MachineResourceField
        id="f"
        pick="browser"
        value={{ type: "fixed", key: "app.safari" }}
        optional
        onChange={onChange}
      />,
    )
    const user = userEvent.setup()
    expect(await screen.findByText("com.apple.Safari")).toBeInTheDocument()
    expect(screen.getByRole("note")).toHaveTextContent("never exported")
    await user.click(screen.getByRole("combobox", { name: "Choose browser…" }))
    const browsers = await screen.findByRole("group", { name: "Browsers" })
    expect(
      within(browsers)
        .getAllByRole("option")
        .map((o) => o.textContent),
    ).toEqual([expect.stringContaining("Firefox"), expect.stringContaining("Safari")])
    // The same value is not bound twice.
    await user.click(within(browsers).getByRole("option", { name: /Safari/ }))
    await waitFor(() => {
      expect(onChange).toHaveBeenCalledWith({ type: "fixed", key: "app.safari" })
    })
    await user.click(screen.getByRole("button", { name: "Use the default browser" }))
    expect(onChange).toHaveBeenLastCalledWith(null)
  })

  it("bind a picked file, and say when a binding is missing", async () => {
    dialog.open.mockResolvedValue("/Users/me/Reports/Q1 report.pdf")
    const onChange = vi.fn()
    render(
      <MachineResourceField
        id="f"
        pick="location"
        value={{ type: "fixed", key: "file.gone" }}
        optional={false}
        onChange={onChange}
      />,
    )
    const user = userEvent.setup()
    expect(await screen.findByRole("alert")).toHaveTextContent("choose it again")
    await user.click(screen.getByRole("button", { name: "Choose file…" }))
    await waitFor(() => {
      expect(onChange).toHaveBeenCalledWith({
        type: "fixed",
        key: expect.stringMatching(/^file\.q1-report-pdf-[a-z0-9]{6}$/) as unknown,
      })
    })
    expect(dialog.open).toHaveBeenCalledWith({ directory: false, multiple: false })
    expect(api.setResourceBinding).toHaveBeenCalledWith(
      expect.stringMatching(/^file\.q1-report-pdf-[a-z0-9]{6}$/),
      "other",
      "/Users/me/Reports/Q1 report.pdf",
    )
  })
})
