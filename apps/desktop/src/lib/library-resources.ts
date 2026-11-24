// Workflows as Action Library entries (docs/data-model.md, "Workflows"):
// one entry per saved workflow of the edited profile, derived from
// `ProfileView.workflowEntries` and never stored. Dropping one writes the
// existing `{ type: "workflow", workflow: id }` target, with no title or
// look of its own: the key refers to the workflow by id, so a rename or an
// edit reaches it.

import { WORKFLOWS_PLUGIN } from "./plugin-features"
import type { BindingTarget, WorkflowEntry } from "./types"
import { compareText } from "@/i18n/format"

export interface LibraryResourceEntry {
  /** The target's encoding (`encodeTarget`): `workflow:<id>`. */
  key: string
  /**
   * Where the library lists it: graph workflows in the Workflows plugin's
   * section (`plugin: "deckpilot.workflows"`, `kind: "workflow"`), step
   * workflows (Multi Actions) in the core's Sequences (`plugin: null`,
   * `kind: "sequence"`).
   */
  section: { plugin: string | null; kind: "workflow" | "sequence" }
  id: string
  label: string
  /** The workflow glyph (`symbols::WORKFLOW` on the device). */
  icon: string
  target: BindingTarget
  /**
   * `ready`: it can be dropped (an invalid graph or one with draft nodes
   * that need configuration, with a warning, `reason`).
   * `readOnly`: a graph workflow while the Workflows plugin is off; keys
   * keep it and do not run it. `blocked`: it needs inputs and cannot be
   * dropped on a key. Neither `readOnly` nor `blocked` is draggable.
   */
  state: "ready" | "readOnly" | "blocked"
  reason?: "workflowsOff" | "needsInputs" | "invalidGraph" | "needsConfiguration"
  /** How many places refer to it. */
  usages: number
}

/** The glyph keys that start a workflow show. */
export const WORKFLOW_ICON = "list-ordered"

/**
 * The library entries of `entries`, ordered by name. `pluginActive`: the
 * Workflows plugin is enabled.
 */
export function workflowResourceEntries(
  entries: WorkflowEntry[],
  pluginActive: boolean,
): LibraryResourceEntry[] {
  return entries
    .map((entry): LibraryResourceEntry => {
      const graph = entry.kind === "graph"
      const base = {
        key: `workflow:${entry.id}`,
        section: graph
          ? { plugin: WORKFLOWS_PLUGIN, kind: "workflow" as const }
          : { plugin: null, kind: "sequence" as const },
        id: entry.id,
        label: entry.name,
        icon: WORKFLOW_ICON,
        target: { type: "workflow", workflow: entry.id } satisfies BindingTarget,
        usages: entry.references,
      }
      if (graph && !pluginActive) return { ...base, state: "readOnly", reason: "workflowsOff" }
      if (entry.block?.code === "needsInputs") {
        return { ...base, state: "blocked", reason: "needsInputs" }
      }
      if (entry.block?.code === "invalidGraph") {
        return { ...base, state: "ready", reason: "invalidGraph" }
      }
      if (entry.block?.code === "needsConfiguration") {
        return { ...base, state: "ready", reason: "needsConfiguration" }
      }
      return { ...base, state: "ready" }
    })
    .sort((a, b) => compareText(a.label, b.label) || a.id.localeCompare(b.id))
}

/** Whether an entry can be dragged onto a key or gesture. */
export function isDroppable(entry: LibraryResourceEntry): boolean {
  return entry.state === "ready"
}
