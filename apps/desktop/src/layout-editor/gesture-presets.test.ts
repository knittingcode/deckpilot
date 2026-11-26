// Gesture presets, the pure part (layout-editor/gesture-presets.ts): what a
// preset does to a control, decided from the device descriptor and the
// actions' declared controls; applied as ordinary bindings or not at all.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { i18n } from "@/i18n"
import { pluginModules } from "@/lib/plugin-actions"
import type {
  ActionInstance,
  ControlBinding,
  ControlId,
  GestureAssignment,
  DeviceLayout,
  DeviceModel,
  GesturePreset,
} from "@/lib/types"
import type { PluginSummary } from "@/plugins/types"
import { BRIGHTNESS, BRIGHTNESS_CONTROL, MK2, deckModule } from "@/test/gesture-presets"

import {
  applyPreset,
  assignmentLabel,
  gesturePresetEntries,
  planPreset,
  presetAction,
  presetName,
  presetSummary,
  presetUnavailable,
  type PresetPlan,
} from "./gesture-presets"

/** A Stream Deck + with dials that only turn (not pressable). */
const PLUS: DeviceModel = {
  ...MK2,
  id: "elgato.streamdeck.plus",
  name: "Stream Deck +",
  capabilities: {
    keys: { rows: 2, columns: 4, image: null },
    encoders: { count: 4, pressable: false },
    touch: null,
  },
}

const key = (index: number) => ({ kind: "key" as const, index })
const dial = (index: number) => ({ kind: "encoder" as const, index })

function layoutWith(bindings: ControlBinding[], model = MK2): DeviceLayout {
  return {
    id: "layout",
    name: "Main",
    model: model.id,
    homePage: "home",
    pages: [{ id: "home", name: "Home", bindings }],
  }
}

/** The preset's gesture at `index`, in declaration order. */
function gestureAt(index: number): GestureAssignment {
  const assignment = BRIGHTNESS_CONTROL.gestures[index]
  if (!assignment) throw new Error(`no gesture ${String(index)}`)
  return assignment
}

const typeOf = (id: string) => (id === BRIGHTNESS.id ? BRIGHTNESS : undefined)

function plan(
  preset: GesturePreset,
  layout: DeviceLayout,
  control: ControlId = key(0),
  model = MK2,
) {
  return planPreset({ preset, model, layout, pageId: "home", control, typeOf })
}

function ok(result: PresetPlan): Extract<PresetPlan, { ok: true }> {
  if (!result.ok) throw new Error(result.reason)
  return result
}

const targets = (n: number) =>
  Array.from({ length: n }, (_, i) => ({ type: "action" as const, action: `a${String(i)}` }))

describe("planning a preset for a control", () => {
  it("puts every gesture on an empty MK.2 key, in gesture mode", () => {
    const result = ok(plan(BRIGHTNESS_CONTROL, layoutWith([])))
    expect(result.items.map((i) => [i.assignment.gesture, i.trigger.type])).toEqual([
      ["click", "click"],
      ["doubleClick", "doubleClick"],
      ["longPress", "longPress"],
    ])
    expect(result.skipped).toEqual([])
  })

  it("keeps a single press and release raw: the press fires at once", () => {
    const raw: GesturePreset = {
      ...BRIGHTNESS_CONTROL,
      gestures: [
        { ...gestureAt(0), gesture: "click" },
        { ...gestureAt(1), gesture: "release" },
      ],
    }
    const result = ok(plan(raw, layoutWith([])))
    expect(result.items.map((i) => i.trigger.type)).toEqual(["press", "release"])
  })

  it("refuses atomically what the device cannot do, unless partial is allowed", () => {
    // A dial that only turns: no single, double or long press at all.
    const turning: GesturePreset = {
      ...BRIGHTNESS_CONTROL,
      gestures: [
        ...BRIGHTNESS_CONTROL.gestures,
        {
          gesture: "rotateClockwise",
          action: "deck.brightness",
          parameters: { change: { type: "choice", value: "brighter" } },
          label: "Brighter",
        },
      ],
    }
    const layout = layoutWith([], PLUS)
    const refused = plan(turning, layout, dial(0), PLUS)
    expect(refused.ok).toBe(false)
    if (!refused.ok) expect(refused.reason).toMatch(/^Dial 1 cannot take its single press/)
    const partial = ok(plan({ ...turning, allowPartial: true }, layout, dial(0), PLUS))
    expect(partial.items.map((i) => i.trigger.type)).toEqual(["rotateClockwise"])
    expect(partial.skipped.map((s) => s.gesture)).toEqual(["click", "doubleClick", "longPress"])
    // Nothing fits at all: refused even when partial.
    const none = plan({ ...BRIGHTNESS_CONTROL, allowPartial: true }, layout, dial(0), PLUS)
    expect(none.ok).toBe(false)
    // A control the device does not have.
    expect(plan(BRIGHTNESS_CONTROL, layoutWith([]), key(40)).ok).toBe(false)
  })

  it("refuses a key whose release could not stay next to the new gestures", () => {
    const layout = layoutWith([
      { control: key(0), trigger: { type: "press" }, target: { type: "back" } },
      { control: key(0), trigger: { type: "release" }, target: { type: "back" } },
    ])
    const result = plan(BRIGHTNESS_CONTROL, layout)
    expect(result.ok).toBe(false)
    if (!result.ok) expect(result.reason).toContain("has a Release action")
  })

  it("leaves a release out on a key that already has gestures, when partial", () => {
    const raw: GesturePreset = {
      ...BRIGHTNESS_CONTROL,
      allowPartial: true,
      gestures: [gestureAt(0), { ...gestureAt(1), gesture: "release" }],
    }
    const layout = layoutWith([
      { control: key(0), trigger: { type: "longPress" }, target: { type: "back" } },
    ])
    const result = ok(plan(raw, layout))
    expect(result.items.map((i) => i.trigger.type)).toEqual(["click"])
    expect(result.skipped.map((s) => s.gesture)).toEqual(["release"])
    expect(plan({ ...raw, allowPartial: false }, layout).ok).toBe(false)
  })
})

describe("applying a plan", () => {
  it("binds an empty key, titled with the preset, as plain bindings", () => {
    const layout = layoutWith([])
    const result = ok(plan(BRIGHTNESS_CONTROL, layout))
    const next = applyPreset(layout, "home", key(0), result, targets(3), "Brightness Control")
    expect(next.pages[0]?.bindings).toEqual([
      {
        control: key(0),
        trigger: { type: "click" },
        target: { type: "action", action: "a0" },
        label: "Brightness Control",
      },
      {
        control: key(0),
        trigger: { type: "doubleClick" },
        target: { type: "action", action: "a1" },
        label: null,
      },
      {
        control: key(0),
        trigger: { type: "longPress" },
        target: { type: "action", action: "a2" },
        label: null,
      },
    ])
    // Nothing of the preset is stored.
    expect(JSON.stringify(next)).not.toContain("brightness-control")
    // The input layout is not changed.
    expect(layout.pages[0]?.bindings).toEqual([])
  })

  it("never switches an occupied raw key to gestures (D2): the press stays immediate", () => {
    const layout = layoutWith([
      { control: key(0), trigger: { type: "press" }, target: { type: "back" }, label: "Mine" },
    ])
    for (const preset of [
      BRIGHTNESS_CONTROL,
      { ...BRIGHTNESS_CONTROL, gestures: [gestureAt(2)] },
    ]) {
      const refused = plan(preset, layout)
      expect(refused.ok).toBe(false)
      if (refused.ok) throw new Error("unreachable")
      expect(refused.reason).toBe(
        "Key 1 acts as soon as it is pressed, and this preset does not change that. Drop it on its double click or long press in the Inspector first (it asks before switching the key to gestures), or use an empty key.",
      )
    }
    // Even partial: nothing of it goes on the raw key.
    expect(plan({ ...BRIGHTNESS_CONTROL, allowPartial: true }, layout).ok).toBe(false)
    // A preset that keeps the key raw (single press and release) still fits.
    const raw: GesturePreset = {
      ...BRIGHTNESS_CONTROL,
      gestures: [gestureAt(0)],
    }
    expect(ok(plan(raw, layout)).items.map((i) => i.trigger)).toEqual([{ type: "press" }])
  })

  it("keeps a key's look and other gestures, replacing only the declared ones", () => {
    const look = { background: "#112233", textColor: "#ffcc00" }
    // The key is in gesture mode already (the user switched it in the Inspector).
    const layout = layoutWith([
      {
        control: key(0),
        trigger: { type: "click" },
        target: { type: "back" },
        label: "Mine",
        appearance: look,
      },
      {
        control: key(0),
        trigger: { type: "doubleClick" },
        target: { type: "action", action: "other" },
      },
      { control: key(1), trigger: { type: "press" }, target: { type: "back" } },
    ])
    const longOnly: GesturePreset = {
      ...BRIGHTNESS_CONTROL,
      gestures: [gestureAt(2)],
    }
    const result = ok(plan(longOnly, layout))
    const next = applyPreset(layout, "home", key(0), result, targets(1), "Brightness Control")
    expect(next.pages[0]?.bindings).toEqual([
      {
        control: key(0),
        trigger: { type: "click" },
        target: { type: "back" },
        label: "Mine",
        appearance: look,
      },
      {
        control: key(0),
        trigger: { type: "doubleClick" },
        target: { type: "action", action: "other" },
      },
      { control: key(1), trigger: { type: "press" }, target: { type: "back" } },
      {
        control: key(0),
        trigger: { type: "longPress" },
        target: { type: "action", action: "a0" },
        label: null,
      },
    ])
    // The whole preset on it: the click's target changes, its look stays.
    const all = ok(plan(BRIGHTNESS_CONTROL, layout))
    const replaced = applyPreset(layout, "home", key(0), all, targets(3), "Brightness Control")
    const click = replaced.pages[0]?.bindings.find(
      (b) => b.control.index === 0 && b.trigger.type === "click",
    )
    expect(click).toEqual({
      control: key(0),
      trigger: { type: "click" },
      target: { type: "action", action: "a0" },
      label: "Mine",
      appearance: look,
    })
  })
})

describe("the actions a preset stands for", () => {
  const made = (change: Partial<ActionInstance>): ActionInstance => ({
    id: "x",
    name: "Decrease brightness by 10%",
    actionType: BRIGHTNESS.id,
    parameters: {
      change: { type: "choice", value: "dimmer" },
      level: { type: "integer", value: 100 },
      step: { type: "integer", value: 10 },
    },
    description: "",
    icon: "monitor",
    label: null,
    ...change,
  })
  const click = gestureAt(0)

  it("creates the action named for the gesture, with the preset's parameters over the defaults", () => {
    expect(presetAction([], BRIGHTNESS_CONTROL, click, BRIGHTNESS)).toEqual({
      input: {
        name: "Decrease brightness by 10%",
        actionType: "deck.brightness",
        parameters: {
          change: { type: "choice", value: "dimmer" },
          level: { type: "integer", value: 100 },
          step: { type: "integer", value: 10 },
        },
        description: "",
        icon: "monitor",
        label: null,
      },
    })
  })

  it("reuses an action that is exactly it, and no other", () => {
    expect(presetAction([made({})], BRIGHTNESS_CONTROL, click, BRIGHTNESS)).toEqual({
      existing: made({}),
    })
    // Renamed, other settings or a label of its own: a new one.
    for (const other of [
      made({ name: "Dim" }),
      made({ parameters: { change: { type: "choice", value: "brighter" } } }),
      made({ label: "-" }),
    ]) {
      expect("input" in presetAction([other], BRIGHTNESS_CONTROL, click, BRIGHTNESS)).toBe(true)
    }
  })
})

describe("availability and texts", () => {
  beforeEach(() => {
    vi.stubEnv("VITE_DECKPILOT_PLUGINS", "1")
  })
  afterEach(() => {
    vi.unstubAllEnvs()
  })

  it("is unavailable while an action it needs cannot run", () => {
    expect(presetUnavailable([deckModule()], BRIGHTNESS_CONTROL, typeOf)).toBeNull()
    expect(presetUnavailable([deckModule({ enabled: false })], BRIGHTNESS_CONTROL, typeOf)).toMatch(
      /Deck module is turned off/,
    )
    expect(presetUnavailable([deckModule()], BRIGHTNESS_CONTROL, () => undefined)).toMatch(
      /not available/,
    )
  })

  it("lists the presets of supported modules", () => {
    const entries = gesturePresetEntries([deckModule(), deckModule({ id: "x", supported: false })])
    expect(entries.map((e) => e.preset.id)).toEqual(["deck.brightness-control"])
  })

  it("keeps a plugin's presets that stay inside its namespace", () => {
    const own: GesturePreset = {
      ...BRIGHTNESS_CONTROL,
      id: "acme.lights.dimmer",
      gestures: [{ ...gestureAt(0), action: "acme.lights.dim" }],
    }
    const foreign: GesturePreset = {
      ...own,
      id: "acme.lights.other",
      gestures: BRIGHTNESS_CONTROL.gestures,
    }
    const summary: PluginSummary = {
      id: "acme.lights",
      name: "Lights",
      publisher: "Acme",
      version: "1.0.0",
      state: "enabled",
      origin: { type: "sideload" },
      actions: [],
      gesturePresets: [own, foreign],
    }
    const [module] = pluginModules([summary])
    expect(module?.gesturePresets?.map((p) => p.id)).toEqual(["acme.lights.dimmer"])
  })

  it("names a built-in preset and its gestures in the window's language", async () => {
    expect(presetName(BRIGHTNESS_CONTROL)).toBe("Brightness Control")
    expect(presetSummary(BRIGHTNESS_CONTROL)).toBe(
      [
        "Single press: Decrease brightness by 10%",
        "Double click: Set brightness to 100%",
        "Long press: Set brightness to 50%",
      ].join("\n"),
    )
    await i18n.changeLanguage("pl")
    try {
      expect(presetName(BRIGHTNESS_CONTROL)).toBe("Sterowanie jasnością")
      expect(assignmentLabel(BRIGHTNESS_CONTROL, gestureAt(0))).toBe("Zmniejsz jasność o 10%")
      // A plugin's texts come translated from the agent: kept as they are.
      const plugin = { ...BRIGHTNESS_CONTROL, id: "acme.lights.dimmer", name: "Ściemniacz" }
      expect(presetName(plugin)).toBe("Ściemniacz")
    } finally {
      await i18n.changeLanguage("en")
    }
  })
})
