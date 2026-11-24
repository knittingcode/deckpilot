// Categories the editorial highlights, each with its blurb; a card opens
// Discover filtered by the category. The name, the count and the icons
// are the catalog's.

import { ArrowRight } from "lucide-react"
import { useId } from "react"

import { useTranslation } from "@/i18n"

import { categoryName, primaryCategory } from "../catalog"
import { PluginAvatar } from "../plugin-tile"
import type { AvailablePlugin } from "../types"
import { SectionHeading } from "./shared"
import { clip, LIMITS } from "./text"

export function CategoryHighlights({
  title,
  categories,
  plugins,
  onCategory,
}: {
  title?: string | undefined
  categories: { id: string; blurb?: string }[]
  plugins: AvailablePlugin[]
  onCategory: (category: string) => void
}) {
  const { t } = useTranslation()
  const headingId = `categories-${useId().replace(/:/g, "")}`
  const name = title ? clip(title, LIMITS.title) : t("marketplace:storefront.categories")
  return (
    <section aria-labelledby={headingId} className="grid gap-3" data-slot="category-highlights">
      <SectionHeading id={headingId} title={name} />
      <ul aria-label={name} className="grid gap-3 @lg:grid-cols-2 @4xl:grid-cols-3">
        {categories.map((c) => {
          const members = plugins.filter((p) => (p.categories ?? []).includes(c.id))
          return (
            <li key={c.id}>
              <button
                type="button"
                className="grid h-full w-full gap-2 rounded-xl bg-card p-4 text-left ring-1 ring-border transition-colors outline-none hover:bg-accent/40 focus-visible:ring-[3px] focus-visible:ring-ring/50"
                onClick={() => {
                  onCategory(c.id)
                }}
              >
                <span className="flex items-center gap-2">
                  <span className="text-sm font-semibold">{categoryName(c.id)}</span>
                  <ArrowRight className="ml-auto size-4 text-muted-foreground" aria-hidden />
                </span>
                {c.blurb && (
                  <span
                    dir="auto"
                    className="line-clamp-2 text-xs [overflow-wrap:anywhere] text-muted-foreground"
                  >
                    {clip(c.blurb, LIMITS.body)}
                  </span>
                )}
                <span className="mt-1 flex items-center gap-2">
                  <span className="flex -space-x-1.5" aria-hidden>
                    {members.slice(0, 4).map((p) => (
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
                    {t("marketplace:collection.count", { count: members.length })}
                  </span>
                </span>
              </button>
            </li>
          )
        })}
      </ul>
    </section>
  )
}
