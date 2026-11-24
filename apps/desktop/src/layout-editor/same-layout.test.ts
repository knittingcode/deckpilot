import { describe, expect, it } from "vitest"

import type { ControlBinding, DeviceLayout } from "@/lib/types"

import { sameLayout } from "./same-layout"

const binding = (label?: string | null): ControlBinding => ({
  control: { kind: "key", index: 1 },
  trigger: { type: "press" },
  target: { type: "action", action: "a-1" },
  ...(label === undefined ? {} : { label }),
})
const layout = (b: ControlBinding): DeviceLayout => ({
  id: "l",
  name: "Main",
  model: "elgato.streamdeck.mk2",
  homePage: "home",
  pages: [{ id: "home", name: "Home", bindings: [b] }],
})

describe("sameLayout", () => {
  it("treats an empty label as no label: null, blank and absent are equal", () => {
    // The draft after a drop (`label: null`) and the agent's stored copy (none).
    expect(sameLayout(layout(binding(null)), layout(binding()))).toBe(true)
    expect(sameLayout(layout(binding("")), layout(binding()))).toBe(true)
    expect(sameLayout(layout(binding("")), layout(binding(null)))).toBe(true)
  })

  it("still sees a label of its own, and any other change", () => {
    expect(sameLayout(layout(binding("Mail")), layout(binding()))).toBe(false)
    expect(sameLayout(layout(binding("Mail")), layout(binding("Docs")))).toBe(false)
    const moved = { ...binding(null), control: { kind: "key", index: 2 } } as ControlBinding
    expect(sameLayout(layout(moved), layout(binding()))).toBe(false)
  })
})
