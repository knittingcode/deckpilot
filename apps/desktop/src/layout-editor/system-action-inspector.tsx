// The inspector's panels for keys that run the System module's Website,
// Open, Open Application or Close Application (docs/system-actions.md): the
// action's settings with the machine-local pickers, a test run with its
// error in words, and for Open Application the key's long press (None or
// Quit application), an ordinary LongPress binding of Close Application in
// the existing gesture model.

import { FlaskConical, Save } from "lucide-react"
import { useId, useState } from "react"
import { toast } from "sonner"

import { ParameterField } from "@/components/parameter-field"
import { Button } from "@/components/ui/button"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { useTranslation } from "@/i18n"
import { api } from "@/lib/api"
import { failureDetail } from "@/lib/failure"
import { usedElsewhere } from "@/lib/layout-navigation"
import { bindingsFor, controlKey, triggerKey, triggerLabel } from "@/lib/layout-model"
import { buildParameters, initialValue, missingRequired } from "@/lib/parameters"
import {
  CLOSE_APPLICATION,
  OPEN,
  OPEN_APPLICATION,
  WEBSITE,
  actionTypeName,
  checkWebsite,
  executionError,
} from "@/lib/system-actions"
import type {
  ActionInstance,
  ActionResult,
  ActionType,
  ControlBinding,
  ControlId,
  DeviceLayout,
  ParameterValue,
} from "@/lib/types"

import { freeActionName } from "./catalog"
import type { LayoutNavigationContext } from "./layout-switch-inspector"
import { type LongPress, longPressOf, withLongPress } from "./system-actions-model"

const SYSTEM_TYPES = [WEBSITE, OPEN, OPEN_APPLICATION, CLOSE_APPLICATION]

/** The panels of the control's System action bindings. */
export function SystemActionPanels({
  draft,
  pageId,
  control,
  actions,
  types,
  gestures,
  context,
  onChange,
}: {
  draft: DeviceLayout
  pageId: string
  control: ControlId
  actions: ActionInstance[]
  types: ActionType[]
  /** The control has gestures (long press). */
  gestures: boolean
  context: LayoutNavigationContext
  onChange: (layout: DeviceLayout) => void
}) {
  const found = bindingsFor(draft, pageId, control).flatMap((binding) => {
    if (binding.target.type !== "action") return []
    const id = binding.target.action
    const action = actions.find((a) => a.id === id)
    const type = types.find((t) => t.id === action?.actionType)
    if (!action || !type || !SYSTEM_TYPES.includes(type.id)) return []
    return [{ binding, action, type }]
  })
  if (found.length === 0) return null
  // The long-press quit of an Open Application key is part of its panel.
  const shown = found.filter(
    ({ binding, action }) =>
      !(
        binding.trigger.type === "longPress" &&
        action.actionType === CLOSE_APPLICATION &&
        found.some(({ action: other }) => other.actionType === OPEN_APPLICATION)
      ),
  )
  const several = shown.length > 1
  return (
    <>
      {shown.map(({ binding, action, type }) => (
        <SystemActionPanel
          key={`${controlKey(control)}:${triggerKey(binding.trigger)}:${action.id}`}
          draft={draft}
          pageId={pageId}
          control={control}
          binding={binding}
          action={action}
          type={type}
          actions={actions}
          gestures={gestures}
          context={context}
          trigger={several ? triggerLabel(binding.trigger) : null}
          onChange={onChange}
        />
      ))}
    </>
  )
}

function SystemActionPanel({
  draft,
  pageId,
  control,
  binding,
  action,
  type,
  actions,
  gestures,
  context,
  trigger,
  onChange,
}: {
  draft: DeviceLayout
  pageId: string
  control: ControlId
  binding: ControlBinding
  action: ActionInstance
  type: ActionType
  actions: ActionInstance[]
  gestures: boolean
  context: LayoutNavigationContext
  trigger: string | null
  onChange: (layout: DeviceLayout) => void
}) {
  const { t } = useTranslation()
  const id = useId()
  const initial = () =>
    Object.fromEntries(
      type.parameters.map((spec) => [spec.key, action.parameters[spec.key] ?? initialValue(spec)]),
    )
  const [values, setValues] = useState<Record<string, ParameterValue>>(initial)
  const [busy, setBusy] = useState(false)
  const [result, setResult] = useState<ActionResult | null>(null)
  const parameters = buildParameters(type.parameters, values)
  const dirty = JSON.stringify(parameters) !== JSON.stringify(action.parameters)
  const missing = missingRequired(type.parameters, values)
  const url = values.url
  const websiteProblem =
    type.id === WEBSITE && url?.type === "text"
      ? checkWebsite(url.value, parameters.browser !== undefined).problem
      : null
  const shared = usedElsewhere(action.id, draft, pageId, binding, context.usages)

  const save = async (next: Record<string, ParameterValue>) => {
    setBusy(true)
    try {
      const updated = await api.updateAction(context.profileId, {
        ...action,
        parameters: buildParameters(type.parameters, next),
      })
      context.onActionSaved(updated)
      toast.success(t("actions:system.panel.saved"))
    } catch (e) {
      toast.error(t("actions:system.panel.saveFailed"), { description: failureDetail(e) })
    } finally {
      setBusy(false)
    }
  }

  const test = async () => {
    setResult(null)
    setBusy(true)
    try {
      setResult(await api.runAction(context.profileId, action.id))
    } catch (e) {
      setResult({ status: "failed", message: failureDetail(e) })
    } finally {
      setBusy(false)
    }
  }

  const force = values.force?.type === "boolean" && values.force.value
  return (
    <section className="grid gap-3 border-t pt-4" aria-labelledby={id} data-slot="system-action">
      <h3 id={id} className="text-sm font-medium">
        {actionTypeName(type)}
        {trigger && <span className="font-normal text-muted-foreground"> · {trigger}</span>}
      </h3>
      {type.parameters.map((spec) => (
        <ParameterField
          key={spec.key}
          spec={spec}
          value={values[spec.key] ?? initialValue(spec)}
          actionType={type.id}
          onChange={(next) => {
            setValues((current) => ({ ...current, [spec.key]: next }))
          }}
        />
      ))}
      {type.id === CLOSE_APPLICATION && (
        <p className="text-xs" role="note" data-mode={force ? "force" : "graceful"}>
          {force ? t("actions:system.panel.closeForce") : t("actions:system.panel.closeGraceful")}
        </p>
      )}
      {type.id === OPEN_APPLICATION && gestures && binding.trigger.type !== "longPress" && (
        <LongPressChoice
          draft={draft}
          pageId={pageId}
          control={control}
          action={action}
          actions={actions}
          context={context}
          onChange={onChange}
        />
      )}
      {shared && (
        <p className="text-xs text-muted-foreground" role="note">
          {t("actions:system.panel.shared", { action: action.name })}
        </p>
      )}
      <div className="flex flex-wrap gap-2">
        <Button
          size="sm"
          disabled={busy || !dirty || missing.length > 0 || websiteProblem !== null}
          onClick={() => void save(values)}
        >
          <Save /> {t("actions:system.panel.save")}
        </Button>
        <Button
          size="sm"
          variant="outline"
          disabled={busy || dirty}
          title={dirty ? t("actions:system.panel.saveFirst") : undefined}
          onClick={() => void test()}
        >
          <FlaskConical /> {t("actions:system.panel.test")}
        </Button>
      </div>
      {missing.length > 0 && (
        <p className="text-xs text-destructive" role="alert">
          {t("actions:system.panel.missing", {
            fields: type.parameters
              .filter((s) => missing.includes(s.key))
              .map((s) => s.label)
              .join(", "),
          })}
        </p>
      )}
      {result && (
        <p
          className={result.status === "succeeded" ? "text-xs" : "text-xs text-destructive"}
          role="status"
        >
          {result.status === "succeeded"
            ? result.message
            : t("actions:system.panel.failed", {
                reason: executionError(result.code) ?? result.message,
                detail: result.message,
              })}
        </p>
      )}
    </section>
  )
}

/** None or Quit application on the key's long press. */
function LongPressChoice({
  draft,
  pageId,
  control,
  action,
  actions,
  context,
  onChange,
}: {
  draft: DeviceLayout
  pageId: string
  control: ControlId
  action: ActionInstance
  actions: ActionInstance[]
  context: LayoutNavigationContext
  onChange: (layout: DeviceLayout) => void
}) {
  const { t } = useTranslation()
  const id = useId()
  const [busy, setBusy] = useState(false)
  const application = action.parameters.application
  const current = longPressOf(draft, pageId, control, actions, application)

  const choose = async (choice: LongPress) => {
    if (choice === current || choice === "other") return
    if (choice === "none") {
      onChange(withLongPress(draft, pageId, control, null).layout)
      return
    }
    setBusy(true)
    try {
      // The same application, quit gracefully: reused when it exists.
      const existing = actions.find(
        (a) =>
          a.actionType === CLOSE_APPLICATION &&
          JSON.stringify(a.parameters.application) === JSON.stringify(application) &&
          !(a.parameters.force?.type === "boolean" && a.parameters.force.value),
      )
      const quit =
        existing ??
        (await api.createAction(context.profileId, {
          name: freeActionName(actions, t("actions:system.panel.quitName", { name: action.name })),
          actionType: CLOSE_APPLICATION,
          parameters: application ? { application } : {},
          description: "",
          icon: null,
          label: null,
        }))
      if (!existing) context.onActionSaved(quit)
      const { layout, removed } = withLongPress(draft, pageId, control, quit)
      onChange(layout)
      if (removed.length > 0) toast.info(t("actions:system.panel.releaseRemoved"))
    } catch (e) {
      toast.error(t("actions:system.panel.saveFailed"), { description: failureDetail(e) })
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="grid gap-1.5">
      <Label htmlFor={id}>{t("actions:system.panel.longPress")}</Label>
      <Select
        value={current}
        disabled={busy || application === undefined}
        onValueChange={(value) => void choose(value as LongPress)}
      >
        <SelectTrigger id={id} size="sm" className="w-full">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="none">{t("actions:system.panel.longPressNone")}</SelectItem>
          <SelectItem value="quit">{t("actions:system.panel.longPressQuit")}</SelectItem>
          {current === "other" && (
            <SelectItem value="other" disabled>
              {t("actions:system.panel.longPressOther")}
            </SelectItem>
          )}
        </SelectContent>
      </Select>
      <p className="text-xs text-muted-foreground">
        {current === "other"
          ? t("actions:system.panel.longPressOtherHint")
          : t("actions:system.panel.longPressHint")}
      </p>
    </div>
  )
}
