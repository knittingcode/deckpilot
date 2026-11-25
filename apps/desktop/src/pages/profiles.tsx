import {
  ArrowLeft,
  Copy,
  Ellipsis,
  Library,
  PanelRight,
  Pencil,
  Plus,
  Power,
  SquarePen,
  Trash2,
} from "lucide-react"
import { useEffect, useId, useLayoutEffect, useRef, useState } from "react"
import { toast } from "sonner"

import { DestructiveDialog, FormDialog } from "@/components/dialogs"
import { EmptyState, ErrorNotice, Loading } from "@/components/page"
import { InlineStatus, Property, PropertyList, SectionHeader } from "@/components/settings-layout"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { Textarea } from "@/components/ui/textarea"
import { useAsync } from "@/hooks/use-async"
import { useTranslation } from "@/i18n"
import { useFormat } from "@/i18n/format"
import { useTitlebar } from "@/hooks/use-titlebar"
import { api } from "@/lib/api"
import { failureMessage, toastFailure } from "@/lib/failure"
import { useAppContext } from "@/lib/app-context"
import { PROJECTS_PLUGIN, useIsPluginActive } from "@/lib/plugin-features"
import { setLayoutProfileFilter } from "@/lib/layout-filter"
import type { ProfileSummary, ProfileView } from "@/lib/types"

import { ProfileSavedActions } from "./profile-saved-actions"

type DialogState =
  | { mode: "create" }
  | { mode: "rename"; profile: ProfileSummary; description: string }
  | { mode: "duplicate"; profile: ProfileSummary }
  | null

/**
 * Settings › Profiles: every profile in one table, and a profile's details
 * in a view of their own (Back returns to the table where it was). Only
 * the explicit commands change anything: opening a profile's details
 * neither activates it nor changes what a device shows. Its layouts are
 * shared references, managed in the Layout Manager filtered to it.
 */
export function ProfilesSettings() {
  const { t } = useTranslation()
  const { profiles, refresh, revision, setEditingProfileId, navigate, openSettings } =
    useAppContext()
  const [dialog, setDialog] = useState<DialogState>(null)
  const [openId, setOpenId] = useState<string | null>(null)
  const [selected, setSelected] = useState<string | null>(null)
  // Where the table was scrolled to when a profile was opened.
  const [savedScroll, setSavedScroll] = useState(0)
  const projectsActive = useIsPluginActive(PROJECTS_PLUGIN)
  // Read only: each profile's description and contents for the table.
  const ids = profiles.profiles.map((p) => p.id)
  const views = useAsync(
    () => Promise.all(ids.map((id) => api.getProfile(id).catch(() => null))),
    `profiles-overview:${revision}:${ids.join(",")}`,
  )
  const viewOf = (id: string) =>
    views.state.status === "ready"
      ? (views.state.data.find((v) => v?.profile.id === id) ?? null)
      : null

  const run = (operation: () => Promise<unknown>, message: string, failed: string) => {
    operation().then(
      () => {
        toast.success(message)
        refresh()
      },
      (e: unknown) => {
        toastFailure(failed, e)
      },
    )
  }
  const activate = (profile: ProfileSummary) => {
    run(
      () => api.setActiveProfile(profile.id),
      t("workspace:profiles.toast.activated", { profile: profile.name }),
      t("workspace:profiles.toast.activateFailed", { profile: profile.name }),
    )
  }

  useTitlebar({
    actions: [
      {
        id: "profiles.new",
        label: t("workspace:profiles.new"),
        icon: Plus,
        placement: "primary",
        onSelect: () => {
          setDialog({ mode: "create" })
        },
      },
    ],
  })

  const openDetails = (profile: ProfileSummary) => {
    api
      .getProfile(profile.id)
      .then((view) => {
        setDialog({ mode: "rename", profile, description: view.profile.description })
      })
      .catch((e: unknown) => {
        toastFailure(t("workspace:profiles.toast.openFailed", { profile: profile.name }), e)
      })
  }
  const show = (profile: ProfileSummary) => {
    setSavedScroll(scrollParent(rowOf(profile.id))?.scrollTop ?? 0)
    setSelected(profile.id)
    setOpenId(profile.id)
  }
  const back = () => {
    setOpenId(null)
  }
  // Back: the table where it was, the profile's row focused.
  const shownBefore = useRef<string | null>(null)
  useLayoutEffect(() => {
    const was = shownBefore.current
    shownBefore.current = openId
    if (openId !== null || was === null) return
    const row = rowOf(was)
    const parent = scrollParent(row)
    if (parent) parent.scrollTop = savedScroll
    row?.querySelector<HTMLElement>("[data-slot='profile-open']")?.focus({ preventScroll: true })
  }, [openId, savedScroll])
  const manageLayouts = (profile: ProfileSummary) => {
    setLayoutProfileFilter(profile.id)
    openSettings("layouts")
  }
  const menu = (profile: ProfileSummary, active: boolean) => (
    <ProfileMenu
      profile={profile}
      active={active}
      onOpen={
        openId === profile.id
          ? undefined
          : () => {
              show(profile)
            }
      }
      onRename={() => {
        openDetails(profile)
      }}
      onActivate={
        active
          ? undefined
          : () => {
              activate(profile)
            }
      }
      onManageLayouts={() => {
        manageLayouts(profile)
      }}
      onDuplicate={() => {
        setDialog({ mode: "duplicate", profile })
      }}
      onDelete={async () => {
        await api.deleteProfile(profile.id)
        toast.success(t("workspace:profiles.toast.deleted"))
        if (openId === profile.id) back()
        refresh()
      }}
    />
  )

  const opened = profiles.profiles.find((p) => p.id === openId)
  return (
    <>
      {opened ? (
        <ProfileDetails
          profile={opened}
          view={viewOf(opened.id)}
          loading={views.state.status === "loading"}
          active={opened.id === profiles.activeProfileId}
          activeName={profiles.profiles.find((p) => p.id === profiles.activeProfileId)?.name}
          projects={projectsActive}
          menu={menu(opened, opened.id === profiles.activeProfileId)}
          onBack={back}
          onEdit={() => {
            openDetails(opened)
          }}
          onActivate={() => {
            activate(opened)
          }}
          onOpenInEditor={() => {
            setEditingProfileId(opened.id)
            navigate("designer")
          }}
          onManageLayouts={() => {
            manageLayouts(opened)
          }}
        />
      ) : (
        <div className="grid min-w-0">
          {profiles.profiles.length === 0 ? (
            <EmptyState variant="plain" title={t("workspace:profiles.empty.title")}>
              {t("workspace:profiles.empty.body")}
            </EmptyState>
          ) : (
            <Table aria-label={t("workspace:profiles.table.label")} className="table-fixed">
              <TableHeader>
                <TableRow className="hover:bg-transparent">
                  <TableHead className="w-40">{t("workspace:profiles.table.name")}</TableHead>
                  <TableHead>{t("workspace:profiles.table.description")}</TableHead>
                  <TableHead className="hidden w-40 md:table-cell">
                    {t("workspace:profiles.table.contents")}
                  </TableHead>
                  <TableHead className="w-24">{t("workspace:profiles.table.status")}</TableHead>
                  <TableHead className="w-10">
                    <span className="sr-only">{t("workspace:profiles.table.actions")}</span>
                  </TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {profiles.profiles.map((profile) => {
                  const active = profile.id === profiles.activeProfileId
                  const view = viewOf(profile.id)
                  const description = view?.profile.description ?? ""
                  return (
                    <TableRow
                      key={profile.id}
                      data-profile={profile.id}
                      data-state={profile.id === selected ? "selected" : undefined}
                      className="cursor-pointer"
                      onClick={(e) => {
                        // The row opens the profile; its menu is its own.
                        if ((e.target as HTMLElement).closest("[data-slot='profile-menu']")) return
                        show(profile)
                      }}
                    >
                      <TableCell className="truncate">
                        <button
                          type="button"
                          data-slot="profile-open"
                          title={profile.name}
                          aria-label={t("workspace:profiles.detail.open", {
                            profile: profile.name,
                          })}
                          className="max-w-full truncate rounded-sm font-medium outline-none hover:underline focus-visible:ring-[3px] focus-visible:ring-ring/50"
                          onClick={(e) => {
                            e.stopPropagation()
                            show(profile)
                          }}
                        >
                          {profile.name}
                        </button>
                      </TableCell>
                      <TableCell
                        className="truncate text-caption text-muted-foreground"
                        title={description || undefined}
                      >
                        {description}
                      </TableCell>
                      <TableCell className="hidden truncate text-caption text-muted-foreground tabular-nums md:table-cell">
                        {view &&
                          t("workspace:profiles.summary", {
                            layouts: t("workspace:profiles.layoutCount", {
                              count: view.profile.layouts.length,
                            }),
                            actions: t("workspace:profiles.actionCount", {
                              count: view.profile.actions.length,
                            }),
                          })}
                      </TableCell>
                      <TableCell>
                        {active ? (
                          <InlineStatus
                            tone="positive"
                            title={t("workspace:profiles.runsOnDevice")}
                          >
                            {t("workspace:profiles.active")}
                          </InlineStatus>
                        ) : (
                          <span className="sr-only">{t("workspace:profiles.inactive")}</span>
                        )}
                      </TableCell>
                      <TableCell data-slot="profile-menu">
                        <div className="flex justify-end">{menu(profile, active)}</div>
                      </TableCell>
                    </TableRow>
                  )
                })}
              </TableBody>
            </Table>
          )}
        </div>
      )}

      <ProfileDialog
        state={dialog}
        onClose={() => {
          setDialog(null)
        }}
        onDone={(id) => {
          setDialog(null)
          if (id) setEditingProfileId(id)
          refresh()
        }}
      />
    </>
  )
}

/** A profile's row in the table. */
function rowOf(id: string): HTMLElement | null {
  return document.querySelector<HTMLElement>(`[data-profile="${CSS.escape(id)}"]`)
}

/** The element that scrolls `element`: the page's content. */
function scrollParent(element: HTMLElement | null): HTMLElement | null {
  for (let node = element?.parentElement ?? null; node; node = node.parentElement) {
    const { overflowY } = getComputedStyle(node)
    if (overflowY === "auto" || overflowY === "scroll") return node
  }
  return null
}

/**
 * One profile: what it is, what it holds, where it is active and its
 * commands. Escape or Back returns to the table.
 */
function ProfileDetails({
  profile,
  view,
  loading,
  active,
  activeName,
  projects,
  menu,
  onBack,
  onEdit,
  onActivate,
  onOpenInEditor,
  onManageLayouts,
}: {
  profile: ProfileSummary
  view: ProfileView | null
  loading: boolean
  active: boolean
  activeName: string | undefined
  projects: boolean
  menu: React.ReactNode
  onBack: () => void
  onEdit: () => void
  onActivate: () => void
  onOpenInEditor: () => void
  onManageLayouts: () => void
}) {
  const { t } = useTranslation()
  const format = useFormat()
  const { agent } = useAppContext()
  const backRef = useRef<HTMLButtonElement>(null)
  useEffect(() => {
    backRef.current?.focus()
  }, [])
  const devices = (agent?.status?.devices ?? []).filter((d) => d.state === "connected")
  const headingId = useId()
  return (
    <section
      aria-labelledby={headingId}
      className="grid gap-6"
      onKeyDown={(e) => {
        if (e.key === "Escape" && !e.defaultPrevented) {
          e.preventDefault()
          onBack()
        }
      }}
    >
      <div>
        <Button ref={backRef} variant="ghost" size="sm" className="-ml-2" onClick={onBack}>
          <ArrowLeft /> {t("workspace:profiles.detail.back")}
        </Button>
      </div>
      <SectionHeader
        id={headingId}
        title={
          <>
            <span className="truncate" title={profile.name}>
              {profile.name}
            </span>
            {active && (
              <InlineStatus tone="positive">{t("workspace:profiles.active")}</InlineStatus>
            )}
          </>
        }
        description={
          view?.profile.description || (
            <span className="italic">{t("workspace:profiles.editing.noDescription")}</span>
          )
        }
        actions={
          <>
            {!active && (
              <Button
                variant="outline"
                size="sm"
                aria-label={t("workspace:profiles.makeActiveLabel", { profile: profile.name })}
                onClick={onActivate}
              >
                {t("workspace:profiles.makeActive")}
              </Button>
            )}
            <Button variant="outline" size="sm" onClick={onEdit}>
              <Pencil /> {t("workspace:profiles.editing.editDetails")}
            </Button>
            {menu}
          </>
        }
      />
      {loading && !view && <Loading />}
      {view && (
        <PropertyList
          className="border-t pt-3"
          aria-label={t("workspace:profiles.editing.details", { profile: profile.name })}
        >
          <Property label={t("workspace:profiles.detail.status")}>
            {active
              ? devices.length > 0
                ? t("workspace:profiles.detail.activeOn", {
                    devices: format.list(
                      devices.map((d) => d.descriptor?.displayName ?? d.modelName),
                    ),
                  })
                : t("workspace:profiles.detail.activeNoDevice")
              : activeName
                ? t("workspace:profiles.detail.inactive", { profile: activeName })
                : t("workspace:profiles.detail.noneActive")}
          </Property>
          <Property label={t("workspace:profiles.editing.actions")}>
            {view.profile.actions.length}
          </Property>
          <Property label={t("workspace:profiles.editing.workflows")}>
            {view.profile.workflows.length}
          </Property>
          {projects && (
            <Property label={t("workspace:profiles.editing.projects")}>
              {view.profile.projects.length}
            </Property>
          )}
          <Property label={t("workspace:profiles.detail.layouts")}>
            <span className="grid justify-items-start gap-1">
              <span>
                {view.profile.layouts.length === 0
                  ? t("workspace:profiles.detail.noLayouts")
                  : t("workspace:profiles.layoutCount", { count: view.profile.layouts.length })}
              </span>
              <span className="text-caption text-muted-foreground">
                {t("workspace:profiles.detail.layoutsShared")}
              </span>
            </span>
          </Property>
        </PropertyList>
      )}
      {view && view.issues.length > 0 && <ErrorNotice error={view.issues.join(" ")} />}
      <div className="flex flex-wrap gap-2 border-t pt-4">
        <Button variant="outline" size="sm" onClick={onManageLayouts}>
          <Library /> {t("workspace:profiles.detail.manageLayouts")}
        </Button>
        <Button variant="outline" size="sm" onClick={onOpenInEditor}>
          <SquarePen /> {t("workspace:profiles.detail.openInEditor")}
        </Button>
      </div>
      {view && <ProfileSavedActions profileId={profile.id} view={view} />}
    </section>
  )
}

/** The profile's commands: open, edit, activate, layouts, duplicate and delete. */
function ProfileMenu({
  profile,
  active,
  onOpen,
  onRename,
  onActivate,
  onManageLayouts,
  onDuplicate,
  onDelete,
}: {
  profile: ProfileSummary
  active: boolean
  /** Absent in the profile's own details. */
  onOpen?: () => void
  onRename: () => void
  /** Absent for the active profile. */
  onActivate?: () => void
  onManageLayouts: () => void
  onDuplicate: () => void
  /** Rejects with the backend's error; the confirmation then shows it. */
  onDelete: () => Promise<void>
}) {
  const { t } = useTranslation()
  const [confirming, setConfirming] = useState(false)
  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            variant="ghost"
            size="icon-sm"
            className="size-7"
            aria-label={t("workspace:profiles.menu.more", { profile: profile.name })}
          >
            <Ellipsis />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          {onOpen && (
            <DropdownMenuItem onSelect={onOpen}>
              <PanelRight /> {t("workspace:profiles.menu.open")}
            </DropdownMenuItem>
          )}
          <DropdownMenuItem onSelect={onRename}>
            <Pencil /> {t("workspace:profiles.menu.rename")}
          </DropdownMenuItem>
          {onActivate && (
            <DropdownMenuItem onSelect={onActivate}>
              <Power /> {t("workspace:profiles.makeActive")}
            </DropdownMenuItem>
          )}
          <DropdownMenuItem onSelect={onManageLayouts}>
            <Library /> {t("workspace:profiles.detail.manageLayouts")}
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={onDuplicate}>
            <Copy /> {t("workspace:profiles.menu.duplicate")}
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem
            variant="destructive"
            onSelect={() => {
              setConfirming(true)
            }}
          >
            <Trash2 /> {t("workspace:profiles.menu.delete")}
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <DestructiveDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={t("workspace:profiles.delete.title", { profile: profile.name })}
        description={
          active ? t("workspace:profiles.delete.active") : t("workspace:profiles.delete.inactive")
        }
        onConfirm={onDelete}
        describeError={(e) =>
          failureMessage(t("workspace:profiles.delete.failed", { profile: profile.name }), e)
        }
      />
    </>
  )
}

function ProfileDialog({
  state,
  onClose,
  onDone,
}: {
  state: DialogState
  onClose: () => void
  onDone: (id?: string) => void
}) {
  if (!state) return null
  return (
    <ProfileForm
      key={state.mode === "create" ? "create" : `${state.mode}:${state.profile.id}`}
      state={state}
      onClose={onClose}
      onDone={onDone}
    />
  )
}

function ProfileForm({
  state,
  onClose,
  onDone,
}: {
  state: NonNullable<DialogState>
  onClose: () => void
  onDone: (id?: string) => void
}) {
  const { t } = useTranslation()
  const initialName =
    state.mode === "create"
      ? ""
      : state.mode === "duplicate"
        ? t("workspace:profiles.form.copyName", { profile: state.profile.name })
        : state.profile.name
  const initialDescription = state.mode === "rename" ? state.description : ""
  const [name, setName] = useState(initialName)
  const [description, setDescription] = useState(initialDescription)
  const titles = {
    create: t("workspace:profiles.form.createTitle"),
    rename: t("workspace:profiles.form.renameTitle"),
    duplicate: t("workspace:profiles.form.duplicateTitle"),
  }
  const failed = {
    create: t("workspace:profiles.form.createFailed"),
    rename: t("workspace:profiles.form.renameFailed"),
    duplicate: t("workspace:profiles.form.duplicateFailed"),
  }

  const submit = async () => {
    if (state.mode === "create") {
      onDone((await api.createProfile(name, description)).id)
    } else if (state.mode === "rename") {
      await api.updateProfileDetails(state.profile.id, name, description)
      onDone()
    } else {
      onDone((await api.duplicateProfile(state.profile.id, name)).id)
    }
    toast.success(t("workspace:profiles.form.saved"))
  }

  return (
    <FormDialog
      open
      title={titles[state.mode]}
      description={
        state.mode === "duplicate" ? t("workspace:profiles.form.duplicateDescription") : undefined
      }
      submitLabel={t("workspace:profiles.form.save")}
      canSubmit={Boolean(name.trim())}
      dirty={name !== initialName || description !== initialDescription}
      onClose={onClose}
      onSubmit={submit}
      describeError={(e) => failureMessage(failed[state.mode], e)}
    >
      <div className="grid gap-2">
        <Label htmlFor="profile-name">{t("workspace:profiles.form.name")}</Label>
        <Input
          id="profile-name"
          value={name}
          autoFocus
          onChange={(e) => {
            setName(e.target.value)
          }}
        />
      </div>
      {state.mode !== "duplicate" && (
        <div className="grid gap-2">
          <Label htmlFor="profile-description">{t("workspace:profiles.form.description")}</Label>
          <Textarea
            id="profile-description"
            value={description}
            onChange={(e) => {
              setDescription(e.target.value)
            }}
          />
        </div>
      )}
    </FormDialog>
  )
}
