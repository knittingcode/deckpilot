// The Scripts page (docs/data-model.md, "Scripts"): the scripts of the
// profile, their code and settings, and the review that trusts a script
// that did not come from this editor. Nothing here runs a script.

import { Copy, Pencil, Plus, ShieldAlert, ShieldCheck, Trash2, TriangleAlert } from "lucide-react"
import { useEffect, useState } from "react"
import { toast } from "sonner"

import { ConfirmDialog, FormDialog } from "@/components/dialogs"
import { EmptyState, ErrorNotice, Loading, Page } from "@/components/page"
import { RequireProfile } from "@/components/require-profile"
import { InlineStatus } from "@/components/settings-layout"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/switch"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { Textarea } from "@/components/ui/textarea"
import { useAsync } from "@/hooks/use-async"
import { useEditingProfile } from "@/hooks/use-profile"
import { useTranslation } from "@/i18n"
import { api } from "@/lib/api"
import { failureMessage, toastFailure } from "@/lib/failure"
import type { Script, ScriptReview, ScriptSettings, ScriptUsage, StoredScript } from "@/lib/types"

import { CodeEditor } from "./code-editor"
import { hasHiddenChars } from "./completions"

export function ScriptsPage() {
  const { t } = useTranslation()
  return (
    <Page
      title={t("workspace:scripts.page.title")}
      description={t("workspace:scripts.page.description")}
      profileScoped
    >
      <RequireProfile>{() => <Scripts />}</RequireProfile>
    </Page>
  )
}

function Scripts() {
  const { t } = useTranslation()
  const { profileId } = useEditingProfile()
  const [generation, setGeneration] = useState(0)
  const list = useAsync(
    () => (profileId ? api.listScripts(profileId) : Promise.resolve([])),
    `${profileId ?? ""}:${generation}`,
  )
  const [selected, setSelected] = useState<string | null>(null)
  const [search, setSearch] = useState("")
  const [creating, setCreating] = useState(false)
  // Unsaved edits of the open script: switching away asks first.
  const [dirty, setDirty] = useState(false)
  const [pending, setPending] = useState<string | null>(null)
  if (!profileId) return null
  if (list.state.status === "loading") return <Loading />
  if (list.state.status === "error") return <ErrorNotice error={list.state.error} />
  const scripts = list.state.data
  const shown = scripts.filter((s) =>
    s.script.name.toLowerCase().includes(search.trim().toLowerCase()),
  )
  const current = scripts.find((s) => s.script.id === selected) ?? null
  const reload = () => {
    setGeneration((g) => g + 1)
  }
  const open = (id: string) => {
    if (dirty && id !== selected) setPending(id)
    else setSelected(id)
  }
  return (
    <div className="grid w-full content-start gap-6 md:grid-cols-[16rem_1fr]">
      <aside className="grid content-start gap-3">
        <div className="flex gap-2">
          <Input
            aria-label={t("workspace:scripts.list.search")}
            placeholder={t("workspace:scripts.list.search")}
            value={search}
            onChange={(e) => {
              setSearch(e.target.value)
            }}
          />
          <Button
            size="icon"
            aria-label={t("workspace:scripts.list.new")}
            title={t("workspace:scripts.list.new")}
            onClick={() => {
              setCreating(true)
            }}
          >
            <Plus />
          </Button>
        </div>
        {scripts.length === 0 ? (
          <EmptyState title={t("workspace:scripts.list.empty")}>
            {t("workspace:scripts.list.emptyHint")}
          </EmptyState>
        ) : (
          <ul className="grid gap-1" aria-label={t("workspace:scripts.list.label")}>
            {shown.map((s) => (
              <li key={s.script.id}>
                <button
                  type="button"
                  className="flex w-full items-center justify-between gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent data-[selected=true]:bg-accent"
                  data-selected={s.script.id === selected}
                  aria-current={s.script.id === selected ? "true" : undefined}
                  onClick={() => {
                    open(s.script.id)
                  }}
                >
                  <span className="truncate">{s.script.name}</span>
                  <TrustChip trust={s.trust} />
                </button>
              </li>
            ))}
          </ul>
        )}
      </aside>
      <section className="min-w-0">
        {current ? (
          <ScriptDetail
            key={`${current.script.id}:${current.script.revision}:${current.trust}`}
            profileId={profileId}
            stored={current}
            onDirty={setDirty}
            onChanged={reload}
            onDeleted={() => {
              setSelected(null)
              setDirty(false)
              reload()
            }}
            onDuplicated={(copy) => {
              setSelected(copy.id)
              reload()
            }}
          />
        ) : (
          scripts.length > 0 && (
            <p className="text-sm text-muted-foreground">{t("workspace:scripts.list.choose")}</p>
          )
        )}
      </section>
      {creating && (
        <NameDialog
          open
          title={t("workspace:scripts.create.title")}
          description={t("workspace:scripts.create.description")}
          submitLabel={t("workspace:scripts.create.submit")}
          initial=""
          onClose={() => {
            setCreating(false)
          }}
          onSubmit={async (name) => {
            const script = await api.createScript(profileId, { name, source: "" })
            setCreating(false)
            setSelected(script.id)
            reload()
          }}
        />
      )}
      <ConfirmDialog
        open={pending !== null}
        onOpenChange={(o) => {
          if (!o) setPending(null)
        }}
        title={t("workspace:scripts.unsaved.title")}
        description={t("workspace:scripts.unsaved.description")}
        confirmLabel={t("workspace:scripts.unsaved.discard")}
        onConfirm={() => {
          setDirty(false)
          setSelected(pending)
          setPending(null)
        }}
      />
    </div>
  )
}

function TrustChip({ trust }: { trust: StoredScript["trust"] }) {
  const { t } = useTranslation()
  return trust === "trusted" ? (
    <InlineStatus tone="positive">{t("workspace:scripts.trust.trusted")}</InlineStatus>
  ) : (
    <InlineStatus tone="warning">{t("workspace:scripts.trust.needsReview")}</InlineStatus>
  )
}

function NameDialog({
  open,
  title,
  description,
  submitLabel,
  initial,
  onClose,
  onSubmit,
}: {
  open: boolean
  title: string
  description: string
  submitLabel: string
  initial: string
  onClose: () => void
  onSubmit: (name: string) => Promise<void>
}) {
  const { t } = useTranslation()
  // Mounted fresh for each opening (the caller renders it only while open).
  const [name, setName] = useState(initial)
  const trimmed = name.trim()
  return (
    <FormDialog
      open={open}
      onClose={onClose}
      title={title}
      description={description}
      submitLabel={submitLabel}
      canSubmit={trimmed.length > 0 && Array.from(trimmed).length <= 64}
      dirty={name !== initial}
      onSubmit={() => onSubmit(trimmed)}
      describeError={(e) => failureMessage(t("workspace:scripts.create.failed"), e)}
    >
      <div className="grid gap-2">
        <Label htmlFor="script-name">{t("workspace:scripts.create.name")}</Label>
        <Input
          id="script-name"
          value={name}
          maxLength={64}
          autoFocus
          onChange={(e) => {
            setName(e.target.value)
          }}
        />
      </div>
    </FormDialog>
  )
}

function ScriptDetail({
  profileId,
  stored,
  onDirty,
  onChanged,
  onDeleted,
  onDuplicated,
}: {
  profileId: string
  stored: StoredScript
  onDirty: (dirty: boolean) => void
  onChanged: () => void
  onDeleted: () => void
  onDuplicated: (copy: Script) => void
}) {
  const { t } = useTranslation()
  const script = stored.script
  const [renaming, setRenaming] = useState(false)
  const [deleting, setDeleting] = useState(false)
  const save = async (next: Script) => {
    try {
      await api.saveScript(profileId, next)
      toast.success(t("workspace:scripts.editor.saved", { name: next.name }))
      onDirty(false)
      onChanged()
    } catch (e) {
      toastFailure(t("workspace:scripts.editor.saveFailed", { name: script.name }), e)
    }
  }
  return (
    <div className="grid gap-4">
      <header className="flex flex-wrap items-center gap-2">
        <h2 className="mr-auto truncate text-lg font-semibold">{script.name}</h2>
        <TrustChip trust={stored.trust} />
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={t("workspace:scripts.item.rename", { name: script.name })}
          onClick={() => {
            setRenaming(true)
          }}
        >
          <Pencil />
        </Button>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={t("workspace:scripts.item.duplicate", { name: script.name })}
          title={t("workspace:scripts.item.duplicateTooltip")}
          onClick={() => {
            api.duplicateScript(profileId, script.id).then(onDuplicated, (e: unknown) => {
              toastFailure(t("workspace:scripts.item.duplicateFailed", { name: script.name }), e)
            })
          }}
        >
          <Copy />
        </Button>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={t("workspace:scripts.item.delete", { name: script.name })}
          onClick={() => {
            setDeleting(true)
          }}
        >
          <Trash2 />
        </Button>
      </header>
      {stored.trust === "needsReview" ? (
        <ReviewPanel profileId={profileId} scriptId={script.id} onTrusted={onChanged} />
      ) : (
        <Tabs defaultValue="code">
          <TabsList>
            <TabsTrigger value="code">{t("workspace:scripts.tabs.code")}</TabsTrigger>
            <TabsTrigger value="settings">{t("workspace:scripts.tabs.settings")}</TabsTrigger>
          </TabsList>
          <TabsContent value="code">
            <CodeTab script={script} onDirty={onDirty} onSave={save} />
          </TabsContent>
          <TabsContent value="settings">
            <SettingsTab script={script} onSave={save} />
          </TabsContent>
        </Tabs>
      )}
      {renaming && (
        <NameDialog
          open
          title={t("workspace:scripts.rename.title")}
          description={t("workspace:scripts.rename.description")}
          submitLabel={t("workspace:scripts.rename.submit")}
          initial={script.name}
          onClose={() => {
            setRenaming(false)
          }}
          onSubmit={async (name) => {
            await api.saveScript(profileId, { ...script, name })
            setRenaming(false)
            onChanged()
          }}
        />
      )}
      {deleting && (
        <DeleteScriptDialog
          open
          profileId={profileId}
          script={script}
          onClose={() => {
            setDeleting(false)
          }}
          onDeleted={onDeleted}
        />
      )}
    </div>
  )
}

function CodeTab({
  script,
  onDirty,
  onSave,
}: {
  script: Script
  onDirty: (dirty: boolean) => void
  onSave: (next: Script) => Promise<void>
}) {
  const { t } = useTranslation()
  const [source, setSource] = useState(script.source)
  const [busy, setBusy] = useState(false)
  const dirty = source !== script.source
  useEffect(() => {
    onDirty(dirty)
  }, [dirty, onDirty])
  return (
    <div className="grid gap-3">
      <CodeEditor
        value={source}
        onChange={setSource}
        label={t("workspace:scripts.editor.label", { name: script.name })}
        syntaxErrorMessage={t("workspace:scripts.editor.syntaxError")}
      />
      {hasHiddenChars(source) && (
        <p className="text-sm text-destructive" role="note">
          {t("workspace:scripts.editor.hiddenChars")}
        </p>
      )}
      <div className="flex items-center justify-end gap-3">
        {dirty && (
          <span className="text-sm text-muted-foreground">
            {t("workspace:scripts.editor.unsaved")}
          </span>
        )}
        <Button
          disabled={!dirty || busy}
          onClick={() => {
            setBusy(true)
            void onSave({ ...script, source }).finally(() => {
              setBusy(false)
            })
          }}
        >
          {t("workspace:scripts.editor.save")}
        </Button>
      </div>
    </div>
  )
}

function SettingsTab({
  script,
  onSave,
}: {
  script: Script
  onSave: (next: Script) => Promise<void>
}) {
  const { t } = useTranslation()
  const [settings, setSettings] = useState<ScriptSettings>(script.settings)
  const seconds = Math.round(settings.timeoutMs / 1000)
  const valid =
    settings.timeoutMs >= 1000 &&
    settings.timeoutMs <= 600_000 &&
    settings.memoryMiB >= 16 &&
    settings.memoryMiB <= 128 &&
    Array.from(settings.description).length <= 500
  const changed = JSON.stringify(settings) !== JSON.stringify(script.settings)
  return (
    <div className="grid max-w-xl gap-4">
      <div className="grid gap-2">
        <Label htmlFor="script-timeout">{t("workspace:scripts.settings.timeout")}</Label>
        <Input
          id="script-timeout"
          type="number"
          min={1}
          max={600}
          value={seconds}
          onChange={(e) => {
            setSettings({ ...settings, timeoutMs: Math.round(Number(e.target.value) * 1000) })
          }}
        />
        <p className="text-xs text-muted-foreground">
          {t("workspace:scripts.settings.timeoutHint")}
        </p>
      </div>
      <div className="grid gap-2">
        <Label htmlFor="script-memory">{t("workspace:scripts.settings.memory")}</Label>
        <Input
          id="script-memory"
          type="number"
          min={16}
          max={128}
          value={settings.memoryMiB}
          onChange={(e) => {
            setSettings({ ...settings, memoryMiB: Number(e.target.value) })
          }}
        />
      </div>
      <div className="flex items-center gap-3">
        <Switch
          id="script-stop"
          checked={settings.stopOnError}
          onCheckedChange={(stopOnError) => {
            setSettings({ ...settings, stopOnError })
          }}
        />
        <Label htmlFor="script-stop">{t("workspace:scripts.settings.stopOnError")}</Label>
      </div>
      <div className="grid gap-2">
        <Label htmlFor="script-description">{t("workspace:scripts.settings.description")}</Label>
        <Textarea
          id="script-description"
          maxLength={500}
          value={settings.description}
          onChange={(e) => {
            setSettings({ ...settings, description: e.target.value })
          }}
        />
      </div>
      {!valid && (
        <p className="text-sm text-destructive">{t("workspace:scripts.settings.invalid")}</p>
      )}
      <div className="flex justify-end">
        <Button
          disabled={!changed || !valid}
          onClick={() => {
            void onSave({ ...script, settings })
          }}
        >
          {t("workspace:scripts.settings.save")}
        </Button>
      </div>
    </div>
  )
}

/**
 * Every permission a grant set gives, in words, for the review: nothing
 * that comes back with trust is left out (a field this does not know is
 * named as such).
 */
function useGrantLines(review: ScriptReview | null): string[] {
  const { t } = useTranslation()
  if (!review) return []
  const grants = review.grants
  const lines: string[] = []
  if (grants.projectsRead) lines.push(t("workspace:scripts.grants.projectsRead"))
  if (grants.repositoriesPaths) lines.push(t("workspace:scripts.grants.repositoriesPaths"))
  for (const scope of grants.fs ?? []) {
    const where =
      "repository" in scope.root
        ? scope.root.repository === "*"
          ? t("workspace:scripts.grants.everyRepository")
          : t("workspace:scripts.grants.repository", { key: scope.root.repository })
        : t("workspace:scripts.grants.folder", {
            label: scope.root.directory.label,
            path: scope.root.directory.path,
          })
    lines.push(
      scope.access === "read"
        ? t("workspace:scripts.grants.fs.read", { where })
        : scope.access === "write"
          ? t("workspace:scripts.grants.fs.write", { where })
          : t("workspace:scripts.grants.fs.delete", { where }),
    )
    if (scope.rootDestructive) lines.push(t("workspace:scripts.grants.rootDestructive", { where }))
  }
  for (const process of grants.process ?? []) {
    const qualifiers = [
      review.commandClasses[process.command] === "runsAnyProgram"
        ? t("workspace:scripts.grants.runsAnyProgram")
        : null,
      process.unrestrictedArgs ? t("workspace:scripts.grants.unrestrictedArgs") : null,
      process.env === "allowed" ? t("workspace:scripts.grants.envAllowed") : null,
      process.cwd === "any"
        ? t("workspace:scripts.grants.cwdAny")
        : process.cwd === "fsScopes"
          ? t("workspace:scripts.grants.cwdScopes")
          : t("workspace:scripts.grants.cwdRepositories"),
    ].filter((q): q is string => q !== null)
    lines.push(
      t("workspace:scripts.grants.process", {
        command: process.command,
        qualifiers: qualifiers.join("; "),
      }),
    )
  }
  if (grants.shell && grants.shell.shells.length > 0) {
    lines.push(t("workspace:scripts.grants.shell", { shells: grants.shell.shells.join(", ") }))
  }
  if (grants.destructive === "allow") lines.push(t("workspace:scripts.grants.destructiveAllow"))
  const limits = grants.limits
  if (limits) {
    lines.push(
      t("workspace:scripts.grants.limits", {
        seconds: Math.round((limits.timeoutMs ?? 60_000) / 1000),
        memory: limits.memoryMiB ?? 64,
        output: limits.maxOutputKiB ?? 256,
      }),
    )
  }
  const known = new Set([
    "projectsRead",
    "repositoriesPaths",
    "fs",
    "process",
    "shell",
    "destructive",
    "limits",
  ])
  for (const key of Object.keys(grants)) {
    if (!known.has(key)) lines.push(t("workspace:scripts.grants.unknown", { key }))
  }
  return lines
}

/**
 * A script that did not come from this editor (imported, duplicated, or
 * changed outside it): its code, read only, with invisible characters
 * marked, and the permissions that would come back. It runs nothing until
 * "Trust this script", which names exactly what was shown.
 */
function ReviewPanel({
  profileId,
  scriptId,
  onTrusted,
}: {
  profileId: string
  scriptId: string
  onTrusted: () => void
}) {
  const { t } = useTranslation()
  const review = useAsync(() => api.reviewScript(profileId, scriptId), `${profileId}:${scriptId}`)
  const [busy, setBusy] = useState(false)
  const lines = useGrantLines(review.state.status === "ready" ? review.state.data : null)
  if (review.state.status === "loading") return <Loading />
  if (review.state.status === "error") return <ErrorNotice error={review.state.error} />
  const data = review.state.data
  return (
    <div className="grid gap-4">
      <Alert>
        <ShieldAlert />
        <AlertTitle>{t("workspace:scripts.review.title")}</AlertTitle>
        <AlertDescription>{t("workspace:scripts.review.description")}</AlertDescription>
      </Alert>
      {hasHiddenChars(data.script.source) && (
        <Alert variant="destructive">
          <TriangleAlert />
          <AlertTitle>{t("workspace:scripts.review.hiddenTitle")}</AlertTitle>
          <AlertDescription>{t("workspace:scripts.review.hiddenDescription")}</AlertDescription>
        </Alert>
      )}
      <CodeEditor
        value={data.script.source}
        readOnly
        label={t("workspace:scripts.review.codeLabel", { name: data.script.name })}
        syntaxErrorMessage={t("workspace:scripts.editor.syntaxError")}
      />
      <div className="grid gap-2">
        <h3 className="text-sm font-medium">{t("workspace:scripts.review.grantsTitle")}</h3>
        {lines.length === 0 ? (
          <p className="text-sm text-muted-foreground">{t("workspace:scripts.review.noGrants")}</p>
        ) : (
          <ul
            className="list-disc pl-5 text-sm"
            aria-label={t("workspace:scripts.review.grantsTitle")}
          >
            {lines.map((line) => (
              <li key={line}>{line}</li>
            ))}
          </ul>
        )}
      </div>
      <div className="flex justify-end">
        <Button
          disabled={busy}
          onClick={() => {
            setBusy(true)
            api
              .trustScript(profileId, scriptId, data.sourceSha256, data.grants)
              .then(
                () => {
                  toast.success(t("workspace:scripts.review.trusted", { name: data.script.name }))
                  onTrusted()
                },
                (e: unknown) => {
                  toastFailure(t("workspace:scripts.review.failed", { name: data.script.name }), e)
                  review.reload()
                },
              )
              .finally(() => {
                setBusy(false)
              })
          }}
        >
          <ShieldCheck />
          {t("workspace:scripts.review.trust")}
        </Button>
      </div>
    </div>
  )
}

/**
 * The guarded delete: lists every use of the script. The default keeps
 * the keys, which then show the script missing; clearing the keys that run
 * it is opt-in.
 */
function DeleteScriptDialog({
  open,
  profileId,
  script,
  onClose,
  onDeleted,
}: {
  open: boolean
  profileId: string
  script: Script
  onClose: () => void
  onDeleted: () => void
}) {
  const { t } = useTranslation()
  // Mounted fresh for each opening (the caller renders it only while open).
  const [usages, setUsages] = useState<ScriptUsage[] | "failed" | null>(null)
  const [clearKeys, setClearKeys] = useState(false)
  useEffect(() => {
    api.scriptUsages(profileId, script.id).then(setUsages, () => {
      setUsages("failed")
    })
  }, [profileId, script.id])
  const uses = (Array.isArray(usages) ? usages : []).flatMap((u) =>
    u.references.map((r) => ({ action: u.actionName, owner: r.ownerName, detail: r.detail })),
  )
  return (
    <ConfirmDialog
      open={open}
      onOpenChange={(o) => {
        if (!o) onClose()
      }}
      title={t("workspace:scripts.delete.title", { name: script.name })}
      description={
        usages === null
          ? t("workspace:scripts.delete.loading")
          : usages === "failed"
            ? t("workspace:scripts.delete.usagesFailed")
            : uses.length === 0
              ? t("workspace:scripts.delete.unused")
              : t("workspace:scripts.delete.used", { count: uses.length })
      }
      confirmLabel={t("workspace:scripts.delete.confirm")}
      onConfirm={async () => {
        // Never before its uses are known: they are what the user confirms.
        if (!Array.isArray(usages)) throw new Error(t("workspace:scripts.delete.notYet"))
        await api.deleteScript(profileId, script.id, clearKeys)
        onDeleted()
      }}
      describeError={(e) =>
        failureMessage(t("workspace:scripts.delete.failed", { name: script.name }), e)
      }
    >
      {uses.length > 0 && (
        <div className="grid gap-3">
          <ul className="list-disc pl-5 text-sm" aria-label={t("workspace:scripts.delete.uses")}>
            {uses.map((use, i) => (
              <li key={i}>
                {use.action} — {use.owner}
                {use.detail ? ` › ${use.detail}` : ""}
              </li>
            ))}
          </ul>
          <label className="flex items-start gap-2 text-sm">
            <Checkbox
              className="mt-0.5"
              checked={clearKeys}
              onCheckedChange={(checked) => {
                setClearKeys(checked === true)
              }}
            />
            {t("workspace:scripts.delete.clearKeys")}
          </label>
        </div>
      )}
    </ConfirmDialog>
  )
}
