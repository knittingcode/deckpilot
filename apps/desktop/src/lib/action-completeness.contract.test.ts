import { describe, expect, it } from "vitest"

import en from "@/i18n/locales/en/actions.json"
import enErrors from "@/i18n/locales/en/errors.json"
import pl from "@/i18n/locales/pl/actions.json"
import plErrors from "@/i18n/locales/pl/errors.json"

import {
  CONFIGURATION_INCOMPLETE,
  type IncompleteUse,
  type ProfileCompleteness,
  incompleteAction,
  incompleteBinding,
  incompleteNode,
  incompleteStep,
  isIncompleteIssue,
  missingParameters,
  refusedAsIncomplete,
} from "./action-completeness"
import type { ActionType, GraphIssue } from "./types"

// The contract of draft actions against what the app backend and the
// agent send (`deckpilot_core::completeness`). The JSON below is what the
// Rust tests `completeness_serializes_as_the_app_reads_it` produce.

const openApp: Pick<ActionType, "parameters"> = {
  parameters: [
    {
      key: "application",
      label: "Application",
      kind: { type: "text", multiline: false },
      required: true,
    },
    {
      key: "arguments",
      label: "Arguments",
      kind: { type: "lines" },
      required: false,
    },
  ],
}

const profile: ProfileCompleteness = {
  incompleteActions: { "a-1": ["application"] },
  incompleteUses: [
    {
      site: {
        kind: "binding",
        layout: "l-1",
        page: "p-1",
        control: { kind: "key", index: 0 },
        trigger: { type: "longPress" },
      },
      action: "a-1",
      missing: ["application"],
    },
    { site: { kind: "step", workflow: "w-1", index: 1 }, action: "a-1", missing: ["application"] },
    {
      site: { kind: "node", workflow: "w-2", node: "open" },
      action: "a-1",
      missing: ["application"],
    },
  ] satisfies IncompleteUse[],
}

describe("the draft action contract", () => {
  it("reads incomplete actions and uses as the backend reports them", () => {
    expect(incompleteAction(profile, "a-1")).toEqual(["application"])
    expect(incompleteAction(profile, "a-2")).toEqual([])
    expect(incompleteAction({}, "a-1")).toEqual([])
    const binding = {
      layout: "l-1",
      page: "p-1",
      control: { kind: "key", index: 0 } as const,
      trigger: { type: "longPress" } as const,
    }
    expect(incompleteBinding(profile, binding)).toEqual(["application"])
    expect(incompleteBinding(profile, { ...binding, trigger: { type: "click" } })).toEqual([])
    expect(incompleteStep(profile, "w-1", 1)).toEqual(["application"])
    expect(incompleteStep(profile, "w-1", 0)).toEqual([])
    expect(incompleteNode(profile, "w-2", "open")).toEqual(["application"])
  })

  it("computes missing parameters as the backend does", () => {
    expect(missingParameters(openApp, {})).toEqual(["application"])
    expect(missingParameters(openApp, { application: { type: "text", value: "  " } })).toEqual([
      "application",
    ])
    expect(missingParameters(openApp, { application: { type: "text", value: "Safari" } })).toEqual(
      [],
    )
    // A reference is a value until the run resolves it.
    expect(
      missingParameters(openApp, { application: { type: "reference", value: "shared.app" } }),
    ).toEqual([])
    expect(missingParameters(openApp, { application: { type: "template", value: " " } })).toEqual([
      "application",
    ])
    // A declared default fills a required parameter.
    const withDefault = {
      parameters: [{ ...openApp.parameters[0], default: { type: "text", value: "Finder" } }],
    } as Pick<ActionType, "parameters">
    expect(missingParameters(withDefault, {})).toEqual([])
  })

  it("reads a refused run", () => {
    expect(
      refusedAsIncomplete({
        code: CONFIGURATION_INCOMPLETE,
        data: { missing: "application,arguments" },
      }),
    ).toEqual(["application", "arguments"])
    expect(refusedAsIncomplete({ code: "reference-missing" })).toBeNull()
    expect(refusedAsIncomplete({})).toBeNull()
  })

  it("tells incomplete graph issues apart", () => {
    const incomplete: GraphIssue = {
      node: "open",
      severity: "error",
      message: "Open application: parameter `application` is required",
      code: "configuration-incomplete",
      parameter: "application",
    }
    expect(isIncompleteIssue(incomplete)).toBe(true)
    expect(isIncompleteIssue({ severity: "error", message: "x" })).toBe(false)
  })

  it("has its texts in every language", () => {
    expect(en.completeness.badge).toBe("Needs configuration")
    expect(pl.completeness.badge).toBe("Wymaga konfiguracji")
    for (const key of ["badge", "missing", "runRefused"] as const) {
      expect(en.completeness[key]).toBeTruthy()
      expect(pl.completeness[key]).toBeTruthy()
    }
    expect(enErrors.pluginAction["configuration-incomplete"]).toBeTruthy()
    expect(plErrors.pluginAction["configuration-incomplete"]).toBeTruthy()
  })
})
