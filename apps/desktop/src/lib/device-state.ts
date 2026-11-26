import type { DeviceAvailability, DeviceInfo } from "./types"

/** The tones of `InlineStatus` (components/settings-layout.tsx). */
export type DeviceStateTone = "positive" | "warning" | "negative" | "neutral"

/**
 * The state a device row shows: the agent's finer state where it reports
 * one (IPC protocol 21), else connected or not connected.
 */
export function deviceAvailability(device: DeviceInfo): DeviceAvailability {
  if (device.availability) return device.availability
  return device.state === "connected" ? "connected" : "notConnected"
}

/** The tone of a device's state: what needs the user's attention stands out. */
export function deviceStateTone(availability: DeviceAvailability): DeviceStateTone {
  switch (availability) {
    case "connected":
      return "positive"
    case "notConnected":
    case "unknown":
    case "reconnecting":
      return "neutral"
    case "unsupported":
    case "busy":
      return "warning"
    case "usbPresentHidUnavailable":
    case "permissionDenied":
    case "communicationError":
      return "negative"
  }
}

/** The label of each state (`devices:state.*`). */
export const DEVICE_STATE_LABEL = {
  connected: "devices:state.connected",
  notConnected: "devices:state.notConnected",
  usbPresentHidUnavailable: "devices:state.usbPresentHidUnavailable",
  permissionDenied: "devices:state.permissionDenied",
  unsupported: "devices:state.unsupported",
  busy: "devices:state.busy",
  communicationError: "devices:state.communicationError",
  reconnecting: "devices:state.reconnecting",
  unknown: "devices:state.unknown",
} as const satisfies Record<DeviceAvailability, string>

/** What to do about a state, for those that need it (`devices:stateHint.*`). */
export const DEVICE_STATE_HINT = {
  usbPresentHidUnavailable: "devices:stateHint.usbPresentHidUnavailable",
  permissionDenied: "devices:stateHint.permissionDenied",
  unsupported: "devices:stateHint.unsupported",
  busy: "devices:stateHint.busy",
  communicationError: "devices:stateHint.communicationError",
  reconnecting: "devices:stateHint.reconnecting",
  unknown: "devices:stateHint.unknown",
} as const satisfies Partial<Record<DeviceAvailability, string>>

/** The hint key of `availability`, if it has one. */
export function deviceStateHint(
  availability: DeviceAvailability,
): (typeof DEVICE_STATE_HINT)[keyof typeof DEVICE_STATE_HINT] | undefined {
  return availability in DEVICE_STATE_HINT
    ? DEVICE_STATE_HINT[availability as keyof typeof DEVICE_STATE_HINT]
    : undefined
}
