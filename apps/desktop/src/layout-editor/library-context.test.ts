import { describe, expect, it } from "vitest"

import type { ActionType } from "@/lib/types"

import { keyLibrary } from "./library-context"
import type { TargetGroup } from "./targets"

const type = (id: string, contexts?: ActionType["contexts"]): ActionType => ({
  id,
  name: id,
  description: "",
  category: "General",
  parameters: [],
  requiresProject: false,
  ...(contexts ? { contexts } : {}),
})
const types = new Map(
  [
    type("t.key", ["key", "gesture"]),
    type("t.step", ["sequence", "workflow"]),
    type("t.display", ["display"]),
    type("t.legacy"),
  ].map((t) => [t.id, t]),
)
const groups: TargetGroup[] = [
  {
    id: "actions",
    label: "Actions",
    options: ["t.key", "t.step", "t.display", "t.legacy"].map((id) => ({
      value: `action:${id}`,
      label: id,
      target: { type: "action", action: id },
      kind: "action" as const,
      actionType: id,
    })),
  },
]
const catalog = [
  { label: "General", entries: [...types.values()].map((t) => ({ type: t, module: "m" })) },
]

describe("the key library", () => {
  it("hides step and node actions, lists display-only ones apart, keeps the rest", () => {
    const library = keyLibrary(groups, catalog, [], (id) => types.get(id))
    expect(library.groups[0]?.options.map((o) => o.actionType)).toEqual(["t.key", "t.legacy"])
    expect(library.catalog[0]?.entries.map((e) => e.type.id)).toEqual(["t.key", "t.legacy"])
    expect(library.displays.options.map((o) => o.actionType)).toEqual(["t.display"])
    expect(library.displays.catalog.map((e) => e.type.id)).toEqual(["t.display"])
  })

  it("is unchanged when no type declares contexts", () => {
    const legacy = new Map([...types.keys()].map((id) => [id, type(id)]))
    const legacyCatalog = [
      { label: "General", entries: [...legacy.values()].map((t) => ({ type: t, module: "m" })) },
    ]
    const library = keyLibrary(groups, legacyCatalog, [], (id) => legacy.get(id))
    expect(library.catalog).toEqual(legacyCatalog)
    expect(library.groups).toEqual(groups)
    expect(library.displays).toEqual({ options: [], catalog: [] })
  })
})
