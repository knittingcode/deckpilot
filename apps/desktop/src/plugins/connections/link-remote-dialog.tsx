// Linking a project repository to a repository on one of the user's
// connections (decisions K1–K3): choose the connection, find the repository
// with the plugin's own `list-repositories`, check what will be linked, and
// confirm. The provider's id comes only from the plugin's record, never
// from text typed here; only then does the app write the link
// (`link_repository_remote`, which checks the connection and merges the
// remote with the stored ones). A plugin never writes it.

import { useEffect, useId, useRef, useState } from "react"

import { ModalShell } from "@/components/modal-shell"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { useTranslation } from "@/i18n"

import {
  type ConnectionPlugin,
  type RemoteRepositoryRecord,
  type RepositoryRemote,
  connectionHost,
  isRemoteRepositoryId,
  loadConnectionPlugins,
  remoteOf,
  useConnectionsClient,
  withLink,
} from "./api"
import { errorText } from "./text"

export function LinkRemoteDialog({
  profileId,
  projectId,
  repositoryId,
  repositoryName,
  remotes,
  onClose,
  onLinked,
}: {
  profileId: string
  projectId: string
  repositoryId: string
  repositoryName: string
  remotes: RepositoryRemote[]
  onClose: () => void
  onLinked: (remotes: RepositoryRemote[]) => void
}) {
  const { t } = useTranslation()
  const client = useConnectionsClient()
  const id = useId()
  const [plugins, setPlugins] = useState<ConnectionPlugin[] | null>(null)
  const [choice, setChoice] = useState("")
  const [search, setSearch] = useState("")
  const [records, setRecords] = useState<RemoteRepositoryRecord[] | null>(null)
  const [picked, setPicked] = useState<RemoteRepositoryRecord | null>(null)
  const [name, setName] = useState(remotes.length === 0 ? "origin" : "")
  const [primary, setPrimary] = useState(!remotes.some((r) => r.primary))
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  // Which search's answer may be shown: a late answer for another
  // connection is dropped, never attached to the one chosen now.
  const searchRef = useRef(0)

  useEffect(() => {
    let active = true
    void loadConnectionPlugins(client).then(
      (loaded) => {
        if (active) setPlugins(loaded)
      },
      (e: unknown) => {
        if (!active) return
        setPlugins([])
        setError(errorText(e))
      },
    )
    return () => {
      active = false
    }
  }, [client])

  const options = (plugins ?? []).flatMap((p) =>
    p.connections.connections
      .filter((c) => c.enabled && c.hasSecret)
      .map((c) => ({ key: `${p.connections.plugin} ${c.id}`, plugin: p.connections.plugin, c })),
  )
  const chosen = options.find((o) => o.key === choice)

  const find = async () => {
    if (!chosen) return
    const ticket = ++searchRef.current
    setBusy(true)
    setError(null)
    setPicked(null)
    try {
      const found = await client.listRepositories(chosen.plugin, chosen.c.id, search)
      if (ticket !== searchRef.current) return
      // Only records with an immutable id can be linked (K2).
      setRecords(found.filter((r) => isRemoteRepositoryId(r.id)))
    } catch (e) {
      if (ticket === searchRef.current) setError(errorText(e))
    } finally {
      if (ticket === searchRef.current) setBusy(false)
    }
  }

  const link = async () => {
    if (!chosen || !picked || !name.trim()) return
    setBusy(true)
    setError(null)
    try {
      // Only the one remote: the app's backend merges it with what is
      // stored now.
      const remote = remoteOf(name.trim(), primary, chosen.c.id, picked)
      const saved = await client.linkRepositoryRemote(
        profileId,
        projectId,
        repositoryId,
        chosen.plugin,
        remote,
      )
      onLinked(saved.remotes ?? withLink(remotes, name.trim(), primary, chosen.c.id, picked))
      onClose()
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <ModalShell
      presentation="dialog"
      open
      size="medium"
      title={t("plugins:connections.link.title", { repository: repositoryName })}
      description={t("plugins:connections.link.description")}
      error={error}
      onClose={onClose}
      onSubmit={() => void link()}
      primary={{
        label: t("plugins:connections.link.confirm"),
        loading: busy && !!picked,
        disabled: !picked || !name.trim() || busy,
      }}
      secondary={{ label: t("common:actions.cancel"), onClick: onClose, variant: "outline" }}
    >
      <div className="grid gap-5">
        {plugins !== null && options.length === 0 && (
          <p className="text-caption text-muted-foreground">
            {t("plugins:connections.link.noConnections")}
          </p>
        )}
        {options.length > 0 && (
          <div className="grid gap-2">
            <Label id={`${id}-connection`}>{t("plugins:connections.link.connection")}</Label>
            <ToggleGroup
              type="single"
              variant="outline"
              aria-labelledby={`${id}-connection`}
              value={choice}
              disabled={busy}
              onValueChange={(value) => {
                searchRef.current += 1
                setChoice(value)
                setRecords(null)
                setPicked(null)
              }}
              className="flex-wrap"
            >
              {options.map((o) => (
                <ToggleGroupItem key={o.key} value={o.key}>
                  {o.c.deployment === "selfHosted"
                    ? `${o.c.displayName} (${connectionHost(o.c)})`
                    : o.c.displayName}
                </ToggleGroupItem>
              ))}
            </ToggleGroup>
          </div>
        )}
        {chosen && (
          <div className="grid gap-2">
            <Label htmlFor={`${id}-search`}>{t("plugins:connections.link.search")}</Label>
            <div className="flex gap-2">
              <Input
                id={`${id}-search`}
                value={search}
                autoComplete="off"
                spellCheck={false}
                onChange={(e) => {
                  setSearch(e.target.value)
                }}
                // Enter searches; it never confirms the link.
                onKeyDown={(e) => {
                  if (e.key !== "Enter") return
                  e.preventDefault()
                  e.stopPropagation()
                  void find()
                }}
              />
              <Button type="button" variant="outline" disabled={busy} onClick={() => void find()}>
                {t("plugins:connections.link.find")}
              </Button>
            </div>
            {records !== null &&
              (records.length === 0 ? (
                <p className="text-caption text-muted-foreground">
                  {t("plugins:connections.link.nothingFound")}
                </p>
              ) : (
                <ul className="grid gap-1" aria-label={t("plugins:connections.link.results")}>
                  {records.map((r) => (
                    <li key={r.id}>
                      <Button
                        type="button"
                        variant={picked?.id === r.id ? "default" : "ghost"}
                        aria-pressed={picked?.id === r.id}
                        className="w-full justify-start"
                        onClick={() => {
                          setPicked(r)
                        }}
                      >
                        {r.path || r.name || r.id}
                      </Button>
                    </li>
                  ))}
                </ul>
              ))}
          </div>
        )}
        {picked && chosen && (
          <div className="grid gap-3" data-testid="link-summary">
            <p className="text-caption text-muted-foreground">
              {t("plugins:connections.link.summary", {
                path: picked.path,
                id: picked.id,
                connection: chosen.c.displayName,
              })}
            </p>
            {picked.httpUrl && (
              <p className="font-mono text-caption break-all select-all">{picked.httpUrl}</p>
            )}
            <div className="grid gap-2">
              <Label htmlFor={`${id}-name`}>{t("plugins:connections.link.remoteName")}</Label>
              <Input
                id={`${id}-name`}
                value={name}
                maxLength={64}
                autoComplete="off"
                spellCheck={false}
                onChange={(e) => {
                  setName(e.target.value)
                }}
              />
              {remotes.some((r) => r.name === name.trim()) && (
                // The link takes the place of the stored remote of that name.
                <p className="text-caption text-muted-foreground" role="note">
                  {t("plugins:connections.link.replaces", { name: name.trim() })}
                </p>
              )}
            </div>
            <div className="flex items-center gap-2">
              <Checkbox
                id={`${id}-primary`}
                checked={primary}
                onCheckedChange={(checked) => {
                  setPrimary(checked === true)
                }}
              />
              <Label htmlFor={`${id}-primary`}>{t("plugins:connections.link.primary")}</Label>
            </div>
          </div>
        )}
      </div>
    </ModalShell>
  )
}
