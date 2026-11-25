// Connections of plugins (plugin API 1.14, IPC protocol 20): the servers a
// plugin is connected to, each with its own address, consent, token and
// state (`deckpilot_ipc::connections`). A token goes from this window to the
// agent only, through the Tauri commands of
// `src-tauri/src/commands/plugins.rs`; no answer carries it back.

import { invoke } from "@tauri-apps/api/core"
import { createContext, useContext } from "react"

import { toPluginError } from "../api"

export type ConnectionDeployment = "cloud" | "selfHosted"

export type ConnectionState =
  | "ok"
  | "unauthorized"
  | "insufficientScope"
  | "unreachable"
  | "tlsFailed"
  | "needsReconnect"
  | "untested"
  | "disabled"
  | "other"

/** One connection. Never its token. */
export interface PluginConnection {
  id: string
  type: string
  displayName: string
  deployment: ConnectionDeployment
  apiUrl: string
  webUrl: string
  allowChanges: boolean
  privateNetwork: boolean
  enabled: boolean
  state: ConnectionState
  stateDetail?: string
  account?: string
  capabilities?: Record<string, unknown>
  isDefault: boolean
  hasSecret: boolean
  secretUpdatedAt?: number
  createdAt: number
}

export interface ConnectionTypeAuth {
  method: string
  cloudSetupUrl?: string
  /** A template with `{web}`: the server the user entered. */
  selfHostedSetupUrl?: string
}

/** A connection type a plugin declares, in the app's language. */
export interface PluginConnectionType {
  id: string
  label: string
  cloudApiUrl?: string
  cloudWebUrl?: string
  selfHosted: boolean
  selfHostedLabel?: string
  selfHostedApiPath?: string
  auth: ConnectionTypeAuth[]
}

export interface PluginConnections {
  plugin: string
  /** The plugin's `connections` permission is granted. */
  granted: boolean
  privateNetworkAllowed: boolean
  changesAllowed: boolean
  types: PluginConnectionType[]
  connections: PluginConnection[]
}

export interface ConnectionTestOutcome {
  ok: boolean
  code?: string
  message?: string
}

export interface ConnectionTested {
  connection: PluginConnection
  outcome: ConnectionTestOutcome
}

export interface NewConnection {
  connectionType: string
  deployment: ConnectionDeployment
  /** The address the user opens in the browser (self-hosted only). */
  url?: string
  displayName: string
  allowChanges: boolean
  privateNetwork: boolean
  /** The token; sent once, never kept here. */
  secret?: string
}

/** A git remote of a project repository (K1–K3). */
export interface RepositoryRemote {
  name: string
  primary?: boolean
  connectionId?: string
  /** The provider's immutable id (`gh:…`, `gl:…`), never a name. */
  remoteRepositoryId?: string
  remoteUrl?: string
  defaultBranch?: string
}

/** A repository the plugin lists on a connection (`list-repositories`). */
export interface RemoteRepositoryRecord {
  id: string
  path: string
  name?: string
  defaultBranch?: string
  webUrl?: string
  httpUrl?: string
  sshUrl?: string
  archived?: boolean
}

export interface ConnectionChange {
  displayName?: string
  enabled?: boolean
  allowChanges?: boolean
  makeDefault?: boolean
}

/** One method per connection request. */
export interface ConnectionsClient {
  /** The installed plugins (`listPlugins`), to ask each for its connections. */
  listPlugins: () => Promise<{ id: string; name: string; state: string }[]>
  list: (plugin: string) => Promise<PluginConnections>
  create: (plugin: string, connection: NewConnection) => Promise<PluginConnection>
  setSecret: (plugin: string, connection: string, secret: string) => Promise<PluginConnection>
  update: (
    plugin: string,
    connection: string,
    change: ConnectionChange,
  ) => Promise<PluginConnection>
  test: (plugin: string, connection: string) => Promise<ConnectionTested>
  remove: (plugin: string, connection: string) => Promise<PluginConnections>
  /** The plugin's internal `list-repositories` on one connection. */
  listRepositories: (
    plugin: string,
    connection: string,
    search: string,
  ) => Promise<RemoteRepositoryRecord[]>
  /** `set_repository_remotes`: the link the user confirmed (K3). */
  setRepositoryRemotes: (
    profileId: string,
    projectId: string,
    repositoryId: string,
    remotes: RepositoryRemote[],
  ) => Promise<unknown>
}

/** An immutable provider id (K2): `gh:`, `gl:` or `bb:` and a short id. */
export function isRemoteRepositoryId(id: string): boolean {
  return /^(gh|gl|bb):[0-9A-Za-z{}_-]{1,64}$/.test(id)
}

interface ActionOutcome {
  status: string
  data?: Record<string, unknown>
  code?: string
  message?: string
}

async function call<T>(command: string, args: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args)
  } catch (error) {
    throw toPluginError(error)
  }
}

export function createTauriConnectionsClient(): ConnectionsClient {
  return {
    listPlugins: () => call("list_plugins", {}),
    list: (plugin) => call("list_plugin_connections", { plugin }),
    create: (plugin, c) =>
      call("create_plugin_connection", {
        plugin,
        connectionType: c.connectionType,
        deployment: c.deployment,
        url: c.url ?? null,
        displayName: c.displayName,
        allowChanges: c.allowChanges,
        privateNetwork: c.privateNetwork,
        secret: c.secret ?? null,
      }),
    setSecret: (plugin, connection, secret) =>
      call("set_plugin_connection_secret", { plugin, connection, secret }),
    update: (plugin, connection, change) =>
      call("update_plugin_connection", {
        plugin,
        connection,
        displayName: change.displayName ?? null,
        enabled: change.enabled ?? null,
        allowChanges: change.allowChanges ?? null,
        makeDefault: change.makeDefault ?? null,
      }),
    test: (plugin, connection) => call("test_plugin_connection", { plugin, connection }),
    remove: (plugin, connection) => call("remove_plugin_connection", { plugin, connection }),
    listRepositories: async (plugin, connection, search) => {
      const params: Record<string, string> = { connection }
      if (search.trim()) params.search = search.trim()
      const outcome = await call<ActionOutcome>("invoke_plugin_action", {
        plugin,
        action: "list-repositories",
        params,
      })
      if (outcome.status !== "ok") {
        throw new Error(outcome.message ?? outcome.code ?? "the repositories could not be listed")
      }
      const items = outcome.data?.items
      return Array.isArray(items) ? (items as RemoteRepositoryRecord[]) : []
    },
    setRepositoryRemotes: (profileId, projectId, repositoryId, remotes) =>
      call("set_repository_remotes", { profileId, projectId, repositoryId, remotes }),
  }
}

export const ConnectionsClientContext = createContext<ConnectionsClient | null>(null)

let tauriClient: ConnectionsClient | undefined

/** The client of the context (tests), else the app's own. */
export function useConnectionsClient(): ConnectionsClient {
  const provided = useContext(ConnectionsClientContext)
  if (provided) return provided
  tauriClient ??= createTauriConnectionsClient()
  return tauriClient
}

/** The host a connection shows: the server's, never a token. */
export function connectionHost(connection: Pick<PluginConnection, "webUrl">): string {
  try {
    return new URL(connection.webUrl).host
  } catch {
    return connection.webUrl
  }
}

/**
 * Where the user creates a token: the cloud page, or the self-hosted
 * template with the entered server (`{web}`). Only an https address of
 * that server; anything else is not offered.
 */
export function setupUrl(
  type: PluginConnectionType,
  deployment: ConnectionDeployment,
  server: string,
): string | undefined {
  const auth = type.auth[0]
  if (!auth) return undefined
  if (deployment === "cloud") return auth.cloudSetupUrl
  const template = auth.selfHostedSetupUrl
  if (!template?.startsWith("{web}")) return undefined
  let origin: string
  try {
    const url = new URL(server.trim())
    if (url.protocol !== "https:" || url.username || url.password) return undefined
    origin = `${url.origin}${url.pathname.replace(/\/+$/, "")}`
  } catch {
    return undefined
  }
  return `${origin}${template.slice("{web}".length)}`
}

export interface ConnectionPlugin {
  name: string
  connections: PluginConnections
}

/**
 * Every installed plugin that declares connection types, with its
 * connections; a plugin that cannot answer now is left out.
 */
export async function loadConnectionPlugins(
  client: ConnectionsClient,
): Promise<ConnectionPlugin[]> {
  const installed = await client.listPlugins()
  const loaded = await Promise.all(
    installed.map(async (p) => {
      try {
        return { name: p.name, connections: await client.list(p.id) }
      } catch {
        return null
      }
    }),
  )
  return loaded.filter((p): p is ConnectionPlugin => !!p && p.connections.types.length > 0)
}

/**
 * `remotes` with the chosen repository as the remote `name`: it replaces a
 * remote of that name, and as the primary one it makes no other primary.
 */
export function withLink(
  remotes: RepositoryRemote[],
  name: string,
  primary: boolean,
  connectionId: string,
  record: RemoteRepositoryRecord,
): RepositoryRemote[] {
  const linked: RepositoryRemote = {
    name,
    primary,
    connectionId,
    remoteRepositoryId: record.id,
    ...(record.httpUrl ? { remoteUrl: record.httpUrl } : {}),
    ...(record.defaultBranch ? { defaultBranch: record.defaultBranch } : {}),
  }
  const others = remotes
    .filter((r) => r.name !== name)
    .map((r) => (primary ? { ...r, primary: false } : r))
  return [...others, linked]
}
