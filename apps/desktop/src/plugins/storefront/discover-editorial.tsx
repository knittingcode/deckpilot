// Discover with a storefront (storefront design §7): the editorial
// sections in the agent's order, each built from the catalog's own
// `AvailablePlugin`s. Catalog-only (`DiscoverHome`, exactly as without the
// feature) when the agent has no `storefront` feature, publishes none,
// refused it, or nothing in it can be placed; also if a section fails to
// render, that section alone is left out.

import { Component, type ErrorInfo, type ReactNode } from "react"

import { Skeleton } from "@/components/ui/skeleton"
import { useTranslation } from "@/i18n"

import type { MarketplaceSection } from "../catalog"
import { officialIds } from "../catalog"
import { DiscoverHome, YourPluginsStrip } from "../home"
import { PluginTile, TileGrid } from "../plugin-tile"
import type { TargetView } from "../storefront-types"
import { BannerCarousel } from "./banner-carousel"
import { CardRow } from "./card-row"
import { CategoryHighlights } from "./category-highlights"
import { CompactList } from "./compact-list"
import { RecentlyAdded } from "./recently-added"
import { editorialSection, type ResolvedSection } from "./resolve"
import { type CatalogProps, SectionHeading } from "./shared"
import { Spotlight } from "./spotlight"
import { clip, LIMITS } from "./text"
import { useStorefront } from "./use-storefront"
import { compareText } from "@/i18n/format"

export function DiscoverEditorial({
  onSection,
  onCategory,
  ...catalog
}: CatalogProps & {
  onSection: (section: MarketplaceSection) => void
  /** Narrows Discover to a category (a banner or category card). */
  onCategory: (category: string) => void
}) {
  const { status, sections } = useStorefront()
  const fallback = <DiscoverHome {...catalog} onSection={onSection} />
  if (status === "loading") return <DiscoverSkeleton />
  if (sections.length === 0 || catalog.snapshot.available.length === 0) return fallback

  const onTarget = (target: TargetView) => {
    if (target.type === "plugin") catalog.actions.openDetails(target.id)
    else if (target.type === "collection") onSection(editorialSection(target.id))
    else onCategory(target.id)
  }

  return (
    <Boundary fallback={fallback}>
      <div className="grid gap-8" data-slot="discover-editorial">
        <YourPluginsStrip
          snapshot={catalog.snapshot}
          actions={catalog.actions}
          onSection={onSection}
        />
        {sections.map((section) => (
          <Boundary key={section.id} fallback={null}>
            <EditorialSection
              section={section}
              catalog={catalog}
              onTarget={onTarget}
              onCategory={onCategory}
            />
          </Boundary>
        ))}
      </div>
    </Boundary>
  )
}

function EditorialSection({
  section,
  catalog,
  onTarget,
  onCategory,
}: {
  section: ResolvedSection
  catalog: CatalogProps
  onTarget: (target: TargetView) => void
  onCategory: (category: string) => void
}) {
  switch (section.kind) {
    case "bannerCarousel":
      return <BannerCarousel title={section.title} items={section.items} onTarget={onTarget} />
    case "collectionRow":
      return section.layout === "compact" ? (
        <CompactList
          title={section.title}
          description={section.description}
          plugins={section.plugins}
          {...catalog}
        />
      ) : (
        <CardRow
          title={section.title}
          description={section.description}
          plugins={section.plugins}
          {...catalog}
        />
      )
    case "spotlight":
      return (
        <Spotlight
          title={section.title}
          description={section.description}
          plugin={section.plugin}
          version={section.version}
          {...catalog}
        />
      )
    case "recentlyAdded":
      return <RecentlyAdded title={section.title} limit={section.limit} {...catalog} />
    case "categoryHighlights":
      return (
        <CategoryHighlights
          title={section.title}
          categories={section.categories}
          plugins={catalog.snapshot.available}
          onCategory={onCategory}
        />
      )
    case "catalog":
      return <CatalogGrid title={section.title} {...catalog} />
  }
}

/** The whole catalog, by name: the "all plugins" section. */
function CatalogGrid({ title, ...catalog }: CatalogProps & { title?: string | undefined }) {
  const { t } = useTranslation()
  const name = title ? clip(title, LIMITS.title) : t("marketplace:storefront.catalog")
  const official = officialIds(catalog.snapshot, catalog.migrations)
  const plugins = [...catalog.snapshot.available].sort((a, b) => compareText(a.name, b.name))
  return (
    <section aria-labelledby="discover-catalog-title" className="grid gap-3">
      <SectionHeading id="discover-catalog-title" title={name} />
      <TileGrid
        label={name}
        items={plugins}
        keyOf={(p) => p.id}
        render={(p) => (
          <PluginTile
            plugin={p}
            snapshot={catalog.snapshot}
            updates={catalog.updates}
            actions={catalog.actions}
            official={official.has(p.id)}
          />
        )}
      />
    </section>
  )
}

/** While the storefront loads: its shape, without moving the page when it arrives. */
function DiscoverSkeleton() {
  const { t } = useTranslation()
  return (
    <div className="grid gap-8" aria-busy="true" aria-label={t("marketplace:storefront.loading")}>
      <Skeleton className="aspect-[3/1] w-full rounded-2xl" />
      <div className="grid gap-3">
        <Skeleton className="h-5 w-40" />
        <div className="flex gap-3 overflow-hidden">
          {[0, 1, 2].map((i) => (
            <Skeleton key={i} className="h-24 w-72 shrink-0 rounded-xl" />
          ))}
        </div>
      </div>
    </div>
  )
}

/** Renders `fallback` instead of children that failed to render. */
class Boundary extends Component<
  { fallback: ReactNode; children: ReactNode },
  { failed: boolean }
> {
  override state = { failed: false }

  static getDerivedStateFromError(): { failed: boolean } {
    return { failed: true }
  }

  override componentDidCatch(error: unknown, info: ErrorInfo): void {
    console.warn("storefront: a Discover section failed to render", error, info.componentStack)
  }

  override render(): ReactNode {
    return this.state.failed ? this.props.fallback : this.props.children
  }
}
