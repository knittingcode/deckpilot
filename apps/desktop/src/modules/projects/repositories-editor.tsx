// The Repositories section of the project form (docs/data-model.md,
// "Repositories"): add (native folder picker first), rename, reorder,
// describe, choose another folder, mark the default and remove. A folder
// is bound on this computer (Settings › Resources) only when the project
// is saved, under the repository's resource key, so a profile never holds a
// path. Removing a repository never removes its folder's binding, and a
// folder that is gone is shown, never removed.
//
// A saved repository can be linked to a repository on one of the user's
// connections ("Link remote…", Git Providers K1–K3): the dialog writes the
// link at once through the app (`link_repository_remote`), not with the
// form's save, and the form only shows the remotes; its save never sends
// them (the backend keeps them by id).

import {
  DndContext,
  KeyboardSensor,
  PointerSensor,
  closestCenter,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core"
import {
  SortableContext,
  arrayMove,
  sortableKeyboardCoordinates,
  useSortable,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable"
import { CSS } from "@dnd-kit/utilities"
import { open as openDialog } from "@tauri-apps/plugin-dialog"
import { FolderPlus, GripVertical, Link2, MoreHorizontal, Trash2 } from "lucide-react"
import { useId, useState } from "react"
import { toast } from "sonner"

import { ConfirmDialog } from "@/components/dialogs"
import { FieldError } from "@/components/field-error"
import { LocationRow } from "@/components/machine-resource-field"
import { StatusIcon } from "@/components/resource-status-icon"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useTranslation } from "@/i18n"
import { failureDetail } from "@/lib/failure"
import { STATUS } from "@/lib/resource-status"
import { baseName } from "@/lib/system-actions"
import type { RepositoryRemote, ResourceCheck } from "@/lib/types"
import { LinkRemoteDialog } from "@/plugins/connections/link-remote-dialog"

import { MAX_REPOSITORY_NAME, type RepositoryRow, proposeKey, rowProblems } from "./repositories"

export function RepositoriesEditor({
  rows,
  onChange,
  defaultId,
  onDefault,
  initialDefaultId,
  projectKey,
  savedRows,
  roleResource,
  checks,
  link,
}: {
  rows: RepositoryRow[]
  onChange: (rows: RepositoryRow[]) => void
  defaultId: string | null
  onDefault: (id: string | null) => void
  /** The default when the form opened, to say what marking another changes. */
  initialDefaultId: string | null
  /** The project's key as saved (`usedBy` names it so), `null` for a new one. */
  projectKey: string | null
  /** The repositories as saved, to tell this project's own uses of a folder. */
  savedRows: RepositoryRow[]
  /** What the project's `repository` role points at now. */
  roleResource: string | null
  checks: ResourceCheck[]
  /**
   * Linking remotes: the saved project, and each saved repository's remotes
   * as stored now. `null` for a project not saved yet (nothing to link to).
   */
  link: {
    profileId: string
    projectId: string
    remotes: Record<string, RepositoryRemote[]>
    onLinked: (repositoryId: string, remotes: RepositoryRemote[]) => void
  } | null
}) {
  const { t } = useTranslation()
  const [linking, setLinking] = useState<RepositoryRow | null>(null)
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  )
  const update = (id: string, patch: Partial<RepositoryRow>) => {
    onChange(rows.map((row) => (row.id === id ? { ...row, ...patch } : row)))
  }
  const move = (from: number, to: number) => {
    if (to < 0 || to >= rows.length) return
    onChange(arrayMove(rows, from, to))
  }
  const onDragEnd = (event: DragEndEvent) => {
    const from = rows.findIndex((r) => r.id === event.active.id)
    const to = rows.findIndex((r) => r.id === event.over?.id)
    if (from >= 0 && to >= 0 && from !== to) move(from, to)
  }
  const pickFolder = async (): Promise<string | null> => {
    try {
      const picked = await openDialog({ directory: true, multiple: false })
      return typeof picked === "string" ? picked : null
    } catch (e) {
      toast.error(t("workspace:projects.repositories.pickFailed"), {
        description: failureDetail(e),
      })
      return null
    }
  }
  const add = async () => {
    const path = await pickFolder()
    if (!path) return
    const folder = baseName(path)
    const key = proposeKey(
      folder,
      rows.map((r) => r.key.trim()),
    )
    onChange([
      ...rows,
      {
        id: crypto.randomUUID(),
        key,
        name: Array.from(folder.trim()).slice(0, MAX_REPOSITORY_NAME).join("") || key,
        description: "",
        resource: null,
        pendingPath: path,
      },
    ])
  }
  const marked = rows.find((r) => r.id === defaultId)
  const defaultChanged =
    marked !== undefined && defaultId !== initialDefaultId && marked.resource !== roleResource
  const removedDefault =
    initialDefaultId !== null && !rows.some((r) => r.id === initialDefaultId) && defaultId === null

  return (
    <section className="grid gap-2" aria-labelledby="repositories-title">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <Label id="repositories-title" asChild>
          <h3>{t("workspace:projects.repositories.title")}</h3>
        </Label>
        <Button type="button" variant="outline" size="sm" onClick={() => void add()}>
          <FolderPlus /> {t("workspace:projects.repositories.add")}
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">
        {t("workspace:projects.repositories.description")}
      </p>
      {rows.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          {t("workspace:projects.repositories.empty")}
        </p>
      ) : (
        <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={onDragEnd}>
          <SortableContext items={rows.map((r) => r.id)} strategy={verticalListSortingStrategy}>
            <ol className="grid gap-2">
              {rows.map((row, index) => (
                <RepositoryItem
                  key={row.id}
                  row={row}
                  index={index}
                  count={rows.length}
                  problems={rowProblems(rows, index)}
                  isDefault={row.id === defaultId}
                  roleResource={roleResource}
                  ownUse={
                    projectKey === null
                      ? null
                      : (() => {
                          const saved = savedRows.find((r) => r.id === row.id)
                          return saved ? `${projectKey} repositories.${saved.key}` : null
                        })()
                  }
                  roleUse={projectKey === null ? null : `${projectKey} repository`}
                  removable={rows.length > 1 || roleResource === null}
                  check={row.resource ? checks.find((c) => c.key === row.resource) : undefined}
                  remotes={link?.remotes[row.id] ?? []}
                  // Only a repository the profile has (saved, or the one its
                  // role stands for) can be linked.
                  linkable={link !== null && savedRows.some((r) => r.id === row.id)}
                  savedName={savedRows.find((r) => r.id === row.id)?.name ?? null}
                  onLink={() => {
                    setLinking(row)
                  }}
                  onChange={(patch) => {
                    update(row.id, patch)
                  }}
                  onDefault={() => {
                    onDefault(row.id)
                  }}
                  onMove={(to) => {
                    move(index, to)
                  }}
                  onChooseFolder={async () => {
                    const path = await pickFolder()
                    if (path) update(row.id, { pendingPath: path })
                  }}
                  onRemove={() => {
                    onChange(rows.filter((r) => r.id !== row.id))
                    if (row.id === defaultId) onDefault(null)
                  }}
                />
              ))}
            </ol>
          </SortableContext>
        </DndContext>
      )}
      {defaultChanged && (
        <p className="text-xs text-muted-foreground" role="status">
          {t("workspace:projects.repositories.roleFollows", { name: marked.name || marked.key })}
        </p>
      )}
      {removedDefault && roleResource && (
        <p className="text-xs text-muted-foreground" role="status">
          {t("workspace:projects.repositories.roleStays")}
        </p>
      )}
      {link && linking && (
        <LinkRemoteDialog
          profileId={link.profileId}
          projectId={link.projectId}
          repositoryId={linking.id}
          repositoryName={
            // As saved: the link is made on the stored repository.
            savedRows.find((r) => r.id === linking.id)?.name ??
            (linking.name.trim() || linking.key.trim())
          }
          remotes={link.remotes[linking.id] ?? []}
          onClose={() => {
            setLinking(null)
          }}
          onLinked={(remotes) => {
            link.onLinked(linking.id, remotes)
            toast.success(t("workspace:projects.repositories.linked"))
          }}
        />
      )}
    </section>
  )
}

/**
 * Stored text as shown: without bidirectional and other invisible format
 * characters, so a URL from an imported profile cannot read as another.
 */
function shown(text: string): string {
  return text.replace(/[\u061c\u200b-\u200f\u202a-\u202e\u2060-\u2069\ufeff]/g, "")
}

function RepositoryItem({
  row,
  index,
  count,
  problems,
  isDefault,
  roleResource,
  ownUse,
  roleUse,
  removable,
  check,
  remotes,
  linkable,
  savedName,
  onLink,
  onChange,
  onDefault,
  onMove,
  onChooseFolder,
  onRemove,
}: {
  row: RepositoryRow
  index: number
  count: number
  problems: ReturnType<typeof rowProblems>
  isDefault: boolean
  roleResource: string | null
  /** How `usedBy` names this repository itself (`SHOP repositories.cms`). */
  ownUse: string | null
  /** How `usedBy` names this project's repository role (`SHOP repository`). */
  roleUse: string | null
  /** The last repository stays while the repository role would bring it back. */
  removable: boolean
  check: ResourceCheck | undefined
  /** Its remotes as stored (shown; the form never edits them). */
  remotes: RepositoryRemote[]
  /** Whether it can be linked now (it is saved). */
  linkable: boolean
  /** Its name as saved: the link is made on the stored repository. */
  savedName: string | null
  onLink: () => void
  onChange: (patch: Partial<RepositoryRow>) => void
  onDefault: () => void
  onMove: (to: number) => void
  onChooseFolder: () => Promise<void>
  onRemove: () => void
}) {
  const { t } = useTranslation()
  const ids = useId()
  const { attributes, listeners, setNodeRef, transform, transition } = useSortable({ id: row.id })
  const label = row.name.trim() || row.key.trim() || t("workspace:projects.repositories.unnamed")
  const keyError =
    problems.key === "duplicate"
      ? t("workspace:projects.repositories.duplicateKey", { key: row.key.trim() })
      : problems.key === "invalid"
        ? t("workspace:projects.repositories.invalidKey")
        : null
  // Not bound yet (a new repository, or another folder chosen): the folder
  // is bound when the project is saved.
  const status = row.pendingPath ? null : (check?.status ?? (row.resource ? "unbound" : null))
  const path = row.pendingPath ?? check?.value ?? null
  const usesRole = row.resource !== null && row.resource === roleResource
  // What else uses this folder besides this repository: other projects'
  // roles and repositories (the role of this one is said on its own).
  const users = (check?.usedBy ?? []).filter((u) => u !== ownUse && !(usesRole && u === roleUse))
  return (
    <li
      ref={setNodeRef}
      style={{ transform: CSS.Transform.toString(transform), transition }}
      className="grid gap-2 rounded-md border bg-background p-2"
      data-slot="repository-row"
      aria-label={t("workspace:projects.repositories.item", { name: label })}
    >
      <div className="flex items-center gap-2">
        <button
          type="button"
          className="cursor-grab rounded p-1 text-muted-foreground hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
          aria-label={t("workspace:projects.repositories.reorder", { name: label })}
          {...attributes}
          {...listeners}
        >
          <GripVertical className="size-4" />
        </button>
        <label className="flex items-center gap-1.5 text-sm">
          <input
            type="radio"
            name="default-repository"
            className="accent-primary"
            checked={isDefault}
            onChange={onDefault}
            aria-label={t("workspace:projects.repositories.markDefault", { name: label })}
          />
          {isDefault
            ? t("workspace:projects.repositories.default")
            : t("workspace:projects.repositories.makeDefault")}
        </label>
        {status && (
          <span className="ml-auto flex items-center gap-1 text-xs text-muted-foreground">
            <StatusIcon status={status} />
            {status === "missing"
              ? t("workspace:projects.repositories.folderMissing")
              : t(STATUS[status].label)}
          </span>
        )}
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              className={status ? undefined : "ml-auto"}
              aria-label={t("workspace:projects.repositories.menu", { name: label })}
            >
              <MoreHorizontal />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem
              disabled={index === 0}
              onSelect={() => {
                onMove(index - 1)
              }}
            >
              {t("workspace:projects.repositories.moveUp")}
            </DropdownMenuItem>
            <DropdownMenuItem
              disabled={index === count - 1}
              onSelect={() => {
                onMove(index + 1)
              }}
            >
              {t("workspace:projects.repositories.moveDown")}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
        <ConfirmDialog
          trigger={
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              disabled={!removable}
              title={removable ? undefined : t("workspace:projects.repositories.lastWithRole")}
              aria-label={t("workspace:projects.repositories.remove", { name: label })}
            >
              <Trash2 />
            </Button>
          }
          title={t("workspace:projects.repositories.removeTitle", { name: label })}
          description={t("workspace:projects.repositories.removeDescription")}
          confirmLabel={t("workspace:projects.repositories.removeConfirm")}
          onConfirm={onRemove}
        >
          {(usesRole || users.length > 0) && (
            <ul className="grid list-disc gap-1 pl-5 text-sm">
              {usesRole && <li>{t("workspace:projects.repositories.usesRole")}</li>}
              {users.map((user) => (
                <li key={user}>{t("workspace:projects.repositories.usedBy", { user })}</li>
              ))}
            </ul>
          )}
        </ConfirmDialog>
      </div>
      <div className="grid grid-cols-[1fr_10rem] gap-2">
        <div className="grid gap-1">
          <Label htmlFor={`${ids}-name`}>{t("workspace:projects.repositories.name")}</Label>
          <Input
            id={`${ids}-name`}
            value={row.name}
            aria-invalid={problems.name ? true : undefined}
            aria-describedby={problems.name ? `${ids}-name-error` : undefined}
            onChange={(e) => {
              onChange({ name: e.target.value })
            }}
          />
          <FieldError
            id={`${ids}-name-error`}
            message={problems.name ? t("workspace:projects.repositories.nameTooLong") : null}
          />
        </div>
        <div className="grid gap-1">
          <Label htmlFor={`${ids}-key`}>{t("workspace:projects.repositories.key")}</Label>
          <Input
            id={`${ids}-key`}
            value={row.key}
            className="font-mono"
            aria-invalid={keyError ? true : undefined}
            aria-describedby={keyError ? `${ids}-key-error` : undefined}
            onChange={(e) => {
              onChange({ key: e.target.value })
            }}
          />
          <FieldError id={`${ids}-key-error`} message={keyError} />
        </div>
      </div>
      <div className="grid gap-1">
        <Label htmlFor={`${ids}-description`}>
          {t("workspace:projects.repositories.descriptionLabel")}
        </Label>
        <Input
          id={`${ids}-description`}
          value={row.description}
          aria-invalid={problems.description ? true : undefined}
          aria-describedby={problems.description ? `${ids}-description-error` : undefined}
          onChange={(e) => {
            onChange({ description: e.target.value })
          }}
        />
        <FieldError
          id={`${ids}-description-error`}
          message={
            problems.description ? t("workspace:projects.repositories.descriptionTooLong") : null
          }
        />
      </div>
      {/* The folder picker of #420/#429: the native dialog, the path, and
          whether a bound folder is still there (`resource_status`). */}
      <LocationRow
        id={`${ids}-folder`}
        resource={row.pendingPath ? null : row.resource}
        path={path}
        folder
        files={false}
        busy={false}
        // "Folder not found" is in the row's header already.
        showGone={false}
        onBrowse={() => void onChooseFolder()}
      />
      {row.pendingPath && (
        <p className="text-xs text-muted-foreground">
          {t("workspace:projects.repositories.boundOnSave")}
        </p>
      )}
      {row.pendingPath && row.resource !== null && users.length > 0 && (
        // The folder is bound under the same key, so whatever else uses
        // that key moves with it.
        <p className="text-xs text-muted-foreground" role="note">
          {t("workspace:projects.repositories.rebindShared", { users: users.join(", ") })}
        </p>
      )}
      <div className="grid gap-1">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <span className="text-sm font-medium" id={`${ids}-remotes`}>
            {t("workspace:projects.repositories.remotes")}
          </span>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={!linkable}
            title={linkable ? undefined : t("workspace:projects.repositories.linkNeedsSave")}
            aria-label={t("workspace:projects.repositories.linkFor", {
              name: savedName || label,
            })}
            onClick={onLink}
          >
            <Link2 /> {t("workspace:projects.repositories.link")}
          </Button>
        </div>
        {remotes.length === 0 ? (
          <p className="text-xs text-muted-foreground">
            {t("workspace:projects.repositories.noRemotes")}
          </p>
        ) : (
          <ul className="grid gap-1 text-xs" aria-labelledby={`${ids}-remotes`}>
            {remotes.map((remote) => (
              <li key={remote.name} className="flex flex-wrap items-baseline gap-x-2">
                <span className="font-mono font-medium">{shown(remote.name)}</span>
                {remote.primary && (
                  <span className="text-muted-foreground">
                    {t("workspace:projects.repositories.primaryRemote")}
                  </span>
                )}
                {remote.remoteUrl && (
                  <span className="font-mono break-all text-muted-foreground">
                    {shown(remote.remoteUrl)}
                  </span>
                )}
                {remote.connectionId && remote.remoteRepositoryId && (
                  <span className="text-muted-foreground">
                    {t("workspace:projects.repositories.linkedRemote", {
                      id: shown(remote.remoteRepositoryId),
                    })}
                  </span>
                )}
              </li>
            ))}
          </ul>
        )}
      </div>
    </li>
  )
}
