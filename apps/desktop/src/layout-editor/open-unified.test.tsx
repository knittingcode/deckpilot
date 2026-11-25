// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

// One Open for files and folders (docs/system-actions.md, "Open folder and
// Open" and "The key's icon"): the library offers Open and no longer Open
// folder, saved Open folder actions keep working, the Open field chooses a
// file or a folder with the native dialogs, and the drawn key shows the
// icon the core computed for what it opens.

import { cleanup, render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { FaceTile } from "@/components/key-face"
import { KeyPreview } from "@/components/key-preview"
import { MachineResourceField } from "@/components/machine-resource-field"
import { i18n } from "@/i18n"
import { creatableTypes } from "@/lib/modules"
import {
  OPEN,
  OPEN_FOLDER,
  actionTypeName,
  machinePick,
  newActionIcon,
  shownActionIcon,
} from "@/lib/system-actions"
import { moduleInfo } from "@/lib/test-fixtures"
import type { ActionInstance, ActionType, KeyFace, MachineResourceBinding } from "@/lib/types"
import { eventAffects } from "@/lib/variables"
import { installDomStubs } from "@/test/dom"
import { moduleGroups } from "@/workflow-builder/catalog"

import { catalogGroups, newAction } from "./catalog"

const api = vi.hoisted(() => ({
  getMachineConfig: vi.fn(),
  setResourceBinding: vi.fn(),
  resourceStatus: vi.fn(),
}))
const dialog = vi.hoisted(() => ({ open: vi.fn() }))

vi.mock("@/lib/api", () => ({ api }))
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: dialog.open }))

beforeAll(() => {
  installDomStubs()
})

let bindings: MachineResourceBinding[] = []

beforeEach(async () => {
  await i18n.changeLanguage("en")
  bindings = []
  api.getMachineConfig.mockImplementation(() => Promise.resolve({ os: "macOs", bindings }))
  api.resourceStatus.mockResolvedValue("ok")
  api.setResourceBinding.mockImplementation((key: string, kind: string, value: string) => {
    bindings = [...bindings.filter((b) => b.key !== key), { key, kind, value } as never]
    return Promise.resolve({ os: "macOs", bindings })
  })
})

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

const location = {
  key: "location",
  label: "Location",
  kind: { type: "resource" as const },
  required: true,
}
const path = {
  key: "path",
  label: "File",
  kind: { type: "text" as const, multiline: false, maxLength: 1024 },
  required: false,
}
const open: ActionType = {
  id: OPEN,
  name: "Open",
  description: "Opens a file, folder or application.",
  category: "System",
  requiresProject: false,
  parameters: [location, path],
  node: { icon: "file" } as ActionType["node"],
  contexts: ["key", "gesture", "sequence", "workflow"],
}
// As the agent reports it now: deprecated, its saved actions kept.
const openFolder: ActionType = {
  ...open,
  id: OPEN_FOLDER,
  name: "Open folder",
  parameters: [
    { ...location, label: "Folder" },
    { ...path, label: "Subfolder" },
  ],
  node: { icon: "folder-open" } as ActionType["node"],
  deprecated: true,
}
const system = moduleInfo({ id: "system", name: "System", actionTypes: [open, openFolder] })

const savedFolder: ActionInstance = {
  id: "reports",
  name: "Reports",
  actionType: OPEN_FOLDER,
  parameters: {
    location: { type: "resource", value: { type: "fixed", key: "folder.reports-abc123" } },
  },
  description: "",
  icon: "folder-open",
  label: null,
}

describe("the Action Library", () => {
  it("offers one Open for files and folders, and no Open folder", () => {
    const entries = catalogGroups([system]).flatMap((g) => g.entries.map((e) => e.type.id))
    expect(entries).toEqual([OPEN])
    expect(actionTypeName(open)).toBe("Open")
    // Nor in the action editor's types or the workflow node types.
    expect(creatableTypes(system.actionTypes).map((t) => t.id)).toEqual([OPEN])
    const [group] = moduleGroups([system], [savedFolder])
    expect(group?.types.map((t) => t.id)).toEqual([OPEN])
  })

  it("keeps saved Open folder actions, as workflow steps too, with their folder picker", () => {
    // A saved action stays listed (and placeable) in the workflow catalog.
    const [group] = moduleGroups([system], [savedFolder])
    expect(group?.entries.map((e) => e.key)).toEqual(["action:reports"])
    expect(group?.entries[0]?.icon).toBe("folder-open")
    // Its fields keep the folders-only picker and its localized name.
    expect(machinePick(OPEN_FOLDER, "location")).toBe("folder")
    expect(actionTypeName(openFolder)).toBe("Open folder")
  })

  it("makes new Open actions without an icon, never an automatic one", () => {
    const made = newAction([], open)
    expect(made.actionType).toBe(OPEN)
    // The keys show what they open; an icon set later is the user's.
    expect(made.icon).toBeNull()
    expect(newActionIcon(open)).toBeNull()
    // Other types still start with their own icon.
    expect(newActionIcon(openFolder)).toBe("folder-open")
    // Lists show Open's document icon for an action without one, and an
    // icon chosen for it as it is.
    expect(shownActionIcon({ actionType: OPEN, icon: null })).toBe("file")
    expect(shownActionIcon({ actionType: OPEN, icon: "rocket" })).toBe("rocket")
    expect(shownActionIcon({ actionType: "general.log", icon: null })).toBeNull()
  })
})

function field(onChange: (value: unknown) => void, value: { type: "fixed"; key: string } | null) {
  return render(
    <MachineResourceField
      id="f"
      pick={machinePick(OPEN, "location") ?? "location"}
      value={value}
      optional={false}
      onChange={onChange}
      advanced={<p>resource key control</p>}
    />,
  )
}

describe("the Open field", () => {
  it("chooses a file or a folder with the native dialogs, without showing keys", async () => {
    const onChange = vi.fn()
    field(onChange, null)
    const user = userEvent.setup()
    await waitFor(() => {
      expect(api.getMachineConfig).toHaveBeenCalled()
    })
    const file = screen.getByRole("button", { name: "Choose file…" })
    const folder = screen.getByRole("button", { name: "Choose folder…" })
    // Both say what each does.
    expect(file).toHaveAccessibleDescription(/a folder to open it in the file manager/)
    expect(folder).toHaveAccessibleDescription(/a folder to open it in the file manager/)
    expect(screen.queryByText("resource key control")).not.toBeInTheDocument()
    expect(screen.getByText("Nothing chosen")).toBeInTheDocument()

    // A file: the file dialog, bound as a file.
    dialog.open.mockResolvedValueOnce("/Users/me/Reports/Q1 report.pdf")
    await user.click(file)
    await waitFor(() => {
      expect(onChange).toHaveBeenLastCalledWith({
        type: "fixed",
        key: expect.stringMatching(/^file\.q1-report-pdf-[a-z0-9]{6}$/) as unknown,
      })
    })
    expect(dialog.open).toHaveBeenLastCalledWith({ directory: false, multiple: false })
    expect(api.setResourceBinding).toHaveBeenLastCalledWith(
      expect.stringMatching(/^file\./),
      "other",
      "/Users/me/Reports/Q1 report.pdf",
    )

    // Then a folder: the folder dialog, bound as a folder.
    dialog.open.mockResolvedValueOnce("/Users/me/Reports")
    await user.click(folder)
    await waitFor(() => {
      expect(onChange).toHaveBeenLastCalledWith({
        type: "fixed",
        key: expect.stringMatching(/^folder\.reports-[a-z0-9]{6}$/) as unknown,
      })
    })
    expect(dialog.open).toHaveBeenLastCalledWith({ directory: true, multiple: false })
    expect(api.setResourceBinding).toHaveBeenLastCalledWith(
      expect.stringMatching(/^folder\./),
      "directory",
      "/Users/me/Reports",
    )

    // And a file again.
    dialog.open.mockResolvedValueOnce("/Users/me/notes.txt")
    await user.click(file)
    await waitFor(() => {
      expect(onChange).toHaveBeenLastCalledWith({
        type: "fixed",
        key: expect.stringMatching(/^file\.notes-txt-[a-z0-9]{6}$/) as unknown,
      })
    })
  })

  it("leaves the choice as it was when a dialog is cancelled", async () => {
    const onChange = vi.fn()
    field(onChange, null)
    const user = userEvent.setup()
    dialog.open.mockResolvedValueOnce(null)
    await user.click(await screen.findByRole("button", { name: "Choose folder…" }))
    await waitFor(() => {
      expect(dialog.open).toHaveBeenCalled()
    })
    expect(onChange).not.toHaveBeenCalled()
    expect(api.setResourceBinding).not.toHaveBeenCalled()
  })

  it("shows a chosen folder or file by name and path, and can be cleared", async () => {
    bindings = [{ key: "folder.reports-abc123", kind: "directory", value: "/Users/me/Reports" }]
    const onChange = vi.fn()
    render(
      <MachineResourceField
        id="f"
        pick="location"
        value={{ type: "fixed", key: "folder.reports-abc123" }}
        optional
        onChange={onChange}
      />,
    )
    expect(await screen.findByText("/Users/me/Reports")).toBeInTheDocument()
    expect(screen.getByText("Reports")).toBeInTheDocument()
    // The key is the computer's, never shown.
    expect(screen.queryByText(/folder\.reports-abc123/)).not.toBeInTheDocument()
    await userEvent.setup().click(screen.getByRole("button", { name: "Clear" }))
    expect(onChange).toHaveBeenCalledWith(null)
  })

  it("keeps advanced resource keys and project roles for saved ones", () => {
    render(
      <MachineResourceField
        id="f"
        pick="location"
        value={{ type: "projectRole", role: "repository" }}
        optional={false}
        onChange={vi.fn()}
        advanced={<p>resource key control</p>}
      />,
    )
    expect(screen.getByText("resource key control")).toBeInTheDocument()
  })

  it("chooses folders only for a saved Open folder", async () => {
    render(
      <MachineResourceField
        id="f"
        pick={machinePick(OPEN_FOLDER, "location") ?? "location"}
        value={null}
        optional={false}
        onChange={vi.fn()}
      />,
    )
    expect(await screen.findByRole("button", { name: "Choose folder…" })).toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "Choose file…" })).not.toBeInTheDocument()
  })
})

const face = (symbol: string | undefined): KeyFace => ({
  style: "action",
  title: "Open",
  ...(symbol ? { symbol } : {}),
})

function drawn(container: HTMLElement): string[] {
  return Array.from(container.querySelectorAll("svg")).map((svg) =>
    Array.from(svg.classList)
      .filter((c) => c.startsWith("lucide-"))
      .join(" "),
  )
}

describe("the drawn key", () => {
  it("shows the icon the core computed for an Open key's target", () => {
    // The action's stored icon is Open's own; the face says what it opens.
    const folder = render(<FaceTile face={face("folder-open")} icon="file" />)
    expect(drawn(folder.container)).toEqual(["lucide-folder-open"])
    folder.unmount()
    const file = render(<FaceTile face={face("file")} icon="file" />)
    expect(drawn(file.container)).toEqual(["lucide-file"])
    file.unmount()
    // A face without a symbol (an older agent): the action's icon.
    const older = render(<FaceTile face={face(undefined)} icon="file" />)
    expect(drawn(older.container)).toEqual(["lucide-file"])
  })

  it("is the renderer's image once it arrives (the Canvas and the Inspector alike)", () => {
    const { container } = render(
      <KeyPreview face={face("folder-open")} image="data:image/png;base64,AAAA" icon="file" />,
    )
    expect(container.querySelector("[data-slot=key-image]")).toHaveAttribute(
      "src",
      "data:image/png;base64,AAAA",
    )
    expect(container.querySelector("svg")).toBeNull()
  })

  it("is drawn again when a binding, an action or the profile changes, without polling", () => {
    // The previews ask again on these agent events (use-display-refresh).
    expect(eventAffects({ type: "resourceBindingsChanged" }, { profile: "p" })).toBe(true)
    expect(eventAffects({ type: "profileChanged", profile: "p" }, { profile: "p" })).toBe(true)
  })
})
