// Adding a connection: the provider, where its server is (the public
// service or the address of the user's own server), the token with the
// permissions it needs explained, read-only or allowed to make changes,
// and a name. Creating it is the consent to exactly that server; the agent
// checks the address (https only) and keeps the token. The connection is
// tested right after.

import { useId, useState } from "react"
import { toast } from "sonner"

import { ModalShell } from "@/components/modal-shell"
import { PasswordField } from "@/components/password-field"
import { Checkbox } from "@/components/ui/checkbox"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/switch"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { type TranslationKey, useTranslation } from "@/i18n"

import {
  type ConnectionDeployment,
  type PluginConnection,
  type PluginConnectionType,
  type PluginConnections,
  setupUrl,
  useConnectionsClient,
} from "./api"
import { errorText, useTestMessage } from "./text"

/** The permissions a token needs, per provider, in plain words. */
const SCOPES: Record<string, { read: TranslationKey; write: TranslationKey }> = {
  github: {
    read: "plugins:connections.scopes.github.read",
    write: "plugins:connections.scopes.github.write",
  },
  gitlab: {
    read: "plugins:connections.scopes.gitlab.read",
    write: "plugins:connections.scopes.gitlab.write",
  },
}

function hostOf(text: string): string | undefined {
  try {
    const url = new URL(text.trim())
    return url.protocol === "https:" ? url.host : undefined
  } catch {
    return undefined
  }
}

export function AddConnectionDialog({
  data,
  onClose,
  onCreated,
}: {
  data: PluginConnections
  onClose: () => void
  onCreated: (connection: PluginConnection) => void
}) {
  const { t } = useTranslation()
  const client = useConnectionsClient()
  const testMessage = useTestMessage()
  const id = useId()
  const [typeId, setTypeId] = useState(data.types[0]?.id ?? "")
  const type: PluginConnectionType | undefined = data.types.find((ty) => ty.id === typeId)
  const deployments: ConnectionDeployment[] = [
    ...(type?.cloudApiUrl ? (["cloud"] as const) : []),
    ...(type?.selfHosted ? (["selfHosted"] as const) : []),
  ]
  const [deployment, setDeployment] = useState<ConnectionDeployment>(deployments[0] ?? "cloud")
  const [server, setServer] = useState("")
  const [privateNetwork, setPrivateNetwork] = useState(false)
  const [allowChanges, setAllowChanges] = useState(false)
  const [secret, setSecret] = useState("")
  const [name, setName] = useState("")
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const selfHosted = deployment === "selfHosted"
  const host = selfHosted ? hostOf(server) : hostOf(type?.cloudWebUrl ?? "")
  const shownName = name.trim() || (type && host ? `${type.label} @ ${host}` : "")
  const tokenPage = type ? setupUrl(type, deployment, server) : undefined
  const scopes = SCOPES[typeId]
  const ready = !!type && !!secret.trim() && !!shownName && (!selfHosted || !!host)

  const create = async () => {
    if (!type || !ready) return
    setSaving(true)
    setError(null)
    try {
      const created = await client.create(data.plugin, {
        connectionType: type.id,
        deployment,
        url: selfHosted ? server.trim() : undefined,
        displayName: shownName,
        allowChanges,
        privateNetwork: selfHosted && privateNetwork,
        secret: secret.trim(),
      })
      setSecret("")
      onCreated(created)
      onClose()
      // Tested at once, so the list shows whether it works.
      try {
        const tested = await client.test(data.plugin, created.id)
        onCreated(tested.connection)
        const message = testMessage(tested.outcome, tested.connection)
        if (tested.outcome.ok) toast.success(message)
        else toast.error(message)
      } catch (e) {
        toast.error(errorText(e))
      }
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
      size="medium"
      title={t("plugins:connections.addDialog.title")}
      description={t("plugins:connections.addDialog.description")}
      error={error}
      onClose={onClose}
      onSubmit={() => void create()}
      primary={{
        label: t("plugins:connections.addDialog.create"),
        loading: saving,
        disabled: !ready,
      }}
      secondary={{ label: t("common:actions.cancel"), onClick: onClose, variant: "outline" }}
    >
      <div className="grid gap-5">
        {data.types.length > 1 && (
          <div className="grid gap-2">
            <Label id={`${id}-type`}>{t("plugins:connections.addDialog.provider")}</Label>
            <ToggleGroup
              type="single"
              variant="outline"
              aria-labelledby={`${id}-type`}
              value={typeId}
              onValueChange={(value) => {
                if (!value) return
                setTypeId(value)
                const next = data.types.find((ty) => ty.id === value)
                setDeployment(next?.cloudApiUrl ? "cloud" : "selfHosted")
                // A consent is for one server: never carried to another.
                setServer("")
                setPrivateNetwork(false)
              }}
            >
              {data.types.map((ty) => (
                <ToggleGroupItem key={ty.id} value={ty.id}>
                  {ty.label}
                </ToggleGroupItem>
              ))}
            </ToggleGroup>
          </div>
        )}
        {deployments.length > 1 && (
          <div className="grid gap-2">
            <Label id={`${id}-where`}>{t("plugins:connections.addDialog.where")}</Label>
            <ToggleGroup
              type="single"
              variant="outline"
              aria-labelledby={`${id}-where`}
              value={deployment}
              onValueChange={(value) => {
                if (!value) return
                setDeployment(value as ConnectionDeployment)
                setServer("")
                setPrivateNetwork(false)
              }}
            >
              <ToggleGroupItem value="cloud">
                {type?.cloudWebUrl ? hostOf(type.cloudWebUrl) : t("plugins:connections.cloud")}
              </ToggleGroupItem>
              <ToggleGroupItem value="selfHosted">
                {type?.selfHostedLabel ?? t("plugins:connections.addDialog.ownServer")}
              </ToggleGroupItem>
            </ToggleGroup>
          </div>
        )}
        {selfHosted && (
          <div className="grid gap-2">
            <Label htmlFor={`${id}-server`}>{t("plugins:connections.addDialog.server")}</Label>
            <Input
              id={`${id}-server`}
              value={server}
              placeholder="https://git.example.com"
              inputMode="url"
              autoComplete="off"
              spellCheck={false}
              onChange={(e) => {
                setServer(e.target.value)
                // Another server: its private network is asked again.
                setPrivateNetwork(false)
              }}
            />
            <p className="text-caption text-muted-foreground">
              {t("plugins:connections.addDialog.serverHint")}
            </p>
            {data.privateNetworkAllowed && (
              <div className="flex items-start gap-2">
                <Checkbox
                  id={`${id}-private`}
                  checked={privateNetwork}
                  onCheckedChange={(checked) => {
                    setPrivateNetwork(checked === true)
                  }}
                />
                <div className="grid gap-1">
                  <Label htmlFor={`${id}-private`}>
                    {t("plugins:connections.addDialog.privateNetwork")}
                  </Label>
                  <p className="text-caption text-muted-foreground">
                    {t("plugins:connections.addDialog.privateNetworkHint")}
                  </p>
                </div>
              </div>
            )}
          </div>
        )}
        {data.changesAllowed && (
          <div className="flex items-start justify-between gap-4">
            <div className="grid gap-1">
              <Label htmlFor={`${id}-changes`}>
                {t("plugins:connections.addDialog.allowChanges")}
              </Label>
              <p className="text-caption text-muted-foreground">
                {t("plugins:connections.addDialog.allowChangesHint")}
              </p>
            </div>
            <Switch id={`${id}-changes`} checked={allowChanges} onCheckedChange={setAllowChanges} />
          </div>
        )}
        <div className="grid gap-2">
          <PasswordField
            id={`${id}-token`}
            label={t("plugins:connections.token")}
            value={secret}
            onChange={setSecret}
            autoComplete="new-password"
            revealable
          />
          <p className="text-caption text-muted-foreground" data-testid="connection-scopes">
            {scopes
              ? t(allowChanges ? scopes.write : scopes.read)
              : t("plugins:connections.scopes.generic")}
          </p>
          {tokenPage && (
            <p className="text-caption text-muted-foreground">
              {t("plugins:connections.addDialog.tokenPage")}{" "}
              <span className="font-mono break-all select-all">{tokenPage}</span>
            </p>
          )}
          <p className="text-caption text-muted-foreground">
            {t("plugins:connections.addDialog.tokenKept")}
          </p>
        </div>
        <div className="grid gap-2">
          <Label htmlFor={`${id}-name`}>{t("plugins:connections.addDialog.name")}</Label>
          <Input
            id={`${id}-name`}
            value={name}
            placeholder={shownName}
            maxLength={64}
            onChange={(e) => {
              setName(e.target.value)
            }}
          />
        </div>
        <p className="text-caption text-muted-foreground">
          {t("plugins:connections.addDialog.tls")}
        </p>
      </div>
    </ModalShell>
  )
}
