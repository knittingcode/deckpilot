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

interface HeldStatus {
  instance: string | null
  status: CallsStatus | null
}

/**
 * Keeps `next` as the status of the agent instance `from`, unless the one
 * held for it has a higher revision (an answer and an event may arrive in
 * either order). Another instance's status is always replaced.
 */
function keeper(
  setHeld: (update: (prev: HeldStatus) => HeldStatus) => void,
  from: string | null,
): (next: CallsStatus | null) => void {
  return (next) => {
    setHeld((prev) =>
      next !== null &&
      prev.instance === from &&
      prev.status !== null &&
      prev.status.revision > next.revision
        ? prev
        : { instance: from, status: next },
    )
  }
}

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
  // The newest status of this agent instance: an answer and an event may
  // arrive in either order, and the higher revision wins, so an older one
  // never rolls back a newer one.
  const [held, setHeld] = useState<HeldStatus>({ instance, status: null })
  const status = held.instance === instance ? held.status : null
  const setStatus = keeper(setHeld, instance)
  const [restoring, setRestoring] = useState<CallsLayout | null>(null)
  // The one preview the app holds (the backend has one at a time): the
  // device it shows on, the page, whether the agent's status confirmed it,
  // and from which status revision on that status decides (`since`: the
  // revision the agent answered right after the begin; `null` until then).
  // Every new preview starts unconfirmed, so a confirmation of an earlier
  // preview (of the same page too) never carries over.
  const [preview, setPreview] = useState<{
    device: DeviceId
    page: CallsPageKind
    confirmed: boolean
    since: number | null
    /** The agent instance it began on: a restarted agent has none. */
    instance: string | null
  } | null>(null)
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
  const shows = (next: CallsStatus | null, device: DeviceId, page: CallsPageKind) =>
    next?.devices.some(
      (entry) =>
        entry.device.model === device.model &&
        entry.device.serial === device.serial &&
        entry.preview === page,
    ) === true
  // Followed while rendering: a status from the begin on (its revision at
  // least `since`) that shows the preview confirms it; one that does not
  // means the agent ended it by itself (a lock, a call, a disconnect, a
  // dismiss, a restart), confirmed or not. An older status decides nothing.
  if (preview !== null && preview.instance !== instance) {
    setPreview(null)
  } else if (preview !== null && preview.since !== null && status !== null) {
    if (status.revision >= preview.since) {
      const shown = shows(status, preview.device, preview.page)
      if (shown && !preview.confirmed) {
        setPreview({ ...preview, confirmed: true })
      } else if (!shown) {
        setPreview(null)
      }
    }
  }
  const togglePreview = (device: DeviceId, page: CallsPageKind) => {
    request.current += 1
    const mine = request.current
    if (
      preview?.device.model === device.model &&
      preview.device.serial === device.serial &&
      preview.page === page
    ) {
      setPreview(null)
      void api.callsPreviewEnd()
      return
    }
    setPending(true)
    api.callsPreviewBegin(device, page).then(
      () => {
        if (mine !== request.current) return
        setPending(false)
        setPreview({ device, page, confirmed: false, since: null, instance })
        // The agent answers in order: a status asked for now is the one
        // after the begin, and from its revision on the status decides. It
        // goes through the revision check like any other status.
        const decide = (since: number) => {
          if (mine !== request.current) return
          setPreview((p) =>
            p?.device === device && p.page === page && p.since === null ? { ...p, since } : p,
          )
        }
        api.getCallsStatus().then(
          (next) => {
            setStatus(next)
            decide(next.revision)
          },
          () => {
            // Nothing could tell whether it shows: it is ended, so the
            // app and the device agree.
            if (mine !== request.current) return
            setPreview(null)
            void api.callsPreviewEnd()
          },
        )
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
    const keepHere = keeper(setHeld, instance)
    api.getCallsStatus().then(
      (next) => {
        if (current) keepHere(next)
      },
      () => {
        if (current) keepHere(null)
      },
    )
    const stop = listen<CallsStatus>(CALLS_EVENT, ({ payload }) => {
      if (current) keepHere(payload)
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
            previewing={preview?.device.model === layout.model ? preview : null}
            pending={pending}
            onTogglePreview={(device, page) => {
              togglePreview(device, page)
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
