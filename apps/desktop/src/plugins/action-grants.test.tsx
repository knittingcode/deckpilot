// @vitest-environment jsdom
// Granting a plugin action's missing permission from the Device Editor's
// library: decided from the agent's structured grants, asked in the
// Marketplace's permission dialog, granted with the existing request.
import "@testing-library/jest-dom/vitest"

import {
  cleanup,
  fireEvent,
  render as renderPlain,
  screen,
  waitFor,
  within,
} from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { LayoutEditor } from "@/layout-editor/editor"
import { moduleInfo } from "@/lib/test-fixtures"
import type { ActionType, DeviceLayout, DeviceModel, ModuleInfo } from "@/lib/types"
import { api, resetApi } from "@/test/api-mock"
import { installDomStubs } from "@/test/dom"
import { builtInModules, profileView, savedByAgent } from "@/test/fixtures"
import { renderWithApp } from "@/test/render"
import { resetToasts, toast } from "@/test/toast"

import { type PluginClient, PluginClientContext } from "./api"
import type { GrantInfo, PluginDetails, PluginSummary } from "./types"
import { usePluginModules } from "./use-plugin-modules"

vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)
vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)

const DOCKER = "deckpilot.docker"
const RAW = "needs the permission `tools:docker:modify`: allow it in Settings › Plugins"
const type = (action: string, name: string): ActionType => ({
  id: `${DOCKER}.${action}`,
  name,
  description: "",
  category: "Docker",
  requiresProject: false,
  parameters: [],
})
const COMPOSE = [
  type("compose-up", "Compose up"),
  type("compose-down", "Compose down"),
  type("compose-restart", "Compose restart"),
]
const status = type("status", "Docker status")

/**
 * The Docker plugin's module as `usePluginModules` builds it: `grantable`
 * comes from the agent's grants (`grantableOf`), never from `unavailable`.
 */
function dockerModule(
  state: string,
  unavailable: Record<string, string>,
  grantable: Record<string, string[]> = state === "enabled"
    ? Object.fromEntries(Object.keys(unavailable).map((id) => [id, ["tools:docker:modify"]]))
    : {},
): ModuleInfo {
  return moduleInfo({
    id: DOCKER,
    name: "Docker",
    enabled: state === "enabled",
    actionTypes: [status, ...COMPOSE],
    plugin: { state, unavailable, grantable },
  })
}
const blocked = (reason = RAW) => Object.fromEntries(COMPOSE.map((t) => [t.id, reason]))

const modify: GrantInfo = {
  key: "tools:docker:modify",
  kind: "tools",
  tier: "high",
  status: "notGranted",
  scope: null,
  optional: true,
  actions: ["compose-up", "compose-down", "compose-restart"],
}
function details(permissions: GrantInfo[], state = "enabled"): PluginDetails {
  return {
    summary: {
      id: DOCKER,
      name: "Docker",
      publisher: "DeckPilot",
      version: "0.1.0",
      state,
      origin: { type: "bundled" },
      actions: [status, ...COMPOSE].map((t) => ({
        action: t.id.slice(DOCKER.length + 1),
        actionType: t.id,
        name: t.name,
        available: true,
      })),
    },
    description: "",
    license: "",
    permissions,
    manifest: {},
    audit: [],
  } as unknown as PluginDetails
}

const mk2: DeviceModel = {
  id: "elgato.streamdeck.mk2",
  vendor: "Elgato",
  name: "Stream Deck MK.2",
  usb: null,
  capabilities: { keys: { rows: 3, columns: 5, image: null }, encoders: null, touch: null },
  support: "verified",
  verification: [],
}
const layout: DeviceLayout = {
  id: "main",
  name: "Main",
  model: mk2.id,
  homePage: "home",
  pages: [{ id: "home", name: "Home", bindings: [] }],
}

let client: { getPlugin: ReturnType<typeof vi.fn>; grantPermissions: ReturnType<typeof vi.fn> }

function render(plugin: ModuleInfo) {
  const ui = (
    <PluginClientContext value={client as unknown as PluginClient}>
      <LayoutEditor
        profileId="profile-1"
        view={profileView({ layouts: [layout] })}
        model={mk2}
        saved={layout}
        onDirtyChange={vi.fn()}
        onSaved={vi.fn()}
      />
    </PluginClientContext>
  )
  const view = renderWithApp(ui, { modules: builtInModules(), pluginModules: [plugin] })
  return { ...view, ui }
}

// `hidden`: also while the dialog is open (it hides the page behind it).
const grantButton = (name: string) =>
  screen.findByRole("button", { name: `Grant permission for ${name} (Docker)`, hidden: true })
const dragHandles = (name: string) =>
  screen.queryAllByRole("button", { name: `Drag ${name} onto a control` })

beforeAll(() => {
  installDomStubs()
  Element.prototype.hasPointerCapture = () => false
  Element.prototype.releasePointerCapture = () => undefined
  Element.prototype.scrollIntoView = () => undefined
})
beforeEach(() => {
  vi.stubEnv("VITE_DECKPILOT_PLUGINS", "1")
  resetApi()
  resetToasts()
  api.previewLayout.mockResolvedValue([])
  api.listIcons.mockResolvedValue([])
  api.listActionTypes.mockResolvedValue([])
  api.getGestureDefaults.mockResolvedValue({ doubleClickMs: 350, longPressMs: 800 })
  api.saveLayout.mockImplementation((_, saved) => Promise.resolve(savedByAgent(saved)))
  client = {
    getPlugin: vi.fn(() => Promise.resolve(details([modify]))),
    grantPermissions: vi.fn(() => Promise.resolve(details([{ ...modify, status: "granted" }]))),
  }
})
afterEach(() => {
  cleanup()
  vi.unstubAllEnvs()
})

describe("granting a permission from the library", () => {
  it("shows the actions that only wait for one, in words, with Grant", async () => {
    render(dockerModule("enabled", blocked()))
    for (const t of COMPOSE) expect(await grantButton(t.name)).toBeEnabled()
    const rows = document.querySelectorAll("[data-slot=grant-entry]")
    expect(rows).toHaveLength(3)
    for (const row of rows) {
      expect(row).toHaveTextContent("Permission required")
      // No permission id and no stale "Settings › Plugins".
      expect(row).not.toHaveTextContent("tools:docker:modify")
      expect(row).not.toHaveTextContent("Settings")
    }
    // An available action of the plugin is draggable as usual.
    expect(dragHandles("Docker status").length).toBeGreaterThan(0)
  })

  it("asks first with exactly the missing permission, grants it once, and enables all its actions", async () => {
    const { rerender, ui, context } = render(dockerModule("enabled", blocked()))
    await userClick(await grantButton("Compose up"))
    const dialog = await screen.findByRole("dialog", { name: "Grant permissions to Docker?" })
    // The plugin's name first, its id after; the tier line fits a tool.
    expect(within(dialog).getByText("Docker", { selector: "span" })).toBeInTheDocument()
    expect(within(dialog).getByText(DOCKER)).toBeInTheDocument()
    expect(dialog).toHaveTextContent(
      "Running tools on your system, within what this plugin supports.",
    )
    expect(dialog).not.toHaveTextContent("secrets")
    expect(dialog).toHaveTextContent("Starting, stopping and changing docker resources")
    expect(dialog).toHaveTextContent(
      "Allows the plugin to start, stop and change docker resources, within what this plugin supports.",
    )
    // Only the one it lacks.
    expect(within(dialog).getAllByRole("checkbox")).toHaveLength(1)
    // High tier: unchecked until the user allows it; nothing to send yet.
    const allow = within(dialog).getByRole("button", { name: "Allow" })
    expect(allow).toBeDisabled()
    expect(client.grantPermissions).not.toHaveBeenCalled()
    fireEvent.click(within(dialog).getByRole("checkbox"))
    fireEvent.click(allow)
    await waitFor(() => {
      expect(client.grantPermissions).toHaveBeenCalledExactlyOnceWith(DOCKER, [
        "tools:docker:modify",
      ])
    })
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
    })
    expect(toast.success).toHaveBeenCalledWith("Permission granted")
    // The agent's change event: the plugin's actions can run now, all three.
    rerender(ui, { ...context, pluginModules: [dockerModule("enabled", {})] })
    await waitFor(() => {
      expect(document.querySelectorAll("[data-slot=grant-entry]")).toHaveLength(0)
    })
    for (const t of COMPOSE) {
      expect(dragHandles(t.name).length).toBeGreaterThan(0)
    }
    // Revoked again elsewhere: Grant comes back.
    rerender(ui, { ...context, pluginModules: [dockerModule("enabled", blocked())] })
    expect(await grantButton("Compose down")).toBeEnabled()
  })

  it("changes nothing when cancelled", async () => {
    render(dockerModule("enabled", blocked()))
    await userClick(await grantButton("Compose up"))
    const dialog = await screen.findByRole("dialog", { name: "Grant permissions to Docker?" })
    fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }))
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
    })
    expect(client.grantPermissions).not.toHaveBeenCalled()
    expect(await grantButton("Compose up")).toBeEnabled()
  })

  it("shows a failure in the dialog and keeps everything as it was", async () => {
    client.grantPermissions.mockRejectedValue(new Error("the store could not be written"))
    render(dockerModule("enabled", blocked()))
    await userClick(await grantButton("Compose up"))
    const dialog = await screen.findByRole("dialog", { name: "Grant permissions to Docker?" })
    fireEvent.click(within(dialog).getByRole("checkbox"))
    fireEvent.click(within(dialog).getByRole("button", { name: "Allow" }))
    expect(await within(dialog).findByRole("alert")).toBeInTheDocument()
    expect(toast.success).not.toHaveBeenCalled()
    expect(document.querySelectorAll("[data-slot=grant-entry]")).toHaveLength(3)
  })

  it("offers each of several missing permissions, and grants only those checked", async () => {
    const network: GrantInfo = {
      key: "network:hub.docker.com",
      kind: "network",
      tier: "medium",
      status: "notGranted",
      scope: null,
      optional: true,
      actions: ["compose-up"],
    }
    client.getPlugin.mockResolvedValue(details([modify, network]))
    render(
      dockerModule("enabled", blocked(), {
        [`${DOCKER}.compose-up`]: ["tools:docker:modify", "network:hub.docker.com"],
      }),
    )
    await userClick(await grantButton("Compose up"))
    const dialog = await screen.findByRole("dialog", { name: "Grant permissions to Docker?" })
    const boxes = within(dialog).getAllByRole("checkbox")
    expect(boxes).toHaveLength(2)
    // The medium one starts checked, the high one does not: grant just it.
    fireEvent.click(within(dialog).getByRole("button", { name: "Allow" }))
    await waitFor(() => {
      expect(client.grantPermissions).toHaveBeenCalledExactlyOnceWith(DOCKER, [
        "network:hub.docker.com",
      ])
    })
  })

  it("offers nothing the plugin does not declare, and no Grant for other causes", async () => {
    // Declared permissions do not name these actions: nothing to grant.
    const { rerender, ui, context } = render(dockerModule("enabled", blocked(), {}))
    // Its reason in words; never the agent's message.
    expect(await screen.findAllByText("Plugin temporarily unavailable")).toHaveLength(3)
    expect(screen.queryByText(RAW)).toBeNull()
    expect(screen.queryByRole("button", { name: /Grant permission/ })).toBeNull()
    // A disabled plugin: its own way back (enable it), never Grant, even
    // with grants recorded.
    rerender(ui, {
      ...context,
      pluginModules: [
        dockerModule("disabled", blocked("disabled"), {
          [`${DOCKER}.compose-up`]: ["tools:docker:modify"],
        }),
      ],
    })
    await waitFor(() => {
      expect(screen.queryByRole("button", { name: /Grant permission/ })).toBeNull()
    })
  })

  it("opens one dialog however often it is clicked, and is used with the keyboard", async () => {
    render(dockerModule("enabled", blocked()))
    const button = await grantButton("Compose up")
    button.focus()
    fireEvent.keyDown(button, { key: "Enter" })
    fireEvent.click(button)
    fireEvent.click(button)
    expect(await screen.findAllByRole("dialog")).toHaveLength(1)
    // Pending: the button waits, with a spinner.
    expect(await grantButton("Compose up")).toBeDisabled()
  })

  it("never starts a drag, assigns or selects anything", async () => {
    render(dockerModule("enabled", blocked()))
    const button = await grantButton("Compose down")
    // A press and a move well past the drag threshold, then release.
    fireEvent.pointerDown(button, { isPrimary: true, button: 0, clientX: 10, clientY: 10 })
    fireEvent.pointerMove(button, { isPrimary: true, clientX: 80, clientY: 60 })
    fireEvent.pointerUp(button, { isPrimary: true, clientX: 80, clientY: 60 })
    fireEvent.mouseDown(button)
    expect(api.createAction).not.toHaveBeenCalled()
    expect(document.querySelector("[aria-pressed=true]")).toBeNull()
    // The Compose actions are still not placeable; the library stays.
    expect(dragHandles("Compose down")).toHaveLength(0)
    expect(await grantButton("Compose down")).toBeEnabled()
  })
})

describe("granting, from what the agent reports", () => {
  /** The Docker plugin as `listPlugins` reports it: its compose actions blocked. */
  function dockerSummary(): PluginSummary {
    return {
      ...details([]).summary,
      actions: [status, ...COMPOSE].map((t) => ({
        action: t.id.slice(DOCKER.length + 1),
        actionType: t.id,
        name: t.name,
        available: t.id === status.id,
        ...(t.id === status.id ? {} : { unavailableReason: RAW }),
        definition: t,
      })),
    }
  }

  it("reads the grants of a plugin with blocked actions and offers Grant (usePluginModules)", async () => {
    const blockedDetails = details([modify])
    blockedDetails.summary = dockerSummary()
    const agent = {
      listPlugins: vi.fn(() => Promise.resolve([dockerSummary()])),
      listKnownPlugins: vi.fn(() => Promise.resolve([])),
      listModuleMigrations: vi.fn(() => Promise.resolve([])),
      subscribe: vi.fn(() => () => undefined),
      getPlugin: vi.fn(() => Promise.resolve(blockedDetails)),
      grantPermissions: client.grantPermissions,
    }
    let built: ModuleInfo[] = []
    function Probe() {
      built = usePluginModules("agent")
      return null
    }
    renderPlain(
      <PluginClientContext value={agent as unknown as PluginClient}>
        <Probe />
      </PluginClientContext>,
    )
    // The hook asks for the blocked plugin's grants and fills `grantable` from them.
    await waitFor(() => {
      expect(built.find((m) => m.id === DOCKER)?.plugin?.grantable).toEqual(
        Object.fromEntries(COMPOSE.map((t) => [t.id, ["tools:docker:modify"]])),
      )
    })
    expect(agent.getPlugin).toHaveBeenCalledWith(DOCKER)
    cleanup()
    // The editor fed with what the hook built: Grant on every blocked action.
    const docker = built.find((m) => m.id === DOCKER)
    if (!docker) throw new Error("no Docker module")
    render(docker)
    for (const t of COMPOSE) expect(await grantButton(t.name)).toBeEnabled()
  })
})

/** A click as the user makes it, outside any drag. */
async function userClick(element: HTMLElement) {
  fireEvent.click(element)
  await Promise.resolve()
}
