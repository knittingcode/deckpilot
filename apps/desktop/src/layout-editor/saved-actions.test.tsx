// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { moduleInfo } from "@/lib/test-fixtures"
import type { ActionType, ControlBinding, DeviceLayout, ProfileView } from "@/lib/types"
import { api, resetApi } from "@/test/api-mock"
import { installDomStubs } from "@/test/dom"
import {
  ACTION_TYPES,
  action,
  agentConnection,
  builtInModules,
  profileView,
  profiles,
} from "@/test/fixtures"
import { renderWithApp } from "@/test/render"
import { resetToasts } from "@/test/toast"

import { useSavedActionTools } from "./saved-actions"

vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)
vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)

// The saved action a key or gesture runs, as the Inspector shows it now that
// the library lists types only: shared by how many places, what it lacks,
// and its definition and uses through the Action Library's own editor and
// list. Nothing is written until the user saves in the editor.

const http = ACTION_TYPES.httpRequest
const url = { url: { type: "text" as const, value: "https://example.com" } }
const shared = action("shared", "Ping server", http.id, { parameters: url })
const single = action("single", "Ping once", http.id, { parameters: url })
const empty = action("empty", "Ping nothing", http.id)
const on = (index: number, id: string): ControlBinding => ({
  control: { kind: "key", index },
  trigger: { type: "press" },
  target: { type: "action", action: id },
})
const layout: DeviceLayout = {
  id: "main",
  name: "Main",
  model: "elgato.streamdeck.mk2",
  homePage: "home",
  pages: [
    {
      id: "home",
      name: "Home",
      bindings: [
        on(0, "shared"),
        on(1, "shared"),
        on(2, "shared"),
        on(3, "single"),
        on(4, "empty"),
      ],
    },
  ],
}
const view: ProfileView = profileView({ actions: [shared, single, empty], layouts: [layout] })

function Lines() {
  const tools = useSavedActionTools({ profileId: "profile-1", view })
  return (
    <>
      <section aria-label="shared">{tools.settingsFor("shared")}</section>
      <section aria-label="single">{tools.settingsFor("single")}</section>
      <section aria-label="empty">{tools.settingsFor("empty")}</section>
      {tools.dialogs}
    </>
  )
}

const WRITES = [
  "createAction",
  "updateAction",
  "duplicateAction",
  "deleteAction",
  "replaceActionReferences",
  "unbindActionReferences",
  "saveLayout",
  "saveWorkflow",
] as const
const writes = () =>
  WRITES.filter((name) => (api[name] as { mock: { calls: unknown[] } }).mock.calls.length > 0)

const render = () => renderWithApp(<Lines />, { modules: builtInModules() })
const line = (name: string) => within(screen.getByRole("region", { name }))

beforeAll(installDomStubs)
beforeEach(() => {
  resetApi()
  resetToasts()
})
afterEach(cleanup)

describe("a binding's saved action in the Inspector", () => {
  it("says how many places share it, only when shared, and writes nothing", () => {
    render()
    expect(line("shared").getByText("Used in 3 places")).toBeInTheDocument()
    expect(line("single").queryByText(/Used in/)).not.toBeInTheDocument()
    expect(writes()).toEqual([])
  })

  it("edits a shared definition, saying it changes every place, and saves it once for all", async () => {
    api.updateAction.mockResolvedValue({ ...shared, name: "Ping all" })
    const { user, context } = render()
    await user.click(line("shared").getByRole("button", { name: "Settings of “Ping server”" }))
    const menu = await screen.findByRole("menu")
    expect(
      within(menu)
        .getAllByRole("menuitem")
        .map((i) => i.textContent.trim()),
    ).toEqual(["Edit shared definition…", "Where used…"])
    await user.click(within(menu).getByRole("menuitem", { name: "Edit shared definition…" }))
    const dialog = await screen.findByRole("dialog", { name: "Ping server" })
    expect(dialog).toHaveTextContent("Shared by 3 places: changes here apply to every one of them.")
    expect(writes()).toEqual([])
    const name = within(dialog).getByRole("textbox", { name: "Name" })
    await user.clear(name)
    await user.type(name, "Ping all")
    await user.click(within(dialog).getByRole("button", { name: "Save" }))
    // The saved action itself: every key that uses it runs the new definition.
    await waitFor(() => {
      expect(api.updateAction).toHaveBeenCalledWith(
        "profile-1",
        expect.objectContaining({ id: "shared", name: "Ping all" }),
      )
    })
    expect(api.duplicateAction).not.toHaveBeenCalled()
    expect(context.refresh).toHaveBeenCalled()
  })

  it("names a single-use action's edit plainly", async () => {
    const { user } = render()
    await user.click(line("single").getByRole("button", { name: "Settings of “Ping once”" }))
    expect(
      within(await screen.findByRole("menu")).getByRole("menuitem", { name: "Edit action…" }),
    ).toBeInTheDocument()
  })

  it("lists where it is used, from the Action Library's list", async () => {
    const { user } = render()
    await user.click(line("shared").getByRole("button", { name: "Settings of “Ping server”" }))
    await user.click(
      within(await screen.findByRole("menu")).getByRole("menuitem", { name: "Where used…" }),
    )
    const dialog = await screen.findByRole("dialog", { name: "Where “Ping server” is used" })
    expect(within(dialog).getAllByRole("listitem")).toHaveLength(3)
    expect(writes()).toEqual([])
  })

  it("shows a draft's required values right there, marked, with what is missing", () => {
    render()
    const draft = line("empty")
    expect(draft.getByText("Needs configuration")).toBeInTheDocument()
    const url = draft.getByRole("textbox", { name: /URL/ })
    expect(url).toHaveAttribute("aria-required", "true")
    expect(url).toHaveAttribute("aria-invalid", "true")
    expect(url).toHaveAccessibleDescription(
      "Required: fill this in before the action can run.",
    )
    // Complete ones: nothing to configure.
    expect(line("shared").queryByText("Needs configuration")).not.toBeInTheDocument()
    expect(line("shared").queryByRole("textbox")).not.toBeInTheDocument()
  })

  it("offers no delete and no new action", async () => {
    const { user } = render()
    await user.click(line("shared").getByRole("button", { name: "Settings of “Ping server”" }))
    const menu = await screen.findByRole("menu")
    expect(within(menu).queryByRole("menuitem", { name: /Delete|New/ })).not.toBeInTheDocument()
  })
})

// A saved action of a legacy id (a built-in module's, now a plugin's): its
// shared definition is edited with the plugin's fields, as the agent routes
// it, and saved under the legacy id.
describe("a saved action of a legacy id", () => {
  const composeDown: ActionType = {
    id: "deckpilot.docker.compose-down",
    name: "Compose down",
    description: "",
    category: "Docker",
    requiresProject: false,
    parameters: [
      {
        key: "removeOrphans",
        label: "Remove orphans",
        kind: { type: "boolean" },
        required: false,
      },
    ],
  }
  const docker = moduleInfo({
    id: "deckpilot.docker",
    name: "Docker",
    enabled: true,
    actionTypes: [composeDown],
    plugin: {
      state: "enabled",
      unavailable: {},
      aliases: [
        {
          legacy: "docker.compose-down",
          target: composeDown.id,
          module: "docker",
          plugin: "deckpilot.docker",
          pluginName: "Docker",
          route: "plugin",
        },
      ],
    },
  })
  const stop = action("stop", "Stop services", "docker.compose-down", {
    parameters: { removeOrphans: { type: "boolean", value: false } },
  })
  const legacyView: ProfileView = profileView({
    actions: [stop, shared],
    layouts: [
      {
        ...layout,
        pages: [
          { id: "home", name: "Home", bindings: [on(0, "stop"), on(1, "stop"), on(2, "shared")] },
        ],
      },
    ],
  })
  function Legacy() {
    const tools = useSavedActionTools({ profileId: "profile-1", view: legacyView })
    return (
      <>
        <section aria-label="stop">{tools.settingsFor("stop")}</section>
        <section aria-label="shared">{tools.settingsFor("shared")}</section>
        {tools.dialogs}
      </>
    )
  }
  const renderLegacy = () =>
    renderWithApp(<Legacy />, {
      modules: builtInModules(),
      pluginModules: [docker],
      profiles: profiles([{ id: "profile-1", name: "Work" }], "profile-1"),
      agent: agentConnection(),
    })

  it("opens with the plugin's fields, saves under the legacy id, and runs", async () => {
    api.updateAction.mockResolvedValue({ ...stop, name: "Stop all" })
    api.runAction.mockResolvedValue({ status: "succeeded", message: "Stopped" })
    const { user } = renderLegacy()
    expect(line("stop").getByText("Used in 2 places")).toBeInTheDocument()
    await user.click(line("stop").getByRole("button", { name: "Settings of “Stop services”" }))
    await user.click(
      within(await screen.findByRole("menu")).getByRole("menuitem", {
        name: "Edit shared definition…",
      }),
    )
    const dialog = await screen.findByRole("dialog", { name: "Stop services" })
    // The plugin's field, not "unavailable".
    expect(within(dialog).getByText("Remove orphans")).toBeInTheDocument()
    expect(dialog).not.toHaveTextContent(/no installed module provides/i)

    // The agent runs it as it is (the legacy id, routed to the plugin).
    await user.click(within(dialog).getByRole("button", { name: "Run" }))
    await waitFor(() => {
      expect(api.runAction).toHaveBeenCalledWith("profile-1", "stop")
    })

    const name = within(dialog).getByRole("textbox", { name: "Name" })
    await user.clear(name)
    await user.type(name, "Stop all")
    await user.click(within(dialog).getByRole("button", { name: "Save" }))
    await waitFor(() => {
      expect(api.updateAction).toHaveBeenCalledWith(
        "profile-1",
        expect.objectContaining({
          id: "stop",
          name: "Stop all",
          // The type id stays the legacy one; the parameters keep their shape.
          actionType: "docker.compose-down",
          parameters: { removeOrphans: { type: "boolean", value: false } },
        }),
      )
    })
  })

  it("leaves a built-in action's editor as it was", async () => {
    const { user } = renderLegacy()
    await user.click(line("shared").getByRole("button", { name: "Settings of “Ping server”" }))
    await user.click(
      within(await screen.findByRole("menu")).getByRole("menuitem", { name: "Edit action…" }),
    )
    const dialog = await screen.findByRole("dialog", { name: "Ping server" })
    expect(within(dialog).getByRole("textbox", { name: /URL/ })).toHaveValue("https://example.com")
  })
})
