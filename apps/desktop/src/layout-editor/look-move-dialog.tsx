import { Button } from "@/components/ui/button"
import { ModalShell } from "@/components/modal-shell"
import { useTranslation } from "@/i18n"
import { LABEL_LIMITS } from "@/lib/key-labels"

import type { LookMove } from "./labels"

/**
 * Asked when removing a gesture moves the key's look, with its labels, onto
 * a binding that has a look of its own: keep that look (the moving labels
 * are deleted), replace it, or merge the labels (the moved ones win; its
 * title, icon and colours stay). A merge that would give the key more
 * labels than it may have is not offered: labels are never cut.
 */
export function LookMoveDialog({
  open,
  labels,
  trigger,
  merged,
  onChoose,
}: {
  open: boolean
  /** How many labels the moving look has. */
  labels: number
  /** The trigger that shows the key now, in words. */
  trigger: string
  /** How many labels a merge would give the key (default: within the limit). */
  merged?: number
  /** `null`: cancelled, nothing is removed. */
  onChoose: (move: LookMove | null) => void
}) {
  const { t } = useTranslation()
  const tooMany = merged !== undefined && merged > LABEL_LIMITS.labels
  return (
    <ModalShell
      presentation="dialog"
      open={open}
      role="alertdialog"
      title={t("editor:labels.move.title")}
      description={t("editor:labels.move.description", { count: labels, trigger })}
      onClose={() => {
        onChoose(null)
      }}
      footer={
        <div className="grid gap-2">
          {tooMany && (
            <p className="text-xs text-muted-foreground" role="note">
              {t("editor:labels.move.mergeTooMany", { total: merged, max: LABEL_LIMITS.labels })}
            </p>
          )}
          <div className="flex flex-wrap justify-end gap-2">
            <Button
              variant="outline"
              onClick={() => {
                onChoose(null)
              }}
            >
              {t("common:actions.cancel")}
            </Button>
            <Button
              variant="outline"
              onClick={() => {
                onChoose("keep")
              }}
            >
              {t("editor:labels.move.keep", { count: labels })}
            </Button>
            <Button
              variant="outline"
              onClick={() => {
                onChoose("replace")
              }}
            >
              {t("editor:labels.move.replace")}
            </Button>
            <Button
              disabled={tooMany}
              onClick={() => {
                onChoose("merge")
              }}
            >
              {t("editor:labels.move.merge")}
            </Button>
          </div>
        </div>
      }
    />
  )
}
