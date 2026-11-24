import { describe, expect, it } from "vitest"

import type { KeyLabel, LabelBox } from "@/lib/key-labels"
import type { ControlBinding, DeviceLayout, KeyAppearance } from "@/lib/types"

import { applyAppearance, normalizeAppearance } from "./appearance"
import {
  NEW_LABEL_TEXT,
  addLabel,
  copiedLabels,
  defaultPlateOpacity,
  duplicateLabel,
  keepLookOnRemoval,
  labelReports,
  lookTransfer,
  moveLabel,
  movedBy,
  movedLook,
  positionFromBox,
  presetLook,
  removeLabel,
  reorderLabels,
  updateLabel,
  withLabels,
} from "./labels"

const label = (id: string, extra: Partial<KeyLabel> = {}): KeyLabel => ({ id, text: id, ...extra })

const box = (extra: Partial<LabelBox> = {}): LabelBox => ({
  id: "a",
  x: 20,
  y: 50,
  width: 32,
  height: 12,
  canvas: { width: 72, height: 72 },
  layer: "overIcon",
  truncated: false,
  overlapsTitle: false,
  plate: true,
  fontPoints: 10,
  lines: 1,
  ...extra,
})

describe("adding and arranging labels", () => {
  it("adds a bottom label with the project's name in front of the others", () => {
    const added = addLabel([label("a")], "bottom")
    expect(added?.labels.map((l) => l.id)).toEqual(["a", added?.id])
    expect(added?.labels[1]).toEqual({
      id: added?.id,
      text: NEW_LABEL_TEXT,
      position: { mode: "preset", at: "bottom" },
    })
    expect(NEW_LABEL_TEXT).toBe("{{ projects.current.name }}")
  })

  it("adds the ribbon as a full-width band at the bottom, 1 unit in, on the automatic plate", () => {
    expect(presetLook("ribbon")).toEqual({
      position: { mode: "anchored", to: "bottom", inset: 1 },
      width: 100,
      background: "auto",
    })
  })

  it("stops at eight labels", () => {
    const eight = Array.from({ length: 8 }, (_, i) => label(`l${i}`))
    expect(addLabel(eight, "top")).toBeNull()
    expect(duplicateLabel(eight, "l0")).toBeNull()
  })

  it("duplicates with a new id right in front, removes and reorders", () => {
    const labels = [label("a", { size: 12 }), label("b")]
    const copy = duplicateLabel(labels, "a")
    expect(copy?.labels.map((l) => l.id)).toEqual(["a", copy?.id, "b"])
    expect(copy?.labels[1]?.size).toBe(12)
    expect(removeLabel(labels, "a").map((l) => l.id)).toEqual(["b"])
    expect(moveLabel(labels, "a", "front").map((l) => l.id)).toEqual(["b", "a"])
    expect(moveLabel(labels, "a", "back").map((l) => l.id)).toEqual(["a", "b"])
    expect(reorderLabels([label("a"), label("b"), label("c")], 0, 2).map((l) => l.id)).toEqual([
      "b",
      "c",
      "a",
    ])
  })

  it("resets a field with null, and keeps width and max width exclusive", () => {
    const [changed] = updateLabel([label("a", { size: 12, maxWidth: 50 })], "a", {
      size: null,
      width: 80,
    })
    expect(changed).toEqual({ id: "a", text: "a", width: 80 })
  })

  it("copies labels to another key with new, distinct ids", () => {
    const copies = copiedLabels([label("a", { size: 9 }), label("b")])
    expect(copies.map((l) => l.size)).toEqual([9, undefined])
    expect(new Set(copies.map((l) => l.id)).size).toBe(2)
    expect(copies.map((l) => l.id)).not.toContain("a")
  })
})

describe("positions from the renderer's boxes", () => {
  it("keeps the label where it is when the mode changes", () => {
    // Centre (36, 56): the bottom third, the middle column.
    expect(positionFromBox(box(), "absolute")).toEqual({ mode: "absolute", x: 500, y: 778 })
    expect(positionFromBox(box(), "anchored")).toEqual({
      mode: "anchored",
      to: "bottom",
      inset: 10,
    })
    expect(positionFromBox(box(), "preset")).toEqual({ mode: "preset", at: "bottom" })
    // Presets have no middle of the sides.
    expect(positionFromBox(box({ x: 2, y: 30 }), "preset")).toEqual({
      mode: "preset",
      at: "center",
    })
  })

  it("nudges by units, and a drag snaps to guides within 4 units", () => {
    expect(movedBy(box(), 1, 0)).toEqual({ mode: "absolute", x: 514, y: 778 })
    // Across to the centre line; down to the text area's bottom edge (centre 60).
    expect(movedBy(box(), 3, 0, true)).toEqual({ mode: "absolute", x: 500, y: 833 })
    expect(movedBy(box(), 6, -10, true)).toEqual({ mode: "absolute", x: 583, y: 639 })
  })
})

describe("what the Inspector reports", () => {
  const colors = { unresolved: false, textColor: "#ffffff", keyBackground: "#000000" }

  it("warns below a contrast of 3:1, only for a colour or no background", () => {
    const grey = label("a", { color: "#444444", background: "none" })
    expect(labelReports(grey, box(), colors)).toContain("lowContrast")
    expect(labelReports({ ...grey, background: "auto" }, box(), colors)).not.toContain(
      "lowContrast",
    )
    expect(labelReports(label("a", { background: "none" }), box(), colors)).toEqual([])
  })

  it("reports a cut, the title covered, a small size and a missing value", () => {
    expect(
      labelReports(label("a"), box({ truncated: true, overlapsTitle: true, fontPoints: 6 }), {
        ...colors,
        unresolved: true,
      }),
    ).toEqual(["unresolved", "truncated", "overlapsTitle", "small"])
  })
})

describe("the look when the key's shown binding changes", () => {
  const from: KeyAppearance = { background: "#112233", labels: [label("a"), label("b")] }
  const to: KeyAppearance = {
    background: "#ffffff",
    icon: { id: "x" },
    labels: [label("b", { size: 20 }), label("c")],
  }

  it("keeps, replaces, or merges only the labels (the moved ones win)", () => {
    expect(movedLook(from, to, "keep")).toBe(to)
    expect(movedLook(from, to, "replace")).toBe(from)
    expect(movedLook(from, to, "merge")).toEqual({
      background: "#ffffff",
      icon: { id: "x" },
      labels: [label("c"), label("a"), label("b")],
    })
    // Nothing of its own: the whole look moves.
    expect(movedLook(from, undefined, "keep")).toBe(from)
  })

  const binding = (trigger: "click" | "longPress", appearance?: KeyAppearance): ControlBinding => ({
    control: { kind: "key", index: 0 },
    trigger: { type: trigger },
    target: { type: "action", action: trigger },
    label: null,
    ...(appearance ? { appearance } : {}),
  })
  const layout = (bindings: ControlBinding[]): DeviceLayout => ({
    id: "l",
    name: "L",
    model: "m",
    homePage: "home",
    pages: [{ id: "home", name: "Home", bindings }],
  })

  it("moves labels onto a binding without a look, and asks when it has one", () => {
    const control = { kind: "key", index: 0 } as const
    const before = layout([binding("click", from), binding("longPress")])
    const after = layout([binding("longPress")])
    expect(lookTransfer(before, after, "home", control)).toMatchObject({
      conflict: false,
      labels: 2,
    })
    const moved = keepLookOnRemoval(before, after, "home", control)
    expect(moved.pages[0]?.bindings[0]?.appearance).toEqual(from)

    const withOwn = layout([binding("longPress", to)])
    expect(lookTransfer(before, withOwn, "home", control)?.conflict).toBe(true)
    const merged = keepLookOnRemoval(before, withOwn, "home", control, "merge")
    expect(merged.pages[0]?.bindings[0]?.appearance?.labels?.map((l) => l.id)).toEqual([
      "c",
      "a",
      "b",
    ])
  })
})

describe("labels survive other look changes", () => {
  it("are kept by normalizing and by a colour change", () => {
    const labels = [label("a")]
    expect(normalizeAppearance({ labels })).toEqual({ labels })
    expect(withLabels([])({ labels, bold: true })).toEqual({ bold: true })
    const key = {
      control: { kind: "key", index: 0 },
      trigger: { type: "press" },
      target: { type: "back" },
      label: null,
      appearance: { labels },
    } as ControlBinding
    const result = applyAppearance(
      {
        id: "l",
        name: "L",
        model: "m",
        homePage: "h",
        pages: [{ id: "h", name: "H", bindings: [key] }],
      },
      "h",
      [{ kind: "key", index: 0 }],
      { background: "#123456" },
    )
    expect(result.layout.pages[0]?.bindings[0]?.appearance).toEqual({
      background: "#123456",
      labels,
    })
  })
})

describe("the plate's opacity", () => {
  it("is 85 over the title from the contract, and auto where the box does not say why", () => {
    expect(defaultPlateOpacity(label("a"), box({ overlapsTitle: true }))).toBe(85)
    // A plate over a mark (85) or a picture (60): the box does not say which.
    expect(defaultPlateOpacity(label("a"), box())).toBe("auto")
    expect(defaultPlateOpacity(label("a"), undefined)).toBe("auto")
    expect(defaultPlateOpacity(label("a", { background: "#112233" }), box())).toBe(100)
    expect(defaultPlateOpacity(label("a", { background: "none" }), box())).toBeNull()
  })

  it("is never written by other changes", () => {
    const [moved] = updateLabel([label("a")], "a", {
      size: 12,
      position: { mode: "preset", at: "top" },
    })
    expect(moved).not.toHaveProperty("backgroundOpacity")
  })
})
