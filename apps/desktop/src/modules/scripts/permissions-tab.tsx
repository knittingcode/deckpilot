// The Permissions tab (docs/scripts-bridge.md): what a trusted script may
// do on this computer, given by the user here only. Folders and programs
// are picked with the system's own dialogs; a container runtime, a
// program that acts as another user and a shell as a command are not
// available (the core refuses them too).

import { open as openDialog } from "@tauri-apps/plugin-dialog"
import { FolderOpen, Plus, ShieldOff, Trash2 } from "lucide-react"
import { useEffect, useRef, useState } from "react"
import { toast } from "sonner"

import { ConfirmDialog } from "@/components/dialogs"
import { InlineStatus } from "@/components/settings-layout"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import { useTranslation } from "@/i18n"
import { api } from "@/lib/api"
import { toastFailure } from "@/lib/failure"
import type {
  ScriptCommandClass,
  ScriptFsAccess,
  ScriptFsScope,
  ScriptGrants,
  ScriptProcessGrant,
  StoredScript,
} from "@/lib/types"

const SHELLS = ["sh", "bash", "zsh", "pwsh", "powershell", "cmd"] as const

function baseName(path: string): string {
  const parts = path.split(/[\\/]/).filter((p) => p !== "")
  return parts[parts.length - 1] ?? path
}

/** A folder's label: its name, cut to the core's 64 characters. */
function folderLabel(path: string): string {
  return Array.from(baseName(path)).slice(0, 64).join("")
}

/** Whether a command is git (a bare name or a picked path). */
function isGit(command: string): boolean {
  return (
    baseName(command)
      .toLowerCase()
      .replace(/\.exe$/, "") === "git"
  )
}

export function PermissionsTab({
  profileId,
  stored,
  onSaved,
}: {
  profileId: string
  stored: StoredScript
  onSaved: () => void
}) {
  const { t } = useTranslation()
  const [grants, setGrants] = useState<ScriptGrants>(stored.grants ?? {})
  const [classes, setClasses] = useState<Record<string, ScriptCommandClass>>({})
  const [command, setCommand] = useState("")
  const [refusal, setRefusal] = useState<string | null>(null)
  const [repository, setRepository] = useState("")
  const [revoking, setRevoking] = useState(false)
  const [busy, setBusy] = useState(false)
  const set = (patch: Partial<ScriptGrants>) => {
    setGrants((g) => ({ ...g, ...patch }))
  }
  const fs = grants.fs ?? []
  const processes = grants.process ?? []
  const shells = grants.shell?.shells ?? []
  // Every program's class, saved ones included, so a launcher is flagged
  // whenever the tab opens.
  const unclassified = processes.map((p) => p.command).filter((c) => !(c in classes))
  const unclassifiedKey = unclassified.join("\n")
  useEffect(() => {
    for (const command of unclassifiedKey === "" ? [] : unclassifiedKey.split("\n")) {
      api.scriptCommandClass(command).then(
        (kind) => {
          setClasses((c) => ({ ...c, [command]: kind }))
        },
        () => undefined,
      )
    }
  }, [unclassifiedKey])
  // One program added at a time (a second click waits for the first).
  const adding = useRef(false)

  const addScope = (scope: ScriptFsScope) => {
    set({ fs: [...fs, scope] })
  }
  const addCommand = async (name: string) => {
    const trimmed = name.trim()
    if (trimmed === "" || adding.current) return
    adding.current = true
    const kind = await api.scriptCommandClass(trimmed).finally(() => {
      adding.current = false
    })
    if (kind === "containerRuntime") {
      setRefusal(t("workspace:scripts.permissions.containerRuntime"))
      return
    }
    if (kind === "elevation") {
      setRefusal(t("workspace:scripts.permissions.elevation"))
      return
    }
    if (kind === "shell") {
      setRefusal(t("workspace:scripts.permissions.shellCommand"))
      return
    }
    setRefusal(null)
    setClasses((c) => ({ ...c, [trimmed]: kind }))
    setGrants((g) => {
      const current = g.process ?? []
      if (current.some((p) => p.command.toLowerCase() === trimmed.toLowerCase())) return g
      return {
        ...g,
        process: [...current, { command: trimmed, cwd: "repositories", env: "none" }],
      }
    })
    setCommand("")
  }
  const updateProcess = (index: number, patch: Partial<ScriptProcessGrant>) => {
    set({ process: processes.map((p, i) => (i === index ? { ...p, ...patch } : p)) })
  }
  const save = () => {
    setBusy(true)
    api
      .setScriptGrants(profileId, stored.script.id, grants)
      .then(
        () => {
          toast.success(t("workspace:scripts.permissions.saved", { name: stored.script.name }))
          onSaved()
        },
        (e: unknown) => {
          toastFailure(
            t("workspace:scripts.permissions.saveFailed", { name: stored.script.name }),
            e,
          )
        },
      )
      .finally(() => {
        setBusy(false)
      })
  }

  return (
    <div className="grid max-w-2xl gap-6">
      <p className="text-sm text-muted-foreground">{t("workspace:scripts.permissions.intro")}</p>

      <section className="grid gap-3" aria-label={t("workspace:scripts.permissions.projects")}>
        <h3 className="text-sm font-medium">{t("workspace:scripts.permissions.projects")}</h3>
        <div className="flex items-center gap-3">
          <Switch
            id="grant-projects"
            checked={grants.projectsRead ?? false}
            onCheckedChange={(projectsRead) => {
              set({ projectsRead })
            }}
          />
          <Label htmlFor="grant-projects">{t("workspace:scripts.grants.projectsRead")}</Label>
        </div>
        <div className="flex items-center gap-3">
          <Switch
            id="grant-paths"
            checked={grants.repositoriesPaths ?? false}
            onCheckedChange={(repositoriesPaths) => {
              set({ repositoriesPaths })
            }}
          />
          <Label htmlFor="grant-paths">{t("workspace:scripts.grants.repositoriesPaths")}</Label>
        </div>
      </section>

      <section className="grid gap-3" aria-label={t("workspace:scripts.permissions.files")}>
        <h3 className="text-sm font-medium">{t("workspace:scripts.permissions.files")}</h3>
        <ul className="grid gap-2">
          {fs.map((scope, index) => (
            <li key={index} className="flex flex-wrap items-center gap-2 text-sm">
              <span className="min-w-0 flex-1 truncate">
                {"repository" in scope.root
                  ? scope.root.repository === "*"
                    ? t("workspace:scripts.grants.everyRepository")
                    : t("workspace:scripts.grants.repository", { key: scope.root.repository })
                  : t("workspace:scripts.grants.folder", {
                      label: scope.root.directory.label,
                      path: scope.root.directory.path,
                    })}
              </span>
              <Select
                value={scope.access}
                onValueChange={(access) => {
                  set({
                    fs: fs.map((s, i) =>
                      i === index ? { ...s, access: access as ScriptFsAccess } : s,
                    ),
                  })
                }}
              >
                <SelectTrigger
                  className="w-40"
                  aria-label={t("workspace:scripts.permissions.accessOf", { index: index + 1 })}
                >
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="read">{t("workspace:scripts.permissions.read")}</SelectItem>
                  <SelectItem value="write">{t("workspace:scripts.permissions.write")}</SelectItem>
                  <SelectItem value="delete">
                    {t("workspace:scripts.permissions.delete")}
                  </SelectItem>
                </SelectContent>
              </Select>
              <label className="flex items-center gap-1.5">
                <Checkbox
                  aria-label={t("workspace:scripts.permissions.rootDestructiveOf", {
                    index: index + 1,
                  })}
                  checked={scope.rootDestructive ?? false}
                  onCheckedChange={(checked) => {
                    set({
                      fs: fs.map((s, i) =>
                        i === index ? { ...s, rootDestructive: checked === true } : s,
                      ),
                    })
                  }}
                />
                {t("workspace:scripts.permissions.rootDestructive")}
              </label>
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label={t("workspace:scripts.permissions.removeScopeOf", { index: index + 1 })}
                onClick={() => {
                  set({ fs: fs.filter((_, i) => i !== index) })
                }}
              >
                <Trash2 />
              </Button>
            </li>
          ))}
        </ul>
        <div className="flex flex-wrap items-center gap-2">
          <Input
            className="w-48"
            aria-label={t("workspace:scripts.permissions.repositoryKey")}
            placeholder={t("workspace:scripts.permissions.repositoryKey")}
            value={repository}
            onChange={(e) => {
              setRepository(e.target.value)
            }}
          />
          <Button
            variant="outline"
            disabled={!/^[a-z][a-z0-9_]{0,31}$/.test(repository.trim())}
            onClick={() => {
              addScope({ root: { repository: repository.trim() }, access: "read" })
              setRepository("")
            }}
          >
            <Plus />
            {t("workspace:scripts.permissions.addRepository")}
          </Button>
          <Button
            variant="outline"
            onClick={() => {
              addScope({ root: { repository: "*" }, access: "read" })
            }}
          >
            <Plus />
            {t("workspace:scripts.permissions.addEveryRepository")}
          </Button>
          <Button
            variant="outline"
            onClick={() => {
              void openDialog({ directory: true, multiple: false }).then((picked) => {
                if (typeof picked !== "string") return
                addScope({
                  root: { directory: { path: picked, label: folderLabel(picked) } },
                  access: "read",
                })
              })
            }}
          >
            <FolderOpen />
            {t("workspace:scripts.permissions.addFolder")}
          </Button>
        </div>
      </section>

      <section className="grid gap-3" aria-label={t("workspace:scripts.permissions.programs")}>
        <h3 className="text-sm font-medium">{t("workspace:scripts.permissions.programs")}</h3>
        <ul className="grid gap-2">
          {processes.map((process, index) => (
            <li key={process.command} className="grid gap-2 rounded-md border p-2 text-sm">
              <div className="flex flex-wrap items-center gap-2">
                <span className="font-mono">{process.command}</span>
                {classes[process.command] === "runsAnyProgram" && (
                  <InlineStatus tone="warning">
                    {t("workspace:scripts.permissions.runsAnyProgram")}
                  </InlineStatus>
                )}
                <Button
                  className="ml-auto"
                  variant="ghost"
                  size="icon-sm"
                  aria-label={t("workspace:scripts.permissions.removeProgram", {
                    command: process.command,
                  })}
                  onClick={() => {
                    set({ process: processes.filter((_, i) => i !== index) })
                  }}
                >
                  <Trash2 />
                </Button>
              </div>
              <div className="flex flex-wrap items-center gap-2">
                <Select
                  value={process.cwd ?? "repositories"}
                  onValueChange={(cwd) => {
                    updateProcess(index, { cwd: cwd as ScriptProcessGrant["cwd"] })
                  }}
                >
                  <SelectTrigger
                    className="w-56"
                    aria-label={t("workspace:scripts.permissions.where", {
                      command: process.command,
                    })}
                  >
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="repositories">
                      {t("workspace:scripts.grants.cwdRepositories")}
                    </SelectItem>
                    <SelectItem value="fsScopes">
                      {t("workspace:scripts.grants.cwdScopes")}
                    </SelectItem>
                    <SelectItem value="any">{t("workspace:scripts.grants.cwdAny")}</SelectItem>
                  </SelectContent>
                </Select>
                <label className="flex items-center gap-1.5">
                  <Checkbox
                    aria-label={t("workspace:scripts.permissions.envOf", {
                      command: process.command,
                    })}
                    checked={process.env === "allowed"}
                    onCheckedChange={(checked) => {
                      updateProcess(index, { env: checked === true ? "allowed" : "none" })
                    }}
                  />
                  {t("workspace:scripts.grants.envAllowed")}
                </label>
                {isGit(process.command) && (
                  <label className="flex items-center gap-1.5">
                    <Checkbox
                      aria-label={t("workspace:scripts.permissions.unrestrictedOf", {
                        command: process.command,
                      })}
                      checked={process.unrestrictedArgs ?? false}
                      onCheckedChange={(checked) => {
                        updateProcess(index, { unrestrictedArgs: checked === true })
                      }}
                    />
                    {t("workspace:scripts.grants.unrestrictedArgs")}
                  </label>
                )}
              </div>
            </li>
          ))}
        </ul>
        <div className="flex flex-wrap items-center gap-2">
          <Input
            className="w-48"
            aria-label={t("workspace:scripts.permissions.programName")}
            aria-invalid={refusal !== null}
            aria-describedby={refusal !== null ? "program-refusal" : undefined}
            placeholder={t("workspace:scripts.permissions.programName")}
            value={command}
            onChange={(e) => {
              setCommand(e.target.value)
              setRefusal(null)
            }}
          />
          <Button
            variant="outline"
            disabled={command.trim() === ""}
            onClick={() => {
              void addCommand(command)
            }}
          >
            <Plus />
            {t("workspace:scripts.permissions.addProgram")}
          </Button>
          <Button
            variant="outline"
            onClick={() => {
              void openDialog({ directory: false, multiple: false }).then((picked) => {
                if (typeof picked === "string") void addCommand(picked)
              })
            }}
          >
            <FolderOpen />
            {t("workspace:scripts.permissions.chooseProgram")}
          </Button>
        </div>
        {refusal && (
          <p id="program-refusal" className="text-sm text-destructive" role="alert">
            {refusal}
          </p>
        )}
        <p className="text-xs text-muted-foreground">
          {t("workspace:scripts.permissions.containerRuntime")}
        </p>
      </section>

      <section className="grid gap-3" aria-label={t("workspace:scripts.permissions.shells")}>
        <h3 className="text-sm font-medium">{t("workspace:scripts.permissions.shells")}</h3>
        <p className="text-xs text-muted-foreground">
          {t("workspace:scripts.permissions.shellsHint")}
        </p>
        <div className="flex flex-wrap gap-4">
          {SHELLS.map((shell) => (
            <label key={shell} className="flex items-center gap-1.5 text-sm">
              <Checkbox
                checked={shells.includes(shell)}
                onCheckedChange={(checked) => {
                  const next =
                    checked === true ? [...shells, shell] : shells.filter((s) => s !== shell)
                  set({ shell: next.length > 0 ? { shells: next } : null })
                }}
              />
              <span className="font-mono">{shell}</span>
            </label>
          ))}
        </div>
      </section>

      <section className="grid gap-3" aria-label={t("workspace:scripts.permissions.deleting")}>
        <h3 className="text-sm font-medium">{t("workspace:scripts.permissions.deleting")}</h3>
        <Select
          value={grants.destructive ?? "ask"}
          onValueChange={(destructive) => {
            set({ destructive: destructive as "ask" | "allow" })
          }}
        >
          <SelectTrigger className="w-80" aria-label={t("workspace:scripts.permissions.deleting")}>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="ask">{t("workspace:scripts.permissions.ask")}</SelectItem>
            <SelectItem value="allow">{t("workspace:scripts.grants.destructiveAllow")}</SelectItem>
          </SelectContent>
        </Select>
      </section>

      <div className="flex flex-wrap justify-end gap-2">
        <Button
          variant="outline"
          onClick={() => {
            setRevoking(true)
          }}
        >
          <ShieldOff />
          {t("workspace:scripts.permissions.revokeAll")}
        </Button>
        <Button disabled={busy} onClick={save}>
          {t("workspace:scripts.permissions.save")}
        </Button>
      </div>
      <ConfirmDialog
        open={revoking}
        onOpenChange={setRevoking}
        title={t("workspace:scripts.permissions.revokeTitle", { name: stored.script.name })}
        description={t("workspace:scripts.permissions.revokeDescription")}
        confirmLabel={t("workspace:scripts.permissions.revokeAll")}
        onConfirm={async () => {
          await api.revokeScriptGrants(profileId, stored.script.id)
          setGrants({})
          onSaved()
        }}
      />
    </div>
  )
}
