// Where an action type can run (design §1, §4 of the action-context audit;
// `ActionContext` of deckpilot-core): on a key's primary press, as a step
// of a Multi Action, as a node of a workflow, as a key's live display, or
// on a key's secondary gesture. The agent says so per type (`contexts`);
// a type that does not is allowed everywhere, exactly as before contexts
// existed. Nothing here is ever derived from an id, a name or a category.

import type { ActionType, Trigger } from "./types"

export type ActionContext = "key" | "sequence" | "workflow" | "display" | "gesture"

/** Every context, in the order they are shown. */
export const ACTION_CONTEXTS: readonly ActionContext[] = [
  "key",
  "gesture",
  "display",
  "sequence",
  "workflow",
]

/**
 * Why an action type cannot run in a context (`IncompatibilityReason::
 * ContextUnsupported` of deckpilot-core). The app words it from these
 * fields, never from the agent's English.
 */
export type ContextIncompatibility = {
  type: "contextUnsupported"
  context: ActionContext
  supported: ActionContext[]
}

const ALL: ReadonlySet<ActionContext> = new Set(ACTION_CONTEXTS)

/** The contexts a type declares; absent: all five (legacy). */
export function effectiveContexts(t: Pick<ActionType, "contexts">): ReadonlySet<ActionContext> {
  return t.contexts ? new Set(t.contexts) : ALL
}

/** The context a use of an action is in. */
export function contextOf(
  slot: { kind: "binding"; trigger: Trigger } | { kind: "step" } | { kind: "node" },
): ActionContext {
  switch (slot.kind) {
    case "step":
      return "sequence"
    case "node":
      return "workflow"
    case "binding":
      return slot.trigger.type === "doubleClick" || slot.trigger.type === "longPress"
        ? "gesture"
        : "key"
  }
}

export function supportsContext(
  t: Pick<ActionType, "contexts">,
  ctx: ActionContext,
): true | ContextIncompatibility {
  const contexts = effectiveContexts(t)
  if (contexts.has(ctx)) return true
  return {
    type: "contextUnsupported",
    context: ctx,
    supported: ACTION_CONTEXTS.filter((c) => contexts.has(c)),
  }
}

/** Shows something on a key but does nothing when pressed: `display` without `key`. */
export function isDisplayOnly(t: Pick<ActionType, "contexts">): boolean {
  const contexts = effectiveContexts(t)
  return contexts.has("display") && !contexts.has("key")
}

// ---------------------------------------------------------------------------
// The slots of the editors (design §1.1, §5.5), on top of the contract.

/**
 * Whether a binding of `trigger` accepts the type: a key's primary slot
 * takes `key` or `display` (a display-only type's press does nothing), a
 * gesture slot only `gesture`.
 */
export function bindingAccepts(
  t: Pick<ActionType, "contexts">,
  trigger: Trigger,
): true | ContextIncompatibility {
  const context = contextOf({ kind: "binding", trigger })
  if (context === "key" && supportsContext(t, "display") === true) return true
  return supportsContext(t, context)
}

/** Whether the type can go on a key at all: its press, its display or a gesture. */
export function placeableOnKey(t: Pick<ActionType, "contexts">): boolean {
  const contexts = effectiveContexts(t)
  return contexts.has("key") || contexts.has("display") || contexts.has("gesture")
}
