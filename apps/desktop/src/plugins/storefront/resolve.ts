// From the agent's storefront view to what Discover shows (pure; storefront
// design §5 "Editorial cannot override trust data").
//
// Editorial supplies ids and copy only. Every card is built from the
// catalog's own `AvailablePlugin` (the signed index and detail): a
// placement never carries, changes or adds a name, publisher, verification,
// permission, version or badge. The agent already dropped placements of
// unknown, revoked or incompatible plugins; this checks again against the
// catalog shown, and drops what it cannot place:
//
// - a section of a kind this UI does not know (a later agent's);
// - a plugin id that is not in the catalog, or has no version that is
//   neither withdrawn nor incompatible here;
// - repeated ids (sections, items, plugins, collections, categories);
// - a banner whose target leads nowhere;
// - a section left with nothing to show.

import { isMediaSha256 } from "../storefront-api"
import {
  type BannerView,
  type ReleaseNoteView,
  type CollectionView,
  type MediaRef,
  type SectionView,
  STOREFRONT_MEDIA_TYPES,
  type StorefrontView,
  type TargetView,
} from "../storefront-types"
import type { AvailablePlugin } from "../types"

/** The design's caps, whatever the agent sends. */
export const CAPS = {
  sections: 12,
  banners: 6,
  rowPlugins: 24,
  categories: 12,
  collections: 16,
  recentMin: 4,
  recentMax: 24,
  recentDefault: 8,
} as const

/** The catalog by id (a `Map`, so ids like `__proto__` are plain keys). */
export type CatalogIndex = ReadonlyMap<string, AvailablePlugin>

export function catalogIndex(plugins: readonly AvailablePlugin[]): CatalogIndex {
  const index = new Map<string, AvailablePlugin>()
  for (const p of plugins) if (!index.has(p.id)) index.set(p.id, p)
  return index
}

/** Whether a plugin may be placed: some version is neither withdrawn nor incompatible here. */
export function placeable(plugin: AvailablePlugin): boolean {
  return (
    plugin.installed !== undefined ||
    plugin.versions.some((v) => !v.withdrawn && (v.incompatible ?? []).length === 0)
  )
}

/** The catalog entries of `ids`, in order, once each, placeable only, at most `max`. */
export function placedPlugins(ids: unknown, index: CatalogIndex, max: number): AvailablePlugin[] {
  if (!Array.isArray(ids)) return []
  const seen = new Set<string>()
  const out: AvailablePlugin[] = []
  for (const id of ids) {
    if (out.length >= max) break
    if (typeof id !== "string" || seen.has(id)) continue
    seen.add(id)
    const plugin = index.get(id)
    if (plugin && placeable(plugin)) out.push(plugin)
  }
  return out
}

/** A media reference the UI may request: a sha256, an allowed type and sane dimensions. */
export function validMediaRef(ref: unknown): ref is MediaRef {
  if (typeof ref !== "object" || ref === null) return false
  const r = ref as Partial<MediaRef>
  return (
    typeof r.sha256 === "string" &&
    isMediaSha256(r.sha256) &&
    STOREFRONT_MEDIA_TYPES.includes(r.mediaType as MediaRef["mediaType"]) &&
    Number.isFinite(r.width) &&
    Number.isFinite(r.height) &&
    (r.width ?? 0) > 0 &&
    (r.height ?? 0) > 0
  )
}

export interface ResolvedCollection {
  id: string
  title: string
  description?: string | undefined
  plugins: AvailablePlugin[]
}

/** The sidebar's editorial collections: known plugins only, none empty, once each. */
export function resolveCollections(
  view: StorefrontView | null,
  index: CatalogIndex,
): ResolvedCollection[] {
  if (!view || !Array.isArray(view.collections)) return []
  const seen = new Set<string>()
  const out: ResolvedCollection[] = []
  for (const c of view.collections as unknown[]) {
    if (out.length >= CAPS.collections) break
    if (typeof c !== "object" || c === null) continue
    const { id, title, description, plugins } = c as Partial<CollectionView>
    if (typeof id !== "string" || id === "" || typeof title !== "string" || seen.has(id)) continue
    seen.add(id)
    const members = placedPlugins(plugins, index, CAPS.rowPlugins)
    if (members.length === 0) continue
    out.push({
      id,
      title,
      description: typeof description === "string" ? description : undefined,
      plugins: members,
    })
  }
  return out
}

export type ResolvedBanner = Omit<BannerView, "target"> & {
  target: TargetView
}

export type ResolvedSection =
  | { kind: "bannerCarousel"; id: string; title?: string; items: ResolvedBanner[] }
  | {
      kind: "collectionRow"
      id: string
      layout: "cards" | "compact"
      title: string
      description?: string
      plugins: AvailablePlugin[]
    }
  | {
      kind: "spotlight"
      id: string
      title?: string
      description?: string
      plugin: AvailablePlugin
      /** Whose release notes to show: only a published, not withdrawn version. */
      version?: string
    }
  | { kind: "recentlyAdded"; id: string; title?: string; limit: number }
  | {
      kind: "categoryHighlights"
      id: string
      title?: string
      categories: { id: string; blurb?: string }[]
    }
  | { kind: "catalog"; id: string; title?: string }

const str = (value: unknown): string | undefined =>
  typeof value === "string" && value.trim() !== "" ? value : undefined

/** Copies only the optional text fields that are present (exactOptionalPropertyTypes). */
function texts<K extends string>(
  section: Partial<Record<K, unknown>>,
  keys: readonly K[],
): Partial<Record<K, string>> {
  const out: Partial<Record<K, string>> = {}
  for (const key of keys) {
    const value = str(section[key])
    if (value !== undefined) out[key] = value
  }
  return out
}

function validTarget(
  target: unknown,
  index: CatalogIndex,
  collections: ReadonlySet<string>,
  categories: ReadonlySet<string>,
): target is TargetView {
  if (typeof target !== "object" || target === null) return false
  const { type, id } = target as Partial<TargetView>
  if (typeof id !== "string") return false
  if (type === "plugin") {
    const plugin = index.get(id)
    return plugin !== undefined && placeable(plugin)
  }
  if (type === "collection") return collections.has(id)
  if (type === "category") return categories.has(id)
  return false
}

/** The categories the catalog uses. */
export function catalogCategories(index: CatalogIndex): Set<string> {
  const set = new Set<string>()
  for (const p of index.values()) for (const c of p.categories ?? []) set.add(c)
  return set
}

/**
 * Discover's sections, in the agent's order, of the kinds this UI knows,
 * each with only what can be placed. An empty result means catalog-only.
 */
export function resolveSections(
  view: StorefrontView | null,
  index: CatalogIndex,
  collections: readonly ResolvedCollection[] = resolveCollections(view, index),
): ResolvedSection[] {
  if (!view || !Array.isArray(view.sections)) return []
  const collectionIds = new Set(collections.map((c) => c.id))
  const categories = catalogCategories(index)
  const seen = new Set<string>()
  const out: ResolvedSection[] = []
  for (const raw of view.sections as unknown[]) {
    if (out.length >= CAPS.sections) break
    if (typeof raw !== "object" || raw === null) continue
    const section = raw as Partial<SectionView>
    const id = section.id
    if (typeof id !== "string" || id === "" || seen.has(id)) continue
    const resolved = resolveSection(id, section, index, collectionIds, categories)
    if (!resolved) continue
    seen.add(id)
    out.push(resolved)
  }
  return out
}

function resolveSection(
  id: string,
  section: Partial<SectionView>,
  index: CatalogIndex,
  collectionIds: ReadonlySet<string>,
  categories: ReadonlySet<string>,
): ResolvedSection | null {
  switch (section.kind) {
    case "bannerCarousel": {
      const seen = new Set<string>()
      const items: ResolvedBanner[] = []
      for (const item of Array.isArray(section.items) ? (section.items as unknown[]) : []) {
        if (items.length >= CAPS.banners) break
        if (typeof item !== "object" || item === null) continue
        const b = item as Partial<BannerView>
        if (typeof b.id !== "string" || seen.has(b.id) || typeof b.title !== "string") continue
        if (!validMediaRef(b.artwork)) continue
        if (!validTarget(b.target, index, collectionIds, categories)) continue
        seen.add(b.id)
        items.push({
          id: b.id,
          title: b.title,
          ...texts(b, ["body", "cta"] as const),
          artwork: b.artwork,
          alt: typeof b.alt === "string" ? b.alt : "",
          ...(b.decorative === true ? { decorative: true } : {}),
          target: { type: b.target.type, id: b.target.id },
        })
      }
      return items.length === 0
        ? null
        : { kind: "bannerCarousel", id, ...texts(section, ["title"] as const), items }
    }
    case "collectionRow": {
      const plugins = placedPlugins(section.plugins, index, CAPS.rowPlugins)
      const title = str(section.title)
      if (plugins.length === 0 || title === undefined) return null
      return {
        kind: "collectionRow",
        id,
        layout: section.layout === "compact" ? "compact" : "cards",
        title,
        ...texts(section, ["description"] as const),
        plugins,
      }
    }
    case "spotlight": {
      const [plugin] = placedPlugins(section.plugins, index, 1)
      if (!plugin) return null
      const version = str(section.version)
      const published =
        version !== undefined && plugin.versions.some((v) => v.version === version && !v.withdrawn)
      return {
        kind: "spotlight",
        id,
        ...texts(section, ["title", "description"] as const),
        plugin,
        ...(published ? { version } : {}),
      }
    }
    case "recentlyAdded": {
      const limit =
        typeof section.limit === "number" && Number.isFinite(section.limit)
          ? Math.min(CAPS.recentMax, Math.max(CAPS.recentMin, Math.floor(section.limit)))
          : CAPS.recentDefault
      return { kind: "recentlyAdded", id, ...texts(section, ["title"] as const), limit }
    }
    case "categoryHighlights": {
      const seen = new Set<string>()
      const list: { id: string; blurb?: string }[] = []
      for (const c of Array.isArray(section.categories) ? (section.categories as unknown[]) : []) {
        if (list.length >= CAPS.categories) break
        if (typeof c !== "object" || c === null) continue
        const { id: cat, blurb } = c as { id?: unknown; blurb?: unknown }
        if (typeof cat !== "string" || seen.has(cat) || !categories.has(cat)) continue
        seen.add(cat)
        const text = str(blurb)
        list.push(text === undefined ? { id: cat } : { id: cat, blurb: text })
      }
      return list.length === 0
        ? null
        : {
            kind: "categoryHighlights",
            id,
            ...texts(section, ["title"] as const),
            categories: list,
          }
    }
    case "catalog":
      return { kind: "catalog", id, ...texts(section, ["title"] as const) }
    default:
      // A kind a later agent knows and this UI does not: skipped.
      return null
  }
}

/** The sidebar destination of an editorial collection. */
export type EditorialSectionId = `editorial:${string}`

export function editorialSection(id: string): EditorialSectionId {
  return `editorial:${id}`
}

export function isEditorialSection(section: string): section is EditorialSectionId {
  return section.startsWith("editorial:")
}

export function editorialCollectionId(section: EditorialSectionId): string {
  return section.slice("editorial:".length)
}

/** The listing's note for `version`, if it has one. */
export function noteFor(
  notes: readonly ReleaseNoteView[] | undefined,
  version: string | undefined,
): ReleaseNoteView | undefined {
  if (!version || !notes) return undefined
  return notes.find((n) => n.version === version && typeof n.text === "string" && n.text !== "")
}
