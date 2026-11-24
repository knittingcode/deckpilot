import { describe, expect, it } from "vitest"

import { bindingAccepts, supportsContext } from "@/lib/action-context"
import { moduleInfo } from "@/lib/test-fixtures"
import type { ActionType } from "@/lib/types"
import { moduleGroups } from "@/workflow-builder/catalog"

import { keyLibrary } from "./library-context"
import type { TargetGroup } from "./targets"

// Wait and Delay as the agent sends them since the built-in declarations
// (deckpilot-core `contexts`, general module): a Multi Action step or a
// workflow node, never a key or a gesture; Wait deprecated for Delay. The
// filters read only these fields, never an id or a name.
const declared = (id: string, name: string, deprecated = false): ActionType => ({
  id,
  name,
  description: "",
  category: "General",
  parameters: [],
  requiresProject: false,
  contexts: ["sequence", "workflow"],
  ...(deprecated ? { deprecated } : {}),
})
const wait = declared("general.wait", "Wait", true)
const delay = declared("general.delay", "Delay")
const nextPage: ActionType = {
  ...declared("deck.next-page", "Next page"),
  contexts: ["key", "gesture"],
}
const types = new Map([wait, delay, nextPage].map((t) => [t.id, t]))

const saved = (id: string, actionType: string): TargetGroup["options"][number] => ({
  value: `action:${id}`,
  label: id,
  target: { type: "action", action: id },
  kind: "action",
  actionType,
})

describe("Wait and Delay with the agent's contexts", () => {
  it("are not in the key library: neither the types nor saved actions of them", () => {
    const library = keyLibrary(
      [
        {
          id: "actions",
          label: "Actions",
          options: [saved("pause", wait.id), saved("short", delay.id), saved("next", nextPage.id)],
        },
      ],
      [
        {
          label: "General",
          entries: [wait, delay, nextPage].map((type) => ({ type, module: "general" })),
        },
      ],
      [],
      (id) => types.get(id),
    )
    expect(library.catalog.flatMap((g) => g.entries.map((e) => e.type.id))).toEqual([nextPage.id])
    expect(library.groups.flatMap((g) => g.options.map((o) => o.label))).toEqual(["next"])
    expect(library.displays).toEqual({ options: [], catalog: [] })
  })

  it("do not go on a key's press or click, nor on a gesture", () => {
    for (const type of [wait, delay]) {
      for (const trigger of [
        { type: "press" },
        { type: "click" },
        { type: "doubleClick" },
        { type: "longPress" },
      ] as const) {
        expect(bindingAccepts(type, trigger)).not.toBe(true)
      }
    }
    expect(bindingAccepts(nextPage, { type: "doubleClick" })).toBe(true)
  })

  it("are Multi Action steps and workflow nodes", () => {
    for (const type of [wait, delay]) {
      expect(supportsContext(type, "sequence")).toBe(true)
      expect(supportsContext(type, "workflow")).toBe(true)
    }
    const [general] = moduleGroups(
      [moduleInfo({ id: "general", name: "General", actionTypes: [wait, delay, nextPage] })],
      [
        { id: "pause", name: "Pause", actionType: wait.id, parameters: {}, icon: null },
        { id: "short", name: "Short delay", actionType: delay.id, parameters: {}, icon: null },
        { id: "next", name: "Forward", actionType: nextPage.id, parameters: {}, icon: null },
      ],
    )
    // New nodes: Delay; Wait is deprecated for it and not offered for new ones.
    expect(general?.types.map((t) => t.id)).toEqual([delay.id])
    // Saved actions of both stay usable as nodes; a key-only one is not one.
    expect(general?.entries.map((e) => e.title)).toEqual(["Pause", "Short delay"])
  })
})
