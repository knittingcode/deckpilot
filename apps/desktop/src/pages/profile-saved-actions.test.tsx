// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { collator } from "@/i18n/format"
import type { DeviceLayout } from "@/lib/types"
import { savedActionRows } from "@/pages/saved-action-rows"
import { ProfilesSettings } from "@/pages/profiles"
import { EditorSelectionContext, type EditorSelection } from "@/shell/editor-selection-context"
import { api, resetApi } from "@/test/api-mock"
import { installDomStubs } from "@/test/dom"
import { action, builtInModules, profileView, profiles, stepWorkflow } from "@/test/fixtures"
import { renderWithApp } from "@/test/render"
import { resetToasts, toast } from "@/test/toast"

vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)
vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)

const desk: DeviceLayout = {
  id: "desk",
  name: "Desk",
  model: "elgato.streamdeck.mk2",
  homePage: "home",
  pages: [
    {
      id: "home",
      name: "Home",
      bindings: [
        {
          control: { kind: "key", index: 0 },
          trigger: { type: "press" },
          target: { type: "action", action: "on-key" },
          label: null,
        },
        {
          control: { kind: "key", index: 1 },
          trigger: { type: "press" },
          target: { type: "action", action: "on-key" },
          label: null,
        },
      ],
    },
  ],
}

const VIEW = profileView({
  id: "profile-1",
  name: "Work",
  layouts: [desk],
  actions: [
    action("brightness", "Brightness", "general.log"),
    action("on-key", "Open docs", "system.open-url"),
    action("in-workflow", "Announce", "general.log"),
    action("url-10", "Open URL 10", "system.open-url"),
    action("url-2", "Open URL 2", "system.open-url"),
  ],
  workflows: [stepWorkflow("wf", "Morning", ["in-workflow"])],
})

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
  api.getProfile.mockResolvedValue(VIEW)
  api.deleteAction.mockResolvedValue(null)
})
afterEach(cleanup)

async function openSavedActions() {
  const view = renderWithApp(
    <EditorSelectionContext value={selection()}>
      <ProfilesSettings />
    </EditorSelectionContext>,
    {
      profiles: profiles([{ id: "profile-1", name: "Work" }], "profile-1"),
      modules: builtInModules(),
    },
  )
  await view.user.click(await screen.findByRole("button", { name: "Open Work" }))
  const table = await screen.findByRole("table", { name: "Saved actions" })
  return { ...view, table }
}

const names = (table: HTMLElement) =>
  within(table)
    .getAllByRole("row")
    .slice(1)
    .map((row) => within(row).getAllByRole("cell")[0]?.textContent)

const rowOf = (table: HTMLElement, name: string) => {
  const row = within(table).getByText(name).closest("tr")
  if (!row) throw new Error(`no row for ${name}`)
  return within(row)
}

describe("Settings › Profiles › a profile › Saved actions", () => {
  it("lists every saved action by name with where it is used, writing nothing", async () => {
    const { table } = await openSavedActions()
    expect(names(table)).toEqual([
      "Announce",
      "Brightness",
      "Open docs",
      "Open URL 2",
      "Open URL 10",
    ])
    expect(rowOf(table, "Brightness").getByText("Not used")).toBeInTheDocument()
    expect(rowOf(table, "Open docs").getByText("2 places")).toBeInTheDocument()
    expect(rowOf(table, "Announce").getByText("1 place")).toBeInTheDocument()
    expect(rowOf(table, "Open docs").getByText("Open URL")).toBeInTheDocument()
    expect(api.deleteAction).not.toHaveBeenCalled()
    expect(api.updateAction).not.toHaveBeenCalled()
  })

  it("shows only the unused ones on request", async () => {
    const { user } = await openSavedActions()
    const filter = screen.getByRole("radiogroup", { name: "Show saved actions" })
    expect(within(filter).getByRole("radio", { name: /^Unused\s*3$/ })).toBeInTheDocument()
    await user.click(within(filter).getByRole("radio", { name: /^Unused\s*3$/ }))
    const table = screen.getByRole("table", { name: "Saved actions" })
    expect(names(table)).toEqual(["Brightness", "Open URL 2", "Open URL 10"])
    await user.click(within(filter).getByRole("radio", { name: /^All\s*5$/ }))
    expect(names(screen.getByRole("table", { name: "Saved actions" }))).toHaveLength(5)
  })

  it("offers Delete only for an unused action, and deletes it only after the confirmation", async () => {
    const { user, table, context } = await openSavedActions()
    expect(
      rowOf(table, "Open docs").queryByRole("button", { name: "Delete Open docs" }),
    ).not.toBeInTheDocument()
    expect(
      rowOf(table, "Announce").queryByRole("button", { name: "Delete Announce" }),
    ).not.toBeInTheDocument()

    await user.click(rowOf(table, "Brightness").getByRole("button", { name: "Delete Brightness" }))
    let dialog = await screen.findByRole("dialog")
    expect(within(dialog).getByText(/Brightness/)).toBeInTheDocument()
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }))
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
    })
    expect(api.deleteAction).not.toHaveBeenCalled()

    await user.click(rowOf(table, "Brightness").getByRole("button", { name: "Delete Brightness" }))
    dialog = await screen.findByRole("dialog")
    await user.click(within(dialog).getByRole("button", { name: "Delete" }))
    await waitFor(() => {
      expect(api.deleteAction).toHaveBeenCalledTimes(1)
    })
    expect(api.deleteAction).toHaveBeenCalledWith("profile-1", "brightness")
    // Nothing else is changed: no other action, no reference.
    expect(api.replaceActionReferences).not.toHaveBeenCalled()
    expect(api.unbindActionReferences).not.toHaveBeenCalled()
    expect(context.refresh).toHaveBeenCalled()
    expect(toast.success).toHaveBeenCalled()
  })

  it("keeps the action and says why when the agent refuses to delete it", async () => {
    api.deleteAction.mockRejectedValue(new Error("the action is in use"))
    const { user, table } = await openSavedActions()
    await user.click(rowOf(table, "Brightness").getByRole("button", { name: "Delete Brightness" }))
    const dialog = await screen.findByRole("dialog")
    await user.click(within(dialog).getByRole("button", { name: "Delete" }))
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      /The action was not deleted: The action is in use\./,
    )
    expect(screen.getByRole("dialog")).toBeInTheDocument()
    expect(toast.success).not.toHaveBeenCalled()
  })

  it("edits an action, used or not, in the action editor", async () => {
    const { user, table } = await openSavedActions()
    await user.click(rowOf(table, "Open docs").getByRole("button", { name: "Edit Open docs" }))
    const dialog = await screen.findByRole("dialog", { name: "Open docs" })
    // The action editor: its name, and where the action is used.
    expect(within(dialog).getByDisplayValue("Open docs")).toBeInTheDocument()
    expect(within(dialog).getByText("Used in")).toBeInTheDocument()
    await user.keyboard("{Escape}")
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
    })
    // Escape closed the editor only: the profile's details stay.
    expect(screen.getByRole("table", { name: "Saved actions" })).toBeInTheDocument()
  })

  it("says when a profile has none", async () => {
    api.getProfile.mockResolvedValue(profileView({ id: "profile-1", name: "Work" }))
    renderWithApp(
      <EditorSelectionContext value={selection()}>
        <ProfilesSettings />
      </EditorSelectionContext>,
      { profiles: profiles([{ id: "profile-1", name: "Work" }], "profile-1") },
    )
    await (
      await import("@testing-library/user-event")
    ).default
      .setup()
      .click(await screen.findByRole("button", { name: "Open Work" }))
    expect(await screen.findByText("This profile has no saved actions.")).toBeInTheDocument()
    expect(screen.queryByRole("table", { name: "Saved actions" })).not.toBeInTheDocument()
  })
})

describe("savedActionRows", () => {
  it("sorts by the language's alphabet, numbers by value", () => {
    const view = profileView({
      actions: [
        action("z", "Zoom", "general.log"),
        action("c", "Ćwiczenia", "general.log"),
        action("b", "cel", "general.log"),
        action("a", "Akcja 10", "general.log"),
        action("d", "Akcja 9", "general.log"),
      ],
    })
    const pl = collator("pl")
    expect(
      savedActionRows(view, "all", (a, b) => pl.compare(a, b)).map((r) => r.action.name),
    ).toEqual(["Akcja 9", "Akcja 10", "cel", "Ćwiczenia", "Zoom"])
  })
})
