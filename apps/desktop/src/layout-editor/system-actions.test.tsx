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
  listBrowsers: vi.fn(),
  applicationIcon: vi.fn(),
  applicationPresent: vi.fn(),
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
  api.getMachineConfig.mockImplementation(() => Promise.resolve({ os: "macOs", bindings }))
  api.applicationIcon.mockResolvedValue("data:image/png;base64,iVBORw0KGgo=")
  api.applicationPresent.mockResolvedValue(true)
  api.listBrowsers.mockResolvedValue({
    browsers: [
      { name: "Firefox", id: "org.mozilla.firefox", kind: "bundle", browser: true },
      { name: "Safari", id: "com.apple.Safari", kind: "bundle", browser: true },
    ],
    default: "com.apple.Safari",
  })
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
    // The application by its name, never its resource key.
    expect(await within(panel).findByText("Safari")).toBeInTheDocument()
    expect(within(panel).queryByText(/app\.safari/)).not.toBeInTheDocument()
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
    const option = await screen.findByRole("option", { name: /TextEdit/ })
    // Each application with the icon the system drew.
    expect(option.querySelector("img")).not.toBeNull()
    await user.click(option)
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
    expect(api.applicationIcon).toHaveBeenCalledWith("com.apple.TextEdit")
  })

  it("say when a chosen application is not installed any more", async () => {
    api.applicationPresent.mockResolvedValue(false)
    bindings = [{ key: "app.gone-123456", kind: "application", value: "org.example.Gone" }]
    render(
      <MachineResourceField
        id="f"
        pick="application"
        value={{ type: "fixed", key: "app.gone-123456" }}
        optional={false}
        onChange={vi.fn()}
      />,
    )
    expect(await screen.findByRole("alert")).toHaveTextContent("not installed on this computer")
  })

  it("show no alert for an application that cannot be checked", async () => {
    api.applicationPresent.mockResolvedValue(null)
    bindings = [
      {
        key: "app.calculator-123456",
        kind: "application",
        value: "Microsoft.WindowsCalculator_8wekyb3d8bbwe!App",
      },
    ]
    render(
      <MachineResourceField
        id="f"
        pick="application"
        value={{ type: "fixed", key: "app.calculator-123456" }}
        optional={false}
        onChange={vi.fn()}
      />,
    )
    await waitFor(() => {
      expect(api.applicationPresent).toHaveBeenCalled()
    })
    expect(screen.queryByRole("alert")).not.toBeInTheDocument()
  })

  it("choose another application with the native dialog, limited to applications", async () => {
    dialog.open.mockResolvedValue("/Applications/Editor.app")
    api.describeApplication.mockResolvedValue({
      name: "Editor",
      id: "com.example.Editor",
      kind: "bundle",
      browser: false,
    })
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
    await waitFor(() => {
      expect(api.getMachineConfig).toHaveBeenCalled()
    })
    await user.click(screen.getByRole("combobox", { name: "Choose application…" }))
    await user.click(await screen.findByRole("option", { name: /Another application/ }))
    await waitFor(() => {
      expect(onChange).toHaveBeenCalled()
    })
    expect(dialog.open).toHaveBeenCalledWith({
      directory: false,
      multiple: false,
      filters: [{ name: "Applications", extensions: ["app"] }],
    })
    expect(api.describeApplication).toHaveBeenCalledWith("/Applications/Editor.app")
  })

  it("show an application chosen from outside the listed folders as chosen, not missing", async () => {
    // Bound and shown before discovery knows it: the system says it is
    // installed, so no alert; after "Another application…" it is named.
    dialog.open.mockResolvedValue("/Users/me/Tools/Editor.app")
    api.describeApplication.mockResolvedValue({
      name: "Editor",
      id: "com.example.Editor",
      kind: "bundle",
      browser: false,
    })
    bindings = [{ key: "app.editor-abc123", kind: "application", value: "com.example.Editor" }]
    const onChange = vi.fn()
    render(
      <MachineResourceField
        id="f"
        pick="application"
        value={{ type: "fixed", key: "app.editor-abc123" }}
        optional={false}
        onChange={onChange}
      />,
    )
    await waitFor(() => {
      expect(api.applicationPresent).toHaveBeenCalledWith("com.example.Editor")
    })
    expect(screen.queryByRole("alert")).not.toBeInTheDocument()
    const user = userEvent.setup()
    await user.click(screen.getByRole("combobox", { name: /Choose application/ }))
    await user.click(await screen.findByRole("option", { name: /Another application/ }))
    expect(await screen.findByRole("combobox", { name: /Editor/ })).toBeInTheDocument()
    expect(screen.queryByRole("alert")).not.toBeInTheDocument()
  })

  it("open with the default browser unless another is chosen", async () => {
    const onChange = vi.fn()
    const { rerender } = render(
      <MachineResourceField id="f" pick="browser" value={null} optional onChange={onChange} />,
    )
    const user = userEvent.setup()
    const select = await screen.findByRole("combobox")
    await waitFor(() => {
      expect(select).toHaveTextContent("Default browser (Safari)")
    })
    await user.click(select)
    await user.click(await screen.findByRole("option", { name: "Firefox" }))
    await waitFor(() => {
      expect(onChange).toHaveBeenCalledWith({
        type: "fixed",
        key: expect.stringMatching(/^app\.firefox-[a-z0-9]{6}$/) as unknown,
      })
    })
    // A chosen browser that is gone: the default browser is used, and said so.
    bindings = [{ key: "app.gone-123456", kind: "application", value: "org.example.Gone" }]
    rerender(
      <MachineResourceField
        id="g"
        pick="browser"
        value={{ type: "fixed", key: "app.gone-123456" }}
        optional
        onChange={onChange}
      />,
    )
    expect(await screen.findByText(/not installed any more/)).toBeInTheDocument()
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
  })

  it("open folders with the folder dialog only", async () => {
    dialog.open.mockResolvedValue("/Users/me/Downloads")
    const onChange = vi.fn()
    render(
      <MachineResourceField
        id="f"
        pick="folder"
        value={null}
        optional={false}
        onChange={onChange}
      />,
    )
    const user = userEvent.setup()
    expect(screen.queryByRole("button", { name: "Choose file…" })).not.toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Choose folder…" }))
    await waitFor(() => {
      expect(onChange).toHaveBeenCalledWith({
        type: "fixed",
        key: expect.stringMatching(/^folder\.downloads-[a-z0-9]{6}$/) as unknown,
      })
    })
    expect(dialog.open).toHaveBeenCalledWith({ directory: true, multiple: false })
  })

  it("keep resource keys and roles in advanced mode, opened for saved ones", async () => {
    const advanced = <p>resource key control</p>
    const { rerender } = render(
      <MachineResourceField
        id="f"
        pick="location"
        value={{ type: "projectRole", role: "repository" }}
        optional={false}
        onChange={vi.fn()}
        advanced={advanced}
      />,
    )
    // A saved project role opens in advanced mode.
    expect(screen.getByText("resource key control")).toBeInTheDocument()
    rerender(
      <MachineResourceField
        id="g"
        key="simple"
        pick="location"
        value={{ type: "fixed", key: "file.report-abc123" }}
        optional={false}
        onChange={vi.fn()}
        advanced={advanced}
      />,
    )
    // A picked file: the simple mode, no key, no role.
    expect(screen.queryByText("resource key control")).not.toBeInTheDocument()
    const user = userEvent.setup()
    await user.click(screen.getByRole("button", { name: "Advanced" }))
    expect(screen.getByText("resource key control")).toBeInTheDocument()
  })
})
