// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { readFileSync } from "node:fs"
import path from "node:path"

import { cleanup, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { DeviceAccessGuide } from "@/components/device-access-guide"
import {
  DEVICE_ACCESS_GUIDE_KEY,
  TROUBLESHOOTING_URL,
  deviceAccessIssue,
  ruleSource,
  udevCommands,
} from "@/lib/device-access"
import type { DeviceAvailability } from "@/lib/types"
import { agentConnection, agentStatus, deviceInfo, updateState } from "@/test/fixtures"
import { installDomStubs } from "@/test/dom"
import { renderWithApp } from "@/test/render"

vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }))

const RULE_FILE = path.join(
  import.meta.dirname,
  "../../../../packaging/linux/70-deckpilot-streamdeck.rules",
)

function agentWith(availability: DeviceAvailability, platform = "linux") {
  return agentConnection({
    status: agentStatus({
      devices: [deviceInfo({ state: "disconnected", availability })],
      distribution: {
        distributionChannel: "github",
        updateProvider: "github_release",
        releaseChannel: "stable",
        applicationVersion: "0.15.0",
        platform,
        architecture: "x86_64",
      },
    }),
  })
}

const appImage = () =>
  updateState("apt", {
    packageFormat: "appimage",
    currentVersion: "0.15.0",
    target: "linux-x86_64",
  })

beforeAll(() => {
  installDomStubs()
})

beforeEach(() => {
  localStorage.clear()
})

afterEach(() => {
  cleanup()
})

describe("which issue the guide is about", () => {
  it("permission denied on Linux, and USB without HID anywhere", () => {
    expect(deviceAccessIssue(agentWith("permissionDenied"), null)).toBe("permission")
    expect(deviceAccessIssue(agentWith("usbPresentHidUnavailable"), null)).toBe("usbWithoutHid")
    expect(deviceAccessIssue(agentWith("usbPresentHidUnavailable", "macos"), null)).toBe(
      "usbWithoutHid",
    )
  })

  it("no permission guide on macOS or Windows, and none for other states", () => {
    expect(deviceAccessIssue(agentWith("permissionDenied", "macos"), null)).toBeNull()
    expect(deviceAccessIssue(agentWith("permissionDenied", "windows"), null)).toBeNull()
    for (const state of ["connected", "notConnected", "busy", "unsupported", "unknown"] as const) {
      expect(deviceAccessIssue(agentWith(state), null)).toBeNull()
    }
    expect(deviceAccessIssue(null, null)).toBeNull()
  })

  it("an agent without its distribution record: the update target decides", () => {
    const agent = agentConnection({
      status: agentStatus({
        devices: [deviceInfo({ state: "disconnected", availability: "permissionDenied" })],
      }),
    })
    expect(deviceAccessIssue(agent, appImage())).toBe("permission")
    expect(deviceAccessIssue(agent, updateState("apt", { target: "darwin-aarch64" }))).toBeNull()
  })
})

describe("the udev fix", () => {
  it("depends on the package type", () => {
    expect(ruleSource("deb")).toBe("deb")
    expect(ruleSource("rpm")).toBe("rpm")
    for (const format of ["appimage", "flatpak", null, undefined] as const) {
      expect(ruleSource(format)).toBe("manual")
    }
    expect(udevCommands("deb", "0.16.0", "linux-x86_64")).toBe(
      "sudo apt install --reinstall ./DeckPilot_0.16.0_amd64.deb",
    )
    expect(udevCommands("rpm", "0.16.0", "linux-x86_64")).toBe(
      "sudo dnf reinstall ./DeckPilot-0.16.0-1.x86_64.rpm",
    )
  })

  it("by hand: the commands the rule file itself documents", () => {
    const rule = readFileSync(RULE_FILE, "utf8")
    const commands = udevCommands("manual", "0.16.0").split("\n")
    expect(commands[0]).toMatch(
      /^curl -fsSLO https:\/\/raw\.githubusercontent\.com\/.+\/70-deckpilot-streamdeck\.rules$/,
    )
    for (const command of commands.slice(1)) {
      expect(rule).toContain(command)
    }
    // Only the rule and a reload: no broad trigger, no chmod, no group.
    expect(commands.join("\n")).not.toMatch(/trigger|chmod|usermod|0666/)
  })
})

describe("the guide", () => {
  it("shows the fix once and remembers that it was closed", async () => {
    const user = userEvent.setup()
    renderWithApp(<DeviceAccessGuide />, {
      agent: agentWith("permissionDenied"),
      updates: appImage(),
    })
    expect(await screen.findByText("DeckPilot may not open your Stream Deck")).toBeInTheDocument()
    expect(screen.getByLabelText("Commands")).toHaveTextContent(
      "sudo install -m 0644 70-deckpilot-streamdeck.rules /etc/udev/rules.d/",
    )
    expect(screen.getByLabelText("Troubleshooting page")).toHaveTextContent(TROUBLESHOOTING_URL)
    expect(screen.getByText(/does not run these commands/)).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Done" }))
    expect(screen.queryByText("DeckPilot may not open your Stream Deck")).not.toBeInTheDocument()
    expect(JSON.parse(localStorage.getItem(DEVICE_ACCESS_GUIDE_KEY) ?? "[]")).toEqual([
      "permission",
    ])
    cleanup()
    renderWithApp(<DeviceAccessGuide />, {
      agent: agentWith("permissionDenied"),
      updates: appImage(),
    })
    expect(screen.queryByText("DeckPilot may not open your Stream Deck")).not.toBeInTheDocument()
  })

  it("a .deb install is told to replug, then to reinstall its package", async () => {
    renderWithApp(<DeviceAccessGuide />, {
      agent: agentWith("permissionDenied"),
      updates: updateState("apt", {
        packageFormat: "deb",
        currentVersion: "0.16.0",
        target: "linux-x86_64",
      }),
    })
    expect(await screen.findByText(/The .deb package installs the rule/)).toBeInTheDocument()
    expect(screen.getByLabelText("Commands")).toHaveTextContent(
      "sudo apt install --reinstall ./DeckPilot_0.16.0_amd64.deb",
    )
  })

  it("USB without HID: replug, no hub, and the troubleshooting page", async () => {
    renderWithApp(<DeviceAccessGuide />, {
      agent: agentWith("usbPresentHidUnavailable"),
      updates: appImage(),
    })
    expect(await screen.findByText("Your Stream Deck is not fully detected")).toBeInTheDocument()
    expect(screen.getByText(/directly to the computer/)).toBeInTheDocument()
    expect(screen.getByLabelText("Troubleshooting page")).toHaveTextContent(TROUBLESHOOTING_URL)
    expect(screen.queryByLabelText("Commands")).not.toBeInTheDocument()
  })

  it("copies the commands; it runs nothing", async () => {
    const user = userEvent.setup()
    const writeText = vi.spyOn(navigator.clipboard, "writeText").mockResolvedValue(undefined)
    renderWithApp(<DeviceAccessGuide />, {
      agent: agentWith("permissionDenied"),
      updates: appImage(),
    })
    await user.click(await screen.findByRole("button", { name: "Copy Commands" }))
    expect(writeText).toHaveBeenCalledWith(udevCommands("manual", "0.15.0", "linux-x86_64"))
  })

  it("shows nothing on macOS for a permission state", () => {
    renderWithApp(<DeviceAccessGuide />, {
      agent: agentWith("permissionDenied", "macos"),
      updates: updateState("apt", { packageFormat: "app", target: "darwin-aarch64" }),
    })
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
  })
})
