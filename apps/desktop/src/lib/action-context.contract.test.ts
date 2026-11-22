import { describe, expect, it } from "vitest"

import {
  type ContextIncompatibility,
  contextOf,
  effectiveContexts,
  isDisplayOnly,
  supportsContext,
} from "./action-context"
import type { ActionType } from "./types"

// The frozen contract of the action-context audit (§4) against what the
// agent and app backend send (`deckpilot_core::contexts`).

const type = (contexts?: ActionType["contexts"]) => (contexts ? { contexts } : {})

describe("the action context contract", () => {
  it("treats a type without contexts as allowed everywhere", () => {
    expect(effectiveContexts(type()).size).toBe(5)
    for (const c of ["key", "gesture", "display", "sequence", "workflow"] as const) {
      expect(supportsContext(type(), c)).toBe(true)
    }
  })

  it("reads the reason exactly as the backend serializes it", () => {
    // `IncompatibilityReason::ContextUnsupported`, as `save_layout` reports it.
    const backend: ContextIncompatibility = {
      type: "contextUnsupported",
      context: "key",
      supported: ["sequence", "workflow"],
    }
    expect(supportsContext(type(["sequence", "workflow"]), "key")).toEqual(backend)
  })

  it("derives the context of a use from its slot only", () => {
    expect(contextOf({ kind: "binding", trigger: { type: "press" } })).toBe("key")
    expect(contextOf({ kind: "binding", trigger: { type: "click" } })).toBe("key")
    expect(contextOf({ kind: "binding", trigger: { type: "longPress" } })).toBe("gesture")
    expect(contextOf({ kind: "binding", trigger: { type: "doubleClick" } })).toBe("gesture")
    expect(contextOf({ kind: "step" })).toBe("sequence")
    expect(contextOf({ kind: "node" })).toBe("workflow")
  })

  it("calls display without key display-only", () => {
    expect(isDisplayOnly(type(["display"]))).toBe(true)
    expect(isDisplayOnly(type(["key", "display", "gesture"]))).toBe(false)
    expect(isDisplayOnly(type())).toBe(false)
  })
})
