// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { act, cleanup, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { agentConnection, agentStatus, deviceInfo } from "@/test/fixtures"
import { api, resetApi } from "@/test/api-mock"
import { installDomStubs } from "@/test/dom"
import { renderWithApp } from "@/test/render"
import { emitBackend, resetBackend } from "@/test/tauri"
import { CALLS_EVENT } from "@/lib/calls"
import type { CallsStatus } from "@/lib/types"
import { CallsSettings } from "@/settings/calls-settings"

vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)
vi.mock("@tauri-apps/api/event", async () => (await import("@/test/tauri")).eventModule)
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }))

const device = deviceInfo()

function status(overrides: Partial<CallsStatus> = {}): CallsStatus {
  return {
    installed: true,
    state: "active",
    provider: "os-signals",
    controls: false,
    devices: [{ device: device.id, overlay: true, shown: true, suspended: false }],
    layouts: [
      {
        model: device.id.model,
        layout: "0b6f3f3e-6c51-4d0e-9a55-2f1f0c1d9a01",
        name: "Calls",
        edited: true,
        templateVersion: "1.0.0",
      },
    ],
    ...overrides,
  }
}

function agent(capabilities: string[] = ["calls"]) {
  return agentConnection({
    status: agentStatus({ devices: [device] }),
    capabilities: capabilities as never,
  })
}

beforeAll(() => {
  installDomStubs()
})

beforeEach(() => {
  resetApi()
  resetBackend()
})

afterEach(() => {
  cleanup()
})

describe("Settings › Devices › Calls", () => {
  it("is not shown without the agent's calls capability or when Calls is not installed", async () => {
    renderWithApp(<CallsSettings />, { agent: agent([]) })
    expect(api.getCallsStatus).not.toHaveBeenCalled()
    cleanup()
    api.getCallsStatus.mockResolvedValue(status({ installed: false }))
    renderWithApp(<CallsSettings />, { agent: agent() })
    await waitFor(() => {
      expect(api.getCallsStatus).toHaveBeenCalled()
    })
    expect(screen.queryByText("Calls")).not.toBeInTheDocument()
  })

  it("says plainly that Teams is detection only", async () => {
    api.getCallsStatus.mockResolvedValue(status())
    renderWithApp(<CallsSettings />, { agent: agent() })
    expect(await screen.findByText("In a call")).toBeInTheDocument()
    expect(screen.getByText("Not supported")).toBeInTheDocument()
    expect(screen.getByText(/Microsoft Teams can only be detected/)).toBeInTheDocument()
    expect(screen.getByText("Shows the call now.")).toBeInTheDocument()
  })

  it("names Teams only for the Teams adapter", async () => {
    api.getCallsStatus.mockResolvedValue(status({ provider: "mock" }))
    renderWithApp(<CallsSettings />, { agent: agent() })
    expect(await screen.findByText("In a call")).toBeInTheDocument()
    expect(screen.getByText(/This call app can only be detected/)).toBeInTheDocument()
    expect(screen.queryByText(/Microsoft Teams/)).not.toBeInTheDocument()
  })

  it("fetches Calls again from a restarted agent", async () => {
    api.getCallsStatus.mockResolvedValue(status({ state: "idle" }))
    const first = agent()
    const view = renderWithApp(<CallsSettings />, { agent: first })
    expect(await screen.findByText("No call")).toBeInTheDocument()
    expect(api.getCallsStatus).toHaveBeenCalledTimes(1)
    api.getCallsStatus.mockResolvedValue(status({ state: "active" }))
    const restarted = agentConnection({
      ...first,
      status: agentStatus({ devices: [device], pid: 4321, startedAt: 99 }),
    })
    view.rerender(<CallsSettings />, { ...view.context, agent: restarted })
    expect(await screen.findByText("In a call")).toBeInTheDocument()
    expect(api.getCallsStatus).toHaveBeenCalledTimes(2)
  })

  it("follows the agent's callsChanged events", async () => {
    api.getCallsStatus.mockResolvedValue(status({ state: "idle" }))
    renderWithApp(<CallsSettings />, { agent: agent() })
    expect(await screen.findByText("No call")).toBeInTheDocument()
    act(() => {
      emitBackend(CALLS_EVENT, status({ state: "incoming" }))
    })
    expect(screen.getByText("Incoming call")).toBeInTheDocument()
  })

  it("turns a device's overlay off", async () => {
    api.getCallsStatus.mockResolvedValue(status())
    api.setCallsOverlay.mockResolvedValue(
      status({ devices: [{ device: device.id, overlay: false, shown: false, suspended: false }] }),
    )
    renderWithApp(<CallsSettings />, { agent: agent() })
    const toggle = await screen.findByRole("switch", { name: "Show calls on this device" })
    expect(toggle).toBeChecked()
    await userEvent.click(toggle)
    expect(api.setCallsOverlay).toHaveBeenCalledWith(device.id, false)
    await waitFor(() => {
      expect(toggle).not.toBeChecked()
    })
  })

  it("restores defaults only after saying which copy keeps the user's version", async () => {
    api.getCallsStatus.mockResolvedValue(status())
    api.restoreCallsLayout.mockResolvedValue("copy-id")
    renderWithApp(<CallsSettings />, { agent: agent() })
    expect(await screen.findByText("Edited")).toBeInTheDocument()
    await userEvent.click(screen.getByRole("button", { name: "Restore defaults" }))
    expect(api.restoreCallsLayout).not.toHaveBeenCalled()
    const dialog = await screen.findByRole("alertdialog")
    expect(dialog).toHaveTextContent("Your version is kept first as the layout “Calls (my version,")
    await userEvent.click(within(dialog).getByRole("button", { name: "Restore defaults" }))
    expect(api.restoreCallsLayout).toHaveBeenCalledWith(
      device.id.model,
      expect.stringMatching(/^Calls \(my version, .+\)$/),
    )
  })
})
