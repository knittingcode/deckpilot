// An editorial row of plugin cards, scrolling sideways (storefront design
// §7). A list named by the row's title; each card is the catalog's own
// tile (`PluginTile`: name, developer, verification, state control from
// `AvailablePlugin`); the editorial supplies only which plugins and the
// title. The scroll buttons move by about a page and are disabled at
// either end; they are hidden when everything fits.

import { ChevronLeft, ChevronRight } from "lucide-react"
import { useCallback, useEffect, useId, useRef, useState } from "react"

import { Button } from "@/components/ui/button"
import { useTranslation } from "@/i18n"

import { officialIds } from "../catalog"
import { PluginTile } from "../plugin-tile"
import type { AvailablePlugin } from "../types"
import { useReducedMotion } from "./motion"
import { type CatalogProps, SectionHeading } from "./shared"
import { clip, LIMITS } from "./text"

export function CardRow({
  title,
  description,
  plugins,
  notes,
  ...catalog
}: CatalogProps & {
  title: string
  description?: string | undefined
  plugins: AvailablePlugin[]
  /** A line on a card (e.g. when it was added), by plugin id. */
  notes?: ReadonlyMap<string, string>
}) {
  const { t } = useTranslation()
  const reduced = useReducedMotion()
  const scroller = useRef<HTMLUListElement>(null)
  const [edges, setEdges] = useState({ start: true, end: true })
  const headingId = `row-${useId().replace(/:/g, "")}`
  const name = clip(title, LIMITS.title)
  const official = officialIds(catalog.snapshot, catalog.migrations)

  const measure = useCallback(() => {
    const el = scroller.current
    if (!el) return
    const max = el.scrollWidth - el.clientWidth
    setEdges({ start: el.scrollLeft <= 1, end: el.scrollLeft >= max - 1 })
  }, [])

  useEffect(() => {
    measure()
    const el = scroller.current
    if (!el || typeof ResizeObserver === "undefined") return
    const observer = new ResizeObserver(measure)
    observer.observe(el)
    return () => {
      observer.disconnect()
    }
  }, [measure, plugins.length])

  const scroll = (direction: -1 | 1) => {
    const el = scroller.current
    if (!el) return
    el.scrollBy({
      left: direction * Math.max(200, el.clientWidth * 0.9),
      behavior: reduced ? "auto" : "smooth",
    })
  }

  const overflows = !(edges.start && edges.end)
  return (
    <section aria-labelledby={headingId} className="grid gap-3" data-slot="card-row">
      <SectionHeading
        id={headingId}
        title={name}
        description={description && clip(description, LIMITS.body)}
      >
        {overflows && (
          <div className="flex shrink-0 gap-1">
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label={t("marketplace:storefront.scrollBack", { title: name })}
              disabled={edges.start}
              onClick={() => {
                scroll(-1)
              }}
            >
              <ChevronLeft />
            </Button>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label={t("marketplace:storefront.scrollForward", { title: name })}
              disabled={edges.end}
              onClick={() => {
                scroll(1)
              }}
            >
              <ChevronRight />
            </Button>
          </div>
        )}
      </SectionHeading>
      <ul
        ref={scroller}
        aria-label={name}
        onScroll={measure}
        className="relative -mx-1 flex snap-x snap-mandatory [scrollbar-width:thin] gap-3 overflow-x-auto scroll-smooth px-1 pb-2 motion-reduce:scroll-auto"
      >
        {plugins.map((plugin) => (
          <li key={plugin.id} className="w-80 shrink-0 snap-start @4xl:w-96">
            <PluginTile
              plugin={plugin}
              snapshot={catalog.snapshot}
              updates={catalog.updates}
              actions={catalog.actions}
              official={official.has(plugin.id)}
              note={notes?.get(plugin.id)}
            />
          </li>
        ))}
      </ul>
    </section>
  )
}
