// The settings of the folder a key opens, in that key's inspector: Auto
// Exit (one row: a switch, and its time inline while it is on), Exit after action
// completes (`autoReturn`, a switch of its own), and for a folder whose
// Back is still on its first free key ("legacy") the one-click "Move Back
// to the first key". Its name is the key header's title field (the
// folder's one title, `folderKeyTitle`). Only controls, no headings or
// explanations. The folder's own protected Back key has an empty
// inspector; the page navigator's menu has the same "Exit after action
// completes".

import { useContext, useId, useState } from "react"
import { ArrowLeft, ArrowRightLeft, Trash2 } from "lucide-react"

import { DialogsContext } from "@/components/dialogs/use-dialogs"

import { Button } from "@/components/ui/button"
import { useInlineNumber } from "@/components/inline-number"
import { InlineNumberInput } from "@/components/inline-number-input"
import { Switch } from "@/components/ui/switch"
import { Trans, useTranslation } from "@/i18n"
import {
  autoExitOf,
  backPosition,
  checkAutoExitSeconds,
  isFolderPage,
  moveBackToReservedKey,
  reservedBackControl,
  setAutoExit,
} from "@/lib/folder-back"
import {
  discardRecovered,
  recoveredFolderLabel,
  recoveredOf,
  resolveFolderLabel,
  restoreRecovered,
} from "@/lib/folder-control"
import { controlLabel, page as findPage, setAutoReturn } from "@/lib/layout-model"
import type { ControlId, DeviceLayout, DeviceModel } from "@/lib/types"

interface FolderSettingsProps {
  model: DeviceModel
  draft: DeviceLayout
  pageId: string
  /** Commits a change; `group` merges consecutive edits into one undo step. */
  onChange: (layout: DeviceLayout, group?: string) => void
  /** First in its panel (a folder key's Inspector): no separator above it. */
  first?: boolean
}

/**
 * The folder `pageId`'s Auto Exit and Exit after action completes (two
 * independent switches), and "Move Back to the first key" for a legacy one.
 */
export function FolderSettings({
  model,
  draft,
  pageId,
  onChange,
  first = false,
}: FolderSettingsProps) {
  const { t } = useTranslation()
  const id = useId()
  const page = findPage(draft, pageId)
  const [moveRefused, setMoveRefused] = useState(false)
  if (!page || !isFolderPage(draft, pageId)) return null
  const legacy = backPosition(draft, pageId) === "legacy"
  return (
    <section
      data-slot="folder-settings"
      aria-label={t("editor:folderSettings.title")}
      className={first ? "grid gap-3" : "grid gap-3 border-t pt-4"}
    >
      <AutoExitRow id={id} draft={draft} pageId={pageId} onChange={onChange} />
      <label className="flex items-center gap-2 text-sm" htmlFor={`${id}-completion`}>
        <Switch
          id={`${id}-completion`}
          checked={page.autoReturn === true}
          onCheckedChange={(checked) => {
            onChange(setAutoReturn(draft, pageId, checked))
          }}
        />
        {t("editor:folderSettings.exitAfterCompletion")}
      </label>
      {legacy && reservedBackControl(model) && (
        <div data-slot="legacy-back" className="grid gap-2">
          <Button
            variant="outline"
            size="sm"
            className="w-fit"
            onClick={() => {
              const moved = moveBackToReservedKey(model, draft, pageId)
              if (!moved) {
                setMoveRefused(true)
                return
              }
              setMoveRefused(false)
              onChange(moved.layout)
            }}
          >
            <ArrowLeft /> {t("editor:folderSettings.moveBack")}
          </Button>
          {moveRefused && (
            <p role="alert" className="text-xs text-destructive">
              {t("editor:folderSettings.noFreeKey")}
            </p>
          )}
        </div>
      )}
    </section>
  )
}

/**
 * Auto Exit in one row: "[switch] Automatic exit" while it is off, and
 * "[switch] Automatic exit after [10] seconds" while it is on, the time an
 * inline field between the words (its order and the unit's plural come
 * from the translation). The time is kept while the switch is off. What is
 * typed is stored with Enter or when the field is left, if it is a whole
 * number of seconds from 1 to 3600 (one undo step each); anything else is
 * never stored: it is shown with why while typed, and Escape, Enter or
 * leaving the field bring back the stored time. The row keeps its height either way, and wraps between its parts
 * when it is narrow.
 */
function AutoExitRow({
  id,
  draft,
  pageId,
  onChange,
}: {
  id: string
  draft: DeviceLayout
  pageId: string
  onChange: (layout: DeviceLayout, group?: string) => void
}) {
  const { t } = useTranslation()
  const stored = autoExitOf(findPage(draft, pageId))
  // What is typed: stored while it is valid, kept (with why) when not.
  const seconds = useInlineNumber({
    stored: stored.seconds,
    check: checkAutoExitSeconds,
    onCommit: (value) => {
      if (value !== null) onChange(setAutoExit(draft, pageId, { ...stored, seconds: value }))
    },
  })
  const error = seconds.error as ReturnType<typeof checkAutoExitSeconds>
  const shownSeconds = seconds.typed ?? stored.seconds
  const field = (
    <InlineNumberInput
      id={`${id}-seconds`}
      field={seconds}
      aria-label={t("editor:folderSettings.timeout")}
      aria-describedby={error ? `${id}-error` : undefined}
    />
  )
  return (
    <div className="grid gap-1.5">
      <div
        data-slot="auto-exit"
        className="flex min-h-8 flex-wrap items-center gap-x-2 gap-y-1 text-sm"
      >
        <Switch
          id={`${id}-enabled`}
          aria-label={t("editor:folderSettings.autoExit")}
          checked={stored.enabled}
          onCheckedChange={(checked) => {
            onChange(setAutoExit(draft, pageId, { ...stored, enabled: checked }))
          }}
        />
        {stored.enabled ? (
          <span data-slot="auto-exit-text" className="flex flex-wrap items-center gap-x-2 gap-y-1">
            <Trans
              i18nKey="editor:folderSettings.autoExitAfter"
              count={shownSeconds}
              components={{ field }}
            />
          </span>
        ) : (
          <label htmlFor={`${id}-enabled`}>{t("editor:folderSettings.autoExit")}</label>
        )}
      </div>
      {stored.enabled && error && (
        <p id={`${id}-error`} role="alert" className="text-xs text-destructive">
          {t(error, { min: 1, max: 3600 })}
        </p>
      )}
    </div>
  )
}

interface FolderControlPanelProps {
  model: DeviceModel
  draft: DeviceLayout
  /** The page the folder key is on. */
  pageId: string
  control: ControlId
  /** The folder the key opens. */
  folder: string
  onChange: (layout: DeviceLayout, group?: string) => void
}

/**
 * The Inspector of a folder key (`lib/folder-control.ts`): it only opens
 * its folder, so it shows the folder's settings and nothing else — Auto
 * Exit, the title and the gestures a layout from before kept on the key
 * when there are any. The folder's name is the key header's title. Its look is the appearance panel below it
 * (inspector.tsx). The folder opens with a double click on its key (or
 * the key's menu), so there is no button for it here.
 */
export function FolderControlPanel({
  model,
  draft,
  pageId,
  control,
  folder,
  onChange,
}: FolderControlPanelProps) {
  return (
    <>
      <FolderSettings
        key={folder}
        model={model}
        draft={draft}
        pageId={folder}
        onChange={onChange}
        first
      />
      <RecoveredTitle draft={draft} pageId={pageId} control={control} onChange={onChange} />
      <RecoveredGestures
        model={model}
        draft={draft}
        pageId={pageId}
        control={control}
        onChange={onChange}
      />
    </>
  )
}

/**
 * The title a folder key had of its own before the folder's name became
 * its one title (migration 13, an import): offered as the folder's name or
 * discarded. Nothing when the key kept none.
 */
function RecoveredTitle({
  draft,
  pageId,
  control,
  onChange,
}: {
  draft: DeviceLayout
  pageId: string
  control: ControlId
  onChange: (layout: DeviceLayout, group?: string) => void
}) {
  const { t } = useTranslation()
  const label = recoveredFolderLabel(draft, pageId, control)
  if (label === undefined) return null
  return (
    <section
      data-slot="recovered-title"
      aria-label={t("editor:folderControl.recoveredTitle.title")}
      className="grid gap-2 border-t pt-4"
    >
      <p className="text-sm">{t("editor:folderControl.recoveredTitle.text", { label })}</p>
      <div className="flex flex-wrap gap-2">
        <Button
          variant="outline"
          size="sm"
          onClick={() => {
            onChange(resolveFolderLabel(draft, pageId, control, true))
          }}
        >
          {t("editor:folderControl.recoveredTitle.use")}
        </Button>
        <Button
          variant="ghost"
          size="sm"
          onClick={() => {
            onChange(resolveFolderLabel(draft, pageId, control, false))
          }}
        >
          <Trash2 /> {t("editor:folderControl.recoveredTitle.discard")}
        </Button>
      </div>
    </section>
  )
}

/**
 * "Recovered gestures (n)": what a folder key ran besides opening its
 * folder before folder keys were core controls (migration 12, an import).
 * Never run; moved to the page's first free key, or discarded after the
 * shared destructive confirmation. Nothing when the key has none.
 */
function RecoveredGestures({
  model,
  draft,
  pageId,
  control,
  onChange,
}: {
  model: DeviceModel
  draft: DeviceLayout
  pageId: string
  control: ControlId
  onChange: (layout: DeviceLayout, group?: string) => void
}) {
  const { t } = useTranslation()
  const dialogs = useContext(DialogsContext)
  const [refused, setRefused] = useState(false)
  const count = recoveredOf(draft, pageId, control).length
  if (count === 0) return null
  return (
    <section
      data-slot="recovered-gestures"
      aria-label={t("editor:folderControl.recovered.title", { count })}
      className="grid gap-2 border-t pt-4"
    >
      <p className="text-sm font-medium">{t("editor:folderControl.recovered.title", { count })}</p>
      <div className="flex flex-wrap gap-2">
        <Button
          variant="outline"
          size="sm"
          onClick={() => {
            const moved = restoreRecovered(model, draft, pageId, control)
            if (!moved) {
              setRefused(true)
              return
            }
            setRefused(false)
            onChange(moved.layout)
          }}
        >
          <ArrowRightLeft /> {t("editor:folderControl.recovered.move")}
        </Button>
        <Button
          variant="ghost"
          size="sm"
          onClick={() => {
            void (async () => {
              const ok = dialogs
                ? await dialogs.destructive({
                    title: t("editor:folderControl.recovered.discardTitle", { count }),
                    description: t("editor:folderControl.recovered.discardDescription", {
                      control: controlLabel(control),
                    }),
                    confirmLabel: t("editor:folderControl.recovered.discard"),
                  })
                : false
              if (ok) onChange(discardRecovered(draft, pageId, control))
            })()
          }}
        >
          <Trash2 /> {t("editor:folderControl.recovered.discard")}
        </Button>
      </div>
      {refused && (
        <p role="alert" className="text-xs text-destructive">
          {t("editor:folderControl.recovered.noFreeKey")}
        </p>
      )}
    </section>
  )
}
