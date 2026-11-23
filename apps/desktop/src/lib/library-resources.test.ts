import { describe, expect, it } from "vitest"

import { isDroppable, workflowResourceEntries } from "./library-resources"
import type { LibraryResourceEntry } from "./library-resources"
import type { WorkflowEntry } from "./types"

function find(listed: LibraryResourceEntry[], id: string): LibraryResourceEntry {
  const found = listed.find((e) => e.id === id)
  if (!found) throw new Error(`no entry ${id}`)
  return found
}

function entry(id: string, name: string, more: Partial<WorkflowEntry> = {}): WorkflowEntry {
  return {
    id,
    name,
    kind: "graph",
    hasTriggers: false,
    block: null,
    references: 0,
    fingerprint: "0-0",
    ...more,
  }
}

describe("workflowResourceEntries", () => {
  const entries = [
    entry("w2", "Release", { references: 3 }),
    entry("w1", "Deploy"),
    entry("s1", "Steps", { kind: "steps" }),
    entry("n1", "Needs input", {
      block: { code: "needsInputs", message: "Needs input needs a value" },
    }),
    entry("b1", "Broken", { block: { code: "invalidGraph", message: "no trigger" } }),
  ]

  it("refers to the workflow by id, ordered by name, in its section", () => {
    const listed = workflowResourceEntries(entries, true)
    expect(listed.map((e) => e.label)).toEqual([
      "Broken",
      "Deploy",
      "Needs input",
      "Release",
      "Steps",
    ])
    const release = find(listed, "w2")
    expect(release).toMatchObject({
      key: "workflow:w2",
      target: { type: "workflow", workflow: "w2" },
      section: { plugin: "deckpilot.workflows", kind: "workflow" },
      usages: 3,
      state: "ready",
    })
    // Multi Actions are the core's Sequences.
    expect(find(listed, "s1").section).toEqual({ plugin: null, kind: "sequence" })
  })

  it("lists workflows that need inputs but cannot drop them", () => {
    const listed = workflowResourceEntries(entries, true)
    const needs = find(listed, "n1")
    expect(needs).toMatchObject({ state: "blocked", reason: "needsInputs" })
    expect(isDroppable(needs)).toBe(false)
    // An invalid graph can be placed, with a warning.
    const broken = find(listed, "b1")
    expect(broken).toMatchObject({ state: "ready", reason: "invalidGraph" })
    expect(isDroppable(broken)).toBe(true)
  })

  it("keeps graph workflows read-only while the plugin is off, and Multi Actions ready", () => {
    const listed = workflowResourceEntries(entries, false)
    for (const e of listed.filter((e) => e.section.kind === "workflow")) {
      expect(e).toMatchObject({ state: "readOnly", reason: "workflowsOff" })
      expect(isDroppable(e)).toBe(false)
    }
    expect(find(listed, "s1").state).toBe("ready")
  })
})
