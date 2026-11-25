import { useState } from "react"

import { useInlineNumber } from "@/components/inline-number"
import { InlineNumberInput } from "@/components/inline-number-input"
import {
  SettingsGroup,
  SettingsNote,
  SettingsRow,
  SettingsSection,
} from "@/components/settings-layout"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import { type TranslationKey, useTranslation } from "@/i18n"
import {
  BACK_DEFAULT_MAX,
  BACK_MAX_RANGE,
  BACK_TIMER_PRESETS,
  BACK_TIMER_RANGE,
  CALL_ACTIONS,
  CALL_REASONS,
  OS_SIGNALS_PROVIDER,
  checkSeconds,
} from "@/lib/calls"
import { controlLabel } from "@/lib/layout-model"
import type {
  BackToWork,
  BackToWorkMode,
  CallsControl,
  CallsLayout,
  CallsPageKind,
  DeviceId,
} from "@/lib/types"

const PAGE_NAMES = {
  incoming: "devices:calls.pages.incoming",
  outgoing: "devices:calls.pages.outgoing",
  active: "devices:calls.pages.active",
} as const satisfies Record<CallsPageKind, TranslationKey>

const MODE_NAMES = {
  timer: "devices:calls.back.modes.timer",
  completion: "devices:calls.back.modes.completion",
  completionOrMax: "devices:calls.back.modes.completionOrMax",
} as const satisfies Record<BackToWorkMode["kind"], TranslationKey>

/**
 * A managed Calls layout's call pages (incoming, outgoing, active), each
 * control with whether the call app supports it. A control it does not
 * support stays listed, disabled, with the reason; for detection-only Teams
 * that is every call control.
 */
export function CallsPagesSection({
  layout,
  provider,
  modelName,
  devices,
  previewing,
  pending,
  onTogglePreview,
}: {
  layout: CallsLayout
  provider?: string
  modelName: string
  /** The connected devices of the layout's model, to preview a page on. */
  devices: DeviceId[]
  /** The page of this layout shown as a preview now, if any. */
  previewing: CallsPageKind | null
  /** A preview is being begun (every preview button waits). */
  pending: boolean
  onTogglePreview: (device: DeviceId, page: CallsPageKind) => void
}) {
  const { t } = useTranslation()
  const [device] = devices
  if (layout.pages.length === 0) return null
  const actionName = (action: string) => {
    const key = CALL_ACTIONS[action as keyof typeof CALL_ACTIONS] as TranslationKey | undefined
    return key ? t(key) : action
  }
  const reason = (control: CallsControl) => {
    if (control.reason === "provider-has-no-api" && provider === OS_SIGNALS_PROVIDER) {
      return t("devices:calls.reasons.teams")
    }
    const key = CALL_REASONS[control.reason as keyof typeof CALL_REASONS] as
      TranslationKey | undefined
    return t(key ?? "devices:calls.reasons.other")
  }
  return (
    <SettingsSection
      title={`${t("devices:calls.pagesTitle")}: ${modelName}`}
      description={t("devices:calls.pagesDescription")}
    >
      {layout.pages.map((page) => (
        <SettingsGroup key={page.page} aria-label={t(PAGE_NAMES[page.page])}>
          <SettingsRow
            label={t("devices:calls.pageLabel", { page: t(PAGE_NAMES[page.page]) })}
            description={page.name === t(PAGE_NAMES[page.page]) ? undefined : page.name}
          >
            {device && (
              <Button
                variant="outline"
                size="sm"
                aria-pressed={previewing === page.page}
                disabled={pending}
                onClick={() => {
                  onTogglePreview(device, page.page)
                }}
              >
                {previewing === page.page
                  ? t("devices:calls.previewEnd")
                  : t("devices:calls.previewShow")}
              </Button>
            )}
          </SettingsRow>
          {page.controls.map((control) => (
            <SettingsRow
              key={`${control.control.kind}-${String(control.control.index)}`}
              label={`${controlLabel(control.control)}: ${actionName(control.action)}`}
              description={control.supported ? undefined : reason(control)}
            >
              {control.supported ? (
                <Badge variant="secondary">{t("devices:calls.supported")}</Badge>
              ) : (
                <Badge variant="outline" aria-disabled="true">
                  {t("devices:calls.controlsNone")}
                </Badge>
              )}
            </SettingsRow>
          ))}
        </SettingsGroup>
      ))}
    </SettingsSection>
  )
}

/** A number of seconds, stored when valid (Enter, or leaving the field). */
function SecondsField({
  id,
  label,
  seconds,
  range,
  onCommit,
}: {
  id: string
  label: string
  seconds: number
  range: readonly [number, number]
  onCommit: (seconds: number) => void
}) {
  const { t } = useTranslation()
  const field = useInlineNumber({
    stored: seconds,
    check: (text) => checkSeconds(text, range),
    onCommit: (value) => {
      if (value !== null && value !== seconds) onCommit(value)
    },
  })
  return (
    <span className="flex items-center gap-2 text-sm">
      <InlineNumberInput
        id={id}
        field={field}
        aria-label={label}
        aria-describedby={field.error ? `${id}-error` : undefined}
      />
      <span>{t("devices:calls.back.seconds")}</span>
      {field.error && (
        <span id={`${id}-error`} role="alert" className="text-destructive">
          {t("devices:calls.back.range", { min: range[0], max: range[1] })}
        </span>
      )}
    </span>
  )
}

/**
 * *Back to work* of a managed Calls layout: when a device that went back to
 * work shows the call again (a 20, 30 or 60 s or custom timer; when what was
 * started ends; or that, at the latest after a time, the default), what a
 * failed or cancelled run does, and whether every device goes. Each change
 * is applied at once.
 */
export function BackToWorkSection({
  layout,
  modelName,
  onChange,
}: {
  layout: CallsLayout
  modelName: string
  onChange: (back: BackToWork) => void
}) {
  const { t } = useTranslation()
  const stored = layout.back
  const id = `calls-back-${layout.model}`
  const [preset, setPreset] = usePreset(stored?.mode ?? { kind: "completion" })
  if (!stored) {
    return (
      <SettingsSection title={`${t("devices:calls.back.title")}: ${modelName}`}>
        <SettingsNote>{t("devices:calls.back.noKey")}</SettingsNote>
      </SettingsSection>
    )
  }
  const back = stored
  const setMode = (kind: BackToWorkMode["kind"]) => {
    if (kind === back.mode.kind) return
    const mode: BackToWorkMode =
      kind === "timer"
        ? { kind, seconds: 30 }
        : kind === "completion"
          ? { kind }
          : { kind, maxSeconds: BACK_DEFAULT_MAX }
    onChange({ ...back, mode })
  }
  const after = (key: "onFailure" | "onCancel", label: string) => (
    <SettingsRow label={label} htmlFor={`${id}-${key}`}>
      <Select
        value={back[key]}
        onValueChange={(value) => {
          onChange({ ...back, [key]: value as BackToWork[typeof key] })
        }}
      >
        <SelectTrigger id={`${id}-${key}`} className="w-64">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="resume">{t("devices:calls.back.after.resume")}</SelectItem>
          <SelectItem value="stay">{t("devices:calls.back.after.stay")}</SelectItem>
        </SelectContent>
      </Select>
    </SettingsRow>
  )
  return (
    <SettingsSection
      title={`${t("devices:calls.back.title")}: ${modelName}`}
      description={t("devices:calls.back.description")}
    >
      <SettingsGroup>
        <SettingsRow label={t("devices:calls.back.mode")} htmlFor={`${id}-mode`}>
          <Select
            value={back.mode.kind}
            onValueChange={(value) => {
              setMode(value as BackToWorkMode["kind"])
            }}
          >
            <SelectTrigger id={`${id}-mode`} className="w-64">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {(Object.keys(MODE_NAMES) as BackToWorkMode["kind"][]).map((kind) => (
                <SelectItem key={kind} value={kind}>
                  {t(MODE_NAMES[kind])}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </SettingsRow>
        {back.mode.kind === "timer" && (
          <SettingsRow label={t("devices:calls.back.timer")} htmlFor={`${id}-preset`}>
            <span className="flex flex-wrap items-center gap-2">
              <Select
                value={preset}
                onValueChange={(value) => {
                  setPreset(value)
                  if (value !== "custom") {
                    onChange({ ...back, mode: { kind: "timer", seconds: Number(value) } })
                  }
                }}
              >
                <SelectTrigger id={`${id}-preset`} className="w-32">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {BACK_TIMER_PRESETS.map((seconds) => (
                    <SelectItem key={seconds} value={String(seconds)}>
                      {t("devices:calls.back.preset", { seconds })}
                    </SelectItem>
                  ))}
                  <SelectItem value="custom">{t("devices:calls.back.custom")}</SelectItem>
                </SelectContent>
              </Select>
              {preset === "custom" && (
                <SecondsField
                  id={`${id}-seconds`}
                  label={t("devices:calls.back.timer")}
                  seconds={back.mode.seconds}
                  range={BACK_TIMER_RANGE}
                  onCommit={(seconds) => {
                    onChange({ ...back, mode: { kind: "timer", seconds } })
                  }}
                />
              )}
            </span>
          </SettingsRow>
        )}
        {back.mode.kind === "completionOrMax" && (
          <SettingsRow label={t("devices:calls.back.max")} htmlFor={`${id}-max`}>
            <SecondsField
              id={`${id}-max`}
              label={t("devices:calls.back.max")}
              seconds={back.mode.maxSeconds}
              range={BACK_MAX_RANGE}
              onCommit={(maxSeconds) => {
                onChange({ ...back, mode: { kind: "completionOrMax", maxSeconds } })
              }}
            />
          </SettingsRow>
        )}
        {back.mode.kind !== "timer" && (
          <>
            {after("onFailure", t("devices:calls.back.onFailure"))}
            {after("onCancel", t("devices:calls.back.onCancel"))}
          </>
        )}
        <SettingsRow
          label={t("devices:calls.back.allDevices")}
          description={t("devices:calls.back.allDevicesDescription")}
          htmlFor={`${id}-all`}
        >
          <Switch
            id={`${id}-all`}
            checked={back.allDevices}
            onCheckedChange={(allDevices) => {
              onChange({ ...back, allDevices })
            }}
          />
        </SettingsRow>
      </SettingsGroup>
    </SettingsSection>
  )
}

/**
 * The timer's preset choice: a preset when the stored time is one, else
 * "custom"; choosing "custom" keeps the field open until a time is stored.
 */
function usePreset(mode: BackToWorkMode): [string, (value: string) => void] {
  const stored =
    mode.kind === "timer" && (BACK_TIMER_PRESETS as readonly number[]).includes(mode.seconds)
      ? String(mode.seconds)
      : "custom"
  const [chosen, setChosen] = useStateSynced(stored)
  return [chosen, setChosen]
}

/** A local choice that follows `stored` whenever it changes. */
function useStateSynced(stored: string): [string, (value: string) => void] {
  const [state, setState] = useState({ stored, value: stored })
  if (state.stored !== stored) {
    setState({ stored, value: stored })
  }
  return [
    state.stored === stored ? state.value : stored,
    (value) => {
      setState({ stored, value })
    },
  ]
}
