// What a saved use of an action that does not support its context can
// become (design §6 of the action-context audit). Nothing is changed
// unless the user confirms; the result is saved through the normal save
// paths (`saveWorkflow`, the layout's Save), which validate it again.

import { addNode, connect, emptyGraph } from "@/workflow-builder/graph"

import { type ContextIncompatibility } from "./action-context"
import { freeWorkflowName } from "./multi-action"
import type { ActionInstance, ActionParameters, Workflow } from "./types"
import { newStep } from "./workflow-model"

/** The migrations a key or gesture binding of an action can be offered. */
export interface BindingMigrations {
  /** The action works as a Multi Action step: wrap it in one. */
  wrap: boolean
  /** The action works as a workflow node: move it into a workflow (needs the Workflows plugin). */
  move: boolean
}

/** The migrations offered for a binding the action's contexts refuse. */
export function bindingMigrations(
  reason: ContextIncompatibility,
  workflowsAvailable: boolean,
): BindingMigrations {
  return {
    wrap: reason.supported.includes("sequence"),
    move: workflowsAvailable && reason.supported.includes("workflow"),
  }
}

/**
 * A new Multi Action running `action` as its only step, with the
 * binding's `overrides` as the step's own (so it runs as the key did).
 */
export function wrappedInMultiAction(
  workflows: Pick<Workflow, "name">[],
  action: Pick<ActionInstance, "id" | "name">,
  overrides: ActionParameters = {},
): Workflow {
  return {
    id: crypto.randomUUID(),
    name: freeWorkflowName(workflows, action.name),
    description: "",
    steps: [{ ...newStep(action.id), overrides: { ...overrides } }],
    projectScope: "execution",
  }
}

/**
 * A new workflow: its trigger, then `action`, with the binding's
 * `overrides` as the node's own (so it runs as the key did).
 */
export function movedToWorkflow(
  workflows: Pick<Workflow, "name">[],
  action: Pick<ActionInstance, "id" | "name">,
  overrides: ActionParameters = {},
): Workflow {
  const start = emptyGraph()
  const added = addNode(
    start,
    {
      type: "action",
      action: action.id,
      ...(Object.keys(overrides).length > 0 ? { overrides: { ...overrides } } : {}),
      policy: { onError: "stop", retry: { attempts: 0, backoffMs: 1000 } },
    },
    { x: 280, y: 0 },
    action.name,
  )
  const trigger = start.nodes[0]?.id ?? "start"
  return {
    id: crypto.randomUUID(),
    name: freeWorkflowName(workflows, action.name),
    description: "",
    steps: [],
    projectScope: "execution",
    graph: connect(added.graph, { from: trigger, port: "out", to: added.id }),
  }
}
