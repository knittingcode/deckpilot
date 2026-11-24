// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import type { Project, ResourceCheck } from "@/lib/types"
import { ProjectsPage } from "@/modules/projects/projects-page"
import { proposeKey, rowProblems, type RepositoryRow } from "@/modules/projects/repositories"
import { api, resetApi } from "@/test/api-mock"
import { installDomStubs } from "@/test/dom"
import { profileView, project } from "@/test/fixtures"
import { renderWithApp } from "@/test/render"
import { resetToasts, toast } from "@/test/toast"

const dialog = vi.hoisted(() => ({ open: vi.fn() }))
vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)
vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: dialog.open }))

const MAIN = "11111111-1111-5111-8111-111111111111"
const CMS = "22222222-2222-4222-8222-222222222222"

const shop: Project = project("SHOP", {
  name: "Online Shop",
  resources: { repository: "shop.repository", docs: "shop.docs" },
  repositories: [
    { id: MAIN, key: "main", name: "Online Shop", resource: "shop.repository" },
    {
      id: CMS,
      key: "cms",
      name: "CMS",
      resource: "shop.cms",
      description: "Content",
      remotes: [{ name: "origin", primary: true, remoteUrl: "https://example.com/cms.git" }],
    },
  ],
  defaultRepository: MAIN,
})

const checks: ResourceCheck[] = [
  {
    key: "shop.repository",
    kind: "directory",
    value: "/src/shop",
    status: "ok",
    usedBy: ["SHOP repository", "SHOP repositories.main"],
  },
  {
    key: "shop.cms",
    kind: "directory",
    value: "/src/gone-cms",
    status: "missing",
    usedBy: ["SHOP repositories.cms", "BLOG cms"],
  },
]

beforeAll(installDomStubs)
beforeEach(() => {
  resetApi()
  resetToasts()
  dialog.open.mockReset()
})
afterEach(cleanup)

async function openEditor(projects: Project[] = [shop]) {
  api.getProfile.mockResolvedValue(profileView({ projects }))
  api.checkResources.mockResolvedValue(checks)
  api.updateProject.mockImplementation((_profile, _id, input) =>
    Promise.resolve({
      ...shop,
      repositories: (input.repositories ?? []).map((r) => ({
        id: r.id ?? "x",
        key: r.key,
        name: r.name,
        resource: r.resource ?? `shop.${r.key}`,
        ...(r.description ? { description: r.description } : {}),
      })),
    }),
  )
  const rendered = renderWithApp(<ProjectsPage />)
  await screen.findByText("Online Shop")
  await rendered.user.click(screen.getByRole("button", { name: "Edit SHOP" }))
  const form = await screen.findByRole("dialog", { name: "Edit SHOP" })
  return { ...rendered, form }
}

function row(form: HTMLElement, name: string): HTMLElement {
  return within(form).getByRole("listitem", { name: `Repository ${name}` })
}

describe("Repositories in the project form", () => {
  it("lists the repositories with their folder, status and default", async () => {
    const { form, user } = await openEditor()
    const main = row(form, "Online Shop")
    expect(within(main).getByRole("textbox", { name: "Repository key" })).toHaveValue("main")
    expect(within(main).getByText("/src/shop")).toBeInTheDocument()
    expect(
      within(main).getByRole("radio", { name: "Default repository: Online Shop" }),
    ).toBeChecked()
    const cms = row(form, "CMS")
    // A folder that is gone is shown, never removed.
    expect(within(cms).getByText("Folder not found")).toBeInTheDocument()
    expect(within(cms).getByText("/src/gone-cms")).toBeInTheDocument()
    expect(within(cms).getByRole("textbox", { name: "Description" })).toHaveValue("Content")
    // With repositories, the role/key pairs are the advanced part, collapsed.
    expect(within(form).queryByRole("textbox", { name: "Role" })).not.toBeInTheDocument()
    await user.click(within(form).getByRole("button", { name: "Resource roles (advanced)" }))
    expect(await within(form).findAllByRole("textbox", { name: "Role" })).toHaveLength(2)
  })

  it("saves renames and keeps metadata the form does not edit", async () => {
    const { form, user } = await openEditor()
    const cms = row(form, "CMS")
    const name = within(cms).getByRole("textbox", { name: "Repository name" })
    await user.clear(name)
    await user.type(name, "Content")
    const key = within(form).getAllByRole("textbox", { name: "Repository key" })[1] as HTMLElement
    await user.clear(key)
    await user.type(key, "content")
    await user.click(within(form).getByRole("button", { name: "Save" }))
    await waitFor(() => {
      expect(api.updateProject).toHaveBeenCalled()
    })
    const input = api.updateProject.mock.calls[0]?.[2]
    expect(input).toEqual({
      key: "SHOP",
      name: "Online Shop",
      resources: { repository: "shop.repository", docs: "shop.docs" },
      repositories: [
        { id: MAIN, key: "main", name: "Online Shop", resource: "shop.repository" },
        { id: CMS, key: "content", name: "Content", resource: "shop.cms", description: "Content" },
      ],
      defaultRepository: MAIN,
    })
    // Remotes are never sent: the backend keeps them by id.
    expect(JSON.stringify(input)).not.toContain("remotes")
    expect(api.setResourceBinding).not.toHaveBeenCalled()
    expect(toast.success).toHaveBeenCalledWith("Project saved")
  })

  it("refuses duplicate and invalid keys inline", async () => {
    const { form, user } = await openEditor()
    const key = within(form).getAllByRole("textbox", { name: "Repository key" })[1] as HTMLElement
    await user.clear(key)
    await user.type(key, "main")
    expect(
      within(form).getAllByText("The key “main” is already used in this project."),
    ).toHaveLength(2)
    expect(within(form).getByRole("button", { name: "Save" })).toBeDisabled()
    await user.clear(key)
    await user.type(key, "Not-Valid")
    expect(
      within(form).getByText(
        "Use a lowercase letter, then up to 31 lowercase letters, digits or _.",
      ),
    ).toBeVisible()
    expect(within(form).getByRole("button", { name: "Save" })).toBeDisabled()
    await user.clear(key)
    await user.type(key, "cms_2")
    expect(within(form).getByRole("button", { name: "Save" })).toBeEnabled()
    expect(api.updateProject).not.toHaveBeenCalled()
  })

  it("adds a repository from the native folder picker and binds its folder on save", async () => {
    dialog.open.mockResolvedValue("/home/me/src/Shop Admin")
    const { form, user } = await openEditor()
    await user.click(within(form).getByRole("button", { name: "Add repository…" }))
    expect(dialog.open).toHaveBeenCalledWith({ directory: true, multiple: false })
    const added = row(form, "Shop Admin")
    expect(within(added).getByRole("textbox", { name: "Repository key" })).toHaveValue("shop_admin")
    expect(within(added).getByText("/home/me/src/Shop Admin")).toBeInTheDocument()
    expect(
      within(added).getByText("This folder is set on this computer when you save."),
    ).toBeInTheDocument()
    await user.click(within(form).getByRole("button", { name: "Save" }))
    await waitFor(() => {
      expect(api.setResourceBinding).toHaveBeenCalledWith(
        "shop.shop_admin",
        "directory",
        "/home/me/src/Shop Admin",
      )
    })
    const sent = api.updateProject.mock.calls[0]?.[2].repositories ?? []
    expect(sent).toHaveLength(3)
    // A new one has an id of its own and no resource: the backend makes one.
    expect(sent[2]).toMatchObject({ key: "shop_admin", name: "Shop Admin" })
    expect(sent[2]?.resource).toBeUndefined()
    expect(sent[2]?.id).toMatch(/^[0-9a-f-]{36}$/)
  })

  it("changes a folder with the native picker, bound under the same key on save", async () => {
    dialog.open.mockResolvedValue("/src/cms-moved")
    const { form, user } = await openEditor()
    await user.click(within(row(form, "CMS")).getByRole("button", { name: "Choose folder…" }))
    expect(within(row(form, "CMS")).getByText("/src/cms-moved")).toBeInTheDocument()
    await user.click(within(form).getByRole("button", { name: "Save" }))
    await waitFor(() => {
      expect(api.setResourceBinding).toHaveBeenCalledWith("shop.cms", "directory", "/src/cms-moved")
    })
  })

  it("does nothing when the folder picker is cancelled", async () => {
    dialog.open.mockResolvedValue(null)
    const { form, user } = await openEditor()
    await user.click(within(form).getByRole("button", { name: "Add repository…" }))
    expect(within(form).getAllByRole("listitem")).toHaveLength(2)
  })

  it("reorders and marks another default, saying the role follows", async () => {
    const { form, user } = await openEditor()
    await user.click(within(row(form, "CMS")).getByRole("button", { name: "More for CMS" }))
    await user.click(await screen.findByRole("menuitem", { name: "Move up" }))
    await user.click(
      within(row(form, "CMS")).getByRole("radio", { name: "Default repository: CMS" }),
    )
    expect(
      within(form).getByText("Actions that use the repository role will use CMS."),
    ).toBeInTheDocument()
    await user.click(within(form).getByRole("button", { name: "Save" }))
    await waitFor(() => {
      expect(api.updateProject).toHaveBeenCalled()
    })
    const input = api.updateProject.mock.calls[0]?.[2]
    expect(input?.repositories?.map((r) => r.key)).toEqual(["cms", "main"])
    expect(input?.defaultRepository).toBe(CMS)
  })

  it("removes a repository after a confirmation that lists its uses, keeping its folder", async () => {
    const { form, user } = await openEditor()
    await user.click(
      within(row(form, "Online Shop")).getByRole("button", { name: "Remove Online Shop" }),
    )
    const confirm = screen.getByRole("alertdialog", { name: "Remove Online Shop?" })
    expect(
      within(confirm).getByText(
        "The repository role of this project points at its folder and keeps pointing there.",
      ),
    ).toBeInTheDocument()
    expect(within(confirm).getByText("Also used by SHOP repository")).toBeInTheDocument()
    await user.click(within(confirm).getByRole("button", { name: "Remove" }))
    expect(
      await within(form).findByText(
        "The repository role still points at the removed repository's folder.",
      ),
    ).toBeInTheDocument()
    expect(within(form).queryByRole("listitem", { name: "Repository Online Shop" })).toBeNull()
    await user.click(within(form).getByRole("button", { name: "Save" }))
    await waitFor(() => {
      expect(api.updateProject).toHaveBeenCalled()
    })
    const input = api.updateProject.mock.calls[0]?.[2]
    expect(input?.repositories?.map((r) => r.id)).toEqual([CMS])
    expect(input?.defaultRepository).toBeNull()
    // The role and every binding stay.
    expect(input?.resources).toEqual({ repository: "shop.repository", docs: "shop.docs" })
    expect(api.removeResourceBinding).not.toHaveBeenCalled()
  })

  it("keeps the last repository while the repository role would bring it back", async () => {
    const single = { ...shop, repositories: shop.repositories?.slice(0, 1) ?? [] }
    const { form, user } = await openEditor([single])
    const remove = within(row(form, "Online Shop")).getByRole("button", {
      name: "Remove Online Shop",
    })
    expect(remove).toBeDisabled()
    expect(remove).toHaveAttribute(
      "title",
      "Remove the repository role first to remove the last repository.",
    )
    // Without the role, it can go.
    await user.click(within(form).getByRole("button", { name: "Resource roles (advanced)" }))
    const roles = within(form).getAllByRole("textbox", { name: "Role" })
    await user.clear(roles[0] as HTMLElement)
    await user.click(
      within(form).getAllByRole("button", { name: "Remove resource" })[0] as HTMLElement,
    )
    expect(
      within(row(form, "Online Shop")).getByRole("button", { name: "Remove Online Shop" }),
    ).toBeEnabled()
  })

  it("asks before discarding repository changes", async () => {
    const { form, user } = await openEditor()
    const name = within(row(form, "CMS")).getByRole("textbox", { name: "Repository name" })
    await user.type(name, "!")
    await user.click(within(form).getByRole("button", { name: "Cancel" }))
    expect(await screen.findByRole("alertdialog")).toBeInTheDocument()
  })

  it("shows a project's repositories in the list", async () => {
    api.getProfile.mockResolvedValue(profileView({ projects: [shop] }))
    api.checkResources.mockResolvedValue(checks)
    renderWithApp(<ProjectsPage />)
    expect(await screen.findByText("Repositories: main (default), cms")).toBeInTheDocument()
  })
})

describe("repository helpers", () => {
  it("proposes a unique key from a folder name", () => {
    expect(proposeKey("Shop Admin", [])).toBe("shop_admin")
    expect(proposeKey("Żółć-API", [])).toBe("zolc_api")
    expect(proposeKey("2024 site", [])).toBe("repo_2024_site")
    expect(proposeKey("!!!", [])).toBe("repo")
    expect(proposeKey("cms", ["cms", "cms_2"])).toBe("cms_3")
    const long = proposeKey("x".repeat(40), ["x".repeat(32)])
    expect(long).toBe(`${"x".repeat(30)}_2`)
    expect(long).toHaveLength(32)
  })

  it("checks keys, names and descriptions like the backend", () => {
    const rows: RepositoryRow[] = [
      { id: "a", key: "main", name: "A", description: "", resource: null, pendingPath: null },
      {
        id: "b",
        key: "main",
        name: "x".repeat(65),
        description: "",
        resource: null,
        pendingPath: null,
      },
      {
        id: "c",
        key: "Bad",
        name: "C",
        description: "d".repeat(501),
        resource: null,
        pendingPath: null,
      },
    ]
    expect(rowProblems(rows, 0)).toEqual({ key: "duplicate" })
    expect(rowProblems(rows, 1)).toEqual({ key: "duplicate", name: "tooLong" })
    expect(rowProblems(rows, 2)).toEqual({ key: "invalid", description: "tooLong" })
  })
})
