import { describe, expect, it } from "vitest"

import contract from "./key-labels.contract.json"
import {
  LABEL_DEFAULTS,
  LABEL_LIMITS,
  isFaceLabel,
  isKeyLabel,
  isLabelBox,
  newLabelId,
  type FaceLabel,
  type KeyLabel,
} from "./key-labels"
import type { ControlFace, KeyAppearance, KeyFace } from "./types"

// The frozen key label contract (docs/key-text.md, "Labels") against what
// Rust serializes: `key-labels.contract.json` is checked on the Rust side by
// crates/deckpilot-render/tests/label_contract.rs.

describe("the key label contract", () => {
  it("has the limits and defaults of the core", () => {
    expect(LABEL_LIMITS).toEqual(contract.limits)
    expect({
      size: LABEL_DEFAULTS.size,
      lines: LABEL_DEFAULTS.lines,
      minSize: LABEL_DEFAULTS.minSize,
      padding: LABEL_DEFAULTS.padding,
      radius: LABEL_DEFAULTS.radius,
      plateOpacity: LABEL_DEFAULTS.plateOpacity,
      colorPlateOpacity: LABEL_DEFAULTS.colorPlateOpacity,
    }).toEqual(contract.defaults)
  })

  it("reads every label the core writes", () => {
    for (const label of contract.keyLabels) {
      expect(isKeyLabel(label), JSON.stringify(label)).toBe(true)
    }
  })

  it("refuses what the core refuses", () => {
    for (const label of contract.invalidKeyLabels) {
      expect(isKeyLabel(label), JSON.stringify(label)).toBe(false)
    }
    expect(isKeyLabel({ id: "x", text: "x".repeat(LABEL_LIMITS.template + 1) })).toBe(false)
    expect(isKeyLabel({ id: "x", text: "ż".repeat(LABEL_LIMITS.template) })).toBe(true)
  })

  it("reads faces and boxes as the backend sends them", () => {
    for (const face of contract.faceLabels) expect(isFaceLabel(face)).toBe(true)
    for (const box of contract.labelBoxes) expect(isLabelBox(box)).toBe(true)
  })

  it("hangs labels on the look, the face and the control face", () => {
    const labels = contract.keyLabels as KeyLabel[]
    const look: KeyAppearance = { labels }
    const face: KeyFace = { style: "action", title: "", labels: contract.faceLabels as FaceLabel[] }
    const control: Pick<ControlFace, "labelBoxes"> = {
      labelBoxes: contract.labelBoxes as ControlFace["labelBoxes"],
    }
    expect(look.labels).toHaveLength(labels.length)
    expect(face.labels).toHaveLength(2)
    expect(control.labelBoxes).toHaveLength(2)
  })

  it("makes new ids of eight hex digits", () => {
    const id = newLabelId(["00000000"])
    expect(id).toMatch(/^[0-9a-f]{8}$/)
    expect(isKeyLabel({ id, text: "x" })).toBe(true)
  })
})
