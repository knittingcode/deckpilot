// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { api, resetApi } from "@/test/api-mock"
import { installDomStubs } from "@/test/dom"
import { renderWithApp } from "@/test/render"
import { ExportDiagnosticsButton } from "@/settings/diagnostics-export"

const dialog = vi.hoisted(() => ({ save: vi.fn() }))
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: dialog.save, open: vi.fn() }))
vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }))

const preview = {
  token: "3",
  fileName: "deckpilot-diagnostics-20261126-102030.json",
  text: '{\n  "schema": "deckpilot.support-bundle",\n  "logs": "device=elgato.streamdeck.mk2#sha256:0011"\n}\n',
  bytes: 90,
  logs: [
    { file: "agent.2026-11-26.log", lines: 120, truncated: true },
    { file: "updates.log", lines: 2, truncated: false },
  ],
  findings: 1,
}

beforeAll(() => {
  installDomStubs()
})

beforeEach(() => {
  resetApi()
  dialog.save.mockReset()
  api.prepareDiagnostics.mockResolvedValue(preview)
  api.saveDiagnostics.mockResolvedValue({ path: "/tmp/d.json", bytes: 90 })
})

afterEach(() => {
  cleanup()
})

describe("Export diagnostics", () => {
  it("shows everything the file contains before it is saved", async () => {
    renderWithApp(<ExportDiagnosticsButton />)
    await userEvent.click(screen.getByRole("button", { name: "Export diagnostics…" }))
    const content = await screen.findByLabelText("Content of the diagnostics file")
    expect(content).toHaveTextContent("deckpilot.support-bundle")
    expect(content).toHaveTextContent("sha256:0011")
    expect(screen.getByText("agent.2026-11-26.log")).toBeInTheDocument()
    expect(screen.getByText("the last 120 lines", { exact: false })).toBeInTheDocument()
    expect(screen.getByText("2 lines", { exact: false })).toBeInTheDocument()
    expect(screen.getByText("(1 finding)", { exact: false })).toBeInTheDocument()
    expect(screen.getByText(/Nothing is uploaded|uploads nothing/)).toBeInTheDocument()
    // Nothing is written until the user picks a file.
    expect(api.saveDiagnostics).not.toHaveBeenCalled()
    expect(dialog.save).not.toHaveBeenCalled()
  })

  it("saves the prepared bundle to the file the user picks", async () => {
    dialog.save.mockResolvedValue("/home/u/d.json")
    renderWithApp(<ExportDiagnosticsButton />)
    await userEvent.click(screen.getByRole("button", { name: "Export diagnostics…" }))
    await screen.findByLabelText("Content of the diagnostics file")
    await userEvent.click(screen.getByRole("button", { name: "Save…" }))
    await waitFor(() => {
      expect(api.saveDiagnostics).toHaveBeenCalledWith("3", "/home/u/d.json")
    })
    expect(dialog.save).toHaveBeenCalledWith(
      expect.objectContaining({ defaultPath: preview.fileName }),
    )
    await waitFor(() => {
      expect(screen.queryByLabelText("Content of the diagnostics file")).not.toBeInTheDocument()
    })
  })

  it("writes nothing when the save dialog is cancelled", async () => {
    dialog.save.mockResolvedValue(null)
    renderWithApp(<ExportDiagnosticsButton />)
    await userEvent.click(screen.getByRole("button", { name: "Export diagnostics…" }))
    await screen.findByLabelText("Content of the diagnostics file")
    await userEvent.click(screen.getByRole("button", { name: "Save…" }))
    await waitFor(() => {
      expect(dialog.save).toHaveBeenCalled()
    })
    expect(api.saveDiagnostics).not.toHaveBeenCalled()
    expect(screen.getByLabelText("Content of the diagnostics file")).toBeInTheDocument()
  })

  it("cancel discards the prepared bundle", async () => {
    renderWithApp(<ExportDiagnosticsButton />)
    await userEvent.click(screen.getByRole("button", { name: "Export diagnostics…" }))
    await screen.findByLabelText("Content of the diagnostics file")
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }))
    expect(api.discardDiagnostics).toHaveBeenCalled()
    expect(api.saveDiagnostics).not.toHaveBeenCalled()
  })

  it("says when the bundle cannot be prepared", async () => {
    api.prepareDiagnostics.mockRejectedValue(
      new Error("the DeckPilot agent executable was not found"),
    )
    renderWithApp(<ExportDiagnosticsButton />)
    await userEvent.click(screen.getByRole("button", { name: "Export diagnostics…" }))
    expect(
      await screen.findByText("The diagnostics could not be prepared", { exact: false }),
    ).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Save…" })).toBeDisabled()
  })
})
