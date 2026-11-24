import { useEffect, useRef, useState } from "react"

import { NeedsConfiguration } from "@/components/needs-configuration"
import { ParameterField } from "@/components/parameter-field"
import { useTranslation } from "@/i18n"
import { missingParameters } from "@/lib/action-completeness"
import { api } from "@/lib/api"
import { errorMessage } from "@/lib/backend-error"
import { buildParameters, initialValue } from "@/lib/parameters"
import type { ActionInstance, ActionType, ParameterValue, Project } from "@/lib/types"

/** How long typing rests before the action is saved. */
const SAVE_DELAY_MS = 500

/**
 * The required settings of a draft action, in the Inspector: an action put
 * on a key or gesture before they were filled in (e.g. "Open application"
 * without its application). Each is marked required and, while empty, says
 * so next to its control; the action has a "Needs configuration" badge
 * until all are filled in. Values are saved to the action itself as they
 * are typed (it is the binding's own draft, used nowhere else); an empty
 * one is left out, never sent blank. Shown while the action is incomplete
 * and kept until the selection changes, so finishing it does not make the
 * field under the cursor disappear.
 */
export function ActionSetup({
  profileId,
  action,
  type,
  projects,
  autoFocus,
  onSaved,
  onFocused,
}: {
  profileId: string
  action: ActionInstance
  type: ActionType
  projects: Project[]
  /** Focus the first missing field once (after a drop). */
  autoFocus: boolean
  onSaved: () => void
  onFocused: () => void
}) {
  const { t } = useTranslation()
  const required = type.parameters.filter((spec) => spec.required)
  const [values, setValues] = useState<Record<string, ParameterValue>>(() => ({
    ...action.parameters,
  }))
  const missing = missingParameters(type, buildParameters(type.parameters, values))
  const [shown] = useState(() => missingParameters(type, action.parameters).length > 0)
  const [error, setError] = useState<string | null>(null)
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const latest = useRef(values)
  // The settings typed here and not saved yet; every other one follows the
  // stored action, which may change elsewhere meanwhile (the shared editor,
  // a rename).
  const dirty = useRef(new Set<string>())
  // The action as it is now, for the save: a delayed save never sends an
  // older name or definition back.
  const current = useRef(action)
  useEffect(() => {
    current.current = action
  })
  const container = useRef<HTMLDivElement>(null)
  const idPrefix = `setup-${action.id}`

  // A change of the stored action: the settings not being typed follow it.
  const stored = JSON.stringify(action.parameters)
  useEffect(() => {
    const next = { ...latest.current }
    let changed = false
    for (const spec of type.parameters) {
      if (dirty.current.has(spec.key)) continue
      const value = action.parameters[spec.key]
      if (JSON.stringify(next[spec.key]) === JSON.stringify(value)) continue
      next[spec.key] = value ?? initialValue(spec)
      changed = true
    }
    if (!changed) return
    latest.current = next
    setValues(next)
    // Only when what is stored changed.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [stored])

  const save = () => {
    if (timer.current) clearTimeout(timer.current)
    timer.current = null
    const base = current.current
    // What is typed here over what is stored now.
    const merged: Record<string, ParameterValue> = { ...base.parameters }
    for (const key of dirty.current) {
      const value = latest.current[key]
      if (value !== undefined) merged[key] = value
    }
    const parameters = buildParameters(type.parameters, merged)
    const sent = new Set(dirty.current)
    if (JSON.stringify(parameters) === JSON.stringify(base.parameters)) {
      dirty.current.clear()
      return
    }
    api.updateAction(profileId, { ...base, parameters }).then(
      () => {
        for (const key of sent) dirty.current.delete(key)
        setError(null)
        onSaved()
      },
      (e: unknown) => {
        setError(errorMessage(e))
      },
    )
  }
  // A pending save is not lost when the selection changes.
  const flush = useRef(save)
  useEffect(() => {
    flush.current = save
  })
  useEffect(
    () => () => {
      if (timer.current) flush.current()
    },
    [],
  )

  // After a drop: the first missing field, ready to type in.
  const first = missingParameters(type, action.parameters)[0]
  useEffect(() => {
    if (!autoFocus || !first) return
    const field = container.current?.querySelector<HTMLElement>(`[id="${idPrefix}-${first}"]`)
    focusTarget(field)?.focus()
    onFocused()
  }, [autoFocus, first, idPrefix, onFocused])

  if (!shown || required.length === 0) return null
  return (
    <div
      ref={container}
      className="grid min-w-0 gap-3 rounded-md border border-dashed border-amber-500/50 p-3"
      data-slot="action-setup"
      role="group"
      aria-label={t("actions:completeness.badgeLabel", { name: action.name })}
      onBlur={(event) => {
        // Leaving the form saves at once.
        if (!event.currentTarget.contains(event.relatedTarget) && timer.current) {
          save()
        }
      }}
    >
      {missing.length > 0 && <NeedsConfiguration name={action.name} className="w-fit" />}
      {required.map((spec) => (
        <ParameterField
          key={spec.key}
          idPrefix={idPrefix}
          spec={spec}
          value={values[spec.key] ?? initialValue(spec)}
          projects={projects}
          actionType={action.actionType}
          error={missing.includes(spec.key) ? t("actions:completeness.fieldRequired") : undefined}
          onChange={(next) => {
            const updated = { ...latest.current, [spec.key]: next }
            latest.current = updated
            dirty.current.add(spec.key)
            setValues(updated)
            if (timer.current) clearTimeout(timer.current)
            timer.current = setTimeout(save, SAVE_DELAY_MS)
          }}
        />
      ))}
      {error && (
        <p className="text-xs text-destructive" role="alert">
          {error}
        </p>
      )}
    </div>
  )
}

const FOCUSABLE = "input, textarea, select, button, [tabindex]:not([tabindex='-1'])"

/**
 * What takes focus for the field `field`: the field itself, or, for a
 * picker shown as a row (a chosen file or folder with its "Choose…"
 * buttons), the row's first control.
 */
function focusTarget(field: HTMLElement | null | undefined): HTMLElement | null {
  if (!field) return null
  if (field.matches(FOCUSABLE)) return field
  return field.parentElement?.querySelector<HTMLElement>(FOCUSABLE) ?? null
}
