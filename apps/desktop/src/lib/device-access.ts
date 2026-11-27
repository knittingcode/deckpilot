import { deviceAvailability } from "@/lib/device-state"
import { browserStorage } from "@/lib/navigation"
import type { AgentConnection, PackageFormat, UpdateState } from "@/lib/types"

/**
 * A one-time guide when a Stream Deck is plugged in but cannot be used
 * (DP-INST-02), from the device states of IPC protocol 21 (#514):
 *
 * - `permission`: Linux says DeckPilot may not open the device: the udev
 *   rule is missing (AppImage, Flatpak) or did not apply yet (a device
 *   plugged in while the package was installed);
 * - `usbWithoutHid`: the USB bus lists the device without its HID
 *   interface (behind a hub or dock).
 *
 * It shows text to copy and never runs a command. The permission guide is
 * Linux only, on purpose: on Windows the device uses the built-in HID
 * driver, and on macOS no permission is known to be needed for a Stream
 * Deck with hidapi's IOKit backend (Input Monitoring refuses keyboard and
 * pointer interfaces; experiments/distribution/macos-sandbox-hid). That
 * is not measured with a Stream Deck, so there is no macOS fix to give
 * (docs/devices.md, "Device access guide").
 */
export type DeviceAccessIssue = "permission" | "usbWithoutHid"

/** Which guides were closed: each one is shown once. */
export const DEVICE_ACCESS_GUIDE_KEY = "deckpilot.deviceAccessGuide.dismissed"

/** The rule of the installed version's tag (`main` for other versions). */
export function ruleUrl(version: string): string {
  const ref = /^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(version) ? `v${version}` : "main"
  return `https://raw.githubusercontent.com/knittingcode/deckpilot/${ref}/packaging/linux/70-deckpilot-streamdeck.rules`
}
/** The troubleshooting page of #513. */
export const TROUBLESHOOTING_URL =
  "https://github.com/knittingcode/deckpilot/blob/main/docs/troubleshooting/usb-hub-resets.md"
export const INSTALLATION_URL =
  "https://github.com/knittingcode/deckpilot/blob/main/website/src/content/docs/en/installation.md#appimage"

/** How the udev rule gets onto this system. */
export type RuleSource = "deb" | "rpm" | "manual"

export function ruleSource(format: PackageFormat | null | undefined): RuleSource {
  if (format === "deb" || format === "rpm") return format
  // AppImage, Flatpak, a development build or an unknown one: by hand.
  return "manual"
}

/**
 * The commands to copy for `source`; the user runs them, the app never
 * does. `target` is the update target (`linux-x86_64`), for the package's
 * file name; `channel` the distribution channel (`apt`: a repository, so
 * the package is reinstalled by name).
 */
export function udevCommands(
  source: RuleSource,
  version: string,
  target = "",
  channel = "",
): string {
  const arm = target.endsWith("aarch64")
  switch (source) {
    case "deb":
      return channel === "apt"
        ? "sudo apt install --reinstall deck-pilot"
        : `sudo apt install --reinstall ./DeckPilot_${version}_${arm ? "arm64" : "amd64"}.deb`
    case "rpm":
      return `sudo dnf reinstall ./DeckPilot-${version}-1.${arm ? "aarch64" : "x86_64"}.rpm`
    case "manual":
      return [
        `curl -fsSLO ${ruleUrl(version)}`,
        "cat 70-deckpilot-streamdeck.rules",
        "sudo install -m 0644 70-deckpilot-streamdeck.rules /etc/udev/rules.d/",
        "sudo udevadm control --reload-rules",
      ].join("\n")
  }
}

function isLinux(agent: AgentConnection | null, updates: UpdateState | null): boolean {
  const platform = agent?.status?.distribution?.platform
  if (platform) return platform === "linux"
  return updates?.target.startsWith("linux") ?? false
}

/**
 * The issue the guide is about now, if any: the first of permission and
 * USB without HID that is present and not in `dismissed`.
 */
export function deviceAccessIssue(
  agent: AgentConnection | null,
  updates: UpdateState | null,
  dismissed: readonly DeviceAccessIssue[] = [],
): DeviceAccessIssue | null {
  const states = (agent?.status?.devices ?? []).map(deviceAvailability)
  const present: DeviceAccessIssue[] = []
  if (states.includes("permissionDenied") && isLinux(agent, updates)) present.push("permission")
  if (states.includes("usbPresentHidUnavailable")) present.push("usbWithoutHid")
  return present.find((issue) => !dismissed.includes(issue)) ?? null
}

export function readDismissed(): DeviceAccessIssue[] {
  try {
    const value: unknown = JSON.parse(browserStorage()?.getItem(DEVICE_ACCESS_GUIDE_KEY) ?? "[]")
    return Array.isArray(value)
      ? value.filter(
          (item): item is DeviceAccessIssue => item === "permission" || item === "usbWithoutHid",
        )
      : []
  } catch {
    return []
  }
}

export function writeDismissed(issues: DeviceAccessIssue[]) {
  try {
    browserStorage()?.setItem(DEVICE_ACCESS_GUIDE_KEY, JSON.stringify(issues))
  } catch {
    // Without storage the guide may show again on the next start.
  }
}
