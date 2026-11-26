// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import type { Script, StoredScript } from "@/lib/types"
import { ScriptsPage } from "@/modules/scripts/scripts-page"
import { api, resetApi } from "@/test/api-mock"
import { installDomStubs } from "@/test/dom"
import { profileView } from "@/test/fixtures"
import { renderWithApp } from "@/test/render"
import { resetToasts, toast } from "@/test/toast"

vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)
vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)

function script(name: string, source = "deckpilot.log('hi')", revision = 1): Script {
  return {
    id: `id-${name}`,
    name,
    source,
    revision,
    settings: { timeoutMs: 60_000, stopOnError: false, memoryMiB: 64, description: "" },
  }
}

function stored(s: Script, trust: StoredScript["trust"] = "trusted"): StoredScript {
  return { script: s, trust, grants: null, createdAt: 1, updatedAt: 1 }
}

const deploy = script("Deploy")
const imported = script("Imported", "evil()‮reversed")

beforeAll(() => {
  installDomStubs()
  // jsdom measures no text; CodeMirror asks for it while it lays out.
  const empty = () => Object.assign([], { item: () => null }) as unknown as DOMRectList
  Range.prototype.getClientRects = empty
  Range.prototype.getBoundingClientRect = () => new DOMRect()
})
beforeEach(() => {
  resetApi()
  resetToasts()
  api.getProfile.mockResolvedValue(profileView({}))
})
afterEach(cleanup)

async function renderScripts(list: StoredScript[]) {
  api.listScripts.mockResolvedValue(list)
  const rendered = renderWithApp(<ScriptsPage />)
  await screen.findByLabelText("Search scripts")
  return rendered
}

describe("hidden characters", () => {
  it("are found as the core finds them, payload ranges included", async () => {
    const { hasHiddenChars } = await import("./completions")
    for (const hidden of [
      "\u202E",
      "\u200B",
      "\u{E0041}",
      "\u{E0100}",
      "\u3164",
      "\u0007",
      "\u00AD",
    ]) {
      expect(hasHiddenChars(`a${hidden}b`), JSON.stringify(hidden)).toBe(true)
    }
    expect(hasHiddenChars("const a = 1\n\tb\r\n")).toBe(false)
  })
})

describe("Scripts page", () => {
  it("creates a script, trusted with nothing granted", async () => {
    api.createScript.mockResolvedValue(script("Build"))
    const { user } = await renderScripts([])
    expect(await screen.findByText("No scripts yet")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "New script" }))
    const dialog = await screen.findByRole("dialog", { name: "New script" })
    expect(within(dialog).getByText(/no permissions/)).toBeInTheDocument()
    await user.type(within(dialog).getByLabelText("Name"), "  Build ")
    await user.click(within(dialog).getByRole("button", { name: "Create" }))
    expect(api.createScript).toHaveBeenCalledWith("profile-1", { name: "Build", source: "" })
  })

  it("saves the editor's code from its revision", async () => {
    api.saveScript.mockResolvedValue({ ...deploy, revision: 2 })
    const { user } = await renderScripts([stored(deploy)])
    await user.click(await screen.findByRole("button", { name: /Deploy/ }))
    const editor = await screen.findByRole("textbox", { name: "Code of Deploy" })
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled()
    await user.click(editor)
    await user.keyboard("{Control>}{End}{/Control};")
    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Save" })).toBeEnabled()
    })
    expect(screen.getByText("Unsaved changes")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Save" }))
    await waitFor(() => {
      expect(api.saveScript).toHaveBeenCalled()
    })
    const call = api.saveScript.mock.calls[0]
    if (!call) throw new Error("not saved")
    const [profile, saved] = call
    expect(profile).toBe("profile-1")
    expect(saved.revision).toBe(1)
    expect(saved.source.endsWith(";")).toBe(true)
    expect(toast.success).toHaveBeenCalledWith("“Deploy” saved")
  })

  it("shows an untrusted script read only, with its hidden characters and permissions, until trusted", async () => {
    api.reviewScript.mockResolvedValue({
      script: imported,
      trust: "needsReview",
      sourceSha256: "a".repeat(64),
      grants: { projectsRead: true, process: [{ command: "make" }] },
      commandClasses: { make: "runsAnyProgram" },
    })
    const { user } = await renderScripts([stored(imported, "needsReview")])
    await user.click(await screen.findByRole("button", { name: /Imported/ }))
    expect(await screen.findByText("Review this script before it runs")).toBeInTheDocument()
    expect(screen.getByText("Invisible characters")).toBeInTheDocument()
    // No editing, no save.
    expect(screen.queryByRole("button", { name: "Save" })).not.toBeInTheDocument()
    const grants = screen.getByRole("list", {
      name: "Permissions that come back when you trust it",
    })
    expect(grants).toHaveTextContent("Read the projects and their repositories")
    expect(grants).toHaveTextContent(
      "Run make (it runs any other program, as broad as a shell; in the project's repositories)",
    )
    await user.click(screen.getByRole("button", { name: "Trust this script" }))
    // Exactly what the review showed.
    expect(api.trustScript).toHaveBeenCalledWith("profile-1", imported.id, "a".repeat(64), {
      projectsRead: true,
      process: [{ command: "make" }],
    })
  })

  it("lists a script's uses before deleting it and clears keys only when asked", async () => {
    api.scriptUsages.mockResolvedValue([
      {
        action: "a1",
        actionName: "Run Deploy",
        references: [
          {
            site: {
              type: "binding",
              layout: "l1",
              page: "p1",
              control: "key:0",
              trigger: { type: "press" },
            },
            ownerName: "Main",
            detail: "Home",
          },
        ],
      },
    ] as never)
    const { user } = await renderScripts([stored(deploy)])
    await user.click(await screen.findByRole("button", { name: /Deploy/ }))
    await user.click(screen.getByRole("button", { name: "Delete Deploy" }))
    const dialog = await screen.findByRole("alertdialog")
    expect(await within(dialog).findByText("Run Deploy — Main › Home")).toBeInTheDocument()
    expect(within(dialog).getByText(/used in 1 place/)).toBeInTheDocument()
    await user.click(within(dialog).getByRole("button", { name: "Delete script" }))
    expect(api.deleteScript).toHaveBeenCalledWith("profile-1", deploy.id, false)
  })

  it("names every permission that comes back, with its reach", async () => {
    api.reviewScript.mockResolvedValue({
      script: imported,
      trust: "needsReview",
      sourceSha256: "b".repeat(64),
      grants: {
        repositoriesPaths: true,
        fs: [
          {
            root: { directory: { path: "/srv/docs", label: "Docs" } },
            access: "delete",
            rootDestructive: true,
          },
        ],
        process: [{ command: "git", unrestrictedArgs: true, env: "allowed", cwd: "any" }],
        shell: { shells: ["sh"] },
        destructive: "allow",
        limits: { timeoutMs: 120_000, memoryMiB: 128, maxOutputKiB: 1024 },
      },
      commandClasses: { git: "plain" },
    })
    const { user } = await renderScripts([stored(imported, "needsReview")])
    await user.click(await screen.findByRole("button", { name: /Imported/ }))
    const grants = await screen.findByRole("list", {
      name: "Permissions that come back when you trust it",
    })
    for (const line of [
      "See the folders of the repositories",
      "Read, change and delete files in the folder “Docs” (/srv/docs)",
      "Remove, empty or replace the folder “Docs” (/srv/docs) itself",
      "Run git (with any arguments, options that run programs included; may set environment variables; in any folder)",
      "Run shell code with sh",
      "Delete files without asking",
      "Limits: 120 s, 128 MiB, 1024 KiB of output",
    ]) {
      expect(grants).toHaveTextContent(line)
    }
  })

  it("never deletes before the uses are known", async () => {
    api.scriptUsages.mockRejectedValue(new Error("busy"))
    const { user } = await renderScripts([stored(deploy)])
    await user.click(await screen.findByRole("button", { name: /Deploy/ }))
    await user.click(screen.getByRole("button", { name: "Delete Deploy" }))
    const dialog = await screen.findByRole("alertdialog")
    expect(await within(dialog).findByText(/could not be listed/)).toBeInTheDocument()
    await user.click(within(dialog).getByRole("button", { name: "Delete script" }))
    expect(api.deleteScript).not.toHaveBeenCalled()
  })
})
