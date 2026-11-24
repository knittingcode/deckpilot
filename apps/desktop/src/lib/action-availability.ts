// Why an action that is already assigned cannot run now, in terms the
// Inspector can explain and offer a way out of (docs/key-text.md, "Keys
// whose plugin is missing").
//
// The key itself only shows the compact missing-plugin face (drawn by the
// agent's renderer, the same on the Canvas, the simulated and the physical
// device); this says *why*: the plugin is not installed, disabled,
// incompatible, failed verification, was withdrawn by the registry, is busy
// for a moment, or waits for a permission. Everything comes from what the
// agent reported (`listPlugins`, `listModuleMigrations`, through
// `lib/plugin-actions.ts`); nothing here installs, enables or grants
// anything. The recovery opens the existing Marketplace or Settings ›
// Plugins, where the user decides.

import { NOT_INSTALLED, isPluginId, isPluginType, ownerOf, pluginsEnabled } from "./plugin-actions"
import type { LegacyAlias, ModuleInfo } from "./types"

/** Why an assigned action cannot run now. */
export type MissingReason =
  /** Its plugin is not installed (any more). */
  | "notInstalled"
  /** The user disabled the plugin (or withdrew it from a migrated module). */
  | "disabled"
  /** The installed version does not provide the action (as stored). */
  | "incompatible"
  /** The plugin's files do not match their record. */
  | "integrity"
  /** The registry revoked the installed version. */
  | "revoked"
  /** The plugin is being installed, updated or rolled back, or its host is away. */
  | "busy"
  /** A permission the action needs is not granted. */
  | "permission"
  /** A built-in module the action belongs to is turned off. */
  | "moduleDisabled"

/** What the Inspector offers to make the action run again. */
export type RecoveryAction = "install" | "enable" | "update" | "permissions" | "enableModule"

export interface MissingAction {
  reason: MissingReason
  /**
   * The plugin (or module) that provides the action. Only for opening the
   * right screen; never shown to the user.
   */
  owner: string
  /** Its name to show; `null` when it is not known here. */
  ownerName: string | null
  /** The one recovery the reason calls for; `null` when it comes back by itself. */
  recovery: RecoveryAction | null
}

const RECOVERY: Record<MissingReason, RecoveryAction | null> = {
  notInstalled: "install",
  disabled: "enable",
  incompatible: "update",
  integrity: "update",
  revoked: "update",
  busy: null,
  permission: "permissions",
  moduleDisabled: "enableModule",
}

const BUSY_STATES = new Set(["installing", "updating", "rollingBack", "uninstalling"])

function missing(reason: MissingReason, owner: string, ownerName: string | null): MissingAction {
  return { reason, owner, ownerName, recovery: RECOVERY[reason] }
}

/** Why `typeId` of the plugin module `module` cannot run; `null` when it can. */
function ofPluginType(module: ModuleInfo, typeId: string): MissingAction | null {
  const plugin = module.plugin
  if (!plugin) return null
  const name = module.name && module.name !== module.id ? module.name : null
  const state = plugin.state
  if (state === NOT_INSTALLED) return missing("notInstalled", module.id, name)
  if (state === "disabled") return missing("disabled", module.id, name)
  if (state === "broken") return missing("integrity", module.id, name)
  if (state === "quarantined") return missing("revoked", module.id, name)
  if (BUSY_STATES.has(state)) return missing("busy", module.id, name)
  if (!module.actionTypes.some((t) => t.id === typeId)) {
    return missing("incompatible", module.id, name)
  }
  if (plugin.unavailable[typeId] === undefined) return null
  // Waiting for a permission only when the plugin's grants say so.
  const grantable = (plugin.grantable?.[typeId]?.length ?? 0) > 0
  return missing(grantable ? "permission" : "busy", module.id, name)
}

/** The alias of a legacy action type, as the plugin modules carry them. */
function aliasOf(modules: ModuleInfo[], typeId: string): LegacyAlias | null {
  for (const module of modules) {
    const alias = module.plugin?.aliases?.find((a) => a.legacy === typeId)
    if (alias) return alias
  }
  return null
}

/**
 * Why an action of `typeId` that is already assigned cannot run now, or
 * `null` when it can (or nothing is known about it here). Legacy action
 * types of built-in modules that moved to a plugin follow the plugin's
 * state, as the agent routes them.
 */
export function missingAction(modules: ModuleInfo[], typeId: string): MissingAction | null {
  const builtin = modules.find(
    (m) => !m.plugin && !isPluginId(m.id) && m.actionTypes.some((t) => t.id === typeId),
  )
  if (pluginsEnabled()) {
    const alias = isPluginType(typeId) ? null : aliasOf(modules, typeId)
    if (alias && !(alias.route === "builtin" && builtin?.enabled)) {
      const plugin = modules.find((m) => m.plugin && m.id === alias.plugin)
      const name = alias.pluginName && alias.pluginName !== alias.plugin ? alias.pluginName : null
      if (!plugin) return missing("notInstalled", alias.plugin, name)
      const found = ofPluginType(plugin, alias.target)
      if (found) return found.ownerName ? found : { ...found, ownerName: name }
      // Withdrawn by the user although the plugin itself could run it.
      return alias.route === "pluginWithdrawn"
        ? missing("disabled", alias.plugin, plugin.name || name)
        : null
    }
    if (isPluginType(typeId)) {
      const owner = ownerOf(typeId) ?? ""
      const plugin = modules.find((m) => m.plugin && m.id === owner)
      return plugin ? ofPluginType(plugin, typeId) : missing("notInstalled", owner, null)
    }
  }
  if (builtin && !builtin.enabled) return missing("moduleDisabled", builtin.id, builtin.name)
  return null
}

/**
 * A signature of what the modules' action types can do now: their ids,
 * enabled states, plugin states, unavailable types and legacy routes. It
 * changes exactly when an assigned action may have become runnable or
 * not, so previews know to be drawn again.
 */
export function catalogSignature(modules: ModuleInfo[]): string {
  return modules
    .map((m) => {
      const plugin = m.plugin
      const types = m.actionTypes.map((t) => t.id).join(",")
      if (!plugin) return `${m.id}:${String(m.enabled)}:${types}`
      const unavailable = Object.keys(plugin.unavailable).sort().join(",")
      const routes = (plugin.aliases ?? []).map((a) => `${a.legacy}>${a.route}`).join(",")
      return `${m.id}:${plugin.state}:${types}:${unavailable}:${routes}:${plugin.route ?? ""}`
    })
    .join("|")
}
