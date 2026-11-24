// Badges (official, bundled-only) come only from agent-derived fields; the
// hardcoded list is a fallback for agents that do not derive them.

import { describe, expect, it } from "vitest"

import { readyRuntime } from "@/test/plugins"

import { BUNDLED_ONLY_PLUGINS, entryState, isBundledOnly, officialIds } from "../catalog"
import type { AvailablePlugin, MarketplaceSnapshot } from "../types"

const plugin = (id: string, extra: Partial<AvailablePlugin> = {}): AvailablePlugin => ({
  id,
  name: id,
  summary: "",
  publisher: id.split(".")[0] ?? "",
  latest: "1.0.0",
  versions: [{ version: "1.0.0", permissions: [] }],
  ...extra,
})

const deriving = readyRuntime({ features: ["storefront"] })
const older = readyRuntime({ features: [] })

function snapshot(runtime: MarketplaceSnapshot["runtime"], available: AvailablePlugin[]) {
  return { runtime, registry: null, available, installed: [] } satisfies MarketplaceSnapshot
}

const context = {
  installed: undefined,
  update: undefined,
  pending: undefined,
  failure: undefined,
  restorable: false,
  canRun: true,
}

describe("trust badges", () => {
  it("uses the agent's bundledOnly when the agent derives it", () => {
    const workflows = [...BUNDLED_ONLY_PLUGINS][0] as string
    // The agent says nothing: not bundled-only, whatever the list says.
    expect(isBundledOnly(plugin(workflows), deriving)).toBe(false)
    expect(isBundledOnly(plugin("acme.x", { bundledOnly: true }), deriving)).toBe(true)
    expect(entryState(plugin(workflows), { ...context, bundledOnly: false }).kind).toBe("install")
  })

  it("falls back to the list for an agent that does not derive it", () => {
    const workflows = [...BUNDLED_ONLY_PLUGINS][0] as string
    expect(isBundledOnly(plugin(workflows), older)).toBe(true)
    expect(isBundledOnly(plugin(workflows))).toBe(true)
    expect(isBundledOnly(plugin("acme.x"), older)).toBe(false)
  })

  it("marks official plugins by the agent's flag, never by a name", () => {
    const available = [
      plugin("deckpilot.weather", { official: true }),
      plugin("deckpilot.lookalike"),
      plugin("acme.deckpilot-official"),
    ]
    const ids = officialIds(snapshot(deriving, available))
    expect([...ids]).toEqual(["deckpilot.weather"])
    // An older agent: the fallback list counts as well.
    expect(
      officialIds(snapshot(older, available)).has([...BUNDLED_ONLY_PLUGINS][0] as string),
    ).toBe(true)
  })
})
