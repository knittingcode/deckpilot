import { Plug } from "lucide-react"

import { Button } from "@/components/ui/button"
import { useTranslation } from "@/i18n"
import type { MissingAction } from "@/lib/action-availability"
import { useActionModules, useAppContext } from "@/lib/app-context"
import { describeValue } from "@/lib/parameters"
import type { ActionInstance, ActionType, BindingTarget, Project } from "@/lib/types"
import { focusPlugin } from "@/plugins/focus"

import { BODY, RECOVERY, TITLE } from "./missing-action-text"

/** Parameter kinds whose values are never shown here (names of secrets). */
const HIDDEN_KINDS = new Set(["credential", "pairs", "code"])

/**
 * The kept settings of `action` that can be shown: by their label, when
 * the action type is known here and the kind holds no secret; the others
 * are only counted.
 */
function keptSettings(
  action: ActionInstance,
  type: ActionType | undefined,
  overrides: Record<string, unknown>,
  projects: Project[],
): { shown: { label: string; value: string }[]; hidden: number } {
  const keys = [...new Set([...Object.keys(action.parameters), ...Object.keys(overrides)])]
  const shown: { label: string; value: string }[] = []
  let hidden = 0
  for (const key of keys) {
    const spec = type?.parameters.find((p) => p.key === key)
    const value =
      (overrides as ActionInstance["parameters"])[key] ?? action.parameters[key] ?? undefined
    if (!spec || !value || HIDDEN_KINDS.has(spec.kind.type)) {
      hidden += 1
      continue
    }
    const text = describeValue(value, projects)
    if (text.length > 0) shown.push({ label: spec.label, value: text })
    else hidden += 1
  }
  return { shown, hidden }
}

/**
 * What the Inspector says about an assignment whose action cannot run now
 * (`lib/action-availability.ts`): the action and its plugin by name, why,
 * that everything about it is kept, and the way back: the existing
 * Marketplace or Settings › Plugins (the user decides there; nothing is
 * installed, enabled or granted from here), another action, or removing
 * the assignment. The key itself only shows the compact missing-plugin
 * face; plugin ids and the agent's raw reasons are never shown.
 */
export function MissingActionNote({
  missing,
  target,
  actions,
  types,
  projects = [],
  title,
  onReplace,
  replaceByDrop = false,
  onRemove,
}: {
  missing: MissingAction
  target: BindingTarget
  actions: ActionInstance[]
  /** Action types known here, for the labels and kinds of kept settings. */
  types: ActionType[]
  projects?: Project[]
  /** The assignment's own title, when it has one. */
  title?: string | null
  /** Moves focus to the chooser of another target. */
  onReplace?: () => void
  /** Another action replaces it by a drop from the Action Library (gesture cards). */
  replaceByDrop?: boolean
  /** Removes the assignment (the gesture, or the trigger's binding). */
  onRemove?: () => void
}) {
  const { t } = useTranslation()
  const { navigate, openSettings } = useAppContext()
  const action = target.type === "action" ? actions.find((a) => a.id === target.action) : undefined
  const modules = useActionModules()
  // The definition the plugin last reported (kept while it is disabled),
  // for the labels and kinds of the kept settings.
  const type = action
    ? (types.find((ty) => ty.id === action.actionType) ??
      modules.flatMap((m) => m.actionTypes).find((ty) => ty.id === action.actionType))
    : undefined
  const actionName = action?.name ?? t("editor:missing.thisAction")
  const plugin = missing.ownerName ?? t("editor:missing.itsPlugin")
  const kept = action
    ? keptSettings(action, type, target.type === "action" ? (target.overrides ?? {}) : {}, projects)
    : { shown: [], hidden: 0 }
  const recover = () => {
    switch (missing.recovery) {
      case "install":
        focusPlugin(missing.owner)
        navigate("marketplace")
        break
      case "enable":
      case "update":
      case "permissions":
        // The plugin's page in the Marketplace manages all three.
        focusPlugin(missing.owner)
        navigate("marketplace")
        break
      case "enableModule":
        openSettings("modules")
        break
      case null:
        break
    }
  }
  return (
    <div
      role="note"
      data-slot="missing-action-note"
      data-reason={missing.reason}
      className="grid gap-2 rounded-md border border-dashed bg-muted/40 p-2.5 text-xs"
    >
      <p className="flex items-center gap-1.5 font-medium text-foreground">
        <Plug aria-hidden className="size-3.5 shrink-0 text-muted-foreground" />
        {t(TITLE[missing.reason])}
      </p>
      <p className="text-muted-foreground">
        {t(BODY[missing.reason], { action: actionName, plugin })}
      </p>
      <div className="grid gap-1 text-muted-foreground" data-slot="missing-action-kept">
        <p>{t("editor:missing.kept")}</p>
        <ul className="grid gap-0.5 ps-3">
          {title && <li>{t("editor:missing.keptTitle", { title })}</li>}
          {kept.shown.map((setting) => (
            <li key={setting.label}>
              {t("editor:missing.keptSetting", { label: setting.label, value: setting.value })}
            </li>
          ))}
          {kept.hidden > 0 && <li>{t("editor:missing.keptHidden", { count: kept.hidden })}</li>}
          <li>{t("editor:missing.keptLook")}</li>
        </ul>
      </div>
      {replaceByDrop && (
        <p className="text-muted-foreground">{t("editor:missing.replaceByDrop")}</p>
      )}
      <div className="flex flex-wrap gap-1.5">
        {missing.recovery && (
          <Button size="xs" variant="secondary" onClick={recover}>
            {t(RECOVERY[missing.recovery])}
          </Button>
        )}
        {onReplace && (
          <Button size="xs" variant="ghost" onClick={onReplace}>
            {t("editor:missing.recovery.replace")}
          </Button>
        )}
        {onRemove && (
          <Button size="xs" variant="ghost" onClick={onRemove}>
            {t("editor:missing.recovery.remove")}
          </Button>
        )}
      </div>
    </div>
  )
}
