// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { act, cleanup, renderHook, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { setLayoutProfileFilter, useLayoutProfileFilter } from "@/lib/layout-filter"
import type { DeviceLayout } from "@/lib/types"
import { ProfilesSettings } from "@/pages/profiles"
import type { EditorSelection } from "@/shell/editor-selection-context"
import { EditorSelectionContext } from "@/shell/editor-selection-context"
import { api, resetApi } from "@/test/api-mock"
import { installDomStubs } from "@/test/dom"
import { agentConnection, agentStatus, deviceInfo, profileView, profiles } from "@/test/fixtures"
import { renderWithApp } from "@/test/render"
import { resetToasts } from "@/test/toast"

vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)
vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)

const LONG =
  "A profile for the client projects of the agency, with a shared project switcher and every tool"
const PROFILES = profiles(
  [
    { id: "profile-1", name: "Work" },
    { id: "profile-2", name: "Home" },
  ],
  "profile-1",
)
const layout = (id: string): DeviceLayout => ({
  id,
  name: id,
  model: "elgato.streamdeck.mk2",
  homePage: `${id}-home`,
  pages: [],
})
const VIEWS = {
  "profile-1": profileView({
    id: "profile-1",
    name: "Work",
    description: LONG,
    layouts: [layout("desk"), layout("travel")],
  }),
  "profile-2": profileView({ id: "profile-2", name: "Home", description: "" }),
}

function selection(): EditorSelection {
  return {
    models: { status: "ready", data: [] },
    modelId: null,
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
  }
}

beforeAll(installDomStubs)
beforeEach(() => {
  resetApi()
  resetToasts()
  setLayoutProfileFilter(null)
  api.getProfile.mockImplementation((id: string) =>
    Promise.resolve(VIEWS[id as keyof typeof VIEWS]),
  )
  api.setActiveProfile.mockResolvedValue(null)
  api.updateProfileDetails.mockResolvedValue(null)
  api.deleteProfile.mockResolvedValue(null)
})
afterEach(cleanup)

function renderPage() {
  const editor = selection()
  const view = renderWithApp(
    // The page's scrolling content, as in the settings.
    <div data-testid="scroller" style={{ overflowY: "auto" }}>
      <EditorSelectionContext value={editor}>
        <ProfilesSettings />
      </EditorSelectionContext>
    </div>,
    {
      profiles: PROFILES,
      agent: agentConnection({
        status: agentStatus({ devices: [deviceInfo({ modelName: "Stream Deck MK.2" })] }),
      }),
    },
  )
  return { ...view, editor }
}

const rowOf = (name: string) => {
  const button = screen.getByRole("button", { name: `Open ${name}` })
  const row = button.closest("tr")
  if (!row) throw new Error(`no row for ${name}`)
  return row
}

/** Nothing about profiles, layouts or devices was written. */
function expectNoWrites(context: { setEditingProfileId: unknown }, editor: EditorSelection) {
  expect(api.setActiveProfile).not.toHaveBeenCalled()
  expect(api.assignLayout).not.toHaveBeenCalled()
  expect(api.unassignLayout).not.toHaveBeenCalled()
  expect(api.setDefaultLayout).not.toHaveBeenCalled()
  expect(api.activateLayout).not.toHaveBeenCalled()
  expect(api.reorderProfileLayouts).not.toHaveBeenCalled()
  expect(context.setEditingProfileId).not.toHaveBeenCalled()
  expect(editor.selectProfile).not.toHaveBeenCalled()
}

describe("Settings › Profiles: the overview", () => {
  it("is one table: name, description, contents, status and a menu", async () => {
    renderPage()
    const table = await screen.findByRole("table", { name: "Profiles" })
    expect(
      within(table)
        .getAllByRole("columnheader")
        .map((h) => h.textContent),
    ).toEqual(["Name", "Description", "Contents", "Status", "Actions"])
    expect(screen.queryByText("All profiles")).not.toBeInTheDocument()
    // No layouts section and no details below the table.
    expect(screen.queryByRole("region", { name: "Layouts" })).not.toBeInTheDocument()
    const work = rowOf("Work")
    await waitFor(() => {
      expect(work).toHaveTextContent("2 layouts · 0 actions")
    })
    // A long description is cut, with the whole of it on hover.
    const description = within(work).getByText(LONG)
    expect(description).toHaveClass("truncate")
    expect(description).toHaveAttribute("title", LONG)
    expect(within(work).getByRole("button", { name: "Open Work" })).toHaveAttribute("title", "Work")
    // The active profile, and only it.
    expect(within(work).getByText("Active")).toBeInTheDocument()
    expect(within(rowOf("Home")).queryByText("Active")).not.toBeInTheDocument()
    expect(rowOf("Home")).toHaveTextContent("0 layouts · 0 actions")
  })
})

describe("Settings › Profiles: a profile's details", () => {
  it("opens from its row and goes back to the table where it was", async () => {
    const { user } = renderPage()
    await screen.findByRole("table")
    const scroller = screen.getByTestId("scroller")
    scroller.scrollTop = 120
    await user.click(rowOf("Home"))
    const details = await screen.findByRole("region", { name: /Home/ })
    expect(screen.queryByRole("table")).not.toBeInTheDocument()
    expect(within(details).getByRole("button", { name: "All profiles" })).toHaveFocus()
    scroller.scrollTop = 0
    await user.click(within(details).getByRole("button", { name: "All profiles" }))
    await screen.findByRole("table")
    expect(scroller.scrollTop).toBe(120)
    expect(rowOf("Home")).toHaveAttribute("data-state", "selected")
    expect(screen.getByRole("button", { name: "Open Home" })).toHaveFocus()
  })

  it("is opened with the keyboard and closed with Escape", async () => {
    const { user } = renderPage()
    await screen.findByRole("table")
    screen.getByRole("button", { name: "Open Work" }).focus()
    await user.keyboard("{Enter}")
    expect(await screen.findByRole("region", { name: /Work/ })).toBeInTheDocument()
    await user.keyboard("{Escape}")
    expect(await screen.findByRole("table")).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Open Work" })).toHaveFocus()
  })

  it("shows what the profile is, holds and where it is active, writing nothing", async () => {
    const { user, context, editor } = renderPage()
    await screen.findByRole("table")
    await user.click(screen.getByRole("button", { name: "Open Work" }))
    const details = await screen.findByRole("region", { name: /Work/ })
    expect(details).toHaveTextContent(LONG)
    expect(details).toHaveTextContent("Active on Stream Deck MK.2")
    expect(details).toHaveTextContent("2 layouts")
    // Shared references, never copies of its own.
    expect(details).toHaveTextContent("Layouts are shared: this profile refers to them")
    expectNoWrites(context, editor)
    expect(context.navigate).not.toHaveBeenCalled()
  })

  it("names the active profile for one that is not, and an empty profile has no layouts", async () => {
    const { user, context, editor } = renderPage()
    await screen.findByRole("table")
    await user.click(screen.getByRole("button", { name: "Open Home" }))
    const details = await screen.findByRole("region", { name: /Home/ })
    expect(details).toHaveTextContent("Not active; devices show Work")
    expect(details).toHaveTextContent("No layouts yet")
    expect(details).toHaveTextContent("No description")
    expectNoWrites(context, editor)
  })

  it("activates a profile only through Make active", async () => {
    const { user, context } = renderPage()
    await screen.findByRole("table")
    await user.click(screen.getByRole("button", { name: "Open Home" }))
    const details = await screen.findByRole("region", { name: /Home/ })
    expect(api.setActiveProfile).not.toHaveBeenCalled()
    await user.click(within(details).getByRole("button", { name: "Make Home active" }))
    await waitFor(() => {
      expect(api.setActiveProfile).toHaveBeenCalledWith("profile-2")
    })
    expect(context.refresh).toHaveBeenCalled()
  })

  it("edits the name and description", async () => {
    const { user } = renderPage()
    await screen.findByRole("table")
    await user.click(screen.getByRole("button", { name: "Open Home" }))
    const details = await screen.findByRole("region", { name: /Home/ })
    await user.click(within(details).getByRole("button", { name: "Edit details…" }))
    const dialog = await screen.findByRole("dialog", { name: "Edit profile" })
    await user.type(within(dialog).getByLabelText("Description"), "Evenings")
    await user.click(within(dialog).getByRole("button", { name: "Save" }))
    await waitFor(() => {
      expect(api.updateProfileDetails).toHaveBeenCalledWith("profile-2", "Home", "Evenings")
    })
  })

  it("deletes only after the confirmation, which says what happens to an active profile", async () => {
    const { user } = renderPage()
    await screen.findByRole("table")
    await user.click(screen.getByRole("button", { name: "Open Work" }))
    const details = await screen.findByRole("region", { name: /Work/ })
    await user.click(within(details).getByRole("button", { name: "More for Work" }))
    await user.click(await screen.findByRole("menuitem", { name: /Delete/ }))
    const confirm = await screen.findByRole("alertdialog")
    expect(confirm).toHaveTextContent("This profile is active on the device")
    expect(confirm).toHaveTextContent("Its layouts are kept")
    expect(api.deleteProfile).not.toHaveBeenCalled()
    await user.click(within(confirm).getByRole("button", { name: "Delete" }))
    await waitFor(() => {
      expect(api.deleteProfile).toHaveBeenCalledWith("profile-1")
    })
    // Back to the table: the profile is gone.
    expect(await screen.findByRole("table")).toBeInTheDocument()
  })

  it("opens the Layout Manager filtered to the profile", async () => {
    const { user, context } = renderPage()
    await screen.findByRole("table")
    await user.click(screen.getByRole("button", { name: "Open Home" }))
    const details = await screen.findByRole("region", { name: /Home/ })
    await user.click(within(details).getByRole("button", { name: "Manage layouts…" }))
    expect(context.navigate).toHaveBeenCalledWith("layouts")
    const { result } = renderHook(() => useLayoutProfileFilter())
    expect(result.current).toBe("profile-2")
    act(() => {
      setLayoutProfileFilter(null)
    })
    expect(result.current).toBeNull()
  })
})

describe("Settings › Profiles: the row menu", () => {
  it("has the profile's commands, used with the keyboard", async () => {
    const { user } = renderPage()
    await screen.findByRole("table")
    screen.getByRole("button", { name: "More for Home" }).focus()
    await user.keyboard("{Enter}")
    const menu = await screen.findByRole("menu")
    expect(
      within(menu)
        .getAllByRole("menuitem")
        .map((i) => i.textContent.trim()),
    ).toEqual(["Open", "Rename…", "Make active", "Manage layouts…", "Duplicate…", "Delete…"])
    // The active profile is not offered to be made active again.
    await user.keyboard("{Escape}")
    await user.click(screen.getByRole("button", { name: "More for Work" }))
    expect(
      within(await screen.findByRole("menu")).queryByRole("menuitem", { name: /Make active/ }),
    ).not.toBeInTheDocument()
  })

  it("says that duplicating copies the layouts and deleting keeps them", async () => {
    const { user } = renderPage()
    await screen.findByRole("table")
    await user.click(screen.getByRole("button", { name: "More for Home" }))
    await user.click(await screen.findByRole("menuitem", { name: /Duplicate/ }))
    expect(await screen.findByRole("dialog")).toHaveTextContent("independent copies of its layouts")
    await user.keyboard("{Escape}")
    await user.click(screen.getByRole("button", { name: "More for Home" }))
    await user.click(await screen.findByRole("menuitem", { name: /Delete/ }))
    const confirm = await screen.findByRole("alertdialog")
    expect(confirm).toHaveTextContent("Its layouts are kept: they stay in Layouts")
  })

  it("opening the menu or a row changes no device state", async () => {
    const { user, context, editor } = renderPage()
    await screen.findByRole("table")
    await user.click(screen.getByRole("button", { name: "More for Home" }))
    await user.keyboard("{Escape}")
    await user.click(rowOf("Home"))
    await screen.findByRole("region", { name: /Home/ })
    expectNoWrites(context, editor)
  })
})
