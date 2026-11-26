// Settings › Profiles › a profile › "Saved actions": every saved action of
// the profile with how many places use it, so one that nothing uses any
// more (no key, gesture or workflow) can still be opened, edited or
// deleted. The Device Editor's library lists action types only, and its
// Inspector opens only the saved actions a control runs.
//
// Nothing is deleted without the user: Delete is offered for an unused
// action only and opens the same confirmation as the action editor's
// Delete, which calls the agent's `delete_action` (refused while anything
// uses the action). There is no "delete all".

import { Pencil, Trash2 } from "lucide-react"
import { useId, useMemo, useState } from "react"
import { toast } from "sonner"

import { ActionEditor } from "@/components/action-editor"
import { ActionIcon } from "@/components/action-icon"
import { DeleteActionDialog } from "@/components/action-references"
import { ModalShell } from "@/components/modal-shell"
import { EmptyState } from "@/components/page"
import { Button } from "@/components/ui/button"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { useTranslation } from "@/i18n"
import { useFormat } from "@/i18n/format"
import { referenceTarget } from "@/lib/action-references"
import { useActionModules, useAppContext } from "@/lib/app-context"
import { requestFocus } from "@/lib/focus"
import { withLegacyAliasModules } from "@/lib/plugin-actions"
import type { ActionInstance, ActionReference, ProfileView } from "@/lib/types"

import { type SavedActionFilter, savedActionRows } from "./saved-action-rows"

type Open = { kind: "edit"; id: string } | { kind: "delete"; id: string }

export function ProfileSavedActions({ profileId, view }: { profileId: string; view: ProfileView }) {
  const { t } = useTranslation()
  const format = useFormat()
  const headingId = useId()
  const { profiles, agent, refresh, navigate, setEditingProfileId } = useAppContext()
  // Legacy ids a plugin replaces are edited with the plugin's fields.
  const modules = withLegacyAliasModules(useActionModules())
  const [filter, setFilter] = useState<SavedActionFilter>("all")
  const [open, setOpen] = useState<Open | null>(null)
  const close = () => {
    setOpen(null)
  }
  const all = useMemo(() => savedActionRows(view, "all", format.compare), [view, format])
  const rows = filter === "all" ? all : all.filter((row) => row.references.length === 0)
  const unused = all.filter((row) => row.references.length === 0).length
  const typeName = (action: ActionInstance) =>
    modules.flatMap((m) => m.actionTypes).find((type) => type.id === action.actionType)?.name ??
    action.actionType
  const canRun = Boolean(agent?.running) && profiles.activeProfileId === profileId
  const openReference = (reference: ActionReference) => {
    const target = referenceTarget(reference)
    close()
    // The place is in this profile: the editor shows it.
    setEditingProfileId(profileId)
    requestFocus(target.focus)
    navigate(target.page)
  }
  const current = open ? all.find((row) => row.action.id === open.id) : undefined

  return (
    <section aria-labelledby={headingId} className="grid gap-3 border-t pt-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h3 id={headingId} className="text-sm font-semibold">
          {t("workspace:profiles.savedActions.title")}
        </h3>
        {all.length > 0 && (
          <ToggleGroup
            type="single"
            variant="outline"
            size="sm"
            value={filter}
            onValueChange={(value) => {
              if (value) setFilter(value as SavedActionFilter)
            }}
            aria-label={t("workspace:profiles.savedActions.show")}
          >
            <ToggleGroupItem value="all" className="px-2.5">
              {t("workspace:profiles.savedActions.all")}
              <span className="text-xs text-muted-foreground tabular-nums">{all.length}</span>
            </ToggleGroupItem>
            <ToggleGroupItem value="unused" className="px-2.5">
              {t("workspace:profiles.savedActions.unused")}
              <span className="text-xs text-muted-foreground tabular-nums">{unused}</span>
            </ToggleGroupItem>
          </ToggleGroup>
        )}
      </div>
      {all.length === 0 ? (
        <EmptyState variant="plain" title={t("workspace:profiles.savedActions.empty")} />
      ) : rows.length === 0 ? (
        <EmptyState variant="plain" title={t("workspace:profiles.savedActions.noneUnused")} />
      ) : (
        <Table aria-labelledby={headingId} className="table-fixed">
          <TableHeader>
            <TableRow className="hover:bg-transparent">
              <TableHead>{t("workspace:profiles.savedActions.name")}</TableHead>
              <TableHead className="hidden md:table-cell">
                {t("workspace:profiles.savedActions.type")}
              </TableHead>
              <TableHead className="w-32">{t("workspace:profiles.savedActions.usedIn")}</TableHead>
              <TableHead className="w-24">
                <span className="sr-only">{t("workspace:profiles.table.actions")}</span>
              </TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map(({ action, references, uses }) => (
              <TableRow key={action.id} data-action={action.id}>
                <TableCell className="truncate">
                  <span className="flex min-w-0 items-center gap-2" title={action.name}>
                    <ActionIcon name={action.icon} className="size-4 shrink-0" />
                    <span className="truncate font-medium">{action.name}</span>
                  </span>
                </TableCell>
                <TableCell className="hidden truncate text-caption text-muted-foreground md:table-cell">
                  {typeName(action)}
                </TableCell>
                <TableCell className="text-caption text-muted-foreground tabular-nums">
                  {references.length === 0
                    ? t("workspace:profiles.savedActions.notUsed")
                    : t("workspace:profiles.savedActions.places", { count: uses })}
                </TableCell>
                <TableCell>
                  <div className="flex justify-end gap-1">
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon-sm"
                      aria-label={t("workspace:profiles.savedActions.edit", { name: action.name })}
                      title={t("workspace:profiles.savedActions.edit", { name: action.name })}
                      onClick={() => {
                        setOpen({ kind: "edit", id: action.id })
                      }}
                    >
                      <Pencil />
                    </Button>
                    {references.length === 0 && (
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon-sm"
                        aria-label={t("workspace:profiles.savedActions.delete", {
                          name: action.name,
                        })}
                        title={t("workspace:profiles.savedActions.delete", { name: action.name })}
                        onClick={() => {
                          setOpen({ kind: "delete", id: action.id })
                        }}
                      >
                        <Trash2 />
                      </Button>
                    )}
                  </div>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}
      {current && open?.kind === "delete" && (
        <DeleteActionDialog
          profileId={profileId}
          action={current.action}
          references={current.references}
          candidates={view.profile.actions}
          onOpen={openReference}
          onClose={close}
          onDeleted={() => {
            close()
            toast.success(t("actions:editor.toast.deleted"))
            refresh()
          }}
        />
      )}
      {current && open?.kind === "edit" && (
        <ModalShell
          presentation="dialog"
          open
          size="large"
          title={current.action.name}
          onClose={close}
          secondary={{ label: t("common:close"), onClick: close, variant: "outline" }}
        >
          <ActionEditor
            key={current.action.id}
            profileId={profileId}
            modules={modules}
            projects={view.profile.projects}
            workflows={view.profile.workflows}
            action={current.action}
            references={current.references}
            candidates={view.profile.actions}
            canRun={canRun}
            onSaved={() => {
              refresh()
            }}
            onDeleted={() => {
              close()
              refresh()
            }}
            onOpenReference={openReference}
          />
        </ModalShell>
      )}
    </section>
  )
}
