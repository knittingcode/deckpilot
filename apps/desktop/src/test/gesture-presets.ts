// Fixtures of gesture preset tests: the deck module's Brightness action and
// its Brightness Control preset as the backend lists them
// (`modules/deck/src/lib.rs`), and a simulated Stream Deck MK.2.

import { moduleInfo } from "@/lib/test-fixtures"
import type { ActionType, DeviceModel, GesturePreset, ModuleInfo } from "@/lib/types"

export const BRIGHTNESS: ActionType = {
  id: "deck.brightness",
  name: "Brightness",
  description: "Sets the display brightness.",
  category: "Device",
  requiresProject: false,
  parameters: [
    {
      key: "change",
      label: "Change",
      kind: { type: "choice", options: ["set", "brighter", "dimmer"] },
      required: true,
      default: { type: "choice", value: "set" },
    },
    {
      key: "level",
      label: "Level (%)",
      kind: { type: "integer", min: 0, max: 100 },
      required: true,
      default: { type: "integer", value: 100 },
    },
    {
      key: "step",
      label: "Step (%)",
      kind: { type: "integer", min: 1, max: 100 },
      required: true,
      default: { type: "integer", value: 10 },
    },
  ],
  node: { icon: "monitor" } as ActionType["node"],
  controls: {
    key: ["press", "release", "click", "double-click", "long-press"],
    encoder: ["rotate-clockwise", "rotate-counter-clockwise", "press", "click"],
  },
}

/** The built-in preset as the deck module declares it (`brightness_control`). */
export const BRIGHTNESS_CONTROL: GesturePreset = {
  id: "deck.brightness-control",
  name: "Brightness Control",
  description: "Single press: dimmer by 10%. Double press: full brightness. Long press: half.",
  category: "Device",
  icon: "monitor",
  allowPartial: false,
  gestures: [
    {
      gesture: "click",
      action: "deck.brightness",
      parameters: {
        change: { type: "choice", value: "dimmer" },
        step: { type: "integer", value: 10 },
      },
      label: "Decrease brightness by 10%",
    },
    {
      gesture: "doubleClick",
      action: "deck.brightness",
      parameters: {
        change: { type: "choice", value: "set" },
        level: { type: "integer", value: 100 },
      },
      label: "Set brightness to 100%",
    },
    {
      gesture: "longPress",
      action: "deck.brightness",
      parameters: {
        change: { type: "choice", value: "set" },
        level: { type: "integer", value: 50 },
      },
      label: "Set brightness to 50%",
    },
  ],
}

export function deckModule(change: Partial<ModuleInfo> = {}): ModuleInfo {
  return moduleInfo({
    id: "deck",
    name: "Deck",
    actionTypes: [BRIGHTNESS],
    gesturePresets: [BRIGHTNESS_CONTROL],
    ...change,
  })
}

export const MK2: DeviceModel = {
  id: "elgato.streamdeck.mk2",
  vendor: "Elgato",
  name: "Stream Deck MK.2",
  usb: null,
  capabilities: { keys: { rows: 3, columns: 5, image: null }, encoders: null, touch: null },
  support: "verified",
  verification: [],
}
