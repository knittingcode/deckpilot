// The catalog of the layout editor's library: the action types of the
// enabled modules and plugins, put on a control directly. Dropping one
// creates the action with the type's default settings (or reuses an action
// that already is exactly that) and binds it; what it still needs is set in
// the Inspector, which says so.

import type { ActionInput } from "@/lib/api"
import { missingParameters } from "@/lib/action-completeness"
import { creatableTypes } from "@/lib/modules"
import type { ActionPreset } from "./editing"
import type {
  ActionInstance,
  ActionParameters,
  ActionType,
  ModuleInfo,
  ParameterValue,
} from "@/lib/types"
import { actionTypeDescription, actionTypeName, newActionIcon } from "@/lib/system-actions"

export interface CatalogEntry {
  type: ActionType
  /** The module or plugin that provides it. */
  module: string
  /** Why a plugin's type cannot be placed now (e.g. a permission it lacks). */
  unavailable?: string
  /**
   * With `unavailable`, when all it lacks are permissions the user can
   * grant: the "Grant" command of its row.
   */
  grant?: { label: string; pending: boolean; onGrant: () => void }
}

export interface CatalogGroup {
  /** The types' category, e.g. "Navigation". */
  label: string
  entries: CatalogEntry[]
}

/** The id of a catalog entry among the editor's draggables. */
export function catalogDragId(actionType: string): string {
  return `library:type:${actionType}`
}

/**
 * The types the library offers for dropping on a control, by category:
 * those of enabled, supported modules and plugins that are not deprecated
 * (a legacy id routed to a plugin is deprecated: only the plugin's type is
 * listed, once). Types that need a value are listed too; the Inspector asks
 * for it. Categories and types keep the order of the modules.
 */
export function catalogGroups(modules: ModuleInfo[]): CatalogGroup[] {
  const groups = new Map<string, CatalogEntry[]>()
  const seen = new Set<string>()
  for (const module of modules) {
    if (!module.enabled || !module.supported) continue
    for (const type of creatableTypes(module.actionTypes)) {
      // Once, whoever provides it.
      if (seen.has(type.id)) continue
      seen.add(type.id)
      const unavailable = module.plugin?.unavailable[type.id]
      const entries = groups.get(type.category) ?? []
      entries.push({ type, module: module.name, ...(unavailable ? { unavailable } : {}) })
      groups.set(type.category, entries)
    }
  }
  return [...groups].map(([label, entries]) => ({ label, entries }))
}

/** The settings of a new action of the type: its declared defaults. */
export function defaultParameters(type: ActionType): ActionParameters {
  const parameters: ActionParameters = {}
  for (const spec of type.parameters) {
    if (spec.default !== undefined) parameters[spec.key] = spec.default
  }
  return parameters
}

function sameValue(a: ParameterValue | undefined, b: ParameterValue | undefined): boolean {
  return JSON.stringify(a ?? null) === JSON.stringify(b ?? null)
}

/**
 * An action of the profile that is exactly a new action of the type would
 * be: the same type, the default settings and no label of its own. It is
 * reused, so dropping "Next page" on five keys leaves one action in the
 * Action Library, not five. An action that still needs configuration
 * (a required parameter has no value) is never reused: each drop gets
 * its own, so configuring one key does not change another.
 */
export function existingDefault(
  actions: ActionInstance[],
  type: ActionType,
): ActionInstance | undefined {
  const defaults = defaultParameters(type)
  if (missingParameters(type, defaults).length > 0) return undefined
  const keys = type.parameters.map((p) => p.key)
  return actions.find(
    (action) =>
      action.actionType === type.id &&
      !action.label &&
      missingParameters(type, action.parameters).length === 0 &&
      keys.every((key) => sameValue(action.parameters[key], defaults[key])),
  )
}

/** A name no action of the profile has: the type's, then "… 2", "… 3". */
export function freeActionName(actions: ActionInstance[], name: string): string {
  const taken = new Set(actions.map((a) => a.name.trim().toLowerCase()))
  if (!taken.has(name.toLowerCase())) return name
  let n = 2
  while (taken.has(`${name} ${n}`.toLowerCase())) n += 1
  return `${name} ${n}`
}

/** What to create for a type dropped on a control. */
export function newAction(actions: ActionInstance[], type: ActionType): ActionInput {
  return {
    name: freeActionName(actions, actionTypeName(type)),
    actionType: type.id,
    parameters: defaultParameters(type),
    description: "",
    icon: newActionIcon(type),
    label: null,
  }
}

/** Entries whose name, description, category or module contain `query`. */
export function filterCatalog(
  groups: CatalogGroup[],
  query: string,
  /** The category's shown name, found too (e.g. "Nawigacja" for "Navigation"). */
  categoryLabel: (category: string) => string = (category) => category,
): CatalogGroup[] {
  const needle = query.trim().toLowerCase()
  if (needle === "") return groups
  return groups
    .map((group) => ({
      ...group,
      entries: group.entries.filter((entry) =>
        `${entry.type.name} ${actionTypeName(entry.type)} ${actionTypeDescription(entry.type)} ${group.label} ${categoryLabel(group.label)} ${entry.module}`
          .toLowerCase()
          .includes(needle),
      ),
    }))
    .filter((group) => group.entries.length > 0)
}

/** The settings of a preset over the type's defaults; empty texts are left out. */
function presetParameters(type: ActionType, preset: ActionPreset): ActionParameters {
  return Object.fromEntries(
    Object.entries({ ...defaultParameters(type), ...preset.parameters }).filter(
      ([, value]) => !(value.type === "text" && value.value === ""),
    ),
  )
}

/**
 * An action of the profile that is exactly what `preset` makes: the same
 * type and settings and no label of its own. Reused like `existingDefault`
 * (never when it needs configuration).
 */
export function existingPreset(
  actions: ActionInstance[],
  type: ActionType,
  preset: ActionPreset,
): ActionInstance | undefined {
  const wanted = presetParameters(type, preset)
  // An incomplete preset is a draft of its own on every drop, like a type.
  if (missingParameters(type, wanted).length > 0) return undefined
  const keys = type.parameters.map((p) => p.key)
  const empty = (value: ParameterValue | undefined) =>
    value === undefined || (value.type === "text" && value.value === "") ? undefined : value
  return actions.find(
    (action) =>
      action.actionType === type.id &&
      !action.label &&
      missingParameters(type, action.parameters).length === 0 &&
      keys.every((key) => sameValue(empty(action.parameters[key]), wanted[key])),
  )
}

/** What to create for a preset dropped on a control. */
export function newPresetAction(
  actions: ActionInstance[],
  type: ActionType,
  preset: ActionPreset,
): ActionInput {
  return {
    name: freeActionName(actions, preset.name),
    actionType: type.id,
    parameters: presetParameters(type, preset),
    description: "",
    icon: preset.icon ?? newActionIcon(type),
    label: null,
  }
}

/** The id of a library entry that puts `preset` (named by `key`) on a control. */
export function presetDragId(key: string): string {
  return `library:preset:${key}`
}
