import { describe, expect, it } from "vitest"

import { bindingMigrations, movedToWorkflow, wrappedInMultiAction } from "./context-migration"

const reason = (supported: ("key" | "sequence" | "workflow" | "display" | "gesture")[]) => ({
  type: "contextUnsupported" as const,
  context: "key" as const,
  supported,
})

describe("migrations of saved uses", () => {
  it("are offered by what the action supports, a workflow only with the Workflows plugin", () => {
    expect(bindingMigrations(reason(["sequence", "workflow"]), true)).toEqual({
      wrap: true,
      move: true,
    })
    expect(bindingMigrations(reason(["sequence", "workflow"]), false)).toEqual({
      wrap: true,
      move: false,
    })
    expect(bindingMigrations(reason(["workflow"]), true)).toEqual({ wrap: false, move: true })
    expect(bindingMigrations(reason(["display"]), true)).toEqual({ wrap: false, move: false })
  })

  it("wrap the action as the only step of a new Multi Action", () => {
    const workflow = wrappedInMultiAction([{ name: "Write log" }], { id: "log", name: "Write log" })
    expect(workflow.name).toBe("Write log 2")
    expect(workflow.graph).toBeUndefined()
    expect(workflow.steps.map((s) => s.action)).toEqual(["log"])
  })

  it("move the action into a new workflow: its trigger, then the action", () => {
    const workflow = movedToWorkflow([], { id: "prs", name: "Pull requests" })
    const graph = workflow.graph
    if (!graph) throw new Error("expected a graph")
    expect(graph.nodes.map((n) => n.type)).toEqual(["trigger", "action"])
    const action = graph.nodes[1]
    expect(action).toMatchObject({ type: "action", action: "prs" })
    expect(graph.edges).toEqual([{ from: "start", port: "out", to: action?.id }])
  })
})
