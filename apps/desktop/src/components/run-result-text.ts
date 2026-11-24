// The words of a run's result where the app shows it: a run refused
// because required settings are missing (`configuration-incomplete`,
// `lib/action-completeness.ts`) is said in the window's language with the
// settings' names, never as the agent's message.

import { i18n } from "@/i18n"
import { refusedAsIncomplete } from "@/lib/action-completeness"
import type { ActionResult, ActionType } from "@/lib/types"

/** The result's message, or the localized refusal of an incomplete action. */
export function runResultText(
  result: Pick<ActionResult, "code" | "data" | "message">,
  name: string,
  type?: Pick<ActionType, "parameters">,
): string {
  const missing = refusedAsIncomplete(result)
  if (missing === null) return result.message
  const refused = i18n.t("actions:completeness.runRefused", { name })
  if (missing.length === 0) return refused
  const labels = missing.map((key) => type?.parameters.find((p) => p.key === key)?.label ?? key)
  return `${refused} ${i18n.t("actions:completeness.missing", { parameters: labels.join(", ") })}`
}

/** Whether a result is the refusal of an incomplete action (its data are keys, not output). */
export function isIncompleteRefusal(result: Pick<ActionResult, "code" | "data">): boolean {
  return refusedAsIncomplete(result) !== null
}
