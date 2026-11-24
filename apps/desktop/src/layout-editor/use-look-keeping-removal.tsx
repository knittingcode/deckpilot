import { useState } from "react"
import { toast } from "sonner"

import { useTranslation } from "@/i18n"
import { triggerLabel } from "@/lib/layout-model"
import type { ControlId, DeviceLayout, Trigger } from "@/lib/types"

import { type LookMove, keepLookOnRemoval, lookTransfer } from "./labels"
import { LookMoveDialog } from "./look-move-dialog"

/** A removal waiting for the user's choice of how the look moves. */
interface PendingRemoval {
  /** The removal itself, applied again to the draft as it is when the user chooses. */
  remove: (layout: DeviceLayout) => DeviceLayout
  labels: number
  merged: number
  to: Trigger
}

/**
 * Removes bindings of `control` keeping the key's title and look on the
 * binding shown after (`keepLookOnRemoval`), in the same step; says when
 * labels moved, and asks first (the returned `dialog`) when that binding has
 * a look of its own. The choice is applied to the latest draft
 * (`onUpdate`), so edits made while the dialog was open stay.
 */
export function useLookKeepingRemoval({
  draft,
  pageId,
  control,
  onChange,
  onUpdate,
}: {
  draft: DeviceLayout
  pageId: string
  control: ControlId
  onChange: (layout: DeviceLayout) => void
  /** Applies an edit to the latest draft; without it, to `draft` as last rendered. */
  onUpdate?: ((edit: (layout: DeviceLayout) => DeviceLayout) => void) | undefined
}) {
  const { t } = useTranslation()
  const [pending, setPending] = useState<PendingRemoval | null>(null)
  const remove = (removal: (layout: DeviceLayout) => DeviceLayout) => {
    const after = removal(draft)
    const transfer = lookTransfer(draft, after, pageId, control)
    if (transfer?.conflict) {
      setPending({
        remove: removal,
        labels: transfer.labels,
        merged: transfer.merged,
        to: transfer.to.trigger,
      })
      return
    }
    onChange(keepLookOnRemoval(draft, after, pageId, control))
    if (transfer && transfer.labels > 0) {
      toast.info(
        t("editor:labels.moved", {
          count: transfer.labels,
          trigger: triggerLabel(transfer.to.trigger),
        }),
      )
    }
  }
  const choose = (move: LookMove | null) => {
    const removal = pending
    setPending(null)
    if (!removal || move === null) return
    const edit = (latest: DeviceLayout) => {
      const after = removal.remove(latest)
      // A merge that no longer fits (labels were added meanwhile) removes nothing.
      if (move === "merge" && lookTransfer(latest, after, pageId, control)?.mergeable === false) {
        return latest
      }
      return keepLookOnRemoval(latest, after, pageId, control, move)
    }
    if (onUpdate) onUpdate(edit)
    else onChange(edit(draft))
  }
  const dialog = (
    <LookMoveDialog
      open={pending !== null}
      labels={pending?.labels ?? 0}
      merged={pending?.merged}
      trigger={pending ? triggerLabel(pending.to) : ""}
      onChoose={choose}
    />
  )
  return { remove, dialog }
}
