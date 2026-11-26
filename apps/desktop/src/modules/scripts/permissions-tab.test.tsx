// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import type { StoredScript } from "@/lib/types"
import { PermissionsTab } from "@/modules/scripts/permissions-tab"
import { api, resetApi } from "@/test/api-mock"
import { installDomStubs } from "@/test/dom"
import { profileView } from "@/test/fixtures"
import { renderWithApp } from "@/test/render"
import { resetToasts } from "@/test/toast"

vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)
vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)
const dialog = { open: vi.fn() }
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: (...args: unknown[]) => dialog.open(...args) as unknown,
}))

const stored: StoredScript = {
  script: {
    id: "s1",
    name: "Deploy",
    source: "",
    revision: 1,
    settings: { timeoutMs: 60_000, stopOnError: false, memoryMiB: 64, description: "" },
  },
  trust: "trusted",
  grants: null,
  createdAt: 1,
  updatedAt: 1,
}

beforeAll(installDomStubs)
beforeEach(() => {
  resetApi()
  resetToasts()
  dialog.open.mockReset()
  api.getProfile.mockResolvedValue(profileView({}))
})
afterEach(cleanup)

function render() {
  const onSaved = vi.fn()
  const rendered = renderWithApp(
    <PermissionsTab profileId="profile-1" stored={stored} onSaved={onSaved} />,
  )
  return { ...rendered, onSaved }
}

describe("Permissions tab", () => {
  it("never grants a container runtime, a program acting as another user or a shell", async () => {
    const { user } = render()
    const name = screen.getByRole("textbox", { name: "Program name" })
    for (const [command, kind, reason] of [
      ["docker", "containerRuntime", "Container runtimes are not available to scripts yet"],
      ["sudo", "elevation", "A script cannot run a program as another user"],
      ["bash", "shell", "A shell is given under Shells"],
    ] as const) {
      api.scriptCommandClass.mockResolvedValueOnce(kind)
      await user.clear(name)
      await user.type(name, command)
      await user.click(screen.getByRole("button", { name: "Add program" }))
      expect(await screen.findByRole("alert")).toHaveTextContent(reason)
    }
    await user.click(screen.getByRole("button", { name: "Save permissions" }))
    expect(api.setScriptGrants).toHaveBeenCalledWith("profile-1", "s1", {})
  })

  it("adds a program, flags a launcher, and a folder from the system's picker", async () => {
    api.scriptCommandClass.mockResolvedValue("runsAnyProgram")
    dialog.open.mockResolvedValue("/srv/work/docs")
    const { user, onSaved } = render()
    await user.type(screen.getByRole("textbox", { name: "Program name" }), "node")
    await user.click(screen.getByRole("button", { name: "Add program" }))
    expect(await screen.findByText("Runs any program")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Choose folder…" }))
    expect(dialog.open).toHaveBeenCalledWith({ directory: true, multiple: false })
    expect(await screen.findByText(/the folder “docs” \(\/srv\/work\/docs\)/)).toBeInTheDocument()
    await user.click(
      screen.getByRole("switch", { name: "Read the projects and their repositories" }),
    )
    await user.click(screen.getByRole("button", { name: "Save permissions" }))
    await waitFor(() => {
      expect(api.setScriptGrants).toHaveBeenCalled()
    })
    expect(api.setScriptGrants).toHaveBeenCalledWith("profile-1", "s1", {
      process: [{ command: "node", cwd: "repositories", env: "none" }],
      fs: [{ root: { directory: { path: "/srv/work/docs", label: "docs" } }, access: "read" }],
      projectsRead: true,
    })
    expect(onSaved).toHaveBeenCalled()
  })

  it("revokes every permission after asking", async () => {
    const { user } = render()
    await user.click(screen.getByRole("button", { name: "Revoke all" }))
    const confirm = await screen.findByRole("alertdialog")
    await user.click(within(confirm).getByRole("button", { name: "Revoke all" }))
    expect(api.revokeScriptGrants).toHaveBeenCalledWith("profile-1", "s1")
  })
})
