// The words of the Inspector's note about an assignment whose plugin is
// missing (`missing-action-note.tsx`), by reason and recovery.

import type { TranslationKey } from "@/i18n"
import {
  type MissingAction,
  type MissingReason,
  type RecoveryAction,
  missingAction,
} from "@/lib/action-availability"
import { pluginActionUnavailable } from "@/lib/plugin-actions"
import type { ModuleInfo } from "@/lib/types"

export const TITLE: Record<MissingReason, TranslationKey> = {
  notInstalled: "editor:missing.title.notInstalled",
  disabled: "editor:missing.title.disabled",
  incompatible: "editor:missing.title.incompatible",
  integrity: "editor:missing.title.integrity",
  revoked: "editor:missing.title.revoked",
  busy: "editor:missing.title.busy",
  permission: "editor:missing.title.permission",
  moduleDisabled: "editor:missing.title.moduleDisabled",
}

export const BODY: Record<MissingReason, TranslationKey> = {
  notInstalled: "editor:missing.body.notInstalled",
  disabled: "editor:missing.body.disabled",
  incompatible: "editor:missing.body.incompatible",
  integrity: "editor:missing.body.integrity",
  revoked: "editor:missing.body.revoked",
  busy: "editor:missing.body.busy",
  permission: "editor:missing.body.permission",
  moduleDisabled: "editor:missing.body.moduleDisabled",
}

export const RECOVERY: Record<RecoveryAction, TranslationKey> = {
  install: "editor:missing.recovery.install",
  enable: "editor:missing.recovery.enable",
  update: "editor:missing.recovery.update",
  permissions: "editor:missing.recovery.permissions",
  enableModule: "editor:missing.recovery.enableModule",
}

/** The short reason, e.g. for a disabled option's title. */
export function missingSummary(missing: MissingAction, t: (key: TranslationKey) => string): string {
  return t(TITLE[missing.reason])
}

/**
 * Why `typeId` cannot be placed now, in words (e.g. "Permission
 * required", "The plugin is disabled"): never the agent's raw reason,
 * which names permission ids and stale places; that stays only when
 * nothing better is known. `null` when it can be placed.
 */
export function unavailableText(
  modules: ModuleInfo[],
  typeId: string,
  t: (key: TranslationKey) => string,
): string | null {
  const raw = pluginActionUnavailable(modules, typeId)
  if (raw === null) return null
  const lost = missingAction(modules, typeId)
  return lost ? missingSummary(lost, t) : raw
}
