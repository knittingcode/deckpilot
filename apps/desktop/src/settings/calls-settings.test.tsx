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
import { i18n } from "@/i18n"
import { CALLS_EVENT } from "@/lib/calls"
import type { BackToWork, CallsLayout, CallsStatus } from "@/lib/types"
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
        pages: [
          {
            page: "active",
            name: "In a call",
            controls: [
              {
                control: { kind: "key", index: 0 },
                action: "mic-toggle",
                supported: false,
                reason: "provider-has-no-api",
              },
              {
                control: { kind: "key", index: 10 },
                action: "return-to-work",
                supported: true,
              },
            ],
          },
        ],
        back: {
          mode: { kind: "completionOrMax", maxSeconds: 60 },
          onFailure: "resume",
          onCancel: "stay",
          allDevices: false,
        },
      },
    ],
    ...overrides,
  }
}

function firstLayout(): CallsLayout {
  const [layout] = status().layouts
  if (!layout) throw new Error("the fixture's layout")
  return layout
}

function firstBack(): BackToWork {
  const { back } = firstLayout()
  if (!back) throw new Error("the fixture's Back to work")
  return back
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
    // The Controls row, and the disabled controls of the call pages.
    expect(screen.getAllByText("Not supported").length).toBeGreaterThanOrEqual(2)
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

  it("lists the call pages with unsupported controls disabled and the reason", async () => {
    api.getCallsStatus.mockResolvedValue(status())
    renderWithApp(<CallsSettings />, { agent: agent() })
    const pages = await screen.findByRole("heading", { name: /Call pages/ })
    const section = pages.closest("section")
    if (!section) throw new Error("the Call pages section")
    expect(within(section).getByText("Key 1: Microphone")).toBeInTheDocument()
    // Teams: the reason names Microsoft's retired API, never a working control.
    expect(within(section).getByText(/Microsoft retired the Teams device API/)).toBeInTheDocument()
    expect(within(section).getByText("Key 11: Back to work")).toBeInTheDocument()
    expect(within(section).getByText("Supported")).toBeInTheDocument()
  })

  it("names the generic reason for a provider other than Teams", async () => {
    api.getCallsStatus.mockResolvedValue(status({ provider: "mock" }))
    renderWithApp(<CallsSettings />, { agent: agent() })
    expect(await screen.findByText(/the call app offers no control/)).toBeInTheDocument()
    expect(screen.queryByText(/Microsoft retired the Teams device API/)).not.toBeInTheDocument()
  })

  it("shows Back to work with completion or a 60 s maximum by default", async () => {
    api.getCallsStatus.mockResolvedValue(status())
    renderWithApp(<CallsSettings />, { agent: agent() })
    const mode = await screen.findByRole("combobox", { name: "Shows the call again" })
    expect(mode).toHaveTextContent(/at the latest after a time \(default\)/)
    expect(screen.getByRole("textbox", { name: "At the latest after" })).toHaveValue("60")
  })

  it("sets a timer from the presets or a custom time, refusing one out of range", async () => {
    const user = userEvent.setup()
    const layout = firstLayout()
    api.getCallsStatus.mockResolvedValue(status())
    const timer = (seconds: number) =>
      status({
        layouts: [{ ...layout, back: { ...firstBack(), mode: { kind: "timer", seconds } } }],
      })
    api.setCallsBackToWork.mockImplementation((_model: string, back: BackToWork) =>
      Promise.resolve(timer(back.mode.kind === "timer" ? back.mode.seconds : 0)),
    )
    renderWithApp(<CallsSettings />, { agent: agent() })
    await user.click(await screen.findByRole("combobox", { name: "Shows the call again" }))
    await user.click(await screen.findByRole("option", { name: "After a time" }))
    expect(api.setCallsBackToWork).toHaveBeenLastCalledWith(device.id.model, {
      ...firstBack(),
      mode: { kind: "timer", seconds: 30 },
    })
    await user.click(await screen.findByRole("combobox", { name: "After" }))
    await user.click(await screen.findByRole("option", { name: "60 s" }))
    expect(api.setCallsBackToWork).toHaveBeenLastCalledWith(device.id.model, {
      ...firstBack(),
      mode: { kind: "timer", seconds: 60 },
    })
    await user.click(screen.getByRole("combobox", { name: "After" }))
    await user.click(await screen.findByRole("option", { name: "Custom" }))
    const field = await screen.findByRole("textbox", { name: "After" })
    const calls = api.setCallsBackToWork.mock.calls.length
    await user.clear(field)
    await user.type(field, "3")
    expect(await screen.findByRole("alert")).toHaveTextContent("5 to 600 seconds")
    await user.keyboard("{Enter}")
    expect(api.setCallsBackToWork).toHaveBeenCalledTimes(calls)
    await user.clear(field)
    await user.type(field, "45{Enter}")
    await waitFor(() => {
      expect(api.setCallsBackToWork).toHaveBeenLastCalledWith(device.id.model, {
        ...firstBack(),
        mode: { kind: "timer", seconds: 45 },
      })
    })
  })

  it("sets completion, what a failure does and all devices", async () => {
    const user = userEvent.setup()
    api.getCallsStatus.mockResolvedValue(status())
    api.setCallsBackToWork.mockResolvedValue(status())
    renderWithApp(<CallsSettings />, { agent: agent() })
    await user.click(await screen.findByRole("combobox", { name: "Shows the call again" }))
    await user.click(await screen.findByRole("option", { name: "When what you started ends" }))
    expect(api.setCallsBackToWork).toHaveBeenLastCalledWith(device.id.model, {
      ...firstBack(),
      mode: { kind: "completion" },
    })
    await user.click(screen.getByRole("combobox", { name: "If it fails" }))
    await user.click(await screen.findByRole("option", { name: "Stay on my layout" }))
    expect(api.setCallsBackToWork).toHaveBeenLastCalledWith(device.id.model, {
      ...firstBack(),
      onFailure: "stay",
    })
    await user.click(screen.getByRole("switch", { name: "All devices" }))
    expect(api.setCallsBackToWork).toHaveBeenLastCalledWith(device.id.model, {
      ...firstBack(),
      allDevices: true,
    })
  })

  it("has nothing to set when the layout has no Back to work key", async () => {
    const { back, ...layout } = firstLayout()
    expect(back).toBeDefined()
    api.getCallsStatus.mockResolvedValue(status({ layouts: [layout] }))
    renderWithApp(<CallsSettings />, { agent: agent() })
    expect(await screen.findByText(/has no Back to work key/)).toBeInTheDocument()
    expect(screen.queryByRole("combobox", { name: "Shows the call again" })).not.toBeInTheDocument()
  })

  it("shows a page on the device as a preview, and ends it", async () => {
    const user = userEvent.setup()
    api.getCallsStatus.mockResolvedValue(status())
    api.callsPreviewBegin.mockResolvedValue(null)
    api.callsPreviewEnd.mockResolvedValue(null)
    const view = renderWithApp(<CallsSettings />, { agent: agent() })
    const show = await screen.findByRole("button", { name: "Show on the device" })
    await user.click(show)
    expect(api.callsPreviewBegin).toHaveBeenCalledWith(device.id, "active")
    const end = await screen.findByRole("button", { name: "End preview" })
    expect(end).toHaveAttribute("aria-pressed", "true")
    await user.click(end)
    expect(api.callsPreviewEnd).toHaveBeenCalled()
    // Leaving the page ends any preview.
    const calls = api.callsPreviewEnd.mock.calls.length
    view.rerender(<div />)
    await waitFor(() => {
      expect(api.callsPreviewEnd.mock.calls.length).toBeGreaterThan(calls)
    })
  })

  it("says why a page cannot be shown during a call", async () => {
    const user = userEvent.setup()
    api.getCallsStatus.mockResolvedValue(status())
    api.callsPreviewBegin.mockRejectedValue(new Error("a call shows on the device"))
    renderWithApp(<CallsSettings />, { agent: agent() })
    await user.click(await screen.findByRole("button", { name: "Show on the device" }))
    expect(await screen.findByRole("button", { name: "Show on the device" })).toHaveAttribute(
      "aria-pressed",
      "false",
    )
  })

  it("ends a preview whose begin is still pending when the settings close", async () => {
    const user = userEvent.setup()
    api.getCallsStatus.mockResolvedValue(status())
    let answer: (value: null) => void = () => undefined
    api.callsPreviewBegin.mockReturnValue(
      new Promise<null>((resolve) => {
        answer = resolve
      }),
    )
    api.callsPreviewEnd.mockResolvedValue(null)
    const view = renderWithApp(<CallsSettings />, { agent: agent() })
    const show = await screen.findByRole("button", { name: "Show on the device" })
    await user.click(show)
    // Pending: no second begin.
    expect(screen.getByRole("button", { name: "Show on the device" })).toBeDisabled()
    view.rerender(<div />)
    await waitFor(() => {
      expect(api.callsPreviewEnd).toHaveBeenCalled()
    })
    // The late answer changes nothing (and the backend ends it at once).
    act(() => {
      answer(null)
    })
    expect(screen.queryByRole("button", { name: "End preview" })).not.toBeInTheDocument()
  })

  it("says why the agent refused a preview, in English and Polish", async () => {
    const user = userEvent.setup()
    const refusal = {
      code: "agent.unsupported",
      message: "a call shows on the device; it always wins over a preview",
      params: { reason: "callsPreviewInCall" },
    }
    api.getCallsStatus.mockResolvedValue(status())
    api.callsPreviewBegin.mockRejectedValue(refusal)
    const { toast } = await import("sonner")
    renderWithApp(<CallsSettings />, { agent: agent() })
    await user.click(await screen.findByRole("button", { name: "Show on the device" }))
    await waitFor(() => {
      expect(JSON.stringify(vi.mocked(toast.error).mock.calls)).toContain(
        "a call always comes first",
      )
    })
    cleanup()
    vi.mocked(toast.error).mockClear()
    await i18n.changeLanguage("pl")
    try {
      renderWithApp(<CallsSettings />, { agent: agent() })
      await user.click(await screen.findByRole("button", { name: "Pokaż na urządzeniu" }))
      await waitFor(() => {
        expect(JSON.stringify(vi.mocked(toast.error).mock.calls)).toContain(
          "rozmowa ma zawsze pierwszeństwo",
        )
      })
    } finally {
      await i18n.changeLanguage("en")
    }
  })

  it("follows the agent when it ends the preview by itself", async () => {
    const user = userEvent.setup()
    const previewing = (page?: "active") =>
      status({
        devices: [
          { device: device.id, overlay: true, shown: true, suspended: false, preview: page },
        ],
      })
    api.getCallsStatus.mockResolvedValue(previewing())
    api.callsPreviewBegin.mockResolvedValue(null)
    renderWithApp(<CallsSettings />, { agent: agent() })
    await user.click(await screen.findByRole("button", { name: "Show on the device" }))
    act(() => {
      emitBackend(CALLS_EVENT, previewing("active"))
    })
    expect(await screen.findByRole("button", { name: "End preview" })).toBeInTheDocument()
    // A real call came: the agent ended it and says so.
    act(() => {
      emitBackend(CALLS_EVENT, previewing())
    })
    expect(await screen.findByRole("button", { name: "Show on the device" })).toHaveAttribute(
      "aria-pressed",
      "false",
    )
  })
})
