// The pickers of the System actions' machine-local parameters
// (docs/system-actions.md): an installed application with its icon, a
// browser ("Open with", the system's default first), or a file or folder
// from the native dialog. What is picked is bound on this computer
// (Settings › Resources, never exported) and the parameter refers to the
// binding's key, so a profile never holds a path. Resource keys and
// project roles are an advanced mode, shown only when asked for or when a
// saved action uses them.

import { open as openDialog } from "@tauri-apps/plugin-dialog"
import { AppWindow, ChevronsUpDown, File, Folder, FolderOpen, Globe, Laptop } from "lucide-react"
import { type ReactNode, useEffect, useId, useState } from "react"
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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { useTranslation } from "@/i18n"
import { type ResourceStatus, api } from "@/lib/api"
import { failureDetail } from "@/lib/failure"
import { applicationFilters, baseName, bindingKeyFor, isLocalReference } from "@/lib/system-actions"
import type {
  InstalledApp,
  MachineResourceBinding,
  Os,
  ResourceKind,
  ResourceRef,
} from "@/lib/types"

/** What the field picks. */
export type MachinePick = "application" | "browser" | "location" | "folder"

/** Icons by application id, fetched once per window. */
const icons = new Map<string, Promise<string | null>>()
/** Most icon requests at a time; the rest wait their turn. */
const MAX_ICON_REQUESTS = 8
let running = 0
const waiting: (() => void)[] = []

async function limited<T>(task: () => Promise<T>): Promise<T> {
  if (running >= MAX_ICON_REQUESTS) {
    await new Promise<void>((resolve) => waiting.push(resolve))
  }
  running += 1
  try {
    return await task()
  } finally {
    running -= 1
    waiting.shift()?.()
  }
}

function iconOf(id: string): Promise<string | null> {
  let icon = icons.get(id)
  if (!icon) {
    icon = limited(() => api.applicationIcon(id)).catch(() => {
      // A failed request is tried again next time the icon is shown.
      icons.delete(id)
      return null
    })
    icons.set(id, icon)
  }
  return icon
}

/** An application's icon as the system draws it, or a generic one. */
export function AppIcon({ id, className = "size-4" }: { id: string | null; className?: string }) {
  // The icon of the id it was fetched for: another id shows the generic one
  // until its own arrives.
  const [loaded, setLoaded] = useState<{ id: string; src: string | null } | null>(null)
  useEffect(() => {
    let cancelled = false
    if (id) {
      void iconOf(id).then((icon) => {
        if (!cancelled) setLoaded({ id, src: icon })
      })
    }
    return () => {
      cancelled = true
    }
  }, [id])
  const src = loaded && loaded.id === id ? loaded.src : null
  return src ? (
    <img src={src} alt="" aria-hidden className={`${className} shrink-0 object-contain`} />
  ) : (
    <AppWindow className={`${className} shrink-0 text-muted-foreground`} aria-hidden />
  )
}

/**
 * A machine-local resource parameter. `advanced` draws the resource key or
 * project role control (the caller's), for advanced mode; it opens by
 * itself for a saved value that uses one.
 */
export function MachineResourceField({
  id,
  pick,
  value,
  optional,
  onChange,
  advanced,
}: {
  id: string
  pick: MachinePick
  value: ResourceRef | null
  optional: boolean
  onChange: (value: ResourceRef | null) => void
  advanced?: ReactNode
}) {
  const { t } = useTranslation()
  const noteId = useId()
  const [bindings, setBindings] = useState<MachineResourceBinding[] | null>(null)
  const [os, setOs] = useState<Os>("other")
  const [busy, setBusy] = useState(false)
  const [advancedMode, setAdvancedMode] = useState(() => !isLocalReference(value))
  // An application chosen with "Another application…", which the list may
  // not have (outside the folders discovery reads).
  const [described, setDescribed] = useState<InstalledApp | null>(null)
  useEffect(() => {
    let cancelled = false
    api.getMachineConfig().then(
      (config) => {
        if (!cancelled) {
          setBindings(config.bindings)
          setOs(config.os)
        }
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

  const browse = async (directory: boolean) => {
    try {
      const picked = await openDialog({ directory, multiple: false })
      if (typeof picked !== "string") return
      await bind(
        directory ? "folder" : "file",
        baseName(picked),
        directory ? "directory" : "other",
        picked,
      )
    } catch (e) {
      toast.error(t("actions:system.picker.failed"), { description: failureDetail(e) })
    }
  }

  /** "Choose another…": the native dialog, limited to applications. */
  const browseApplication = async () => {
    try {
      const filters = applicationFilters(os, t("actions:system.picker.applicationFiles"))
      const picked = await openDialog({ directory: false, multiple: false, filters })
      if (typeof picked !== "string") return
      const app = await api.describeApplication(picked)
      setDescribed(app)
      await bind("app", app.name, "application", app.id)
    } catch (e) {
      toast.error(t("actions:system.picker.failed"), { description: failureDetail(e) })
    }
  }

  const toggle = (
    <Button
      type="button"
      variant="ghost"
      size="sm"
      className="h-6 px-1.5 text-xs text-muted-foreground"
      aria-pressed={advancedMode}
      onClick={() => {
        setAdvancedMode(!advancedMode)
      }}
    >
      {advancedMode ? t("actions:system.picker.simple") : t("actions:system.picker.advanced")}
    </Button>
  )

  if (advancedMode) {
    return (
      <div className="grid gap-1.5" data-slot="machine-resource" data-mode="advanced">
        {advanced}
        <div className="flex flex-wrap items-center gap-2">
          <p className="text-xs text-muted-foreground">{t("actions:system.picker.advancedNote")}</p>
          {toggle}
        </div>
      </div>
    )
  }

  return (
    <div className="grid gap-1.5" data-slot="machine-resource" data-mode="local">
      {pick === "browser" ? (
        <BrowserSelect
          id={id}
          value={bound?.value ?? null}
          unbound={key !== null && bindings !== null && !bound}
          disabled={busy}
          onPick={(app) => {
            if (app) void bind("app", app.name, "application", app.id)
            else onChange(null)
          }}
        />
      ) : pick === "application" ? (
        <ApplicationPicker
          id={id}
          value={bound?.value ?? null}
          described={described}
          disabled={busy}
          onPick={(app) => void bind("app", app.name, "application", app.id)}
          onBrowse={() => void browseApplication()}
        />
      ) : (
        <LocationRow
          id={id}
          resource={bound ? key : null}
          path={bound?.value ?? null}
          folder={pick === "folder" || bound?.kind === "directory"}
          busy={busy}
          files={pick === "location"}
          onBrowse={(directory) => void browse(directory)}
        />
      )}
      {key !== null && bindings !== null && !bound && pick !== "browser" && (
        <p className="text-xs text-destructive" role="alert">
          {t("actions:system.picker.unboundHint")}
        </p>
      )}
      <div className="flex flex-wrap items-center gap-2">
        {key && (
          <Badge variant="outline" className="shrink-0 gap-1">
            <Laptop className="size-3" aria-hidden />
            {t("actions:system.picker.thisComputer")}
          </Badge>
        )}
        <p id={noteId} className="text-xs text-muted-foreground" role="note">
          {t("actions:system.picker.localHint")}
        </p>
        {optional && value && pick !== "browser" && (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-6 px-1.5 text-xs"
            disabled={busy}
            onClick={() => {
              onChange(null)
            }}
          >
            {t("actions:system.picker.clear")}
          </Button>
        )}
        {toggle}
      </div>
    </div>
  )
}

/** A chosen file or folder: its name, its path, and the native dialogs. */
function LocationRow({
  id,
  resource,
  path,
  folder,
  files,
  busy,
  onBrowse,
}: {
  id: string
  /** The bound resource's key, to check that `path` is still there. */
  resource: string | null
  path: string | null
  folder: boolean
  /**
   * Files and folders can be chosen (Open); otherwise folders only (the
   * legacy Open folder, kept for saved keys).
   */
  files: boolean
  busy: boolean
  onBrowse: (directory: boolean) => void
}) {
  const { t } = useTranslation()
  const Icon = folder ? Folder : File
  // A bound file or folder may have been moved or deleted since: asked by
  // its key (never by path), again whenever it is bound to another path.
  const [status, setStatus] = useState<{ for: string; status: ResourceStatus | null } | null>(null)
  const checked = resource !== null && path !== null ? `${resource}\u0000${path}` : null
  useEffect(() => {
    if (checked === null || resource === null) return
    let cancelled = false
    api.resourceStatus(resource).then(
      (answer) => {
        if (!cancelled) setStatus({ for: checked, status: answer })
      },
      () => {
        // It cannot be checked (an older app, no store): no alert.
        if (!cancelled) setStatus({ for: checked, status: null })
      },
    )
    return () => {
      cancelled = true
    }
  }, [checked, resource])
  const gone = checked !== null && status?.for === checked && status.status === "missing"
  const hintId = useId()
  return (
    <div className="grid gap-1.5">
      <div className="flex min-w-0 items-center gap-2" id={id}>
        <Icon className="size-4 shrink-0 text-muted-foreground" aria-hidden />
        {path ? (
          <span className="grid min-w-0">
            <span className="truncate text-sm font-medium">{baseName(path)}</span>
            <span className="truncate text-xs text-muted-foreground select-text" title={path}>
              {path}
            </span>
          </span>
        ) : (
          <span className="text-sm text-muted-foreground">
            {t("actions:system.picker.nothing")}
          </span>
        )}
      </div>
      <div className="flex flex-wrap gap-2">
        {files && (
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={busy}
            aria-describedby={hintId}
            onClick={() => {
              onBrowse(false)
            }}
          >
            <File /> {t("actions:system.picker.chooseFile")}
          </Button>
        )}
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={busy}
          aria-describedby={files ? hintId : undefined}
          onClick={() => {
            onBrowse(true)
          }}
        >
          <FolderOpen /> {t("actions:system.picker.chooseFolder")}
        </Button>
      </div>
      {files && (
        <p id={hintId} className="text-xs text-muted-foreground" data-slot="file-or-folder">
          {t("actions:system.picker.fileOrFolder")}
        </p>
      )}
      {gone && (
        <p className="text-xs text-destructive" role="alert">
          {t("actions:system.picker.locationGone")}
        </p>
      )}
    </div>
  )
}

const DEFAULT = "__default__"

/** "Open with": the default browser first, then every installed one. */
function BrowserSelect({
  id,
  value,
  unbound,
  disabled,
  onPick,
}: {
  id: string
  /** The bound browser's id, or `null` for the default browser. */
  value: string | null
  unbound: boolean
  disabled: boolean
  onPick: (app: InstalledApp | null) => void
}) {
  const { t } = useTranslation()
  const [found, setFound] = useState<{ browsers: InstalledApp[]; default?: string } | null>(null)
  useEffect(() => {
    let cancelled = false
    api.listBrowsers().then(
      (list) => {
        if (!cancelled) setFound(list)
      },
      () => {
        if (!cancelled) setFound({ browsers: [] })
      },
    )
    return () => {
      cancelled = true
    }
  }, [])
  const browsers = found?.browsers ?? []
  const system = browsers.find((b) => b.id === found?.default)
  const gone =
    (value !== null && found !== null && !browsers.some((b) => b.id === value)) || unbound
  return (
    <div className="grid gap-1.5">
      <Select
        value={value !== null && browsers.some((b) => b.id === value) ? value : DEFAULT}
        disabled={disabled}
        onValueChange={(choice) => {
          onPick(choice === DEFAULT ? null : (browsers.find((b) => b.id === choice) ?? null))
        }}
      >
        <SelectTrigger id={id} className="w-full">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value={DEFAULT}>
            <Globe className="size-4" aria-hidden />
            {system
              ? t("actions:system.picker.defaultBrowserIs", { name: system.name })
              : t("actions:system.picker.defaultBrowser")}
          </SelectItem>
          {browsers.map((browser) => (
            <SelectItem key={browser.id} value={browser.id}>
              <AppIcon id={browser.id} />
              {browser.name}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      {gone && (
        <p className="text-xs text-muted-foreground" role="note">
          {t("actions:system.picker.browserGone")}
        </p>
      )}
    </div>
  )
}

/** A searchable list of the installed applications, with their icons. */
function ApplicationPicker({
  id,
  value,
  described,
  disabled,
  onPick,
  onBrowse,
}: {
  id: string
  /** The bound application's id (or path). */
  value: string | null
  /** The application just chosen with "Another application…". */
  described: InstalledApp | null
  disabled: boolean
  onPick: (app: InstalledApp) => void
  onBrowse: () => void
}) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const [apps, setApps] = useState<InstalledApp[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
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
  }, [])
  const chosen = value
    ? (apps?.find((a) => a.id === value) ?? (described?.id === value ? described : undefined))
    : undefined
  // Not in the list: asked whether it is still installed (it may be outside
  // the folders discovery reads, or uninstalled).
  // `installed: null`: it cannot be checked, so no alert either way.
  const [present, setPresent] = useState<{ id: string; installed: boolean | null } | null>(null)
  const unlisted = value !== null && apps !== null && !chosen ? value : null
  useEffect(() => {
    if (unlisted === null) return
    let cancelled = false
    api.applicationPresent(unlisted).then(
      (installed) => {
        if (!cancelled) setPresent({ id: unlisted, installed })
      },
      () => {
        if (!cancelled) setPresent({ id: unlisted, installed: null })
      },
    )
    return () => {
      cancelled = true
    }
  }, [unlisted])
  const gone = unlisted !== null && present?.id === unlisted && present.installed === false
  const label = t("actions:system.picker.chooseApplication")
  return (
    <>
      <Popover open={open} onOpenChange={setOpen}>
        <PopoverTrigger asChild>
          <Button
            id={id}
            type="button"
            variant="outline"
            role="combobox"
            aria-label={chosen ? `${label} ${chosen.name}` : label}
            aria-expanded={open}
            disabled={disabled}
            className="w-full justify-between font-normal"
          >
            <span className="flex min-w-0 items-center gap-2">
              {value ? <AppIcon id={chosen ? chosen.id : null} /> : null}
              <span className="truncate">
                {chosen ? chosen.name : value ? baseName(value) : label}
              </span>
            </span>
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
              {(apps ?? []).length > 0 && (
                <CommandGroup heading={t("actions:system.picker.applications")}>
                  {(apps ?? []).map((app) => (
                    <CommandItem
                      key={app.id}
                      value={`${app.name} ${app.id}`}
                      onSelect={() => {
                        setOpen(false)
                        onPick(app)
                      }}
                    >
                      <AppIcon id={app.id} />
                      <span className="min-w-0 flex-1 truncate">{app.name}</span>
                    </CommandItem>
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
                  <FolderOpen aria-hidden /> {t("actions:system.picker.browse")}
                </CommandItem>
              </CommandGroup>
            </CommandList>
          </Command>
        </PopoverContent>
      </Popover>
      {gone && (
        <p className="text-xs text-destructive" role="alert">
          {t("actions:system.picker.applicationGone")}
        </p>
      )}
    </>
  )
}
