// Deleting a workflow that something refers to (docs/data-model.md,
// "Workflows"): the dialog lists every reference, grouped; the keys and
// gestures that start it stay ("Workflow missing") unless the user also
// clears them; graphs and Random/Switch actions are never edited. The
// delete is confirmed with the fingerprint of the references shown, so it
// fails, and the dialog shows the new list, when they changed meanwhile.

import { type ReactNode, useState } from "react"

import { ConfirmDialog } from "@/components/confirm-dialog"
import { Checkbox } from "@/components/ui/checkbox"
import { Label } from "@/components/ui/label"
import { useTranslation } from "@/i18n"
import { api, errorCode, errorMessage } from "@/lib/api"
import { workflowInUse } from "@/lib/backend-error"
import { describeUsage } from "@/lib/format"
import { groupReferences, workflowReferences } from "@/lib/workflow-references"
import type { ProfileView, Usage } from "@/lib/types"

export function WorkflowDeleteDialog({
  trigger,
  profileId,
  view,
  workflow,
  title,
  description,
  onDeleted,
}: {
  trigger: ReactNode
  profileId: string
  view: ProfileView
  workflow: { id: string; name: string }
  title: string
  description: string
  /** After the delete; `cleared`: how many keys and gestures were cleared. */
  onDeleted: (cleared: number) => void
}) {
  const { t } = useTranslation()
  // What the dialog shows: the view's references, or the newer ones a
  // refused delete returned.
  const [shown, setShown] = useState<{ usages: Usage[]; fingerprint?: string } | null>(null)
  const [clear, setClear] = useState(false)
  const [clearSession, setClearSession] = useState(false)
  const usages = shown?.usages ?? workflowReferences(view, workflow.id)
  const fingerprint =
    shown?.fingerprint ?? view.workflowEntries?.find((e) => e.id === workflow.id)?.fingerprint
  const groups = groupReferences(usages)
  const section = (label: string, items: Usage[]) =>
    items.length > 0 && (
      <div className="grid gap-1">
        <p className="text-xs font-medium">{label}</p>
        <ul className="relative grid max-h-40 gap-0.5 overflow-y-auto text-sm text-muted-foreground">
          {items.map((usage, index) => (
            <li key={index}>{describeUsage(usage)}</li>
          ))}
        </ul>
      </div>
    )
  return (
    <ConfirmDialog
      trigger={trigger}
      onOpenChange={(open) => {
        if (!open) {
          setShown(null)
          setClear(false)
          setClearSession(false)
        }
      }}
      title={title}
      description={
        usages.length > 0 ? t("workflows:delete.referenced", { count: usages.length }) : description
      }
      onConfirm={async () => {
        // A Work Session setting is never cleared without the user's say.
        if (groups.workSession.length > 0 && !clearSession) {
          throw new Error(t("workflows:delete.workSessionRequired"))
        }
        try {
          const done =
            fingerprint === undefined
              ? await api.deleteWorkflow(profileId, workflow.id)
              : await api.deleteWorkflow(profileId, workflow.id, {
                  fingerprint,
                  clearBindings: clear,
                  clearWorkSession: clearSession,
                })
          // Older builds answered nothing.
          onDeleted((done as { cleared?: number } | null)?.cleared ?? 0)
        } catch (e) {
          // The references changed (or the build gave no fingerprint):
          // show them as they are now and let the user confirm again.
          const now = workflowInUse(e)
          if (now) {
            setShown({ usages: now.usages, fingerprint: now.fingerprint })
            const workSession = errorCode(e) === "invalid.workflowInWorkSession"
            throw new Error(
              t(workSession ? "workflows:delete.workSessionRequired" : "workflows:delete.changed"),
              { cause: e },
            )
          }
          throw e
        }
      }}
      describeError={(e) => (e instanceof Error ? e.message : errorMessage(e))}
    >
      {usages.length > 0 && (
        <div className="grid gap-3" data-testid="workflow-references">
          {section(t("workflows:delete.keys"), groups.keys)}
          {section(t("workflows:delete.actions"), groups.actions)}
          {section(t("workflows:delete.workflows"), groups.workflows)}
          {groups.keys.length > 0 && (
            <div className="grid gap-1">
              <div className="flex items-center gap-1.5">
                <Checkbox
                  id={`clear-${workflow.id}`}
                  checked={clear}
                  onCheckedChange={(value) => {
                    setClear(value === true)
                  }}
                />
                <Label htmlFor={`clear-${workflow.id}`} className="text-sm font-normal">
                  {t("workflows:delete.clear", { count: groups.keys.length })}
                </Label>
              </div>
              <p className="text-xs text-muted-foreground">
                {clear ? t("workflows:delete.clearHint") : t("workflows:delete.keepHint")}
              </p>
            </div>
          )}
          {groups.workSession.length > 0 && (
            <div className="grid gap-1">
              {section(t("workflows:delete.workSession"), groups.workSession)}
              <div className="flex items-center gap-1.5">
                <Checkbox
                  id={`clear-session-${workflow.id}`}
                  checked={clearSession}
                  onCheckedChange={(value) => {
                    setClearSession(value === true)
                  }}
                />
                <Label htmlFor={`clear-session-${workflow.id}`} className="text-sm font-normal">
                  {t("workflows:delete.clearWorkSession", { count: groups.workSession.length })}
                </Label>
              </div>
              <p className="text-xs text-muted-foreground">
                {t("workflows:delete.workSessionHint")}
              </p>
            </div>
          )}
          {(groups.actions.length > 0 || groups.workflows.length > 0) && (
            <p className="text-xs text-muted-foreground">{t("workflows:delete.untouched")}</p>
          )}
        </div>
      )}
    </ConfirmDialog>
  )
}
