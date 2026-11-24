// A storefront collection opened from the sidebar: its editorial title and
// description, then its plugins as the catalog's tiles.

import { EmptyState } from "@/components/page"
import { useTranslation } from "@/i18n"

import { officialIds } from "../catalog"
import { PluginTile, TileGrid } from "../plugin-tile"
import { type EditorialSectionId, editorialCollectionId } from "./resolve"
import type { CatalogProps } from "./shared"
import { clip, LIMITS } from "./text"
import { useStorefront } from "./use-storefront"

export function EditorialCollection({
  section,
  ...catalog
}: CatalogProps & { section: EditorialSectionId }) {
  const { t } = useTranslation()
  const { collections } = useStorefront()
  const id = editorialCollectionId(section)
  const collection = collections.find((c) => c.id === id)
  const official = officialIds(catalog.snapshot, catalog.migrations)
  const title = collection ? clip(collection.title, LIMITS.title) : t("marketplace:nav.collections")
  return (
    <section aria-labelledby="marketplace-collection-title" className="grid gap-4">
      <div className="grid gap-1">
        <h2
          id="marketplace-collection-title"
          dir="auto"
          className="text-xl font-semibold tracking-tight [overflow-wrap:anywhere]"
        >
          {title}
        </h2>
        {collection?.description && (
          <p dir="auto" className="text-sm [overflow-wrap:anywhere] text-muted-foreground">
            {clip(collection.description, LIMITS.body)}
          </p>
        )}
      </div>
      {!collection ? (
        <EmptyState variant="plain" title={t("marketplace:collection.empty.title")}>
          {t("marketplace:collection.empty.text")}
        </EmptyState>
      ) : (
        <TileGrid
          label={title}
          items={collection.plugins}
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
      )}
    </section>
  )
}
