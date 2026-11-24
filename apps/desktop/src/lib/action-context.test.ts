import { afterEach, describe, expect, it } from "vitest"

import { i18n } from "@/i18n"

import {
  bindingAccepts,
  contextOf,
  effectiveContexts,
  isDisplayOnly,
  placeableOnKey,
  supportsContext,
} from "./action-context"
import { contextReason } from "./action-context-text"
import type { ActionType } from "./types"

const type = (contexts?: ActionType["contexts"]) => (contexts ? { contexts } : {})

afterEach(async () => {
  await i18n.changeLanguage("en")
})

describe("action contexts", () => {
  it("are all five when a type declares none, so nothing changes without declarations", () => {
    expect([...effectiveContexts(type())].sort()).toEqual(
      ["display", "gesture", "key", "sequence", "workflow"].sort(),
    )
    for (const c of ["key", "sequence", "workflow", "display", "gesture"] as const) {
      expect(supportsContext(type(), c)).toBe(true)
    }
    expect(isDisplayOnly(type())).toBe(false)
  })

  it("never come from an id or a name: a type named like Wait that declares nothing goes everywhere", () => {
    const wait: ActionType = {
      id: "general.wait",
      name: "Wait",
      description: "Waits.",
      category: "General",
      parameters: [],
      requiresProject: false,
    }
    expect(supportsContext(wait, "key")).toBe(true)
    expect(bindingAccepts(wait, { type: "doubleClick" })).toBe(true)
    expect(placeableOnKey(wait)).toBe(true)
  })

  it("say where a use is: press, click, rotation and touch are a key; double click and long press a gesture", () => {
    for (const trigger of [
      { type: "press" },
      { type: "release" },
      { type: "click" },
      { type: "rotateClockwise" },
      { type: "touch", gesture: "tap" },
    ] as const) {
      expect(contextOf({ kind: "binding", trigger })).toBe("key")
    }
    expect(contextOf({ kind: "binding", trigger: { type: "doubleClick" } })).toBe("gesture")
    expect(contextOf({ kind: "binding", trigger: { type: "longPress" } })).toBe("gesture")
    expect(contextOf({ kind: "step" })).toBe("sequence")
    expect(contextOf({ kind: "node" })).toBe("workflow")
  })

  it("give a structured reason, in a fixed order", () => {
    expect(supportsContext(type(["workflow", "sequence"]), "key")).toEqual({
      type: "contextUnsupported",
      context: "key",
      supported: ["sequence", "workflow"],
    })
  })

  it("let a key take a display-only type, but not a gesture", () => {
    const display = type(["display"])
    expect(isDisplayOnly(display)).toBe(true)
    expect(bindingAccepts(display, { type: "press" })).toBe(true)
    expect(bindingAccepts(display, { type: "longPress" })).toMatchObject({ context: "gesture" })
    expect(placeableOnKey(display)).toBe(true)
    expect(placeableOnKey(type(["sequence", "workflow"]))).toBe(false)
    expect(isDisplayOnly(type(["key", "display"]))).toBe(false)
  })

  it("are worded from the structured fields, in English and Polish", async () => {
    const reason = supportsContext(type(["sequence", "workflow"]), "key")
    if (reason === true) throw new Error("expected a reason")
    expect(contextReason(reason)).toBe(
      "This action does not work here: Key. It works as: Multi Action step, Workflow node.",
    )
    await i18n.changeLanguage("pl")
    expect(contextReason(reason)).toBe(
      "Ta akcja nie działa tutaj: Klawisz. Działa jako: Krok multiakcji, Węzeł przepływu pracy.",
    )
    expect(
      [
        "editor:migrate.wrapSequence",
        "editor:migrate.moveWorkflow",
        "editor:migrate.removeStep",
        "editor:migrate.keep",
        "editor:context.notHere",
        "editor:library.displays",
      ].map((key) => i18n.t(key as never)),
    ).toEqual([
      "Opakuj w multiakcję",
      "Przenieś do przepływu pracy",
      "Usuń krok",
      "Zostaw",
      "Niedostępne tutaj",
      "Wyświetlacze",
    ])
  })
})
