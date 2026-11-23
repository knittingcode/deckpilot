// Contexts in words (`lib/action-context.ts`), in the window's language.
// A reason is worded from its structured fields, never from the agent's
// English text.

import { type TranslationKey, i18n } from "@/i18n"

import { ACTION_CONTEXTS, type ActionContext, type ContextIncompatibility } from "./action-context"

const LABELS = {
  key: "actions:contexts.key",
  sequence: "actions:contexts.sequence",
  workflow: "actions:contexts.workflow",
  display: "actions:contexts.display",
  gesture: "actions:contexts.gesture",
} as const satisfies Record<ActionContext, TranslationKey>

/** "Key", "Multi Action step", … */
export function contextLabel(context: ActionContext): string {
  return i18n.t(LABELS[context])
}

/** "This action does not work here: Key. It works as: Workflow node." */
export function contextReason(reason: ContextIncompatibility): string {
  const supported = ACTION_CONTEXTS.filter((c) => reason.supported.includes(c)).map(contextLabel)
  return i18n.t("editor:context.unsupported", {
    where: contextLabel(reason.context),
    supported: supported.join(", "),
  })
}
