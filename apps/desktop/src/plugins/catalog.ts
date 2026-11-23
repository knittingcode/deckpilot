// What the Marketplace home and a plugin's page show, from the agent's data
// alone (pure; no requests). Nothing here invents data: the registry has no
// ratings, download counts or popularity, so there is no "Popular" or
// "Featured" list; "Recently added" orders by the publication dates the
// registry signs.

import { type TranslationKey, i18n } from "@/i18n"

import type { PluginErrorText } from "./api"
import { hasStorefrontFeature } from "./storefront-api"
import type { EditorialSectionId } from "./storefront/resolve"
import type { MarketplaceActions } from "./marketplace"
import {
  categoryLabel,
  compareSemver,
  declaredConfiguration,
  offeredVersion,
  type UpdateCandidate,
} from "./model"
import type {
  AvailablePlugin,
  AvailableVersion,
  KnownPlugin,
  MarketplaceSnapshot,
  ModuleMigration,
  PluginDetails,
  PluginNavigationEntry,
  PluginRuntimeStatus,
} from "./types"

/**
 * DeckPilot's privileged plugins: they reach users only as the copies
 * bundled with DeckPilot (the store refuses their kinds from any other
 * source), so the Marketplace never offers to install them.
 *
 * FALLBACK ONLY, for agents without the `storefront` feature: an agent
 * with it derives `AvailablePlugin.bundledOnly` itself (sent only when
 * true), and then this list is not consulted, so it cannot drift from
 * what the agent knows. Remove it once no supported agent lacks the
 * feature.
 */
export const BUNDLED_ONLY_PLUGINS: ReadonlySet<string> = new Set([
  "deckpilot.docker",
  "deckpilot.git",
  "deckpilot.projects",
  "deckpilot.workflows",
  "deckpilot.workspace",
])

/**
 * Whether the agent derives the `official` and `bundledOnly` flags itself
 * (feature `storefront`): then they are the only source, and their
 * absence means false.
 */
export function agentDerivesBadges(
  runtime: Pick<PluginRuntimeStatus, "features"> | null | undefined,
): boolean {
  return hasStorefrontFeature(runtime)
}

/**
 * Whether a plugin is only ever installed as DeckPilot's bundled copy: as
 * the agent says, or (an agent that does not say) by the fallback list.
 */
export function isBundledOnly(
  plugin: Pick<AvailablePlugin, "id" | "bundledOnly">,
  runtime?: Pick<PluginRuntimeStatus, "features"> | null,
): boolean {
  if (plugin.bundledOnly === true) return true
  return !agentDerivesBadges(runtime) && BUNDLED_ONLY_PLUGINS.has(plugin.id)
}

/**
 * The ids of DeckPilot's official plugins as far as this computer knows:
 * those the agent flags (`official`, derived by the agent, never by
 * editorial), those installed from DeckPilot's own bundle, the bundled
 * copies the agent can restore, and the plugins DeckPilot's built-in
 * modules move to; for an agent that does not derive the flags, also the
 * fallback `BUNDLED_ONLY_PLUGINS`. Never a guess from a name.
 */
export function officialIds(
  snapshot: MarketplaceSnapshot,
  migrations: ModuleMigration[] = [],
): Set<string> {
  return new Set([
    ...snapshot.available.filter((p) => p.official === true).map((p) => p.id),
    ...snapshot.installed
      .filter((p) => p.summary.origin.type === "bundled")
      .map((p) => p.summary.id),
    ...(snapshot.known ?? []).filter((k) => k.restorable === true).map((k) => k.id),
    ...migrations.map((m) => m.plugin),
    ...(agentDerivesBadges(snapshot.runtime) ? [] : BUNDLED_ONLY_PLUGINS),
  ])
}

/** A request about one plugin that failed, kept until it is retried or succeeds. */
export interface PluginFailure {
  operation: "install" | "update"
  error: PluginErrorText
}

/** What the Marketplace may do with one catalog entry right now. */
export type EntryState =
  | { kind: "install"; version: AvailableVersion }
  | { kind: "installing" }
  | { kind: "update"; candidate: UpdateCandidate }
  | { kind: "updating" }
  /** Installed and enabled, with a toolbar view to open. */
  | { kind: "open"; entry: PluginNavigationEntry }
  /** Installed and enabled, with settings or credentials and no view. */
  | { kind: "configure" }
  /** Installed, turned off: enabling asks for consent first. */
  | { kind: "enable" }
  | { kind: "installed" }
  /** Broken or quarantined: the Library says why and what to do. */
  | { kind: "attention" }
  /** No version runs on this DeckPilot or platform. */
  | { kind: "incompatible"; version: AvailableVersion | undefined }
  | { kind: "withdrawn"; reason: string }
  /** Comes with DeckPilot; never installed from the Marketplace. */
  | { kind: "bundled"; restorable: boolean }
  | { kind: "failed"; failure: PluginFailure }

export interface EntryContext {
  installed: PluginDetails | undefined
  update: UpdateCandidate | undefined
  /** A request of this page in flight for the plugin. */
  pending: "install" | "update" | undefined
  failure: PluginFailure | undefined
  /** The agent can restore a removed bundled copy (`restoreBundled`). */
  restorable: boolean
  /** Plugins can be enabled and run here. */
  canRun: boolean
  /** Only ever DeckPilot's bundled copy (`isBundledOnly`); by default from the plugin alone. */
  bundledOnly?: boolean
}

/** The newest version the registry lists, installable or not. */
export function newestVersion(plugin: AvailablePlugin): AvailableVersion | undefined {
  return [...plugin.versions].sort((a, b) => compareSemver(b.version, a.version))[0]
}

/** The version a plugin's page describes: the one offered, else the newest. */
export function shownVersion(plugin: AvailablePlugin): AvailableVersion | undefined {
  return offeredVersion(plugin) ?? newestVersion(plugin)
}

export function entryState(plugin: AvailablePlugin, ctx: EntryContext): EntryState {
  const { installed } = ctx
  if (ctx.failure) return { kind: "failed", failure: ctx.failure }
  if (ctx.pending === "install") return { kind: "installing" }
  if (ctx.pending === "update") return { kind: "updating" }
  if (installed) {
    const state = installed.summary.state
    if (state === "installing") return { kind: "installing" }
    if (state === "updating" || state === "rollingBack") return { kind: "updating" }
    if (state === "broken" || state === "quarantined") return { kind: "attention" }
    if (ctx.update) return { kind: "update", candidate: ctx.update }
    if (state === "disabled") return ctx.canRun ? { kind: "enable" } : { kind: "installed" }
    if (state !== "enabled") return { kind: "installed" }
    const entry = installed.summary.navigation?.[0]
    if (entry) return { kind: "open", entry }
    const declared = declaredConfiguration(installed.manifest)
    if (declared.settings.length > 0 || declared.credentials.length > 0) {
      return { kind: "configure" }
    }
    return { kind: "installed" }
  }
  if (ctx.bundledOnly ?? isBundledOnly(plugin)) {
    return { kind: "bundled", restorable: ctx.restorable }
  }
  const offered = offeredVersion(plugin)
  if (offered) return { kind: "install", version: offered }
  const newest = newestVersion(plugin)
  if (newest?.withdrawn) return { kind: "withdrawn", reason: newest.withdrawn }
  return { kind: "incompatible", version: newest }
}

/** When the registry first published the plugin (its oldest listed version). */
export function firstPublished(plugin: AvailablePlugin): number | undefined {
  const dates = plugin.versions
    .map((v) => v.publishedAt)
    .filter((d): d is number => typeof d === "number")
  return dates.length > 0 ? Math.min(...dates) : undefined
}

/** When the registry last published a version of the plugin. */
export function lastPublished(plugin: AvailablePlugin): number | undefined {
  const dates = plugin.versions
    .map((v) => v.publishedAt)
    .filter((d): d is number => typeof d === "number")
  return dates.length > 0 ? Math.max(...dates) : undefined
}

/**
 * Plugins newest first by the date the registry first published them;
 * plugins without a date follow, by name.
 */
export function byRecency(plugins: AvailablePlugin[]): AvailablePlugin[] {
  return [...plugins].sort((a, b) => {
    const x = firstPublished(a)
    const y = firstPublished(b)
    if (x !== undefined && y !== undefined && x !== y) return y - x
    if (x === undefined && y !== undefined) return 1
    if (y === undefined && x !== undefined) return -1
    return a.name.localeCompare(b.name)
  })
}

/** What the catalog is narrowed to; kept by the Marketplace across a plugin's page. */
export interface CatalogFilter {
  query: string
  category: string | null
}

/** Whether the catalog is narrowed at all. */
export function isFiltering(filter: CatalogFilter): boolean {
  return filter.query.trim() !== "" || filter.category !== null
}

// ---------------------------------------------------------------- categories

/** The manifest's categories (`plugin-manifest.schema.json`), in words. */
const CATEGORY_LABELS = {
  "developer-tools": "marketplace:category.developerTools",
  productivity: "marketplace:category.productivity",
  communication: "marketplace:category.communication",
  monitoring: "marketplace:category.monitoring",
  media: "marketplace:category.media",
  "smart-home": "marketplace:category.smartHome",
  other: "marketplace:category.other",
} as const satisfies Record<string, TranslationKey>

/** A category in the window's language; one the app does not know, from its id. */
export function categoryName(category: string): string {
  const key = (CATEGORY_LABELS as Record<string, TranslationKey | undefined>)[category]
  return key ? i18n.t(key) : categoryLabel(category)
}

/** Every category of the catalog with how many plugins it has, by name. */
export function categoryCounts(plugins: AvailablePlugin[]): { id: string; count: number }[] {
  const counts = new Map<string, number>()
  for (const p of plugins) {
    for (const c of p.categories ?? []) counts.set(c, (counts.get(c) ?? 0) + 1)
  }
  return [...counts]
    .map(([id, count]) => ({ id, count }))
    .sort((a, b) => categoryName(a.id).localeCompare(categoryName(b.id)))
}

/** The first category of a plugin, if it names one. */
export function primaryCategory(plugin: Pick<AvailablePlugin, "categories">): string | undefined {
  return plugin.categories?.[0]
}

// --------------------------------------------------------------- collections

/**
 * The Marketplace's built-in collections and which registry categories
 * belong to each: the one place their membership is defined. They are
 * the sidebar's fallback: a signed storefront's collections
 * (`StorefrontView.collections`) replace them. The categories are the manifest schema's (`plugin-manifest.schema.json`):
 * `developer-tools`, `productivity`, `communication`, `monitoring`, `media`,
 * `smart-home`, `other`. `other` belongs to no collection: it is whatever
 * fits nowhere else (today the Memory game), so it never decides one.
 */
export const COLLECTIONS = {
  code: { categories: ["developer-tools"] },
  play: { categories: ["media"] },
  work: { categories: ["productivity", "communication"] },
  automate: { categories: ["smart-home"] },
  monitor: { categories: ["monitoring"] },
} as const satisfies Record<string, { categories: readonly string[] }>

export type CollectionId = keyof typeof COLLECTIONS

export const COLLECTION_IDS = Object.keys(COLLECTIONS) as CollectionId[]

/** Whether a plugin is in a built-in collection, by its categories. */
export function inCollection(plugin: AvailablePlugin, id: CollectionId): boolean {
  const categories: readonly string[] = COLLECTIONS[id].categories
  return (plugin.categories ?? []).some((c) => categories.includes(c))
}

/** The registry's plugins in a collection, by name. */
export function collectionPlugins(plugins: AvailablePlugin[], id: CollectionId): AvailablePlugin[] {
  return plugins.filter((p) => inCollection(p, id)).sort((a, b) => a.name.localeCompare(b.name))
}

/**
 * Where the Marketplace's sidebar leads: its own sections, a built-in
 * collection, or a storefront collection (`editorial:<id>`).
 */
export type MarketplaceSection =
  "discover" | "installed" | "updates" | "categories" | CollectionId | EditorialSectionId

// -------------------------------------------------------------------- search

/** Entries whose name, id, summary, description, publisher or category contain every word. */
export function matchesSearch(plugin: AvailablePlugin, query: string): boolean {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean)
  if (words.length === 0) return true
  const text = [
    plugin.name,
    plugin.id,
    plugin.summary,
    plugin.description,
    plugin.publisher,
    plugin.publisherInfo?.displayName,
    ...(plugin.categories ?? []).flatMap((c) => [c, categoryName(c)]),
  ]
    .filter(Boolean)
    .join(" ")
    .toLowerCase()
  return words.every((word) => text.includes(word))
}

/** The name people know the developer by. */
export function developerName(
  plugin: Pick<AvailablePlugin, "publisher" | "publisherInfo">,
): string {
  return plugin.publisherInfo?.displayName ?? plugin.publisher
}

/**
 * A plugin name as people should see it. A manifest may name its plugin
 * with a key of its own locale files (`t:plugin.name`); the agent resolves
 * those for installed plugins, so the installed name wins, then the
 * registry's, then the id: never the raw key.
 */
export function pluginDisplayName(
  id: string,
  name: string | undefined,
  snapshot: Pick<MarketplaceSnapshot, "installed" | "available" | "known">,
): string {
  const usable = (text: string | undefined) => (text && !text.startsWith("t:") ? text : undefined)
  return (
    usable(name) ??
    usable(snapshot.installed.find((p) => p.summary.id === id)?.summary.name) ??
    usable(snapshot.available.find((p) => p.id === id)?.name) ??
    usable(snapshot.known?.find((k: KnownPlugin) => k.id === id)?.name) ??
    id
  )
}

/** What a catalog entry's state is, from the snapshot and this page's requests. */
export function entryContextOf(
  plugin: AvailablePlugin,
  snapshot: MarketplaceSnapshot,
  updates: UpdateCandidate[],
  actions: MarketplaceActions,
): EntryContext {
  return {
    installed: snapshot.installed.find((p) => p.summary.id === plugin.id),
    update: updates.find((u) => u.plugin.summary.id === plugin.id),
    pending: actions.pending.get(plugin.id),
    failure: actions.failures.get(plugin.id),
    restorable:
      actions.canRestore &&
      (snapshot.known ?? []).some((k) => k.id === plugin.id && k.restorable === true),
    canRun: actions.canRun,
    bundledOnly: isBundledOnly(plugin, snapshot.runtime),
  }
}

export function useEntry(
  plugin: AvailablePlugin,
  snapshot: MarketplaceSnapshot,
  updates: UpdateCandidate[],
  actions: MarketplaceActions,
): { state: EntryState; context: EntryContext } {
  const context = entryContextOf(plugin, snapshot, updates, actions)
  return { state: entryState(plugin, context), context }
}

function record(value: unknown): Record<string, unknown> | undefined {
  return typeof value === "object" && value !== null
    ? (value as Record<string, unknown>)
    : undefined
}

function strings(value: unknown): string[] | undefined {
  return Array.isArray(value) ? value.filter((v): v is string => typeof v === "string") : undefined
}

/**
 * The catalog entry of an installed plugin the registry does not list
 * (DeckPilot's bundled plugins, a local package, a development folder),
 * from its verified manifest, so every installed plugin has a page. The
 * permissions are the manifest's; the agent's grants are on the page too.
 */
export function listingOf(item: PluginDetails): AvailablePlugin {
  const manifest = record(item.manifest)
  const engines = record(manifest?.engines)
  const repository = record(manifest?.repository)
  const repo =
    typeof repository?.url === "string"
      ? /^https:\/\/github\.com\/([^/]+\/[^/]+)$/.exec(repository.url)?.[1]
      : undefined
  const permissions = Array.isArray(manifest?.permissions)
    ? (manifest.permissions as AvailablePlugin["versions"][number]["permissions"])
    : []
  const platforms = strings(manifest?.platforms)
  const categories = strings(manifest?.categories)
  const summary = item.description || item.summary.name
  return {
    id: item.summary.id,
    name: item.summary.name,
    summary,
    description: item.description || summary,
    publisher: item.summary.publisher,
    installed: item.summary.version,
    versions: [
      {
        version: item.summary.version,
        permissions,
        ...(typeof engines?.deckpilot === "string" ? { deckpilot: engines.deckpilot } : {}),
        ...(platforms ? { platforms } : {}),
      },
    ],
    ...(item.license ? { license: item.license } : {}),
    ...(item.homepage ? { homepage: item.homepage } : {}),
    ...(repo ? { sourceRepository: repo } : {}),
    ...(categories && categories.length > 0 ? { categories } : {}),
    // No `icon`: an installed plugin's own icon (`item.icon`) is shown from
    // `PluginDetails`; `AvailablePlugin.icon` is a storefront image.
  }
}
