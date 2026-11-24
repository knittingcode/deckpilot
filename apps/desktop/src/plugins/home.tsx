// The Marketplace's catalog sections: "Discover plugins" (heading, the
// search/install-from-file/refresh row and the categories; the matching
// plugins right below while searching or filtering) and "Recently added"
// (every registry plugin not installed here, newest first: the browsable
// catalog). Nothing is ranked by popularity, which the registry does not
// know. Installed plugins are managed in "Your plugins", which search and
// categories never hide.

import { ArrowRight, X } from "lucide-react"
import type { ReactNode } from "react"

import { EmptyState } from "@/components/page"
import { Button } from "@/components/ui/button"
import { useTranslation } from "@/i18n"
import { useFormat } from "@/i18n/format"

import {
  byRecency,
  COLLECTION_IDS,
  COLLECTIONS,
  type CollectionId,
  categoryCounts,
  categoryName,
  collectionPlugins,
  type MarketplaceSection,
  pluginDisplayName,
  primaryCategory,
  firstPublished,
  matchesSearch,
  officialIds,
} from "./catalog"
import type { MarketplaceActions } from "./marketplace"
import { formatDate, type UpdateCandidate } from "./model"
import { COLLECTION_LOOK } from "./collection-look"
import { PluginAvatar, PluginTile, TileGrid } from "./plugin-tile"
import { EditorialCollection } from "./storefront/editorial-collection"
import { type EditorialSectionId, isEditorialSection } from "./storefront/resolve"
import type { AvailablePlugin, MarketplaceSnapshot, ModuleMigration } from "./types"

/** A titled section of the Marketplace page. */
export function PageSection({
  id,
  title,
  description,
  children,
}: {
  id: string
  title: string
  description?: string
  children: ReactNode
}) {
  return (
    <section aria-labelledby={`${id}-title`} className="grid gap-3">
      <div className="grid gap-0.5">
        <h3 id={`${id}-title`} className="scroll-mt-4 text-base font-semibold">
          {title}
        </h3>
        {description && <p className="text-xs text-muted-foreground">{description}</p>}
      </div>
      {children}
    </section>
  )
}

export function DiscoverSection({
  snapshot,
  updates,
  migrations,
  actions,
  category,
  onCategory,
}: {
  snapshot: MarketplaceSnapshot
  updates: UpdateCandidate[]
  migrations: ModuleMigration[]
  actions: MarketplaceActions
  /** The category the catalog is narrowed to; `null` for all. */
  category: string | null
  onCategory: (category: string | null) => void
}) {
  const { t } = useTranslation()
  const format = useFormat()
  const plugins = snapshot.available
  const categories = categoryCounts(plugins)
  const shown = plugins
    .filter((p) => category === null || (p.categories ?? []).includes(category))
    .sort((a, b) => format.compare(a.name, b.name))
  const scope = category === null ? null : categoryName(category)

  return (
    <section aria-labelledby="marketplace-discover-title" className="grid gap-4">
      <div className="grid gap-1">
        <h2 id="marketplace-discover-title" className="text-xl font-semibold tracking-tight">
          {t("marketplace:home.title")}
        </h2>
        <p className="text-sm text-muted-foreground">{t("marketplace:home.subtitle")}</p>
      </div>
      {categories.length > 0 && (
        <div
          role="group"
          aria-label={t("marketplace:discover.categories")}
          className="flex flex-wrap gap-1.5"
        >
          {[null, ...categories].map((c) => {
            const pressed = (c?.id ?? null) === category
            return (
              <Button
                key={c?.id ?? "all"}
                variant={pressed ? "secondary" : "outline"}
                size="sm"
                className="h-7 rounded-full px-3 text-xs"
                aria-pressed={pressed}
                onClick={() => {
                  onCategory(c?.id ?? null)
                }}
              >
                {c === null ? t("marketplace:discover.allCategories") : categoryName(c.id)}
                {c !== null && (
                  <span className="text-muted-foreground tabular-nums">{c.count}</span>
                )}
              </Button>
            )
          })}
        </div>
      )}

      {plugins.length === 0 ? (
        <EmptyState variant="plain" title={t("marketplace:discover.empty.title")}>
          {snapshot.registryError
            ? t("marketplace:discover.empty.registryError")
            : !snapshot.registry
              ? t("marketplace:discover.empty.noRegistry")
              : snapshot.registry.freshness === "neverVerified"
                ? t("marketplace:discover.empty.neverVerified")
                : t("marketplace:discover.empty.none")}
        </EmptyState>
      ) : (
        scope !== null && (
          <div className="grid gap-3">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <p className="text-sm text-muted-foreground" role="status">
                {t("marketplace:home.resultsIn", { count: shown.length, scope })}
              </p>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => {
                  onCategory(null)
                }}
              >
                <X /> {t("marketplace:home.clear")}
              </Button>
            </div>
            <Tiles
              label={t("marketplace:discover.list")}
              plugins={shown}
              snapshot={snapshot}
              updates={updates}
              migrations={migrations}
              actions={actions}
            />
          </div>
        )
      )}
    </section>
  )
}

/**
 * What the sidebar's search finds in the whole catalog (names, summaries,
 * descriptions, publishers, categories), as cards with each plugin's state.
 */
export function SearchResults({
  query,
  snapshot,
  updates,
  migrations,
  actions,
}: {
  query: string
  snapshot: MarketplaceSnapshot
  updates: UpdateCandidate[]
  migrations: ModuleMigration[]
  actions: MarketplaceActions
}) {
  const { t } = useTranslation()
  const format = useFormat()
  const found = snapshot.available
    .filter((p) => matchesSearch(p, query))
    .sort((a, b) => format.compare(a.name, b.name))
  return (
    <section aria-labelledby="marketplace-results-title" className="grid gap-4">
      <div className="grid gap-1">
        <h2 id="marketplace-results-title" className="text-xl font-semibold tracking-tight">
          {t("marketplace:search.resultsFor", { query: query.trim() })}
        </h2>
        <p className="text-sm text-muted-foreground" role="status">
          {t("marketplace:home.results", { count: found.length })}
        </p>
      </div>
      {found.length === 0 ? (
        <EmptyState variant="plain" title={t("marketplace:discover.noMatch.title")}>
          {t("marketplace:discover.noMatch.text")}
        </EmptyState>
      ) : (
        <Tiles
          label={t("marketplace:discover.list")}
          plugins={found}
          snapshot={snapshot}
          updates={updates}
          migrations={migrations}
          actions={actions}
        />
      )}
    </section>
  )
}

/**
 * Every registry plugin not installed here, newest first by the date the
 * registry first published it (undated ones last, by name): the catalog to
 * browse. Installed plugins are in "Your plugins" instead, so nothing is
 * listed twice.
 */
export function RecentSection({
  snapshot,
  updates,
  migrations,
  actions,
  limit,
}: {
  snapshot: MarketplaceSnapshot
  updates: UpdateCandidate[]
  migrations: ModuleMigration[]
  actions: MarketplaceActions
  /** At most this many, the newest. */
  limit?: number
}) {
  const { t } = useTranslation()
  const installed = new Set(snapshot.installed.map((p) => p.summary.id))
  const plugins = byRecency(
    snapshot.available.filter((p) => !installed.has(p.id) && !p.installed),
  ).slice(0, limit)
  const official = officialIds(snapshot, migrations)
  if (snapshot.available.length === 0) return null
  return (
    <PageSection
      id="marketplace-recent"
      title={t("marketplace:home.recent.title")}
      description={t("marketplace:home.recent.text")}
    >
      {plugins.length === 0 ? (
        <p className="text-sm text-muted-foreground">{t("marketplace:home.recent.allInstalled")}</p>
      ) : (
        <TileGrid
          label={t("marketplace:home.recent.title")}
          items={plugins}
          keyOf={(p: AvailablePlugin) => p.id}
          render={(p) => {
            const at = firstPublished(p)
            return (
              <PluginTile
                plugin={p}
                snapshot={snapshot}
                updates={updates}
                actions={actions}
                official={official.has(p.id)}
                note={
                  at === undefined
                    ? undefined
                    : t("marketplace:home.recent.added", { date: formatDate(at) })
                }
              />
            )
          }}
        />
      )}
    </PageSection>
  )
}

/** A grid of catalog tiles with the plugins' official marks. */
function Tiles({
  label,
  plugins,
  snapshot,
  updates,
  migrations,
  actions,
}: {
  label: string
  plugins: AvailablePlugin[]
  snapshot: MarketplaceSnapshot
  updates: UpdateCandidate[]
  migrations: ModuleMigration[]
  actions: MarketplaceActions
}) {
  const official = officialIds(snapshot, migrations)
  return (
    <TileGrid
      label={label}
      items={plugins}
      keyOf={(p) => p.id}
      render={(p) => (
        <PluginTile
          plugin={p}
          snapshot={snapshot}
          updates={updates}
          actions={actions}
          official={official.has(p.id)}
        />
      )}
    />
  )
}

/**
 * Discover without a search and without a storefront (catalog-only): a
 * compact strip of your plugins (managed under Installed), the newest ones
 * and the built-in collections. With a storefront, `DiscoverEditorial`
 * (storefront/discover-editorial.tsx) shows its sections instead and falls
 * back to this.
 */
export function DiscoverHome({
  snapshot,
  updates,
  migrations,
  actions,
  onSection,
}: {
  snapshot: MarketplaceSnapshot
  updates: UpdateCandidate[]
  migrations: ModuleMigration[]
  actions: MarketplaceActions
  onSection: (section: MarketplaceSection) => void
}) {
  const { t } = useTranslation()
  return (
    <>
      <YourPluginsStrip snapshot={snapshot} actions={actions} onSection={onSection} />
      <RecentSection
        snapshot={snapshot}
        updates={updates}
        migrations={migrations}
        actions={actions}
        limit={6}
      />
      {snapshot.available.length > 0 && (
        <PageSection id="marketplace-collections" title={t("marketplace:nav.collections")}>
          <ul
            aria-label={t("marketplace:nav.collections")}
            className="grid gap-3 @lg:grid-cols-2 @4xl:grid-cols-3"
          >
            {COLLECTION_IDS.map((id) => (
              <li key={id}>
                <CollectionCard
                  id={id}
                  plugins={collectionPlugins(snapshot.available, id)}
                  onOpen={() => {
                    onSection(id)
                  }}
                />
              </li>
            ))}
          </ul>
        </PageSection>
      )}
    </>
  )
}

function CollectionCard({
  id,
  plugins,
  onOpen,
}: {
  id: CollectionId
  plugins: AvailablePlugin[]
  onOpen: () => void
}) {
  const { t } = useTranslation()
  const look = COLLECTION_LOOK[id]
  const Icon = look.icon
  return (
    <button
      type="button"
      className="grid h-full w-full gap-2 rounded-xl bg-card p-4 text-left ring-1 ring-border transition-colors outline-none hover:bg-accent/40 focus-visible:ring-[3px] focus-visible:ring-ring/50"
      onClick={onOpen}
    >
      <span className="flex items-center gap-2">
        <Icon className="size-4 text-muted-foreground" aria-hidden />
        <span className="text-sm font-semibold">{t(look.title)}</span>
        <ArrowRight className="ml-auto size-4 text-muted-foreground" aria-hidden />
      </span>
      <span className="text-xs text-muted-foreground">{t(look.text)}</span>
      <span className="mt-1 flex items-center gap-2">
        <span className="flex -space-x-1.5" aria-hidden>
          {plugins.slice(0, 4).map((p) => (
            <PluginAvatar
              key={p.id}
              name={p.name}
              category={primaryCategory(p)}
              media={p.icon}
              size="sm"
            />
          ))}
        </span>
        <span className="text-xs text-muted-foreground">
          {t("marketplace:collection.count", { count: plugins.length })}
        </span>
      </span>
    </button>
  )
}

/** The installed plugins, compactly; managing them is Installed's. */
export function YourPluginsStrip({
  snapshot,
  actions,
  onSection,
}: {
  snapshot: MarketplaceSnapshot
  actions: MarketplaceActions
  onSection: (section: MarketplaceSection) => void
}) {
  const { t } = useTranslation()
  const format = useFormat()
  if (snapshot.installed.length === 0) return null
  const shown = [...snapshot.installed]
    .sort((a, b) => format.compare(a.summary.name, b.summary.name))
    .slice(0, 8)
  return (
    <section aria-labelledby="marketplace-yours-title" className="grid gap-2">
      <div className="flex items-center justify-between gap-2">
        <h3 id="marketplace-yours-title" className="text-base font-semibold">
          {t("marketplace:sections.yours.title")}
        </h3>
        <Button
          variant="ghost"
          size="sm"
          onClick={() => {
            onSection("installed")
          }}
        >
          {t("marketplace:home.manageAll", { count: snapshot.installed.length })} <ArrowRight />
        </Button>
      </div>
      <ul aria-label={t("marketplace:sections.yours.title")} className="flex flex-wrap gap-2">
        {shown.map((item) => {
          const name = pluginDisplayName(item.summary.id, item.summary.name, snapshot)
          const listed = snapshot.available.find((p) => p.id === item.summary.id)
          return (
            <li key={item.summary.id}>
              <button
                type="button"
                className="flex items-center gap-2 rounded-full bg-card py-1 pr-3.5 pl-1 text-sm ring-1 ring-border transition-colors outline-none hover:bg-accent/40 focus-visible:ring-[3px] focus-visible:ring-ring/50"
                aria-label={t("marketplace:installed.detailsNamed", { name })}
                onClick={() => {
                  actions.openDetails(item.summary.id)
                }}
              >
                <PluginAvatar
                  name={name}
                  category={listed ? primaryCategory(listed) : undefined}
                  src={item.icon}
                  size="sm"
                  muted={item.summary.state !== "enabled"}
                />
                <span className="max-w-40 truncate">{name}</span>
              </button>
            </li>
          )
        })}
      </ul>
    </section>
  )
}

/**
 * One collection: a storefront's (`editorial:<id>`), or a built-in one with
 * its registry plugins, or why there are none.
 */
export function CollectionSection({
  id,
  snapshot,
  updates,
  migrations,
  actions,
  onCategories,
}: {
  id: CollectionId | EditorialSectionId
  snapshot: MarketplaceSnapshot
  updates: UpdateCandidate[]
  migrations: ModuleMigration[]
  actions: MarketplaceActions
  /** Opens Categories (from an empty built-in collection). */
  onCategories?: () => void
}) {
  if (isEditorialSection(id)) {
    return (
      <EditorialCollection
        section={id}
        snapshot={snapshot}
        updates={updates}
        migrations={migrations}
        actions={actions}
      />
    )
  }
  return (
    <BuiltInCollection
      id={id}
      snapshot={snapshot}
      updates={updates}
      migrations={migrations}
      actions={actions}
      onCategories={onCategories}
    />
  )
}

function BuiltInCollection({
  id,
  snapshot,
  updates,
  migrations,
  actions,
  onCategories,
}: {
  id: CollectionId
  snapshot: MarketplaceSnapshot
  updates: UpdateCandidate[]
  migrations: ModuleMigration[]
  actions: MarketplaceActions
  /** Opens Categories (from the empty state). */
  onCategories?: () => void
}) {
  const { t } = useTranslation()
  const look = COLLECTION_LOOK[id]
  const plugins = collectionPlugins(snapshot.available, id)
  return (
    <section aria-labelledby="marketplace-collection-title" className="grid gap-4">
      <div className="grid gap-1">
        <h2 id="marketplace-collection-title" className="text-xl font-semibold tracking-tight">
          {t(look.title)}
        </h2>
        <p className="text-sm text-muted-foreground">{t(look.text)}</p>
      </div>
      {plugins.length === 0 ? (
        <EmptyState variant="plain" title={t("marketplace:collection.empty.title")}>
          <p>{t("marketplace:collection.empty.text")}</p>
          {/* Which categories fill it: nothing is made up to fill it. */}
          <p>
            {t("marketplace:collection.empty.categories", {
              categories: COLLECTIONS[id].categories.map((c) => categoryName(c)).join(", "),
            })}
          </p>
          {onCategories && (
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="mt-3"
              onClick={onCategories}
            >
              {t("marketplace:collection.empty.browse")}
            </Button>
          )}
        </EmptyState>
      ) : (
        <Tiles
          label={t(look.title)}
          plugins={plugins}
          snapshot={snapshot}
          updates={updates}
          migrations={migrations}
          actions={actions}
        />
      )}
    </section>
  )
}

/** Every category the registry uses, each opening the catalog filtered by it. */
export function CategoriesSection({
  snapshot,
  onCategory,
}: {
  snapshot: MarketplaceSnapshot
  onCategory: (category: string) => void
}) {
  const { t } = useTranslation()
  const categories = categoryCounts(snapshot.available)
  return (
    <section aria-labelledby="marketplace-categories-title" className="grid gap-4">
      <div className="grid gap-1">
        <h2 id="marketplace-categories-title" className="text-xl font-semibold tracking-tight">
          {t("marketplace:nav.categories")}
        </h2>
        <p className="text-sm text-muted-foreground">{t("marketplace:categories.text")}</p>
      </div>
      {categories.length === 0 ? (
        <EmptyState variant="plain" title={t("marketplace:categories.empty")} />
      ) : (
        <ul
          aria-label={t("marketplace:nav.categories")}
          className="grid gap-3 @lg:grid-cols-2 @4xl:grid-cols-3"
        >
          {categories.map((c) => {
            const members = snapshot.available.filter((p) => (p.categories ?? []).includes(c.id))
            return (
              <li key={c.id}>
                <button
                  type="button"
                  className="flex h-full w-full items-center gap-3 rounded-xl bg-card p-4 text-left ring-1 ring-border transition-colors outline-none hover:bg-accent/40 focus-visible:ring-[3px] focus-visible:ring-ring/50"
                  onClick={() => {
                    onCategory(c.id)
                  }}
                >
                  <PluginAvatar name={categoryName(c.id)} category={c.id} size="md" />
                  <span className="grid min-w-0 flex-1 gap-0.5">
                    <span className="text-sm font-semibold">{categoryName(c.id)}</span>
                    <span className="truncate text-xs text-muted-foreground">
                      {members.map((p) => p.name).join(", ")}
                    </span>
                  </span>
                  <span className="text-xs text-muted-foreground tabular-nums">{c.count}</span>
                </button>
              </li>
            )
          })}
        </ul>
      )}
    </section>
  )
}
