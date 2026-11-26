import { describe, expect, it } from "vitest"

import en from "@/i18n/locales/en/devices.json"
import pl from "@/i18n/locales/pl/devices.json"
import {
  DEVICE_STATE_HINT,
  DEVICE_STATE_LABEL,
  deviceAvailability,
  deviceStateHint,
  deviceStateTone,
} from "./device-state"
import type { DeviceAvailability, DeviceInfo } from "./types"

function device(state: DeviceInfo["state"], availability?: DeviceAvailability): DeviceInfo {
  return {
    id: { model: "elgato.streamdeck.mk2", serial: "A1" },
    modelName: "Stream Deck MK.2",
    state,
    layout: { type: "missing" },
    page: null,
    source: "elgato-hid",
    ...(availability ? { availability } : {}),
  }
}

const ALL = Object.keys(DEVICE_STATE_LABEL) as DeviceAvailability[]

describe("device states", () => {
  it("uses the agent's state, and the connection state from older agents", () => {
    expect(deviceAvailability(device("disconnected", "permissionDenied"))).toBe("permissionDenied")
    expect(deviceAvailability(device("disconnected", "usbPresentHidUnavailable"))).toBe(
      "usbPresentHidUnavailable",
    )
    expect(deviceAvailability(device("connected"))).toBe("connected")
    expect(deviceAvailability(device("disconnected"))).toBe("notConnected")
  })

  it("makes states that need the user stand out", () => {
    expect(deviceStateTone("connected")).toBe("positive")
    expect(deviceStateTone("notConnected")).toBe("neutral")
    expect(deviceStateTone("permissionDenied")).toBe("negative")
    expect(deviceStateTone("usbPresentHidUnavailable")).toBe("negative")
    expect(deviceStateTone("busy")).toBe("warning")
  })

  it("has an English and a Polish label for every state, and hints except for plain ones", () => {
    for (const availability of ALL) {
      const key = DEVICE_STATE_LABEL[availability].replace("devices:state.", "")
      expect(en.state[key as keyof typeof en.state]).toBeTruthy()
      expect(pl.state[key as keyof typeof pl.state]).toBeTruthy()
    }
    expect(deviceStateHint("connected")).toBeUndefined()
    expect(deviceStateHint("notConnected")).toBeUndefined()
    for (const hint of Object.values(DEVICE_STATE_HINT)) {
      const key = hint.replace("devices:stateHint.", "")
      expect(en.stateHint[key as keyof typeof en.stateHint]).toBeTruthy()
      expect(pl.stateHint[key as keyof typeof pl.stateHint]).toBeTruthy()
    }
    expect(en.stateHint.permissionDenied).toMatch(/udev/)
  })
})
