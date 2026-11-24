// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { DialogProvider } from "@/components/dialogs/dialog-service"
import type { DeviceLayout } from "@/lib/types"
import { LayoutDesignerPage } from "@/pages/layout-designer"

import { LayoutMenu } from "./layout-management"
import { resolveLayout } from "./layout-selection"
import { type EditorSelection, EditorSelectionContext } from "@/shell/editor-selection-context"
import { api, resetApi } from "@/test/api-mock"
import { installDomStubs } from "@/test/dom"
import {
  agentConnection,
  agentStatus,
  deviceInfo,
  deviceModel,
  profileView,
  profiles,
} from "@/test/fixtures"
import { renderWithApp } from "@/test/render"
import { assignment, sharedLayout } from "@/test/shared-layouts"
import { resetToasts, toast } from "@/test/toast"

vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)
vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)

const mk2 = deviceModel()
const MK2 = mk2.id
const XL = "elgato.streamdeck.xl"

const layout = (id: string, name: string, model = MK2): DeviceLayout => ({
  id,
  name,
  model,
  homePage: `${id}-home`,
  pages: [{ id: `${id}-home`, name: "Home", bindings: [] }],
})
const main = layout("main", "Main")
const teams = layout("teams", "Teams")
const big = layout("big", "Big board", XL)

const work = assignment("profile-1", "Work", { isDefault: true })
const home = assignment("profile-2", "Home")

function selection(change: Partial<EditorSelection> = {}): EditorSelection {
  return {
    models: { status: "ready", data: [mk2] },
    modelId: MK2,
    device: null,
    layoutId: null,
    selectDevice: vi.fn(() => true),
    selectProfile: vi.fn(() => true),
    selectLayout: vi.fn(() => true),
    showLayout: vi.fn(),
    adoptModel: vi.fn(),
    setDirty: vi.fn(),
    guard: vi.fn(() => true),
    isDirty: vi.fn(() => false),
    place: vi.fn(() => undefined),
    rememberPlace: vi.fn(),
    ...change,
  }
}

beforeAll(installDomStubs)
beforeEach(() => {
  resetApi()
  resetToasts()
  api.getProfile.mockResolvedValue(
    profileView({ layouts: [main, teams, big], activeLayouts: { [MK2]: "main" } }),
  )
  api.listSharedLayouts.mockResolvedValue([
    sharedLayout({ id: "main", name: "Main", profiles: [work, home] }),
    sharedLayout({ id: "teams", name: "Teams", profiles: [assignment("profile-1", "Work")] }),
    sharedLayout({ id: "spare", name: "Spare", profiles: [] }),
    sharedLayout({ id: "media", name: "Media", profiles: [home] }),
    sharedLayout({
      id: "big",
      name: "Big board",
      model: XL,
      modelName: "Stream Deck XL",
      profiles: [assignment("profile-1", "Work")],
    }),
  ])
  api.previewLayout.mockResolvedValue([])
  api.listIcons.mockResolvedValue([])
  api.pageTree.mockResolvedValue({ pages: [], warnings: [] })
  api.listActionTypes.mockResolvedValue([])
  api.getGestureDefaults.mockResolvedValue({ doubleClickMs: 300, longPressMs: 700 })
  api.assignLayout.mockResolvedValue(null)
  api.activateLayout.mockResolvedValue(null)
  api.createLayout.mockResolvedValue({ ...layout("fresh", "Fresh") })
  api.duplicateLayout.mockResolvedValue("main-copy")
  api.renameSharedLayout.mockResolvedValue(null)
  api.deleteLayout.mockResolvedValue(null)
})
afterEach(cleanup)

/**
 * The toolbar's layout selector beside the Device Editor, as in the main
 * window: both read the shell's editor selection.
 */
async function renderDesigner(editor = selection(), agent = agentConnection()) {
  const view = renderWithApp(
    <DialogProvider>
      <EditorSelectionContext value={editor}>
        <LayoutMenu />
        <LayoutDesignerPage />
      </EditorSelectionContext>
    </DialogProvider>,
    { profiles: profiles([{ id: "profile-1", name: "Work" }]), agent },
  )
  await screen.findByRole("group", { name: /controls$/ })
  return { ...view, editor }
}

const trigger = (name = /^Layout: Main/) => screen.findByRole("button", { name })

async function openMenu(user: { click: (e: Element) => Promise<void> }, name?: RegExp) {
  await waitFor(async () => {
    expect(await trigger(name)).toBeEnabled()
  })
  await user.click(await trigger(name))
  return screen.findByRole("menu")
}

describe("resolveLayout", () => {
  it("takes the chosen layout, else the default, else the first", () => {
    expect(resolveLayout([main, teams], "teams", "main")).toBe(teams)
    expect(resolveLayout([main, teams], null, "teams")).toBe(teams)
    expect(resolveLayout([main, teams], "gone", "gone")).toBe(main)
    expect(resolveLayout<DeviceLayout>([], null, null)).toBeUndefined()
  })
})

describe("the toolbar's layout selector", { timeout: 20_000 }, () => {
  it("lists only this profile's layouts for the device's model, the default starred", async () => {
    const { user } = await renderDesigner()
    expect(await trigger(/^Layout: Main, default$/)).toBeInTheDocument()
    const menu = await openMenu(user)
    const own = within(menu).getByRole("group", { name: "Layouts of this profile" })
    const items = within(own).getAllByRole("menuitem")
    expect(items.map((i) => i.textContent)).toEqual(["Main", "Teams"])
    expect(items[0]).toHaveAttribute("aria-current", "true")
    // A star per layout: filled for the profile's default, outlined otherwise.
    expect(
      within(own)
        .getAllByRole("menuitemcheckbox")
        .map((i) => i.getAttribute("aria-label")),
    ).toEqual(["“Main” is the default layout", "Set “Teams” as default"])
    // Not other profiles' layouts (Media, Spare), not other models' (Big board).
    expect(menu).not.toHaveTextContent(/Media|Spare|Big board|From other profiles/)
    expect(within(menu).queryByRole("group", { name: /other/i })).not.toBeInTheDocument()
    // Below one divider: only New and Manage; the rest is in the Layout Manager.
    expect(
      within(menu)
        .getAllByRole("menuitem")
        .slice(2)
        .map((i) => i.textContent.trim()),
    ).toEqual(["New layout…", "Manage layouts…"])
    expect(menu.querySelectorAll("[role='separator']")).toHaveLength(1)
    expect(
      within(menu).queryByRole("menuitem", { name: /^(Duplicate|Rename|Delete|Set as default)/ }),
    ).toBeNull()
  })

  describe("the default star", () => {
    const star = (menu: HTMLElement, name: RegExp) =>
      within(menu).getByRole("menuitemcheckbox", { name })

    it("is filled for the default, outlined for the others, with tooltips", async () => {
      const { user } = await renderDesigner()
      const menu = await openMenu(user)
      const main = star(menu, /Main/)
      const teams = star(menu, /Teams/)
      expect(main).toHaveAttribute("aria-checked", "true")
      expect(main.querySelector("svg")).toHaveClass("fill-warning", "text-warning")
      expect(teams).toHaveAttribute("aria-checked", "false")
      expect(teams.querySelector("svg")).not.toHaveClass("fill-warning")
      expect(teams.querySelector("svg")).toHaveClass("text-muted-foreground")
      await user.hover(teams)
      expect(await screen.findByRole("tooltip")).toHaveTextContent("Set as default")
      await user.hover(main)
      await waitFor(() => {
        expect(screen.getByRole("tooltip")).toHaveTextContent("Default layout")
      })
    })

    it("sets the default without switching the layout or closing the menu", async () => {
      const { user, editor, context } = await renderDesigner()
      const menu = await openMenu(user)
      await user.click(star(menu, /Teams/))
      await waitFor(() => {
        expect(api.activateLayout).toHaveBeenCalledWith("profile-1", "teams")
      })
      expect(editor.selectLayout).not.toHaveBeenCalled()
      expect(screen.getByRole("menu")).toBeInTheDocument()
      await waitFor(() => {
        expect(context.refresh).toHaveBeenCalled()
      })
      expect(toast.success).toHaveBeenCalledWith("Teams is used for Stream Deck MK.2")
    })

    it("does nothing for the default's own star", async () => {
      const { user } = await renderDesigner()
      const menu = await openMenu(user)
      await user.click(star(menu, /Main/))
      expect(api.activateLayout).not.toHaveBeenCalled()
      expect(screen.getByRole("menu")).toBeInTheDocument()
    })

    it("keeps the old default when saving fails, with an error", async () => {
      api.activateLayout.mockRejectedValue(new Error("disk full"))
      const { user, context } = await renderDesigner()
      const menu = await openMenu(user)
      await user.click(star(menu, /Teams/))
      await waitFor(() => {
        expect(toast.error).toHaveBeenCalled()
      })
      expect(context.refresh).not.toHaveBeenCalled()
      expect(star(menu, /Main/)).toHaveAttribute("aria-checked", "true")
      expect(star(menu, /Teams/)).toHaveAttribute("aria-checked", "false")
    })

    it("is disabled while the default is being saved", async () => {
      let resolve = () => {}
      api.activateLayout.mockReturnValue(
        new Promise<null>((done) => {
          resolve = () => {
            done(null)
          }
        }),
      )
      const { user } = await renderDesigner()
      const menu = await openMenu(user)
      await user.click(star(menu, /Teams/))
      await waitFor(() => {
        expect(star(menu, /Teams/)).toHaveAttribute("aria-disabled", "true")
      })
      expect(star(menu, /Main/)).toHaveAttribute("aria-disabled", "true")
      resolve()
      await waitFor(() => {
        expect(star(menu, /Teams/)).not.toHaveAttribute("aria-disabled")
      })
      expect(api.activateLayout).toHaveBeenCalledTimes(1)
    })

    it("is reached with the arrow keys after its layout and set with Enter", async () => {
      const { user, editor } = await renderDesigner()
      ;(await trigger()).focus()
      await user.keyboard("{Enter}")
      const menu = await screen.findByRole("menu")
      // Main, Main's star, Teams, Teams' star.
      await user.keyboard("{ArrowDown}{ArrowDown}{ArrowDown}")
      expect(document.activeElement).toBe(star(menu, /Teams/))
      await user.keyboard("{Enter}")
      await waitFor(() => {
        expect(api.activateLayout).toHaveBeenCalledWith("profile-1", "teams")
      })
      expect(editor.selectLayout).not.toHaveBeenCalled()
    })
  })

  it("tells apart layouts of the same name by their contents", async () => {
    api.getProfile.mockResolvedValue(
      profileView({ layouts: [main, layout("main-2", "Main")], activeLayouts: { [MK2]: "main" } }),
    )
    api.listSharedLayouts.mockResolvedValue([
      sharedLayout({ id: "main", name: "Main", pages: 2, bindings: 9, profiles: [work] }),
      sharedLayout({ id: "main-2", name: "Main", pages: 1, bindings: 3, profiles: [work] }),
    ])
    const { user } = await renderDesigner()
    const menu = await openMenu(user)
    await waitFor(() => {
      expect(
        within(menu)
          .getAllByRole("menuitem")
          .slice(0, 2)
          .map((i) => i.textContent),
      ).toEqual(["Main2 pages · 9 keys", "Main1 page · 3 keys"])
    })
  })

  it("truncates a long name, with the full name on hover", async () => {
    const long = "A very long layout name that does not fit into the menu at all"
    api.getProfile.mockResolvedValue(
      profileView({ layouts: [main, layout("long", long)], activeLayouts: { [MK2]: "main" } }),
    )
    const { user } = await renderDesigner()
    const menu = await openMenu(user)
    const item = within(menu).getByRole("menuitem", { name: long })
    expect(item).toHaveAttribute("title", long)
    expect(within(item).getByText(long)).toHaveClass("truncate")
  })

  it("selects a layout of the profile by its id", async () => {
    const { user, editor } = await renderDesigner()
    await openMenu(user)
    await user.click(await screen.findByRole("menuitem", { name: /^Teams/ }))
    expect(editor.selectLayout).toHaveBeenCalledWith("teams")
    expectNoWrites()
  })

  it("is used with the keyboard", async () => {
    const { user, editor } = await renderDesigner()
    await waitFor(async () => {
      expect(await trigger()).toBeEnabled()
    })
    ;(await trigger()).focus()
    await user.keyboard("{Enter}")
    await screen.findByRole("menu")
    // The first item has the focus; after its star comes Teams.
    await user.keyboard("{ArrowDown}{ArrowDown}")
    expect(document.activeElement).toHaveTextContent(/^Teams/)
    await user.keyboard("{Enter}")
    expect(editor.selectLayout).toHaveBeenCalledWith("teams")
  })

  it("asks before discarding unsaved changes, and keeps them when cancelled", async () => {
    const editor = selection({ isDirty: vi.fn(() => true) })
    const { user } = await renderDesigner(editor)
    await openMenu(user)
    await user.click(await screen.findByRole("menuitem", { name: /^Teams/ }))
    const dialog = await screen.findByRole("alertdialog")
    expect(dialog).toHaveTextContent("Discard unsaved changes?")
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }))
    await waitFor(() => {
      expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument()
    })
    expect(editor.selectLayout).not.toHaveBeenCalled()

    await openMenu(user)
    await user.click(await screen.findByRole("menuitem", { name: /^Teams/ }))
    await user.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", {
        name: "Discard changes",
      }),
    )
    await waitFor(() => {
      expect(editor.selectLayout).toHaveBeenCalledWith("teams")
    })
    expect(editor.setDirty).toHaveBeenCalledWith(false)
  })

  it("has no shared note for a layout only this profile uses", async () => {
    await renderDesigner(selection({ layoutId: "teams" }))
    await trigger(/^Layout: Teams$/)
    await waitFor(() => {
      expect(api.listSharedLayouts).toHaveBeenCalled()
    })
    expect(screen.queryByText(/Shared: edits apply/)).not.toBeInTheDocument()
  })

  it("opens the layout the device shows when none is chosen", async () => {
    const agent = agentConnection({
      status: agentStatus({
        devices: [deviceInfo({ layout: { type: "compatible", layout: "teams", name: "Teams" } })],
      }),
    })
    await renderDesigner(selection(), agent)
    await trigger(/^Layout: Teams/)
    expect(screen.queryByText(/The device shows/)).not.toBeInTheDocument()
  })

  it("says which layout the device shows and opens it", async () => {
    const agent = agentConnection({
      status: agentStatus({
        devices: [deviceInfo({ layout: { type: "compatible", layout: "teams", name: "Teams" } })],
      }),
    })
    // Main chosen in the toolbar while the device shows Teams.
    const { user, editor } = await renderDesigner(selection({ layoutId: "main" }), agent)
    expect(await screen.findByText("The device shows Teams.")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Open Teams" }))
    expect(editor.showLayout).toHaveBeenCalledWith("teams", MK2)
    // The selector marks it as the one on the device.
    await openMenu(user)
    const item = await screen.findByRole("menuitem", { name: /^Teams/ })
    expect(within(item).getByRole("img", { name: "Shown on the device" })).toBeInTheDocument()
  })

  it("does not hint when the device shows the open layout", async () => {
    const agent = agentConnection({
      status: agentStatus({
        devices: [deviceInfo({ layout: { type: "compatible", layout: "main", name: "Main" } })],
      }),
    })
    await renderDesigner(selection(), agent)
    await trigger()
    expect(screen.queryByText(/The device shows/)).not.toBeInTheDocument()
  })

  it("creates a layout for the model and opens it", async () => {
    const { user, editor, context } = await renderDesigner()
    await openMenu(user)
    await user.click(await screen.findByRole("menuitem", { name: /New layout/ }))
    const dialog = await screen.findByRole("dialog", { name: "New layout for Stream Deck MK.2" })
    await user.type(within(dialog).getByLabelText("Name"), "Fresh")
    await user.click(within(dialog).getByRole("button", { name: "Create" }))
    await waitFor(() => {
      expect(api.createLayout).toHaveBeenCalledWith("profile-1", MK2, "Fresh", true)
    })
    await waitFor(() => {
      expect(editor.showLayout).toHaveBeenCalledWith("fresh", MK2)
    })
    expect(context.refresh).toHaveBeenCalled()
  })

  it("opens the Layout Library to manage layouts", async () => {
    const { user, context } = await renderDesigner()
    await openMenu(user)
    await user.click(await screen.findByRole("menuitem", { name: /Manage layouts/ }))
    // The Layout Manager is a settings page.
    expect(context.openSettings).toHaveBeenCalledWith("layouts")
  })
})

/** Nothing about layouts or profiles was written. */
function expectNoWrites() {
  for (const call of [
    api.assignLayout,
    api.unassignLayout,
    api.activateLayout,
    api.createLayout,
    api.duplicateLayout,
    api.duplicateSharedLayout,
    api.renameSharedLayout,
    api.deleteLayout,
    api.deleteSharedLayout,
  ]) {
    expect(call).not.toHaveBeenCalled()
  }
}

/** The selector alone, for profile switches and empty profiles. */
function renderMenu(editingProfileId = "profile-1") {
  const editor = selection()
  const ui = (
    <DialogProvider>
      <EditorSelectionContext value={editor}>
        <LayoutMenu />
      </EditorSelectionContext>
    </DialogProvider>
  )
  const people = profiles([
    { id: "profile-1", name: "Work" },
    { id: "profile-2", name: "Home" },
  ])
  const view = renderWithApp(ui, { profiles: people, editingProfileId })
  return { ...view, editor, ui, people }
}

describe("the layout selector across profiles", { timeout: 20_000 }, () => {
  const homeMain = layout("home-main", "Home desk")
  const homeAlt = layout("home-alt", "Home alt")
  beforeEach(() => {
    api.getProfile.mockImplementation((id: string) =>
      Promise.resolve(
        id === "profile-2"
          ? profileView({
              id: "profile-2",
              name: "Home",
              layouts: [homeAlt, homeMain, big],
              activeLayouts: { [MK2]: "home-main" },
            })
          : profileView({ layouts: [main, teams, big], activeLayouts: { [MK2]: "main" } }),
      ),
    )
  })

  it("lists the new profile's layouts and opens its default, writing nothing", async () => {
    const { rerender, ui, context } = renderMenu()
    expect(await trigger(/^Layout: Main, default$/)).toBeInTheDocument()
    rerender(ui, { ...context, editingProfileId: "profile-2" })
    expect(await trigger(/^Layout: Home desk, default$/)).toBeInTheDocument()
    expect(api.getProfile).toHaveBeenLastCalledWith("profile-2")
    expectNoWrites()
  })

  it("opens the first compatible layout of a profile without a default", async () => {
    api.getProfile.mockResolvedValue(
      profileView({ id: "profile-2", layouts: [big, homeAlt, homeMain], activeLayouts: {} }),
    )
    renderMenu("profile-2")
    expect(await trigger(/^Layout: Home alt/)).toBeInTheDocument()
    expectNoWrites()
  })

  it("says when the profile has no layouts for the device, and offers a new one in it", async () => {
    api.getProfile.mockResolvedValue(profileView({ layouts: [big], activeLayouts: {} }))
    const { user } = renderMenu()
    const menu = await openMenu(user, /^Choose a layout$/)
    expect(
      within(menu).getByRole("menuitem", { name: "No layouts for this device in this profile" }),
    ).toHaveAttribute("aria-disabled", "true")
    expect(menu).not.toHaveTextContent("Big board")
    await user.click(within(menu).getByRole("menuitem", { name: /New layout/ }))
    const dialog = await screen.findByRole("dialog", { name: "New layout for Stream Deck MK.2" })
    await user.type(within(dialog).getByLabelText("Name"), "Fresh")
    await user.click(within(dialog).getByRole("button", { name: "Create" }))
    await waitFor(() => {
      expect(api.createLayout).toHaveBeenCalledWith("profile-1", MK2, "Fresh", true)
    })
    expect(api.assignLayout).not.toHaveBeenCalled()
  })
})

describe("the Device Editor without layout controls", { timeout: 20_000 }, () => {
  it("has no layout name field, layout chooser or layout buttons of its own", async () => {
    renderWithApp(
      <DialogProvider>
        <EditorSelectionContext value={selection()}>
          <LayoutDesignerPage />
        </EditorSelectionContext>
      </DialogProvider>,
      { profiles: profiles([{ id: "profile-1", name: "Work" }]) },
    )
    await screen.findByRole("group", { name: /controls$/ })
    expect(screen.queryByRole("textbox", { name: "Layout name" })).not.toBeInTheDocument()
    expect(screen.queryByRole("button", { name: /^Layout:/ })).not.toBeInTheDocument()
    for (const name of [/New layout/, /Duplicate/, /Use for this model/, /^Delete$/])
      expect(screen.queryByRole("button", { name })).not.toBeInTheDocument()
    // The editing bar keeps the editing commands.
    const bar = screen.getByRole("toolbar", { name: "Layout editing" })
    for (const name of ["Undo", "Redo", "Revert", "Save"])
      expect(within(bar).getByRole("button", { name })).toBeInTheDocument()
  })
})
