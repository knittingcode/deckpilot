// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { act, cleanup, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { installDomStubs } from "@/test/dom"
import { availablePlugins, createFakePluginClient, readyRuntime } from "@/test/plugins"
import { openDetails as openPluginDetails, openInstalled } from "@/test/marketplace"
import { renderWithApp } from "@/test/render"
import { resetToasts, toast } from "@/test/toast"

import { PluginClientContext, PluginError } from "./api"
import { PluginsSettings } from "./marketplace"
import type { AgentConnection } from "@/lib/types"

import type { AvailablePlugin, ManifestPermission } from "./types"

vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)
vi.mock("./client", () => ({
  createTauriPluginClient: () => {
    throw new Error("the Tauri client is not used in tests")
  },
}))

beforeAll(installDomStubs)
beforeEach(resetToasts)
afterEach(cleanup)

const BASE = "acme.base"
const DASH = "acme.dash"
const CALENDAR = "acme.calendar"

const net = (host: string): ManifestPermission => ({
  kind: "network",
  hosts: [host],
  methods: ["GET"],
  justification: `Reads ${host}.`,
})

/** Project Dash requires Base Library and can use Calendar. */
function withDependencies(): AvailablePlugin[] {
  const plugin = (
    id: string,
    name: string,
    permissions: ManifestPermission[],
    dependencies: NonNullable<AvailablePlugin["versions"][number]["dependencies"]> = [],
  ): AvailablePlugin => ({
    id,
    name,
    summary: `${name} for tests.`,
    publisher: "acme",
    publisherInfo: { namespace: "acme", login: "acme", verified: true },
    latest: "1.0.0",
    versions: [{ version: "1.0.0", permissions, dependencies }],
  })
  return [
    ...availablePlugins(),
    plugin(BASE, "Base Library", [net("api.base.example")]),
    plugin(CALENDAR, "Calendar", [net("api.calendar.example")]),
    plugin(
      DASH,
      "Project Dash",
      [{ kind: "notifications", justification: "Says when a build fails." }],
      [
        { id: BASE, version: ">=1.0.0", state: "missing" },
        {
          id: CALENDAR,
          version: ">=1.0.0 <2.0.0",
          optional: true,
          justification: "Shows your next meeting.",
          actions: ["next-meeting"],
          state: "missing",
        },
      ],
    ),
  ]
}

function render(features = ["dependencies"]) {
  const client = createFakePluginClient({
    available: withDependencies(),
    runtime: readyRuntime({ features }),
  })
  const result = renderWithApp(
    <PluginClientContext value={client}>
      <PluginsSettings />
    </PluginClientContext>,
  )
  return { ...result, client }
}

type User = ReturnType<typeof render>["user"]

/** A plugin's page: from its tile in Discover, or (installed) from Installed. */
const openDetails = (user: User, name: string) => openPluginDetails(user, name)

/** Installs Project Dash with every permission of the plan. */
async function installDash(user: User) {
  await openDetails(user, "Project Dash")
  await user.click(screen.getByRole("button", { name: "Install Project Dash" }))
  const dialog = await screen.findByRole("dialog", { name: "Install Project Dash 1.0.0?" })
  await user.click(within(dialog).getByRole("button", { name: "Allow all 2" }))
  await user.click(within(dialog).getByRole("button", { name: "Install 2" }))
  await waitFor(() => {
    expect(toast.success).toHaveBeenCalledWith("Project Dash installed with 1 more")
  })
}

const installedItem = (name: string) =>
  within(
    within(screen.getByRole("list", { name: "Installed plugins" }))
      .getByText(name, { selector: "span" })
      .closest("li") as HTMLElement,
  )

describe("plugin dependencies", () => {
  it("shows a version's required dependencies and optional integrations", async () => {
    const { user } = render()
    await openDetails(user, "Project Dash")
    const required = screen.getByRole("list", { name: "Requires" })
    expect(within(required).getByText(BASE)).toBeInTheDocument()
    expect(required).toHaveTextContent("Not installed")
    const optional = screen.getByRole("list", { name: "Optional integrations" })
    expect(optional).toHaveTextContent("Shows your next meeting.")
    expect(optional).toHaveTextContent("Used by: next-meeting")
  })

  it("shows the whole plan and consents per plugin, dependencies first", async () => {
    const { user, client } = render()
    await openDetails(user, "Project Dash")
    await user.click(screen.getByRole("button", { name: "Install Project Dash" }))
    const dialog = await screen.findByRole("dialog", { name: "Install Project Dash 1.0.0?" })
    expect(client.planInstall).toHaveBeenCalledWith(DASH, "1.0.0", [])
    const steps = within(within(dialog).getByRole("list", { name: "Install plan" }))
      .getAllByRole("listitem")
      .filter((li) => li.parentElement?.getAttribute("aria-label") === "Install plan")
    expect(steps.map((li) => li.textContent)).toEqual([
      expect.stringContaining(BASE),
      expect.stringContaining(DASH),
    ])
    expect(steps[0]).toHaveTextContent("acme.dash 1.0.0 needs >=1.0.0")
    // Nothing is installed without consent to every permission.
    const install = within(dialog).getByRole("button", { name: "Install 2" })
    expect(install).toBeDisabled()
    // The dependency's own permission, checked on its own.
    await user.click(within(dialog).getByRole("checkbox", { name: /^Allow Base Library: / }))
    expect(install).toBeDisabled()
    await user.click(within(dialog).getByRole("checkbox", { name: /^Allow Project Dash: / }))
    await user.click(install)
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith("Project Dash installed with 1 more")
    })
    // Each plugin gets exactly its own permissions.
    expect(client.installPlan).toHaveBeenCalledWith(
      expect.objectContaining({ plugin: DASH }),
      [],
      { [BASE]: ["network:api.base.example"], [DASH]: ["notifications"] },
      true,
    )
    expect(client.installed()).toEqual(expect.arrayContaining([BASE, DASH]))
  })

  it("installs an optional integration only when it is selected", async () => {
    const { user, client } = render()
    await openDetails(user, "Project Dash")
    await user.click(screen.getByRole("button", { name: "Install Project Dash" }))
    const dialog = await screen.findByRole("dialog", { name: "Install Project Dash 1.0.0?" })
    const plan = within(dialog).getByRole("list", { name: "Install plan" })
    expect(plan).not.toHaveTextContent(CALENDAR)
    await user.click(within(dialog).getByRole("checkbox", { name: /Also install Calendar/ }))
    await waitFor(() => {
      expect(client.planInstall).toHaveBeenLastCalledWith(DASH, "1.0.0", [CALENDAR])
    })
    expect(await within(dialog).findByText("Allow all 3")).toBeInTheDocument()
    await user.click(within(dialog).getByRole("button", { name: "Allow all 3" }))
    await user.click(within(dialog).getByRole("button", { name: "Install 3" }))
    await waitFor(() => {
      expect(client.installed()).toEqual(expect.arrayContaining([BASE, CALENDAR, DASH]))
    })
  })

  it("turns off the plugins that need one, after showing them", async () => {
    const { user, client } = render()
    await installDash(user)
    await user.click(screen.getByRole("button", { name: /Back to plugins/ }))
    await openInstalled()
    await user.click(
      installedItem("Base Library").getByRole("switch", { name: "Disable Base Library" }),
    )
    const dialog = await screen.findByRole("dialog", { name: "Turn off Base Library?" })
    expect(within(dialog).getByRole("list", { name: "Also turned off" })).toHaveTextContent(
      "Project Dash",
    )
    await user.click(within(dialog).getByRole("button", { name: "Turn off 2" }))
    await waitFor(() => {
      expect(client.removePlugin).toHaveBeenCalledWith(BASE, false, undefined, true, [])
    })
  })

  it("offers the dependencies installed only for an uninstalled plugin", async () => {
    const { user, client } = render()
    await installDash(user)
    await user.click(screen.getByRole("button", { name: /Back to plugins/ }))
    await openInstalled()
    await user.click(installedItem("Project Dash").getByRole("button", { name: "Uninstall" }))
    const confirm = screen.getByRole("alertdialog", { name: "Uninstall Project Dash?" })
    await user.click(within(confirm).getByRole("button", { name: "Uninstall" }))
    const dialog = await screen.findByRole("dialog", { name: "Uninstall Project Dash?" })
    // Never removed unless checked.
    const orphan = within(dialog).getByRole("checkbox", { name: /Also uninstall Base Library/ })
    expect(orphan).not.toBeChecked()
    await user.click(orphan)
    await user.click(within(dialog).getByRole("button", { name: "Uninstall" }))
    await waitFor(() => {
      expect(client.removePlugin).toHaveBeenCalledWith(DASH, true, expect.anything(), false, [BASE])
    })
    await waitFor(() => {
      expect(client.installed()).not.toContain(BASE)
    })
  })

  it("installs one plugin at a time with an agent without dependency plans", async () => {
    const { user, client } = render([])
    await openDetails(user, "Hello Weather")
    await user.click(screen.getByRole("button", { name: "Install Hello Weather" }))
    expect(
      await screen.findByRole("dialog", { name: "Install Hello Weather 1.1.0?" }),
    ).toBeInTheDocument()
    expect(client.planInstall).not.toHaveBeenCalled()
  })

  // DeckPilot 0.11.0: the agent refused a plan, the toast went unnoticed
  // and the Install button seemed to do nothing.
  it("shows a refusal while planning the install as a dialog, never nothing", async () => {
    const { user, client } = render()
    client.planInstall.mockRejectedValueOnce(
      new PluginError(
        "invalidPackage",
        "acme.dash is not a verified DeckPilot release; nothing was installed",
      ),
    )
    await openDetails(user, "Project Dash")
    await user.click(screen.getByRole("button", { name: "Install Project Dash" }))
    const dialog = await screen.findByRole("alertdialog", {
      name: "Project Dash 1.0.0 cannot be installed",
    })
    expect(within(dialog).getByRole("alert")).toHaveTextContent(
      "acme.dash is not a verified DeckPilot release; nothing was installed",
    )
    expect(
      screen.queryByRole("dialog", { name: "Install Project Dash 1.0.0?" }),
    ).not.toBeInTheDocument()
    expect(client.installPlan).not.toHaveBeenCalled()
    await user.keyboard("{Escape}")
    await waitFor(() => {
      expect(
        screen.queryByRole("alertdialog", { name: "Project Dash 1.0.0 cannot be installed" }),
      ).not.toBeInTheDocument()
    })
  })
})

// Planning an install downloads and verifies what it would install: the
// agent reports that as progress (fetching, verifying) and sends no
// terminal event once the plan is made or refused.
describe("progress while planning an install", () => {
  /** The agent's reports of a plan's downloads, before it answers. */
  function reportDownloads(client: ReturnType<typeof render>["client"]) {
    const real = client.planInstall.getMockImplementation()
    if (!real) throw new Error("the fake plans installs")
    client.planInstall.mockImplementation((id, version, optional) => {
      for (const plugin of [BASE, DASH]) {
        client.emit({ type: "progress", plugin, operation: "install", stage: "fetching" })
        client.emit({ type: "progress", plugin, operation: "install", stage: "verifying" })
      }
      return real(id, version, optional)
    })
  }

  it("leaves nothing installing when the plan's dialog is cancelled", async () => {
    const { user, client } = render()
    reportDownloads(client)
    await openDetails(user, "Project Dash")
    await user.click(screen.getByRole("button", { name: "Install Project Dash" }))
    await screen.findByRole("dialog", { name: "Install Project Dash 1.0.0?" })
    await user.keyboard("{Escape}")
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
    })
    expect(screen.getByRole("button", { name: "Install Project Dash" })).toBeEnabled()
    expect(screen.queryByRole("button", { name: /Installing…/ })).not.toBeInTheDocument()
    expect(screen.queryByRole("status", { name: "Plugin operations" })).not.toBeInTheDocument()
    expect(client.installPlan).not.toHaveBeenCalled()
  })

  it("leaves nothing installing when the plan is refused", async () => {
    const { user, client } = render()
    client.planInstall.mockImplementationOnce(() => {
      client.emit({ type: "progress", plugin: DASH, operation: "install", stage: "verifying" })
      return Promise.reject(new PluginError("invalidPackage", "not a verified release"))
    })
    await openDetails(user, "Project Dash")
    await user.click(screen.getByRole("button", { name: "Install Project Dash" }))
    await screen.findByRole("alertdialog", { name: "Project Dash 1.0.0 cannot be installed" })
    await user.keyboard("{Escape}")
    await waitFor(() => {
      expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument()
    })
    expect(screen.queryByRole("status", { name: "Plugin operations" })).not.toBeInTheDocument()
    expect(screen.queryByRole("button", { name: /Installing…/ })).not.toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Try installing Project Dash again" })).toBeEnabled()
  })

  it("forgets a download reported after the plan was answered once its dialog closes", async () => {
    const { user, client } = render()
    await openDetails(user, "Project Dash")
    await user.click(screen.getByRole("button", { name: "Install Project Dash" }))
    await screen.findByRole("dialog", { name: "Install Project Dash 1.0.0?" })
    // Late: after the plan's answer, while its dialog is open.
    act(() => {
      client.emit({ type: "progress", plugin: DASH, operation: "install", stage: "verifying" })
    })
    expect(screen.getByRole("status", { name: "Plugin operations", hidden: true })).toBeTruthy()
    await user.keyboard("{Escape}")
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
    })
    expect(screen.queryByRole("status", { name: "Plugin operations" })).not.toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Install Project Dash" })).toBeEnabled()
  })

  it("counts a download as installing only while this page has a confirmed request", async () => {
    const { user, client } = render()
    await openDetails(user, "Project Dash")
    act(() => {
      client.emit({ type: "progress", plugin: DASH, operation: "install", stage: "fetching" })
    })
    // Shown as what the agent reports, but nothing waits on it here.
    expect(screen.getByRole("status", { name: "Plugin operations" })).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Install Project Dash" })).toBeEnabled()
    // Past downloading, the agent is installing it (another window's request).
    act(() => {
      client.emit({ type: "progress", plugin: DASH, operation: "install", stage: "installing" })
    })
    expect(screen.getByRole("button", { name: /Installing…/ })).toBeDisabled()
  })

  it("forgets the old agent's progress when the agent changes", async () => {
    const client = createFakePluginClient({
      available: withDependencies(),
      runtime: readyRuntime({ features: ["dependencies"] }),
    })
    const agent = (version: string) => ({
      running: true,
      status: { version } as NonNullable<AgentConnection["status"]>,
      canStart: true,
      compatibility: null,
      appVersion: "0.13.0",
      protocol: 1,
    })
    const ui = (
      <PluginClientContext value={client}>
        <PluginsSettings />
      </PluginClientContext>
    )
    const { rerender, context } = renderWithApp(ui, { agent: agent("0.13.0") })
    await screen.findByRole("heading", { level: 2 })
    act(() => {
      client.emit({ type: "progress", plugin: DASH, operation: "install", stage: "installing" })
    })
    expect(screen.getByRole("status", { name: "Plugin operations" })).toBeInTheDocument()
    // The agent was restarted: the install it reported will never finish.
    rerender(ui, { ...context, agent: agent("0.13.1") })
    await waitFor(() => {
      expect(screen.queryByRole("status", { name: "Plugin operations" })).not.toBeInTheDocument()
    })
  })
})
