// What a blocked plugin action waits for, from the agent's structured
// grants only; and how a `tools` grant's risk is described.
import { afterEach, describe, expect, it, vi } from "vitest"

import { missingAction } from "@/lib/action-availability"
import { grantableOf } from "@/lib/plugin-actions"
import { moduleInfo } from "@/lib/test-fixtures"

import { grantsByTier } from "./permissions"
import type { GrantInfo, PluginDetails } from "./types"

const DOCKER = "deckpilot.docker"
const modify: GrantInfo = {
  key: "tools:docker:modify",
  kind: "tools",
  tier: "high",
  status: "notGranted",
  scope: null,
  optional: true,
  actions: ["compose-up"],
}
function details(permissions: GrantInfo[], state = "enabled", available = false): PluginDetails {
  return {
    summary: {
      id: DOCKER,
      name: "Docker",
      publisher: "DeckPilot",
      version: "0.1.0",
      state,
      origin: { type: "bundled" },
      actions: [
        { action: "compose-up", actionType: `${DOCKER}.compose-up`, name: "Compose up", available },
        { action: "status", actionType: `${DOCKER}.status`, name: "Status", available: true },
      ],
    },
    permissions,
  } as unknown as PluginDetails
}

describe("grantableOf", () => {
  it("lists the declared optional permissions a blocked action names and lacks", () => {
    expect(grantableOf(details([modify]))).toEqual({
      [`${DOCKER}.compose-up`]: ["tools:docker:modify"],
    })
  })

  it("lists nothing granted, undeclared, required, for available actions or a plugin that is off", () => {
    expect(grantableOf(details([{ ...modify, status: "granted" }]))).toEqual({})
    expect(grantableOf(details([{ ...modify, actions: ["logs"] }]))).toEqual({})
    expect(grantableOf(details([{ ...modify, optional: false }]))).toEqual({})
    expect(grantableOf(details([modify], "enabled", true))).toEqual({})
    expect(grantableOf(details([modify], "disabled"))).toEqual({})
  })
})

describe("why an assigned plugin action cannot run", () => {
  afterEach(() => {
    vi.unstubAllEnvs()
  })
  const module = (grantable?: Record<string, string[]>, reason = "anything at all") =>
    moduleInfo({
      id: DOCKER,
      name: "Docker",
      enabled: true,
      actionTypes: [
        {
          id: `${DOCKER}.compose-up`,
          name: "Compose up",
          description: "",
          category: "Docker",
          requiresProject: false,
          parameters: [],
        },
      ],
      plugin: {
        state: "enabled",
        unavailable: { [`${DOCKER}.compose-up`]: reason },
        ...(grantable ? { grantable } : {}),
      },
    })

  it("waits for a permission only when the grants say so, whatever the agent's words", () => {
    vi.stubEnv("VITE_DECKPILOT_PLUGINS", "1")
    const typeId = `${DOCKER}.compose-up`
    expect(missingAction([module({ [typeId]: ["tools:docker:modify"] })], typeId)?.reason).toBe(
      "permission",
    )
    // The word "permission" alone no longer decides it.
    expect(missingAction([module({}, "needs the permission `x`")], typeId)?.reason).toBe("busy")
    expect(missingAction([module(undefined, "needs the permission `x`")], typeId)?.reason).toBe(
      "busy",
    )
  })
})

describe("the risk line of a tools grant", () => {
  const grant = (key: string) => ({ key, tier: "high" as const })

  it("describes running tools, from the grant's kind", () => {
    const [high] = grantsByTier([grant("tools:docker:modify")])
    expect(high?.description).toBe(
      "Running tools on your system, within what this plugin supports.",
    )
  })

  it("keeps the tier's words for other kinds, and adds the tools line when mixed", () => {
    const [only] = grantsByTier([grant("privateNetwork")])
    expect(only?.description).not.toContain("Running tools")
    const [mixed] = grantsByTier([grant("privateNetwork"), grant("tools:docker:modify")])
    expect(mixed?.description).toContain(only?.description ?? "")
    expect(mixed?.description).toContain("Running tools on your system")
  })
})
