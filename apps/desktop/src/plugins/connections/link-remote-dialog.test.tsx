// @vitest-environment jsdom
// Linking a project repository to a remote one (K1–K3) against a mock
// client: the provider's id comes only from the plugin's record, the user
// confirms before anything is written, and cancelling writes nothing.
// Tested automatically, not verified manually.
import "@testing-library/jest-dom/vitest"

import { cleanup, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest"

import { installDomStubs } from "@/test/dom"

import {
  type ConnectionsClient,
  ConnectionsClientContext,
  type RepositoryRemote,
  remoteHost,
  remotePath,
  servesRemote,
  withLink,
} from "./api"
import { LinkRemoteDialog } from "./link-remote-dialog"

vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)

beforeAll(installDomStubs)
afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})

const PLUGIN = "deckpilot.git-providers"
const CONNECTION = "cn_01JD7Q0000000000000000000B"

function client(): ConnectionsClient {
  return {
    listPlugins: vi.fn(() =>
      Promise.resolve([{ id: PLUGIN, name: "Git Providers", state: "enabled" }]),
    ),
    list: vi.fn(() =>
      Promise.resolve({
        plugin: PLUGIN,
        granted: true,
        privateNetworkAllowed: true,
        changesAllowed: true,
        types: [{ id: "gitlab", label: "GitLab", selfHosted: true, auth: [] }],
        connections: [
          {
            id: CONNECTION,
            type: "gitlab",
            displayName: "Work GitLab",
            deployment: "selfHosted" as const,
            apiUrl: "https://code.example.com/api/v4",
            webUrl: "https://code.example.com",
            allowChanges: false,
            privateNetwork: false,
            enabled: true,
            state: "ok" as const,
            isDefault: true,
            hasSecret: true,
            createdAt: 1,
          },
        ],
      }),
    ),
    create: vi.fn(),
    setSecret: vi.fn(),
    update: vi.fn(),
    test: vi.fn(),
    remove: vi.fn(),
    listRepositories: vi.fn(() =>
      Promise.resolve([
        {
          id: "gl:4711",
          path: "group/app",
          defaultBranch: "main",
          httpUrl: "https://code.example.com/group/app.git",
        },
        // A record without an immutable id is never offered (K2).
        { id: "group/other", path: "group/other" },
      ]),
    ),
    listRepositoryRemotes: vi.fn(() => Promise.resolve([])),
    resolveRemote: vi.fn(() => Promise.resolve([])),
    setRepositoryRemotes: vi.fn(() => Promise.resolve(null)),
    linkRepositoryRemote: vi.fn(
      (_p: string, _pr: string, _r: string, _plugin: string, remote: RepositoryRemote) =>
        Promise.resolve({ remotes: [remote] }),
    ),
  }
}

function setup(remotes: RepositoryRemote[] = [], c: ConnectionsClient = client()) {
  const onLinked = vi.fn()
  const onClose = vi.fn()
  render(
    <ConnectionsClientContext value={c}>
      <LinkRemoteDialog
        profileId="p1"
        projectId="pr1"
        repositoryId="r1"
        repositoryName="App"
        remotes={remotes}
        onClose={onClose}
        onLinked={onLinked}
      />
    </ConnectionsClientContext>,
  )
  return { c, onLinked, onClose, user: userEvent.setup() }
}

describe("linking a repository to a remote one", () => {
  it("links the plugin's record after the user confirms", async () => {
    const { c, onLinked, user } = setup()
    const dialog = within(
      await screen.findByRole("dialog", { name: "Link App to a remote repository" }),
    )
    const link = dialog.getByRole("button", { name: "Link" })
    expect(link).toBeDisabled()
    await user.click(await dialog.findByRole("radio", { name: "Work GitLab (code.example.com)" }))
    await user.type(dialog.getByLabelText("Find a repository"), "app")
    await user.click(dialog.getByRole("button", { name: "Find" }))
    await waitFor(() => {
      expect(c.listRepositories).toHaveBeenCalledWith(PLUGIN, CONNECTION, "app")
    })
    const results = within(dialog.getByRole("list", { name: "Repositories found" }))
    expect(results.queryByText("group/other")).not.toBeInTheDocument()
    expect(link).toBeDisabled()
    await user.click(results.getByRole("button", { name: "group/app" }))
    expect(dialog.getByTestId("link-summary")).toHaveTextContent(
      "group/app (gl:4711) on Work GitLab will be linked.",
    )
    // Enter in the search field searches again; it never links.
    await user.type(dialog.getByLabelText("Find a repository"), "{Enter}")
    await waitFor(() => {
      expect(c.listRepositories).toHaveBeenCalledTimes(2)
    })
    expect(c.linkRepositoryRemote).not.toHaveBeenCalled()
    await user.click(
      within(dialog.getByRole("list", { name: "Repositories found" })).getByRole("button", {
        name: "group/app",
      }),
    )
    await user.click(link)
    const expected: RepositoryRemote = {
      name: "origin",
      primary: true,
      connectionId: CONNECTION,
      remoteRepositoryId: "gl:4711",
      remoteUrl: "https://code.example.com/group/app.git",
      defaultBranch: "main",
    }
    // Only the one remote: the app merges it with what is stored now.
    await waitFor(() => {
      expect(c.linkRepositoryRemote).toHaveBeenCalledWith("p1", "pr1", "r1", PLUGIN, expected)
    })
    expect(c.setRepositoryRemotes).not.toHaveBeenCalled()
    expect(onLinked).toHaveBeenCalledWith([expected])
  })

  it("cannot switch connections while a search runs", async () => {
    const { c, user } = setup()
    let answer: (records: never[]) => void = () => {}
    vi.mocked(c.listRepositories).mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          answer = resolve
        }),
    )
    const dialog = within(
      await screen.findByRole("dialog", { name: "Link App to a remote repository" }),
    )
    const choice = await dialog.findByRole("radio", { name: "Work GitLab (code.example.com)" })
    await user.click(choice)
    await user.click(dialog.getByRole("button", { name: "Find" }))
    expect(choice).toBeDisabled()
    answer([])
    expect(await dialog.findByText("No repository found.")).toBeInTheDocument()
    expect(choice).toBeEnabled()
  })

  it("says when the link replaces a stored remote of the same name", async () => {
    const { user } = setup([
      { name: "origin", primary: true, remoteUrl: "https://old.example.com/a.git" },
    ])
    const dialog = within(
      await screen.findByRole("dialog", { name: "Link App to a remote repository" }),
    )
    await user.click(await dialog.findByRole("radio", { name: "Work GitLab (code.example.com)" }))
    await user.click(dialog.getByRole("button", { name: "Find" }))
    await user.click(await dialog.findByRole("button", { name: "group/app" }))
    const name = dialog.getByLabelText("Git remote name")
    expect(dialog.queryByRole("note")).toBeNull()
    await user.type(name, "origin")
    expect(dialog.getByRole("note")).toHaveTextContent(
      "This replaces the remote “origin” stored now.",
    )
    await user.clear(name)
    await user.type(name, "upstream")
    expect(dialog.queryByRole("note")).toBeNull()
  })

  it("writes nothing when cancelled", async () => {
    const { c, onClose, user } = setup()
    const dialog = within(
      await screen.findByRole("dialog", { name: "Link App to a remote repository" }),
    )
    await user.click(dialog.getByRole("button", { name: "Cancel" }))
    expect(onClose).toHaveBeenCalled()
    expect(c.setRepositoryRemotes).not.toHaveBeenCalled()
    expect(c.linkRepositoryRemote).not.toHaveBeenCalled()
  })
})

describe("the remotes of the repository's folder", () => {
  const ORIGIN = "git@code.example.com:group/app.git"
  function detecting(): ConnectionsClient {
    const c = client()
    vi.mocked(c.listRepositoryRemotes).mockResolvedValue([
      { name: "origin", url: ORIGIN },
      // No connection serves this server: not suggested.
      { name: "mirror", url: "https://mirror.example.org/group/app.git" },
    ])
    return c
  }

  it("links the record the plugin matched to a folder remote, with the folder's URL", async () => {
    const c = detecting()
    vi.mocked(c.resolveRemote).mockResolvedValue([
      {
        id: "gl:4711",
        path: "group/app",
        defaultBranch: "main",
        httpUrl: "https://code.example.com/group/app.git",
      },
    ])
    const { onLinked, user } = setup([], c)
    const dialog = within(
      await screen.findByRole("dialog", { name: "Link App to a remote repository" }),
    )
    expect(c.listRepositoryRemotes).toHaveBeenCalledWith("p1", "pr1", "r1")
    const find = await dialog.findByRole("button", { name: "Find origin on Work GitLab" })
    expect(dialog.queryByText("mirror")).toBeNull()
    await user.click(find)
    await waitFor(() => {
      expect(c.resolveRemote).toHaveBeenCalledWith(PLUGIN, CONNECTION, ORIGIN)
    })
    expect(await dialog.findByTestId("link-summary")).toHaveTextContent(
      "group/app (gl:4711) on Work GitLab will be linked.",
    )
    expect(dialog.getByLabelText("Git remote name")).toHaveValue("origin")
    // Nothing is linked before the user confirms.
    expect(c.linkRepositoryRemote).not.toHaveBeenCalled()
    await user.click(dialog.getByRole("button", { name: "Link" }))
    const expected: RepositoryRemote = {
      name: "origin",
      primary: true,
      connectionId: CONNECTION,
      remoteRepositoryId: "gl:4711",
      remoteUrl: ORIGIN,
      defaultBranch: "main",
    }
    await waitFor(() => {
      expect(c.linkRepositoryRemote).toHaveBeenCalledWith("p1", "pr1", "r1", PLUGIN, expected)
    })
    expect(onLinked).toHaveBeenCalledWith([expected])
  })

  it("searches by the remote's path when the plugin does not match it", async () => {
    const c = detecting()
    // A record without an immutable id is no match (K2).
    vi.mocked(c.resolveRemote).mockResolvedValue([{ id: "group/app", path: "group/app" }])
    const { user } = setup([], c)
    const dialog = within(
      await screen.findByRole("dialog", { name: "Link App to a remote repository" }),
    )
    await user.click(await dialog.findByRole("button", { name: "Find origin on Work GitLab" }))
    expect(
      await dialog.findByText("origin was not found on Work GitLab. Search for it below."),
    ).toBeInTheDocument()
    expect(dialog.getByLabelText("Find a repository")).toHaveValue("group/app")
    expect(dialog.queryByTestId("link-summary")).toBeNull()
    expect(c.linkRepositoryRemote).not.toHaveBeenCalled()
  })

  it("works without suggestions when the folder cannot be read", async () => {
    const c = client()
    vi.mocked(c.listRepositoryRemotes).mockRejectedValue(new Error("no-folder"))
    const { user } = setup([], c)
    const dialog = within(
      await screen.findByRole("dialog", { name: "Link App to a remote repository" }),
    )
    await user.click(await dialog.findByRole("radio", { name: "Work GitLab (code.example.com)" }))
    expect(dialog.queryByText("In the repository's folder")).toBeNull()
    expect(dialog.queryByRole("alert")).toBeNull()
  })

  it("reads hosts and paths of git remote URLs", () => {
    expect(remoteHost("https://Code.Example.com:8443/group/app.git")).toBe("code.example.com")
    expect(remoteHost("ssh://git@code.example.com:2222/group/app.git")).toBe("code.example.com")
    expect(remoteHost("git@code.example.com:group/app.git")).toBe("code.example.com")
    expect(remoteHost("/srv/git/app.git")).toBeNull()
    expect(remoteHost("file:///srv/git/app.git")).toBeNull()
    expect(remotePath("git@code.example.com:group/sub/app.git")).toBe("group/sub/app")
    expect(remotePath("https://code.example.com/group/app.git/")).toBe("group/app")
    const gitlab = { webUrl: "https://code.example.com" }
    expect(servesRemote(gitlab, "git@code.example.com:group/app.git")).toBe(true)
    expect(servesRemote(gitlab, "https://code.example.com.evil.test/group/app.git")).toBe(false)
  })
})

describe("the remotes after a link", () => {
  const record = { id: "gh:1", path: "o/r", httpUrl: "https://github.com/o/r.git" }
  it("replaces a remote of the same name and keeps one primary", () => {
    const remotes: RepositoryRemote[] = [
      { name: "origin", primary: true, remoteUrl: "https://old.example.com/a.git" },
      { name: "upstream", primary: false },
    ]
    expect(withLink(remotes, "origin", true, CONNECTION, record)).toEqual([
      { name: "upstream", primary: false },
      {
        name: "origin",
        primary: true,
        connectionId: CONNECTION,
        remoteRepositoryId: "gh:1",
        remoteUrl: "https://github.com/o/r.git",
      },
    ])
    const added = withLink(remotes, "fork", true, CONNECTION, record)
    expect(added.filter((r) => r.primary).map((r) => r.name)).toEqual(["fork"])
  })
})
