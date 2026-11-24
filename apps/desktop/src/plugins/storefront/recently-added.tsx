// "Recently added" as an editorial section: the editorial picks only the
// title and how many; which plugins, and their order, come from the dates
// the registry signs (`byRecency`, `AvailableVersion.publishedAt`).
// Plugins installed here are left out, as on the catalog-only Discover.

import { useTranslation } from "@/i18n"

import { byRecency, firstPublished } from "../catalog"
import { formatDate } from "../model"
import { CardRow } from "./card-row"
import type { CatalogProps } from "./shared"

export function RecentlyAdded({
  title,
  limit,
  ...catalog
}: CatalogProps & { title?: string | undefined; limit: number }) {
  const { t } = useTranslation()
  const installed = new Set(catalog.snapshot.installed.map((p) => p.summary.id))
  const plugins = byRecency(
    catalog.snapshot.available.filter((p) => !installed.has(p.id) && !p.installed),
  ).slice(0, limit)
  if (plugins.length === 0) return null
  const notes = new Map<string, string>()
  for (const p of plugins) {
    const at = firstPublished(p)
    if (at !== undefined)
      notes.set(p.id, t("marketplace:home.recent.added", { date: formatDate(at) }))
  }
  return (
    <CardRow
      title={title ?? t("marketplace:home.recent.title")}
      plugins={plugins}
      notes={notes}
      {...catalog}
    />
  )
}
