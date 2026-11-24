// The app's reading of draft actions while they are edited: what a use of
// a saved action (a binding, a Multi Action step) still needs, from the
// type's schema with the same rule as the agent (`missingParameters` of
// `lib/action-completeness.ts`). The agent's answer wins once saved; this
// keeps the marks right between a change and that answer.

import { missingParameters } from "./action-completeness"
import type { ActionInstance, ActionParameters, ActionType } from "./types"

/**
 * The required parameters a use of `actionId` leaves missing: the
 * action's own values with the use's overrides on top. Empty when it is
 * complete, and for an action or type not known here.
 */
export function missingForUse(
  actionId: string,
  overrides: ActionParameters | undefined,
  actions: ActionInstance[],
  typeOf: (id: string) => ActionType | undefined,
): string[] {
  const action = actions.find((a) => a.id === actionId)
  const type = action ? typeOf(action.actionType) : undefined
  if (!action || !type) return []
  return missingParameters(type, { ...action.parameters, ...(overrides ?? {}) })
}
