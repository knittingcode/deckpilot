// Settings › Connections: the servers plugins are connected to (plugin API
// 1.14), listed per plugin that declares connection types, with Add, Test,
// Replace token, Turn off and Remove. Tokens are typed here, sent to the
// agent once and never shown again.

import { Plus } from "lucide-react"
import { useEffect, useState } from "react"
import { toast } from "sonner"

import { ConfirmDialog } from "@/components/confirm-dialog"
import { Loading } from "@/components/page"
import { ModalShell } from "@/components/modal-shell"
import { PasswordField } from "@/components/password-field"
import {
  InlineStatus,
  SettingsGroup,
  SettingsNote,
  SettingsRow,
  SettingsSection,
  type StatusTone,
} from "@/components/settings-layout"
import { Button } from "@/components/ui/button"
import { type TranslationKey, useTranslation } from "@/i18n"

import { AddConnectionDialog } from "./add-connection-dialog"
import {
  type ConnectionState,
  type PluginConnection,
  type PluginConnections,
  connectionHost,
  loadConnectionPlugins,
  useConnectionsClient,
} from "./api"
import { errorText, useTestMessage } from "./text"

const STATES: Record<ConnectionState, { label: TranslationKey; tone: StatusTone }> = {
  ok: { label: "plugins:connections.state.ok", tone: "positive" },
  unauthorized: { label: "plugins:connections.state.unauthorized", tone: "negative" },
  insufficientScope: { label: "plugins:connections.state.insufficientScope", tone: "warning" },
  unreachable: { label: "plugins:connections.state.unreachable", tone: "warning" },
  tlsFailed: { label: "plugins:connections.state.tlsFailed", tone: "negative" },
  needsReconnect: { label: "plugins:connections.state.needsReconnect", tone: "warning" },
  untested: { label: "plugins:connections.state.untested", tone: "neutral" },
  disabled: { label: "plugins:connections.state.disabled", tone: "neutral" },
  other: { label: "plugins:connections.state.other", tone: "neutral" },
}

interface Loaded {
  name: string
  connections: PluginConnections
}

async function loadAll(
  client: ReturnType<typeof useConnectionsClient>,
): Promise<{ plugins: Loaded[]; error: string | null }> {
  try {
    return { plugins: await loadConnectionPlugins(client), error: null }
  } catch (e) {
    return { plugins: [], error: errorText(e) }
  }
}

export function ConnectionsSettings() {
  const { t } = useTranslation()
  const client = useConnectionsClient()
  const [plugins, setPlugins] = useState<Loaded[] | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let active = true
    void loadAll(client).then((loaded) => {
      if (!active) return
      setPlugins(loaded.plugins)
      setError(loaded.error)
    })
    return () => {
      active = false
    }
  }, [client])

  const replace = (plugin: string, next: PluginConnections) => {
    setPlugins((current) =>
      (current ?? []).map((p) =>
        p.connections.plugin === plugin ? { ...p, connections: next } : p,
      ),
    )
  }
  const replaceOne = (plugin: string, connection: PluginConnection) => {
    setPlugins((current) =>
      (current ?? []).map((p) => {
        if (p.connections.plugin !== plugin) return p
        const known = p.connections.connections.some((c) => c.id === connection.id)
        const connections = known
          ? p.connections.connections.map((c) => (c.id === connection.id ? connection : c))
          : [...p.connections.connections, connection]
        return { ...p, connections: { ...p.connections, connections } }
      }),
    )
  }

  if (plugins === null) return <Loading />
  return (
    <div className="grid gap-8">
      {error && <SettingsNote role="alert">{error}</SettingsNote>}
      {plugins.length === 0 && !error && (
        <SettingsNote>{t("plugins:connections.none")}</SettingsNote>
      )}
      {plugins.map((p) => (
        <PluginConnectionsSection
          key={p.connections.plugin}
          name={p.name}
          data={p.connections}
          onReplace={(next) => {
            replace(p.connections.plugin, next)
          }}
          onChanged={(c) => {
            replaceOne(p.connections.plugin, c)
          }}
        />
      ))}
    </div>
  )
}

function PluginConnectionsSection({
  name,
  data,
  onReplace,
  onChanged,
}: {
  name: string
  data: PluginConnections
  onReplace: (next: PluginConnections) => void
  onChanged: (connection: PluginConnection) => void
}) {
  const { t } = useTranslation()
  const [adding, setAdding] = useState(false)
  return (
    <SettingsSection
      title={name}
      description={t("plugins:connections.intro")}
      actions={
        <Button
          size="sm"
          variant="outline"
          disabled={!data.granted}
          onClick={() => {
            setAdding(true)
          }}
        >
          <Plus aria-hidden />
          {t("plugins:connections.add")}
        </Button>
      }
    >
      {!data.granted && <SettingsNote>{t("plugins:connections.notGranted")}</SettingsNote>}
      {data.connections.length === 0 ? (
        <SettingsNote>{t("plugins:connections.empty")}</SettingsNote>
      ) : (
        <SettingsGroup>
          {data.connections.map((c) => (
            <ConnectionRow
              key={c.id}
              plugin={data.plugin}
              connection={c}
              typeLabel={data.types.find((ty) => ty.id === c.type)?.label ?? c.type}
              onChanged={onChanged}
              onRemoved={onReplace}
            />
          ))}
        </SettingsGroup>
      )}
      {adding && (
        <AddConnectionDialog
          data={data}
          onClose={() => {
            setAdding(false)
          }}
          onCreated={(connection) => {
            onChanged(connection)
          }}
        />
      )}
    </SettingsSection>
  )
}

function ConnectionRow({
  plugin,
  connection,
  typeLabel,
  onChanged,
  onRemoved,
}: {
  plugin: string
  connection: PluginConnection
  typeLabel: string
  onChanged: (connection: PluginConnection) => void
  onRemoved: (next: PluginConnections) => void
}) {
  const { t } = useTranslation()
  const client = useConnectionsClient()
  const testMessage = useTestMessage()
  const [busy, setBusy] = useState(false)
  const [replacing, setReplacing] = useState(false)
  const [removing, setRemoving] = useState(false)
  const [turningOff, setTurningOff] = useState(false)
  const state = STATES[connection.state]
  const run = async (work: () => Promise<void>) => {
    setBusy(true)
    try {
      await work()
    } catch (e) {
      toast.error(errorText(e))
    } finally {
      setBusy(false)
    }
  }
  const details = [
    typeLabel,
    connection.deployment === "selfHosted" ? connectionHost(connection) : null,
    connection.account,
  ].filter(Boolean)
  return (
    <SettingsRow
      align="start"
      label={connection.displayName}
      description={
        <span className="grid gap-1">
          <span>{details.join(" · ")}</span>
          <span className="flex flex-wrap gap-x-3 gap-y-1">
            <InlineStatus tone={state.tone}>{t(state.label)}</InlineStatus>
            {connection.isDefault && (
              <InlineStatus>{t("plugins:connections.default")}</InlineStatus>
            )}
            <InlineStatus>
              {connection.allowChanges
                ? t("plugins:connections.canChange")
                : t("plugins:connections.readOnly")}
            </InlineStatus>
            {connection.privateNetwork && (
              <InlineStatus>{t("plugins:connections.privateNetwork")}</InlineStatus>
            )}
          </span>
        </span>
      }
    >
      <Button
        size="sm"
        variant="outline"
        disabled={busy || !connection.enabled}
        onClick={() =>
          void run(async () => {
            const tested = await client.test(plugin, connection.id)
            onChanged(tested.connection)
            const message = testMessage(tested.outcome, tested.connection)
            if (tested.outcome.ok) toast.success(message)
            else toast.error(message)
          })
        }
      >
        {t("plugins:connections.test.action")}
      </Button>
      <Button
        size="sm"
        variant="outline"
        disabled={busy}
        onClick={() => {
          setReplacing(true)
        }}
      >
        {t("plugins:connections.replaceToken")}
      </Button>
      <Button
        size="sm"
        variant="outline"
        disabled={busy}
        onClick={() => {
          if (connection.enabled) {
            setTurningOff(true)
            return
          }
          void run(async () => {
            onChanged(await client.update(plugin, connection.id, { enabled: true }))
          })
        }}
      >
        {connection.enabled ? t("plugins:connections.turnOff") : t("plugins:connections.turnOn")}
      </Button>
      <Button
        size="sm"
        variant="outline"
        disabled={busy}
        onClick={() => {
          setRemoving(true)
        }}
      >
        {t("plugins:connections.remove.action")}
      </Button>
      {replacing && (
        <TokenDialog
          connection={connection}
          onClose={() => {
            setReplacing(false)
          }}
          onSave={async (secret) => {
            onChanged(await client.setSecret(plugin, connection.id, secret))
            toast.success(t("plugins:connections.tokenReplaced"))
          }}
        />
      )}
      <ConfirmDialog
        open={turningOff}
        onOpenChange={setTurningOff}
        title={t("plugins:connections.turnOffDialog.title", { name: connection.displayName })}
        description={t("plugins:connections.turnOffDialog.description")}
        confirmLabel={t("plugins:connections.turnOff")}
        onConfirm={async () => {
          onChanged(await client.update(plugin, connection.id, { enabled: false }))
        }}
        describeError={errorText}
      />
      <ConfirmDialog
        open={removing}
        onOpenChange={setRemoving}
        destructive
        title={t("plugins:connections.remove.title", { name: connection.displayName })}
        description={t("plugins:connections.remove.description")}
        confirmLabel={t("plugins:connections.remove.confirm")}
        onConfirm={async () => {
          onRemoved(await client.remove(plugin, connection.id))
        }}
        describeError={errorText}
      />
    </SettingsRow>
  )
}

/** A new token for a connection (reconnect): its id and server stay. */
function TokenDialog({
  connection,
  onClose,
  onSave,
}: {
  connection: PluginConnection
  onClose: () => void
  onSave: (secret: string) => Promise<void>
}) {
  const { t } = useTranslation()
  const [secret, setSecret] = useState("")
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const save = async () => {
    if (!secret.trim()) return
    setSaving(true)
    try {
      await onSave(secret.trim())
      setSecret("")
      onClose()
    } catch (e) {
      setError(errorText(e))
    } finally {
      setSaving(false)
    }
  }
  return (
    <ModalShell
      presentation="dialog"
      open
      size="compact"
      title={t("plugins:connections.tokenDialog.title", { name: connection.displayName })}
      description={t("plugins:connections.tokenDialog.description", {
        host: connectionHost(connection),
      })}
      error={error}
      onClose={onClose}
      onSubmit={() => void save()}
      primary={{
        label: t("plugins:connections.tokenDialog.save"),
        loading: saving,
        disabled: !secret.trim(),
      }}
      secondary={{ label: t("common:actions.cancel"), onClick: onClose, variant: "outline" }}
    >
      <PasswordField
        id="connection-token"
        label={t("plugins:connections.token")}
        value={secret}
        onChange={setSecret}
        autoComplete="new-password"
        autoFocus
        revealable
      />
    </ModalShell>
  )
}
