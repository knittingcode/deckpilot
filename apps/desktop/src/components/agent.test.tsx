// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import {
  AgentBadge,
  AgentCompatibilityNotice,
  AgentIndicator,
  AgentStopped,
  DeviceRow,
} from "@/components/agent"
import { i18n } from "@/i18n"
import type { AgentCompatibility, DeviceInfo } from "@/lib/types"
import { api, resetApi } from "@/test/api-mock"
import { installDomStubs } from "@/test/dom"
import { agentConnection, agentStatus } from "@/test/fixtures"
import { renderWithApp } from "@/test/render"
import { resetToasts, toast } from "@/test/toast"

vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)
vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)

beforeAll(installDomStubs)
beforeEach(() => {
  resetApi()
  resetToasts()
})
afterEach(cleanup)

const versions = { agent: "0.4.0", app: "0.5.0" }

describe("agent compatibility notice", () => {
  it.each<[string, AgentCompatibility]>([
    ["outdated", { type: "agentOutdated", ...versions }],
    ["newer", { type: "agentNewer", agent: "0.6.0", app: "0.5.0" }],
    ["unknown", { type: "unknown", ...versions }],
  ])("explains an %s agent and restarts it at once when nothing runs", async (_, compatibility) => {
    const restarted = agentConnection()
    api.restartAgent.mockResolvedValue(restarted)
    const { user, context } = renderWithApp(<AgentCompatibilityNotice />, {
      agent: agentConnection({ compatibility }),
    })
    const alert = screen.getByRole("alert")
    expect(alert).toHaveTextContent("The running agent does not match this app")
    expect(alert).toHaveTextContent(
      `Agent ${compatibility.type === "compatible" ? "" : compatibility.agent}, app 0.5.0.`,
    )
    expect(alert).toHaveTextContent("Restart the agent so both run the same version.")
    await user.click(screen.getByRole("button", { name: "Restart agent" }))
    expect(api.restartAgent).toHaveBeenCalledWith(false)
    await waitFor(() => {
      expect(context.setAgent).toHaveBeenCalledWith(restarted)
    })
    expect(toast.success).toHaveBeenCalledWith("The agent was restarted")
  })

  it("refuses an agent of another installation and replaces it with this app's", async () => {
    const restarted = agentConnection()
    api.restartAgent.mockResolvedValue(restarted)
    const { user, context } = renderWithApp(<AgentCompatibilityNotice />, {
      agent: agentConnection({
        compatibility: { type: "otherDistribution", agent: "apt", app: "github" },
      }),
    })
    const alert = screen.getByRole("alert")
    expect(alert).toHaveTextContent("The running agent belongs to another installation")
    expect(alert).toHaveTextContent(
      "The agent was installed from the APT repository, this app from GitHub Releases.",
    )
    await user.click(screen.getByRole("button", { name: "Restart agent" }))
    expect(api.restartAgent).toHaveBeenCalledWith(false)
    await waitFor(() => {
      expect(context.setAgent).toHaveBeenCalledWith(restarted)
    })
  })

  it("says when the agent must be replaced and which features are missing", () => {
    renderWithApp(<AgentCompatibilityNotice />, {
      agent: agentConnection({
        compatibility: { type: "updateRequired", agent: "0.2.0", app: "0.5.0" },
        capabilities: ["workflows", "notifications"],
      }),
    })
    const alert = screen.getByRole("alert")
    expect(alert).toHaveTextContent("The running agent is too old for this app")
    expect(alert).toHaveTextContent("Nothing can be configured until it is replaced")
    expect(alert).toHaveTextContent(
      "Until then these are unavailable: the encrypted configuration, background workflow runs and their history.",
    )
    expect(alert).not.toHaveTextContent("workflow notifications")
  })

  it("lists no missing features while the capabilities are unknown", () => {
    renderWithApp(<AgentCompatibilityNotice />, {
      agent: agentConnection({ compatibility: { type: "agentOutdated", ...versions } }),
    })
    expect(screen.getByRole("alert")).not.toHaveTextContent("unavailable")
  })

  it("asks before interrupting running workflows", async () => {
    api.restartAgent.mockResolvedValue(agentConnection())
    const { user } = renderWithApp(<AgentCompatibilityNotice />, {
      agent: agentConnection({
        compatibility: { type: "agentOutdated", ...versions },
        status: agentStatus({ runningExecutions: 2 }),
      }),
    })
    expect(screen.getByRole("alert")).toHaveTextContent(
      "2 workflows are running in the agent. Restarting now interrupts them",
    )
    await user.click(screen.getByRole("button", { name: "Restart agent…" }))
    expect(api.restartAgent).not.toHaveBeenCalled()
    await user.click(screen.getByRole("button", { name: "Wait for them" }))
    expect(screen.queryByRole("button", { name: "Interrupt workflows and restart" })).toBeNull()

    await user.click(screen.getByRole("button", { name: "Restart agent…" }))
    await user.click(screen.getByRole("button", { name: "Interrupt workflows and restart" }))
    expect(api.restartAgent).toHaveBeenCalledWith(true)
  })

  it.each([
    ["compatible", agentConnection()],
    [
      "stopped",
      agentConnection({ running: false, compatibility: { type: "unknown", ...versions } }),
    ],
    ["not yet known", agentConnection({ compatibility: null })],
  ])("stays hidden when the agent is %s", (_, agent) => {
    renderWithApp(<AgentCompatibilityNotice />, { agent })
    expect(screen.queryByRole("alert")).not.toBeInTheDocument()
  })

  it("reports a failed restart", async () => {
    api.restartAgent.mockRejectedValue(new Error("socket busy"))
    const { user } = renderWithApp(<AgentCompatibilityNotice />, {
      agent: agentConnection({ compatibility: { type: "agentOutdated", ...versions } }),
    })
    await user.click(screen.getByRole("button", { name: "Restart agent" }))
    await waitFor(() => {
      expect(toast.error).toHaveBeenCalledWith("The agent was not restarted", {
        description: "Socket busy. Check its log in Settings › Diagnostics.",
      })
    })
    expect(screen.getByRole("button", { name: "Restart agent" })).toBeEnabled()
  })
})

describe("stopped agent", () => {
  it("starts the agent and reports when it does not come up", async () => {
    api.startAgent.mockResolvedValue(agentConnection({ running: false }))
    const { user, context } = renderWithApp(<AgentStopped />, {
      agent: agentConnection({ running: false }),
    })
    await user.click(screen.getByRole("button", { name: "Start agent" }))
    await waitFor(() => {
      expect(context.setAgent).toHaveBeenCalled()
    })
    expect(toast.error).toHaveBeenCalledWith("The agent did not start. Check its log for details.")
  })

  it("says that a crashed agent is being restarted", () => {
    renderWithApp(<AgentStopped />, {
      agent: agentConnection({
        running: false,
        supervision: {
          state: "restarting",
          restart: 2,
          max: 3,
          delayMs: 5000,
          exit: { success: false, code: null, signal: 9 },
        },
      }),
    })
    const alert = screen.getByRole("alert")
    expect(alert).toHaveTextContent("The agent stopped unexpectedly")
    expect(alert).toHaveTextContent(
      "It ended (signal 9). DeckPilot starts it again in 5 seconds (restart 2 of 3).",
    )
    expect(screen.getByRole("button", { name: "Start agent" })).toBeEnabled()
  })

  it("shows a stopped agent after too many restarts, with a Restart action", async () => {
    const restarted = agentConnection()
    api.startAgent.mockResolvedValue(restarted)
    const { user, context } = renderWithApp(<AgentStopped />, {
      agent: agentConnection({
        running: false,
        supervision: {
          state: "stopped",
          restarts: 3,
          windowMinutes: 10,
          exit: { success: false, code: 101, signal: null },
        },
      }),
    })
    const alert = screen.getByRole("alert")
    expect(alert).toHaveTextContent("DeckPilot agent stopped")
    expect(alert).toHaveTextContent(
      "It ended unexpectedly (exit code 101) after being restarted 3 times in 10 minutes. It is not started again by itself.",
    )
    expect(alert).toHaveTextContent("Check its log in Settings › Diagnostics")
    await user.click(screen.getByRole("button", { name: "Restart" }))
    expect(api.startAgent).toHaveBeenCalledTimes(1)
    await waitFor(() => {
      expect(context.setAgent).toHaveBeenCalledWith(restarted)
    })
  })

  it("says when a crashed agent could not be started again", () => {
    renderWithApp(<AgentStopped />, {
      agent: agentConnection({
        running: false,
        supervision: {
          state: "stopped",
          restarts: 0,
          windowMinutes: 10,
          exit: { success: false, code: 1, signal: null },
        },
      }),
    })
    expect(screen.getByRole("alert")).toHaveTextContent(
      "It ended unexpectedly (exit code 1) and could not be started again.",
    )
  })

  it("shows a Windows crash code in hex", () => {
    renderWithApp(<AgentStopped />, {
      agent: agentConnection({
        running: false,
        supervision: {
          state: "stopped",
          restarts: 0,
          windowMinutes: 10,
          exit: { success: false, code: -1073741819, signal: null },
        },
      }),
    })
    expect(screen.getByRole("alert")).toHaveTextContent("(exit code 0xC0000005)")
  })

  it("explains when the agent cannot be started from the app", () => {
    renderWithApp(<AgentStopped />, { agent: agentConnection({ running: false, canStart: false }) })
    expect(screen.getByRole("button", { name: "Start agent" })).toBeDisabled()
    expect(screen.getByText(/executable was not found/)).toBeInTheDocument()
  })

  it("labels the agent state in badges and the titlebar indicator", async () => {
    const onOpen = vi.fn()
    const { user, rerender, context } = renderWithApp(
      <>
        <AgentBadge />
        <AgentIndicator onOpen={onOpen} />
      </>,
      { agent: null },
    )
    expect(screen.getByText("Checking…")).toBeInTheDocument()
    expect(screen.getByRole("button", { name: /Checking the agent/ })).toBeInTheDocument()
    rerender(
      <>
        <AgentBadge />
        <AgentIndicator onOpen={onOpen} />
      </>,
      { ...context, agent: agentConnection({ running: false }) },
    )
    expect(screen.getByText("Stopped")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Agent stopped, open agent settings" }))
    expect(onOpen).toHaveBeenCalled()
  })
})

describe("device rows", () => {
  const device = (overrides: Partial<DeviceInfo>): DeviceInfo => ({
    id: { model: "elgato.streamdeck.mk2", serial: "AL1" },
    modelName: "Stream Deck MK.2",
    state: "disconnected",
    layout: { type: "missing" },
    page: null,
    source: "elgato-hid",
    ...overrides,
  })

  it("say why a present device is not usable, in English and Polish", async () => {
    renderWithApp(<DeviceRow device={device({ availability: "permissionDenied" })} />)
    expect(screen.getByText("No permission")).toBeInTheDocument()
    expect(screen.getByText(/udev rule/)).toBeInTheDocument()
    expect(screen.queryByText("Disconnected")).not.toBeInTheDocument()
    cleanup()
    renderWithApp(<DeviceRow device={device({ availability: "usbPresentHidUnavailable" })} />)
    expect(screen.getByText("On USB, not usable")).toBeInTheDocument()
    expect(screen.getByText(/power-cycle the hub/)).toBeInTheDocument()
    cleanup()
    await i18n.changeLanguage("pl")
    try {
      renderWithApp(<DeviceRow device={device({ availability: "busy" })} />)
      expect(screen.getByText("Używane przez inny program")).toBeInTheDocument()
    } finally {
      await i18n.changeLanguage("en")
    }
  })

  it("show connected and disconnected from an agent before device states", () => {
    renderWithApp(<DeviceRow device={device({ state: "connected" })} />)
    expect(screen.getByText("Connected")).toBeInTheDocument()
    cleanup()
    renderWithApp(<DeviceRow device={device({})} />)
    expect(screen.getByText("Disconnected")).toBeInTheDocument()
  })
})
