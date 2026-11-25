// @vitest-environment jsdom
// Settings › Connections against a mock client (no agent, no network): the
// list per plugin, adding a self-hosted GitLab with its token (sent once,
// never shown back), the permissions explained, testing, a new token,
// turning off and removing. Tested automatically, not verified manually.
import "@testing-library/jest-dom/vitest"

import { cleanup, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { installDomStubs } from "@/test/dom"
import { resetToasts, toast } from "@/test/toast"

import { PluginError } from "../api"
import {
  type ConnectionsClient,
  ConnectionsClientContext,
  type PluginConnection,
  type PluginConnections,
  setupUrl,
} from "./api"
import { ConnectionsSettings } from "./connections-settings"

vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)

beforeAll(installDomStubs)
beforeEach(resetToasts)
afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})

const PLUGIN = "deckpilot.git-providers"
const TOKEN = "dp-test-token-value"

function connection(extra: Partial<PluginConnection> = {}): PluginConnection {
  return {
    id: "cn_01JD7Q0000000000000000000A",
    type: "github",
    displayName: "octo @ github.com",
    deployment: "cloud",
    apiUrl: "https://api.github.com",
    webUrl: "https://github.com",
    allowChanges: false,
    privateNetwork: false,
    enabled: true,
    state: "ok",
    account: "octo",
    isDefault: true,
    hasSecret: true,
    createdAt: 1,
    ...extra,
  }
}

function connections(extra: Partial<PluginConnections> = {}): PluginConnections {
  return {
    plugin: PLUGIN,
    granted: true,
    privateNetworkAllowed: true,
    changesAllowed: true,
    types: [
      {
        id: "github",
        label: "GitHub",
        cloudApiUrl: "https://api.github.com",
        cloudWebUrl: "https://github.com",
        selfHosted: true,
        selfHostedLabel: "GitHub Enterprise Server",
        selfHostedApiPath: "/api/v3",
        auth: [
          {
            method: "pat",
            cloudSetupUrl: "https://github.com/settings/personal-access-tokens/new",
            selfHostedSetupUrl: "{web}/settings/tokens/new",
          },
        ],
      },
      {
        id: "gitlab",
        label: "GitLab",
        cloudApiUrl: "https://gitlab.com/api/v4",
        cloudWebUrl: "https://gitlab.com",
        selfHosted: true,
        selfHostedLabel: "Self-managed GitLab",
        selfHostedApiPath: "/api/v4",
        auth: [
          {
            method: "pat",
            cloudSetupUrl: "https://gitlab.com/-/user_settings/personal_access_tokens",
            selfHostedSetupUrl: "{web}/-/user_settings/personal_access_tokens",
          },
        ],
      },
    ],
    connections: [connection()],
    ...extra,
  }
}

function setup(data: PluginConnections = connections()) {
  const gitlab = connection({
    id: "cn_01JD7Q0000000000000000000B",
    type: "gitlab",
    displayName: "Work GitLab",
    deployment: "selfHosted",
    apiUrl: "https://code.example.com:8443/gitlab/api/v4",
    webUrl: "https://code.example.com:8443",
    state: "untested",
    account: undefined,
    isDefault: true,
  })
  const client: ConnectionsClient = {
    listPlugins: vi.fn(() =>
      Promise.resolve([
        { id: PLUGIN, name: "Git Providers", state: "enabled" },
        { id: "deckpilot.git", name: "Git", state: "enabled" },
      ]),
    ),
    list: vi.fn((plugin: string) =>
      plugin === PLUGIN
        ? Promise.resolve(data)
        : Promise.resolve({ ...connections({ plugin, types: [], connections: [] }) }),
    ),
    create: vi.fn(() => Promise.resolve(gitlab)),
    setSecret: vi.fn((_p: string, id: string) =>
      Promise.resolve(connection({ id, state: "untested" })),
    ),
    update: vi.fn((_p: string, id: string, change) =>
      Promise.resolve(
        connection({
          id,
          enabled: change.enabled ?? true,
          state: change.enabled === false ? "disabled" : "ok",
        }),
      ),
    ),
    test: vi.fn((_p: string, id: string) =>
      Promise.resolve({
        connection: id === gitlab.id ? { ...gitlab, state: "ok", account: "ana" } : connection(),
        outcome: { ok: true },
      }),
    ),
    remove: vi.fn(() => Promise.resolve(connections({ connections: [] }))),
  }
  render(
    <ConnectionsClientContext value={client}>
      <ConnectionsSettings />
    </ConnectionsClientContext>,
  )
  return { client, user: userEvent.setup() }
}

const section = () => screen.findByRole("region", { name: "Git Providers" })

describe("Settings › Connections", () => {
  it("lists the connections of plugins that declare connection types", async () => {
    const { client } = setup()
    const s = within(await section())
    expect(s.getByText("octo @ github.com")).toBeInTheDocument()
    expect(s.getByText("GitHub · octo")).toBeInTheDocument()
    expect(s.getByText("Connected")).toBeInTheDocument()
    expect(s.getByText("Default")).toBeInTheDocument()
    expect(s.getByText("Read-only")).toBeInTheDocument()
    // A plugin without connection types has no section.
    expect(screen.queryByRole("region", { name: "Git" })).not.toBeInTheDocument()
    expect(client.list).toHaveBeenCalledWith("deckpilot.git")
  })

  it("cannot add a connection before the plugin may use connections", async () => {
    setup(connections({ granted: false }))
    const s = within(await section())
    expect(s.getByRole("button", { name: "Add connection" })).toBeDisabled()
    expect(s.getByText(/not allowed to use connections/)).toBeInTheDocument()
  })

  it("adds a self-managed GitLab with its token, then tests it", async () => {
    const { client, user } = setup()
    await user.click(within(await section()).getByRole("button", { name: "Add connection" }))
    const dialog = within(await screen.findByRole("dialog", { name: "Add a connection" }))
    await user.click(dialog.getByRole("radio", { name: "GitLab" }))
    await user.click(dialog.getByRole("radio", { name: "Self-managed GitLab" }))
    const create = dialog.getByRole("button", { name: "Add and test" })
    expect(create).toBeDisabled()
    await user.type(dialog.getByLabelText("Server address"), "https://code.example.com:8443/gitlab")
    // The permissions in plain words, read-only by default.
    expect(dialog.getByTestId("connection-scopes")).toHaveTextContent("read_api")
    await user.click(dialog.getByRole("switch", { name: "Allow changes" }))
    expect(dialog.getByTestId("connection-scopes")).toHaveTextContent("with the api scope")
    // Where the token is made: on the server the user entered.
    expect(
      dialog.getByText(
        "https://code.example.com:8443/gitlab/-/user_settings/personal_access_tokens",
      ),
    ).toBeInTheDocument()
    // A private network only by its own choice, never pre-selected.
    const lan = dialog.getByRole("checkbox", {
      name: "This server is on my company or home network",
    })
    expect(lan).not.toBeChecked()
    await user.type(dialog.getByLabelText("Token"), TOKEN)
    expect(create).toBeEnabled()
    await user.click(create)
    await waitFor(() => {
      expect(client.test).toHaveBeenCalledWith(PLUGIN, "cn_01JD7Q0000000000000000000B")
    })
    expect(client.create).toHaveBeenCalledWith(PLUGIN, {
      connectionType: "gitlab",
      deployment: "selfHosted",
      url: "https://code.example.com:8443/gitlab",
      displayName: "GitLab @ code.example.com:8443",
      allowChanges: true,
      privateNetwork: false,
      secret: TOKEN,
    })
    expect(toast.success).toHaveBeenCalledWith("The connection works, signed in as ana.")
    // The token is never shown back.
    expect(document.body).not.toHaveTextContent(TOKEN)
    expect(await screen.findByText("Work GitLab")).toBeInTheDocument()
  })

  it("shows the agent's refusal of an address and keeps the dialog open", async () => {
    const { client, user } = setup()
    vi.mocked(client.create).mockRejectedValueOnce(
      new PluginError("invalidRequest", "the address must start with https://"),
    )
    await user.click(within(await section()).getByRole("button", { name: "Add connection" }))
    const dialog = within(await screen.findByRole("dialog", { name: "Add a connection" }))
    await user.click(dialog.getByRole("radio", { name: "GitHub Enterprise Server" }))
    await user.type(dialog.getByLabelText("Server address"), "https://ghe.example.com")
    await user.type(dialog.getByLabelText("Token"), TOKEN)
    await user.click(dialog.getByRole("button", { name: "Add and test" }))
    expect(await dialog.findByText(/must start with https:\/\//)).toBeInTheDocument()
    expect(client.test).not.toHaveBeenCalled()
  })

  it("tests, turns off, replaces the token of and removes a connection", async () => {
    const { client, user } = setup()
    const s = within(await section())
    await user.click(s.getByRole("button", { name: "Test" }))
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith("The connection works, signed in as octo.")
    })

    await user.click(s.getByRole("button", { name: "Replace token" }))
    const tokenDialog = within(
      await screen.findByRole("dialog", { name: "New token for octo @ github.com" }),
    )
    await user.type(tokenDialog.getByLabelText("Token"), TOKEN)
    await user.click(tokenDialog.getByRole("button", { name: "Save token" }))
    await waitFor(() => {
      expect(client.setSecret).toHaveBeenCalledWith(PLUGIN, "cn_01JD7Q0000000000000000000A", TOKEN)
    })
    expect(document.body).not.toHaveTextContent(TOKEN)

    await user.click(s.getByRole("button", { name: "Turn off" }))
    await waitFor(() => {
      expect(client.update).toHaveBeenCalledWith(PLUGIN, "cn_01JD7Q0000000000000000000A", {
        enabled: false,
      })
    })
    expect(await s.findByText("Turned off")).toBeInTheDocument()

    await user.click(s.getByRole("button", { name: "Remove" }))
    const confirm = within(await screen.findByRole("alertdialog"))
    await user.click(confirm.getByRole("button", { name: "Remove" }))
    await waitFor(() => {
      expect(client.remove).toHaveBeenCalledWith(PLUGIN, "cn_01JD7Q0000000000000000000A")
    })
    expect(await s.findByText("No connections yet.")).toBeInTheDocument()
  })

  it("reports a failed test with the plugin's reason", async () => {
    const { client, user } = setup()
    vi.mocked(client.test).mockResolvedValueOnce({
      connection: connection({ state: "unauthorized" }),
      outcome: { ok: false, code: "unauthorized", message: "the token was rejected" },
    })
    const s = within(await section())
    await user.click(s.getByRole("button", { name: "Test" }))
    await waitFor(() => {
      expect(toast.error).toHaveBeenCalledWith(
        "The connection does not work: the token was rejected",
      )
    })
    expect(await s.findByText("Token rejected")).toBeInTheDocument()
  })
})

describe("the token page", () => {
  const type = connections().types[1]
  it("is on the server the user entered, https only", () => {
    expect(setupUrl(type, "cloud", "")).toBe(
      "https://gitlab.com/-/user_settings/personal_access_tokens",
    )
    expect(setupUrl(type, "selfHosted", "https://code.example.com/gitlab/")).toBe(
      "https://code.example.com/gitlab/-/user_settings/personal_access_tokens",
    )
    for (const bad of ["http://code.example.com", "https://u:p@code.example.com", "not a url"]) {
      expect(setupUrl(type, "selfHosted", bad)).toBeUndefined()
    }
  })
})
