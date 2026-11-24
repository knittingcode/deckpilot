// Discover's banners (storefront design §7), after the WAI-ARIA carousel
// pattern with tabbed slide picking:
//
// - never moves by itself (no autoplay, no timer of any kind);
// - Previous and Next buttons, and a tablist of dots; Left and Right
//   arrows (and Home, End) move between slides, wrapping around;
// - the region is `aria-roledescription="carousel"`, each slide a `group`
//   `aria-roledescription="slide"` labelled "n of m"; hidden slides are
//   out of the accessibility tree and the tab order;
// - with reduced motion there is no transition at all;
// - the artwork's text alternative is the editorial `alt`, or `alt=""`
//   for decorative art;
// - the call to action navigates inside the Marketplace (a plugin's page,
//   a collection, a category), never to a web page.

import { ChevronLeft, ChevronRight } from "lucide-react"
import { type KeyboardEvent, useId, useRef, useState } from "react"

import { Button } from "@/components/ui/button"
import { useTranslation } from "@/i18n"
import { cn } from "@/lib/utils"

import type { TargetView } from "../storefront-types"
import { useReducedMotion } from "./motion"
import type { ResolvedBanner } from "./resolve"
import { clip, LIMITS } from "./text"
import { VerifiedImage } from "./verified-image"

export function BannerCarousel({
  title,
  items,
  onTarget,
}: {
  title?: string | undefined
  items: ResolvedBanner[]
  onTarget: (target: TargetView) => void
}) {
  const { t } = useTranslation()
  const reduced = useReducedMotion()
  const [current, setCurrent] = useState(0)
  const tabs = useRef<(HTMLButtonElement | null)[]>([])
  const base = `banner-${useId().replace(/:/g, "")}`
  const count = items.length
  const index = Math.min(current, count - 1)
  const label = title ? clip(title, LIMITS.title) : t("marketplace:storefront.banners")

  const show = (next: number, focusTab = false) => {
    const wrapped = ((next % count) + count) % count
    setCurrent(wrapped)
    if (focusTab) tabs.current[wrapped]?.focus()
  }

  const onKeyDown = (e: KeyboardEvent<HTMLElement>) => {
    // On the carousel's controls (Previous, the dots, Next) only.
    const onTab = (e.target as HTMLElement).getAttribute("role") === "tab"
    const moves: Record<string, number | undefined> = {
      ArrowLeft: index - 1,
      ArrowRight: index + 1,
      ...(onTab ? { Home: 0, End: count - 1 } : {}),
    }
    const next = moves[e.key]
    if (next === undefined || count < 2) return
    e.preventDefault()
    show(next, onTab)
  }

  return (
    <section
      aria-roledescription={t("marketplace:storefront.carousel")}
      aria-label={label}
      data-slot="banner-carousel"
      data-motion={reduced ? "reduce" : "full"}
      className="grid gap-2"
    >
      <div className="relative overflow-hidden rounded-2xl ring-1 ring-border">
        {items.map((item, i) => {
          const active = i === index
          return (
            <div
              key={item.id}
              id={`${base}-slide-${String(i)}`}
              role="group"
              aria-roledescription={t("marketplace:storefront.slide")}
              aria-label={t("marketplace:storefront.slideOf", { number: i + 1, count })}
              hidden={!active}
              data-active={active}
              className={cn(
                "relative",
                // A short fade as a slide comes in; none with reduced motion.
                !reduced && "animate-in duration-300 fade-in-0 motion-reduce:animate-none",
              )}
            >
              <VerifiedImage
                media={item.artwork}
                alt={item.decorative ? "" : clip(item.alt, LIMITS.alt)}
                decorative={item.decorative === true}
                aspect={3}
                className="w-full"
              />
              <div className="pointer-events-none absolute inset-0 bg-gradient-to-r from-black/80 via-black/45 to-transparent" />
              <div className="absolute inset-y-0 left-0 grid max-w-[min(32rem,70%)] content-end gap-2 p-5 @2xl:p-7">
                <h3
                  dir="auto"
                  className="line-clamp-2 text-lg leading-tight font-semibold [overflow-wrap:anywhere] text-white @2xl:text-2xl"
                >
                  {clip(item.title, LIMITS.title)}
                </h3>
                {item.body && (
                  <p
                    dir="auto"
                    className="line-clamp-2 text-sm [overflow-wrap:anywhere] text-white/85"
                  >
                    {clip(item.body, LIMITS.body)}
                  </p>
                )}
                <div>
                  <Button
                    size="sm"
                    variant="secondary"
                    className="mt-1 max-w-full rounded-full px-4"
                    tabIndex={active ? 0 : -1}
                    onClick={() => {
                      onTarget(item.target)
                    }}
                  >
                    <span className="truncate">
                      {item.cta ? clip(item.cta, LIMITS.cta) : t("marketplace:storefront.open")}
                    </span>
                    <ChevronRight aria-hidden />
                  </Button>
                </div>
              </div>
            </div>
          )
        })}
      </div>
      {count > 1 && (
        <div className="flex items-center justify-center gap-2" onKeyDown={onKeyDown}>
          <Button
            variant="ghost"
            size="icon-sm"
            aria-controls={`${base}-slide-${String(index)}`}
            aria-label={t("marketplace:storefront.previous")}
            onClick={() => {
              show(index - 1)
            }}
          >
            <ChevronLeft />
          </Button>
          <div
            role="tablist"
            aria-label={t("marketplace:storefront.slides")}
            className="flex items-center gap-1"
          >
            {items.map((item, i) => {
              const active = i === index
              return (
                <button
                  key={item.id}
                  ref={(el) => {
                    tabs.current[i] = el
                  }}
                  type="button"
                  role="tab"
                  aria-selected={active}
                  aria-controls={`${base}-slide-${String(i)}`}
                  aria-label={t("marketplace:storefront.slideNamed", {
                    number: i + 1,
                    title: clip(item.title, 60),
                  })}
                  tabIndex={active ? 0 : -1}
                  className="grid size-6 place-items-center rounded-full outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50"
                  onClick={() => {
                    show(i)
                  }}
                >
                  <span
                    aria-hidden
                    className={cn(
                      "block h-1.5 rounded-full",
                      !reduced && "transition-all duration-200 motion-reduce:transition-none",
                      active ? "w-4 bg-foreground" : "w-1.5 bg-muted-foreground/50",
                    )}
                  />
                </button>
              )
            })}
          </div>
          <Button
            variant="ghost"
            size="icon-sm"
            aria-controls={`${base}-slide-${String(index)}`}
            aria-label={t("marketplace:storefront.next")}
            onClick={() => {
              show(index + 1)
            }}
          >
            <ChevronRight />
          </Button>
        </div>
      )}
    </section>
  )
}
