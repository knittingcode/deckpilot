// The device editor's library by context (lib/action-context.ts): it offers
// what can go on a key (its press, its display or a gesture), and lists the
// types that only show something on the key apart, under "Displays". What
// runs only as a Multi Action step or a workflow node is not offered here
// at all. Types without declared contexts go everywhere, as before.

import { isDisplayOnly, placeableOnKey } from "@/lib/action-context"
import type { ActionType } from "@/lib/types"

import type { CatalogEntry, CatalogGroup } from "./catalog"
import type { PresetEntry } from "./library-panel"
import type { TargetGroup, TargetOption } from "./targets"

export interface KeyLibrary {
  groups: TargetGroup[]
  catalog: CatalogGroup[]
  presets: PresetEntry[]
  /** Display-only actions and types, shown in their own section. */
  displays: { options: TargetOption[]; catalog: CatalogEntry[] }
}

export function keyLibrary(
  groups: TargetGroup[],
  catalog: CatalogGroup[],
  presets: PresetEntry[],
  typeOf: (actionType: string) => ActionType | undefined,
): KeyLibrary {
  const displays: KeyLibrary["displays"] = { options: [], catalog: [] }
  const shownGroups = groups.map((group) => ({
    ...group,
    options: group.options.filter((option) => {
      const type = option.actionType ? typeOf(option.actionType) : undefined
      if (!type) return true
      if (!placeableOnKey(type)) return false
      if (isDisplayOnly(type)) {
        displays.options.push(option)
        return false
      }
      return true
    }),
  }))
  const shownCatalog = catalog
    .map((group) => ({
      ...group,
      entries: group.entries.filter((entry) => {
        if (!placeableOnKey(entry.type)) return false
        if (isDisplayOnly(entry.type)) {
          displays.catalog.push(entry)
          return false
        }
        return true
      }),
    }))
    .filter((group) => group.entries.length > 0)
  return {
    groups: shownGroups,
    catalog: shownCatalog,
    presets: presets.filter((p) => placeableOnKey(p.type)),
    displays,
  }
}
