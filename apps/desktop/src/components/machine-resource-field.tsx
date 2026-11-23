// The pickers of the System actions' machine-local parameters
// (docs/system-actions.md): an installed application or browser from the
// list this computer reports (`list_applications`), or a file or folder
// from the native file dialog. What is picked is bound on this computer
// (Settings › Resources, never exported) and the parameter refers to the
// binding's key, so a profile never holds a path.

import { open as openDialog } from "@tauri-apps/plugin-dialog"
import { AppWindow, ChevronsUpDown, File, Folder, Globe, Laptop } from "lucide-react"
import { useEffect, useId, useState } from "react"
import { toast } from "sonner"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command"
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover"
import { useTranslation } from "@/i18n"
import { api } from "@/lib/api"
import { failureDetail } from "@/lib/failure"
import { baseName, bindingKeyFor } from "@/lib/system-actions"
import type { InstalledApp, MachineResourceBinding, ResourceKind, ResourceRef } from "@/lib/types"

/** What the field picks. */
export type MachinePick = "application" | "browser" | "location"

/**
 * A machine-local resource parameter: what it is bound to on this
 * computer, marked as such, and the pickers that bind it. `optional`
 * offers clearing it (Website's browser: the default browser).
 */
export function MachineResourceField({
  id,
  pick,
  value,
  optional,
  onChange,
}: {
  id: string
  pick: MachinePick
  value: ResourceRef | null
  optional: boolean
  onChange: (value: ResourceRef | null) => void
}) {
  const { t } = useTranslation()
  const noteId = useId()
  const [bindings, setBindings] = useState<MachineResourceBinding[] | null>(null)
  const [busy, setBusy] = useState(false)
  useEffect(() => {
    let cancelled = false
    api.getMachineConfig().then(
      (config) => {
        if (!cancelled) setBindings(config.bindings)
      },
      () => {
        if (!cancelled) setBindings([])
      },
    )
    return () => {
      cancelled = true
    }
  }, [])
  const key = value?.type === "fixed" ? value.key : null
  const bound = key ? bindings?.find((b) => b.key === key) : undefined

  /** Binds `chosen` on this computer and points the parameter at it. */
  const bind = async (
    prefix: "app" | "file" | "folder",
    name: string,
    kind: ResourceKind,
    chosen: string,
  ) => {
    setBusy(true)
    try {
      const current = bindings ?? (await api.getMachineConfig()).bindings
      const next = bindingKeyFor(prefix, name, kind, chosen, current)
      const config = await api.setResourceBinding(next, kind, chosen)
      setBindings(config.bindings)
      onChange({ type: "fixed", key: next })
    } catch (e) {
      toast.error(t("actions:system.picker.failed"), { description: failureDetail(e) })
    } finally {
      setBusy(false)
    }
  }

  const pickPath = async (directory: boolean) => {
    try {
      const picked = await openDialog({ directory, multiple: false })
      if (typeof picked !== "string") return
      if (pick === "location") {
        await bind(
          directory ? "folder" : "file",
          baseName(picked),
          directory ? "directory" : "other",
          picked,
        )
        return
      }
      const app = await api.describeApplication(picked)
      await bind("app", app.name, "application", app.id)
    } catch (e) {
      toast.error(t("actions:system.picker.failed"), { description: failureDetail(e) })
    }
  }

  const shown = bound
    ? bound.value
    : key
      ? t("actions:system.picker.unbound", { key })
      : value?.type === "projectRole"
        ? t("actions:system.picker.role", { role: value.role })
        : pick === "browser"
          ? t("actions:system.picker.defaultBrowser")
          : t("actions:system.picker.nothing")
  const Icon = pick === "browser" ? Globe : pick === "location" ? File : AppWindow
  return (
    <div className="grid gap-1.5" data-slot="machine-resource">
      <div className="flex min-w-0 items-center gap-2">
        <Icon className="size-4 shrink-0 text-muted-foreground" aria-hidden />
        <span
          id={id}
          className="min-w-0 flex-1 truncate text-sm select-text"
          title={bound?.value}
          aria-describedby={noteId}
        >
          {shown}
        </span>
        {key && (
          <Badge variant="outline" className="shrink-0 gap-1">
            <Laptop className="size-3" aria-hidden />
            {t("actions:system.picker.thisComputer")}
          </Badge>
        )}
      </div>
      <div className="flex flex-wrap gap-2">
        {pick === "location" ? (
          <>
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={busy}
              onClick={() => void pickPath(false)}
            >
              <File /> {t("actions:system.picker.chooseFile")}
            </Button>
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={busy}
              onClick={() => void pickPath(true)}
            >
              <Folder /> {t("actions:system.picker.chooseFolder")}
            </Button>
          </>
        ) : (
          <ApplicationPicker
            browsers={pick === "browser"}
            disabled={busy}
            onPick={(app) => void bind("app", app.name, "application", app.id)}
            onBrowse={() => void pickPath(false)}
          />
        )}
        {optional && value && (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            disabled={busy}
            onClick={() => {
              onChange(null)
            }}
          >
            {pick === "browser"
              ? t("actions:system.picker.useDefault")
              : t("actions:system.picker.clear")}
          </Button>
        )}
      </div>
      <p id={noteId} className="text-xs text-muted-foreground" role="note">
        {key ? t("actions:system.picker.localNote", { key }) : t("actions:system.picker.localHint")}
      </p>
      {key && bindings !== null && !bound && (
        <p className="text-xs text-destructive" role="alert">
          {t("actions:system.picker.unboundHint")}
        </p>
      )}
    </div>
  )
}

/** A searchable list of the installed applications (or browsers). */
function ApplicationPicker({
  browsers,
  disabled,
  onPick,
  onBrowse,
}: {
  browsers: boolean
  disabled: boolean
  onPick: (app: InstalledApp) => void
  onBrowse: () => void
}) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const [apps, setApps] = useState<InstalledApp[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    if (!open || apps !== null) return
    let cancelled = false
    api.listApplications().then(
      (list) => {
        if (!cancelled) setApps(list)
      },
      (e: unknown) => {
        if (!cancelled) {
          setApps([])
          setError(failureDetail(e))
        }
      },
    )
    return () => {
      cancelled = true
    }
  }, [open, apps])
  const label = browsers
    ? t("actions:system.picker.chooseBrowser")
    : t("actions:system.picker.chooseApplication")
  const listed = apps ?? []
  const preferred = listed.filter((a) => a.browser)
  const others = browsers ? listed.filter((a) => !a.browser) : listed
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button
          type="button"
          variant="outline"
          size="sm"
          role="combobox"
          aria-label={label}
          aria-expanded={open}
          disabled={disabled}
        >
          {label}
          <ChevronsUpDown className="opacity-50" aria-hidden />
        </Button>
      </PopoverTrigger>
      <PopoverContent className="w-80 p-0" align="start">
        <Command>
          <CommandInput
            placeholder={t("actions:system.picker.search")}
            aria-label={t("actions:system.picker.search")}
          />
          <CommandList>
            {apps === null ? (
              <p className="p-3 text-sm text-muted-foreground" role="status">
                {t("actions:system.picker.loading")}
              </p>
            ) : (
              <CommandEmpty>
                {error
                  ? t("actions:system.picker.listFailed", { detail: error })
                  : t("actions:system.picker.noMatches")}
              </CommandEmpty>
            )}
            {browsers && preferred.length > 0 && (
              <CommandGroup heading={t("actions:system.picker.browsers")}>
                {preferred.map((app) => (
                  <AppItem
                    key={app.id}
                    app={app}
                    onSelect={() => {
                      setOpen(false)
                      onPick(app)
                    }}
                  />
                ))}
              </CommandGroup>
            )}
            {others.length > 0 && (
              <CommandGroup
                heading={
                  browsers
                    ? t("actions:system.picker.otherApplications")
                    : t("actions:system.picker.applications")
                }
              >
                {others.map((app) => (
                  <AppItem
                    key={app.id}
                    app={app}
                    onSelect={() => {
                      setOpen(false)
                      onPick(app)
                    }}
                  />
                ))}
              </CommandGroup>
            )}
            <CommandGroup>
              <CommandItem
                value="__browse__"
                onSelect={() => {
                  setOpen(false)
                  onBrowse()
                }}
              >
                <Folder aria-hidden /> {t("actions:system.picker.browse")}
              </CommandItem>
            </CommandGroup>
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  )
}

function AppItem({ app, onSelect }: { app: InstalledApp; onSelect: () => void }) {
  return (
    <CommandItem value={`${app.name} ${app.id}`} onSelect={onSelect}>
      <AppWindow aria-hidden />
      <span className="min-w-0 flex-1 truncate">{app.name}</span>
      <span className="max-w-32 truncate text-xs text-muted-foreground" title={app.id}>
        {app.kind === "path" ? "" : app.id}
      </span>
    </CommandItem>
  )
}
