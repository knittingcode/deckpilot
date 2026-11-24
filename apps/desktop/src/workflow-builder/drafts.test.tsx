// @vitest-environment jsdom
// Draft nodes (`lib/action-completeness.ts`): a node whose action's
// required settings are not filled in is marked "Needs configuration" and
// its missing settings say so; it does not keep the workflow from being
// saved (it does not run until configured).
import "@testing-library/jest-dom/vitest"

import { cleanup, render, screen, within } from "@testing-library/react"
import { ReactFlowProvider } from "@xyflow/react"
import { afterEach, describe, expect, it } from "vitest"

import type { ActionInstance, ActionType, GraphIssue, GraphNode, WorkflowGraph } from "@/lib/types"

import type { Lookup } from "./describe"
import { DEFAULT_POLICY, blockingIssues } from "./graph"
import { NodeInspector } from "./inspector"
import { NodeView } from "./node-view"

const at = { x: 0, y: 0 }
const openUrl: ActionType = {
  id: "system.open-url",
  name: "Open web page",
  description: "",
  category: "System",
  requiresProject: false,
  parameters: [
    { key: "url", label: "URL", kind: { type: "text", multiline: false }, required: true },
  ],
}
const draft: ActionInstance = {
  id: "a-draft",
  name: "Open docs",
  actionType: openUrl.id,
  parameters: {},
  description: "",
  icon: null,
  label: null,
}
const lookup: Lookup = {
  actions: [draft],
  types: [openUrl],
  workflows: [],
  projects: [],
  inputs: [{ key: "page", label: "Page", kind: "text" }],
  outputs: [],
}
const incomplete: GraphIssue = {
  node: "open",
  severity: "error",
  message: "parameter `url` is required",
  code: "configuration-incomplete",
  parameter: "url",
}

function node(extra: Partial<GraphNode & { type: "action" }> = {}): GraphNode {
  return {
    id: "open",
    type: "action",
    action: draft.id,
    policy: DEFAULT_POLICY,
    position: at,
    ...extra,
  }
}
function graphWith(n: GraphNode): WorkflowGraph {
  return {
    version: 1,
    nodes: [{ id: "start", type: "trigger", position: at }, n],
    edges: [{ from: "start", port: "out", to: n.id }],
  }
}
function inspect(n: GraphNode) {
  return render(
    <NodeInspector
      graph={graphWith(n)}
      node={n}
      issues={[incomplete]}
      lookup={lookup}
      projects={[]}
      loopTargets={[]}
      onChange={() => undefined}
      onDelete={() => undefined}
    />,
  )
}

afterEach(cleanup)

describe("a draft node", () => {
  it("is marked, and says next to each missing setting that it is required", () => {
    inspect(node())
    expect(screen.getByText("Needs configuration")).toBeInTheDocument()
    expect(
      screen.getByText("Required: fill this in before the action can run."),
    ).toBeInTheDocument()
  })

  it("lists a missing setting by its label, in words, and other problems as they are", () => {
    const other: GraphIssue = { node: "open", severity: "error", message: "no such output" }
    render(
      <NodeInspector
        graph={graphWith(node())}
        node={node()}
        issues={[incomplete, other]}
        lookup={lookup}
        projects={[]}
        loopTargets={[]}
        onChange={() => undefined}
        onDelete={() => undefined}
      />,
    )
    const problems = screen.getByRole("list", { name: "Problems" })
    // From the issue's code and parameter, not the agent's English.
    expect(problems).toHaveTextContent("open: Missing: URL")
    expect(problems).not.toHaveTextContent("parameter `url` is required")
    expect(problems).toHaveTextContent("open: no such output")
  })

  it("is complete once the node fills the setting, fixed or mapped", () => {
    inspect(node({ overrides: { url: { type: "text", value: "https://docs.example.com" } } }))
    expect(screen.queryByText("Needs configuration")).not.toBeInTheDocument()
    cleanup()
    inspect(node({ mappings: { url: { type: "template", template: "{{ inputs.page }}" } } }))
    expect(screen.queryByText("Needs configuration")).not.toBeInTheDocument()
    expect(screen.queryByText(/Required: fill this in/)).not.toBeInTheDocument()
  })

  it("is drawn on the canvas as needing configuration, not as broken", () => {
    render(
      <ReactFlowProvider>
        <NodeView
          {...({
            id: "open",
            selected: false,
            data: {
              type: "action",
              title: "Open docs",
              detail: "",
              icon: null,
              issues: [incomplete],
            },
          } as unknown as Parameters<typeof NodeView>[0])}
        />
      </ReactFlowProvider>,
    )
    const badge = screen.getByText("Needs configuration")
    expect(badge).toBeInTheDocument()
    // No error icon or error line for it.
    expect(screen.queryByLabelText(/problem/i)).not.toBeInTheDocument()
    expect(screen.queryByText(incomplete.message)).not.toBeInTheDocument()
    expect(
      within(badge.closest("div") as HTMLElement).getByText("Needs configuration"),
    ).toBeTruthy()
  })

  it("does not keep the workflow from being saved; other errors still do", () => {
    expect(blockingIssues([incomplete])).toEqual([])
    const broken: GraphIssue = { node: "open", severity: "error", message: "no such output" }
    expect(blockingIssues([incomplete, broken])).toEqual([broken])
    expect(blockingIssues([{ severity: "warning", message: "slow" }])).toEqual([])
  })
})
