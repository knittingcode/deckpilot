// The rows of Settings › Profiles › a profile › "Saved actions"
// (profile-saved-actions.tsx): the profile's saved actions by name, with
// where each is used.

import { actionReferences, isDirect } from "@/lib/action-references"
import type { ActionInstance, ActionReference, ProfileView } from "@/lib/types"

export type SavedActionFilter = "all" | "unused"

export interface SavedActionRow {
  action: ActionInstance
  /** Every place that uses it (as the agent's `action_references` lists them). */
  references: ActionReference[]
  /** The places that run it themselves (not the triggers of a workflow that does). */
  uses: number
}

/** The profile's saved actions by name, with their uses; `unused`: only those nothing uses. */
export function savedActionRows(
  view: ProfileView,
  filter: SavedActionFilter,
  compare: (a: string, b: string) => number,
): SavedActionRow[] {
  return view.profile.actions
    .map((action) => {
      const references = actionReferences(view.profile, action.id)
      return { action, references, uses: references.filter(isDirect).length }
    })
    .filter((row) => filter === "all" || row.references.length === 0)
    .sort((a, b) => compare(a.action.name, b.action.name) || a.action.id.localeCompare(b.action.id))
}
