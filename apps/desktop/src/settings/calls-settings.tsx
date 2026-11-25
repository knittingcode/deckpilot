import { RotateCcw } from "lucide-react"
import { Fragment, useEffect, useRef, useState } from "react"
import { listen } from "@tauri-apps/api/event"
import { toast } from "sonner"

import { SettingsGroup, SettingsRow, SettingsSection } from "@/components/settings-layout"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Switch } from "@/components/ui/switch"
import { type TranslationKey, useTranslation } from "@/i18n"
import { useFormat } from "@/i18n/format"
import { api } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"
import {
  CALLS_EVENT,
  type CallStateName,
  callStateName,
  callsRefusal,
  OS_SIGNALS_PROVIDER,
} from "@/lib/calls"
import { toastFailure } from "@/lib/failure"
import type { CallsLayout, CallsPageKind, CallsStatus, DeviceId } from "@/lib/types"
import { BackToWorkSection, CallsPagesSection } from "@/settings/calls-pages"

const STATE_LABELS = {
  idle: "devices:calls.states.idle",
  incoming: "devices:calls.states.incoming",
  outgoing: "devices:calls.states.outgoing",
  connecting: "devices:calls.states.connecting",
  active: "devices:calls.states.active",
  ending: "devices:calls.states.ending",
  ended: "devices:calls.states.ended",
  failed: "devices:calls.states.failed",
  unknown: "devices:calls.states.unknown",
} as const satisfies Record<CallStateName, TranslationKey>

/**
 * Calls in Settings › Devices (docs/calls.md): the call's state, the overlay
 * per device and *Restore defaults* of the managed layouts. Shown only when
 * the agent has Calls (protocol 19) and Calls is installed.
 */
export function CallsSettings() {
  const { t } = useTranslation()
  const { agent } = useAppContext()
  const supported = agent?.running === true && agent.capabilities?.includes("calls") === true
  // A restarted agent is a new instance: its Calls are fetched again.
  const instance = agent?.status ? `${agent.status.pid}:${agent.status.startedAt}` : null
  const [status, setStatus] = useState<CallsStatus | null>(null)
  const [restoring, setRestoring] = useState<CallsLayout | null>(null)
  // The one preview the app holds (the backend has one at a time).
  const [preview, setPreview] = useState<{ model: string; page: CallsPageKind } | null>(null)
  const [pending, setPending] = useState(false)
  // The latest preview request: a result of an older one is ignored.
  const request = useRef(0)
  // The preview is the settings': it ends when they close.
  useEffect(
    () => () => {
      request.current += 1
      void api.callsPreviewEnd()
    },
    [],
  )
  // The agent ended the preview by itself (a lock, a call, a disconnect,
  // a dismiss, a restart): once its status named the preview, it no longer
  // does.
  const previewModel = preview?.model
  const previewPage = preview?.page
  const shownByAgent =
    status?.devices.some(
      (entry) => entry.device.model === previewModel && entry.preview === previewPage,
    ) === true
  // Which preview the agent's status confirmed ("model/page"), followed
  // while rendering.
  const [confirmed, setConfirmed] = useState<string | null>(null)
  const previewKey =
    previewModel !== undefined && previewPage !== undefined
      ? `${previewModel}/${previewPage}`
      : null
  if (previewKey !== null && shownByAgent && confirmed !== previewKey) {
    setConfirmed(previewKey)
  } else if (previewKey !== null && !shownByAgent && confirmed === previewKey) {
    setConfirmed(null)
    setPreview(null)
  }
  const togglePreview = (model: string, device: DeviceId, page: CallsPageKind) => {
    request.current += 1
    const mine = request.current
    if (preview?.model === model && preview.page === page) {
      setPreview(null)
      void api.callsPreviewEnd()
      return
    }
    setPending(true)
    api.callsPreviewBegin(device, page).then(
      () => {
        if (mine !== request.current) return
        setPending(false)
        setPreview({ model, page })
      },
      (e: unknown) => {
        if (mine !== request.current) return
        setPending(false)
        setPreview(null)
        toastFailure(t("devices:calls.previewFailed"), callsRefusal(e))
      },
    )
  }

  useEffect(() => {
    if (!supported) return
    let current = true
    api.getCallsStatus().then(
      (next) => {
        if (current) setStatus(next)
      },
      () => {
        if (current) setStatus(null)
      },
    )
    const stop = listen<CallsStatus>(CALLS_EVENT, ({ payload }) => {
      if (current) setStatus(payload)
    })
    return () => {
      current = false
      void stop.then((unlisten) => {
        unlisten()
      })
    }
  }, [supported, instance])

  if (!supported || !status?.installed) return null
  const deviceName = (model: string, serial: string) => {
    const name =
      agent.status?.devices.find((d) => d.id.model === model && d.id.serial === serial)
        ?.modelName ?? model
    return `${name} (${serial})`
  }
  // The model's name from a connected device of it, else its id.
  const modelName = (model: string) =>
    agent.status?.devices.find((d) => d.id.model === model)?.modelName ?? model
  return (
    <>
      <SettingsSection
        title={t("devices:calls.title")}
        description={t("devices:calls.description")}
      >
        <SettingsGroup>
          <SettingsRow label={t("devices:calls.state")}>
            <span>{t(STATE_LABELS[callStateName(status.state)])}</span>
          </SettingsRow>
          {!status.controls && (
            <SettingsRow
              label={t("devices:calls.controlsTitle")}
              description={
                status.provider === OS_SIGNALS_PROVIDER
                  ? t("devices:calls.detectionOnly")
                  : t("devices:calls.noControls")
              }
            >
              <Badge variant="secondary">{t("devices:calls.controlsNone")}</Badge>
            </SettingsRow>
          )}
        </SettingsGroup>
        <SettingsGroup>
          {status.devices.map((entry) => {
            const id = `calls-overlay-${entry.device.serial}`
            return (
              <SettingsRow
                key={id}
                htmlFor={id}
                label={deviceName(entry.device.model, entry.device.serial)}
                description={
                  entry.suspended
                    ? t("devices:calls.suspended")
                    : entry.shown
                      ? t("devices:calls.shown")
                      : t("devices:calls.overlayDescription")
                }
              >
                <Switch
                  id={id}
                  checked={entry.overlay}
                  aria-label={t("devices:calls.overlay")}
                  onCheckedChange={(on) => {
                    api.setCallsOverlay(entry.device, on).then(setStatus, (e: unknown) => {
                      toastFailure(t("devices:calls.overlayFailed"), e)
                    })
                  }}
                />
              </SettingsRow>
            )
          })}
        </SettingsGroup>
        {status.layouts.length > 0 && (
          <SettingsGroup>
            {status.layouts.map((layout) => (
              <SettingsRow
                key={layout.layout}
                label={layout.name}
                description={t("devices:calls.layoutDescription", {
                  model: modelName(layout.model),
                })}
              >
                <span className="flex items-center gap-2">
                  {layout.edited && <Badge variant="outline">{t("devices:calls.edited")}</Badge>}
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => {
                      setRestoring(layout)
                    }}
                  >
                    <RotateCcw /> {t("devices:calls.restore")}
                  </Button>
                </span>
              </SettingsRow>
            ))}
          </SettingsGroup>
        )}
        <RestoreDialog
          layout={restoring}
          onClose={() => {
            setRestoring(null)
          }}
        />
      </SettingsSection>
      {status.layouts.map((layout) => (
        <Fragment key={layout.layout}>
          <CallsPagesSection
            layout={layout}
            provider={status.provider}
            modelName={modelName(layout.model)}
            devices={status.devices
              .map((entry) => entry.device)
              .filter((device) => device.model === layout.model)}
            previewing={preview?.model === layout.model ? preview.page : null}
            pending={pending}
            onTogglePreview={(device, page) => {
              togglePreview(layout.model, device, page)
            }}
          />
          <BackToWorkSection
            layout={layout}
            modelName={modelName(layout.model)}
            onChange={(back) => {
              api.setCallsBackToWork(layout.model, back).then(setStatus, (e: unknown) => {
                toastFailure(t("devices:calls.back.failed"), e)
              })
            }}
          />
        </Fragment>
      ))}
    </>
  )
}

function RestoreDialog({ layout, onClose }: { layout: CallsLayout | null; onClose: () => void }) {
  const { t } = useTranslation()
  const format = useFormat()
  const copyName = t("devices:calls.copyName", { date: format.date(new Date()) })
  return (
    <AlertDialog
      open={layout !== null}
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{t("devices:calls.restoreTitle")}</AlertDialogTitle>
          <AlertDialogDescription>
            {layout?.edited
              ? t("devices:calls.restoreKeeps", { copy: copyName })
              : t("devices:calls.restoreUnedited")}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>{t("common:actions.cancel")}</AlertDialogCancel>
          <AlertDialogAction
            onClick={() => {
              if (!layout) return
              api.restoreCallsLayout(layout.model, copyName).then(
                (copy) => {
                  toast.success(
                    copy
                      ? t("devices:calls.restoredKept", { copy: copyName })
                      : t("devices:calls.restored"),
                  )
                },
                (e: unknown) => {
                  toastFailure(t("devices:calls.restoreFailed"), e)
                },
              )
              onClose()
            }}
          >
            {t("devices:calls.restoreConfirm")}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
