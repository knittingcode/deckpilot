// A plugin's screenshots (storefront design §7): a row of thumbnails, each
// a `figure` with its caption as `figcaption`; one opens a lightbox dialog
// (focus trapped inside, Escape closes it and returns focus, Left and
// Right move between screenshots). Every image has the listing's text
// alternative. The agent already filtered them to this platform.

import { ChevronLeft, ChevronRight } from "lucide-react"
import { useId, useRef, useState } from "react"

import { Button } from "@/components/ui/button"
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog"
import { useTranslation } from "@/i18n"

import type { ScreenshotView } from "../storefront-types"
import { validMediaRef } from "./resolve"
import { clip, LIMITS } from "./text"
import { VerifiedImage } from "./verified-image"

/** At most this many (the design's limit). */
const MAX_SHOTS = 8

export function ScreenshotCarousel({
  screenshots,
  name,
}: {
  screenshots: ScreenshotView[]
  /** The plugin's name, for an alternative when a screenshot has none. */
  name: string
}) {
  const { t } = useTranslation()
  const shots = screenshots.filter((s) => validMediaRef(s.media)).slice(0, MAX_SHOTS)
  const [open, setOpen] = useState<number | null>(null)
  const thumbs = useRef<(HTMLButtonElement | null)[]>([])
  const headingId = `screenshots-${useId().replace(/:/g, "")}`
  if (shots.length === 0) return null
  const count = shots.length
  const altOf = (shot: ScreenshotView, i: number) =>
    clip(shot.alt, LIMITS.alt) || t("marketplace:details.screenshotOf", { name, number: i + 1 })
  const current = open === null ? null : shots[open]

  const move = (by: number) => {
    setOpen((i) => (i === null ? i : (i + by + count) % count))
  }

  return (
    <section aria-labelledby={headingId} className="grid gap-2" data-slot="screenshots">
      <h4 id={headingId} className="text-base font-semibold">
        {t("marketplace:details.screenshots")}
      </h4>
      <ul
        aria-label={t("marketplace:details.screenshots")}
        className="relative -mx-1 flex snap-x snap-mandatory gap-3 overflow-x-auto px-1 pb-2"
      >
        {shots.map((shot, i) => (
          <li key={`${shot.media.sha256}-${String(i)}`} className="w-72 shrink-0 snap-start">
            <figure className="grid gap-1.5">
              <button
                ref={(el) => {
                  thumbs.current[i] = el
                }}
                type="button"
                className="block w-full overflow-hidden rounded-lg ring-1 ring-border outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50"
                aria-label={t("marketplace:storefront.openScreenshot", {
                  number: i + 1,
                  count,
                  alt: altOf(shot, i),
                })}
                onClick={() => {
                  setOpen(i)
                }}
              >
                <VerifiedImage media={shot.media} alt={altOf(shot, i)} className="w-full" />
              </button>
              {shot.caption && (
                <figcaption
                  dir="auto"
                  className="line-clamp-2 text-xs [overflow-wrap:anywhere] text-muted-foreground"
                >
                  {clip(shot.caption, LIMITS.caption)}
                </figcaption>
              )}
            </figure>
          </li>
        ))}
      </ul>

      <Dialog
        open={current !== null && current !== undefined}
        onOpenChange={(next) => {
          if (!next) setOpen(null)
        }}
      >
        {current && open !== null && (
          <DialogContent
            className="max-w-[min(calc(100%-2rem),72rem)] gap-3 p-4 sm:max-w-[min(calc(100%-2rem),72rem)]"
            onKeyDown={(e) => {
              // Keys used here never reach the plugin page (its Escape goes back).
              if (e.key === "Escape") e.stopPropagation()
              if (count > 1 && (e.key === "ArrowLeft" || e.key === "ArrowRight")) {
                e.preventDefault()
                e.stopPropagation()
                move(e.key === "ArrowLeft" ? -1 : 1)
              }
            }}
            onCloseAutoFocus={(e) => {
              // Back to the thumbnail of the screenshot last shown.
              const thumb = thumbs.current[open]
              if (thumb) {
                e.preventDefault()
                thumb.focus()
              }
            }}
          >
            <DialogTitle className="pr-8 text-sm">
              {t("marketplace:storefront.screenshotOf", { number: open + 1, count, name })}
            </DialogTitle>
            <DialogDescription className="sr-only">
              {t("marketplace:storefront.lightboxHint")}
            </DialogDescription>
            <figure className="grid gap-2">
              <VerifiedImage
                key={current.media.sha256}
                media={current.media}
                alt={altOf(current, open)}
                fit="contain"
                className="max-h-[70vh] w-full rounded-md bg-background"
              />
              {current.caption && (
                <figcaption
                  dir="auto"
                  className="text-sm [overflow-wrap:anywhere] text-muted-foreground"
                >
                  {clip(current.caption, LIMITS.caption)}
                </figcaption>
              )}
            </figure>
            {count > 1 && (
              <div className="flex items-center justify-between">
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => {
                    move(-1)
                  }}
                >
                  <ChevronLeft /> {t("marketplace:storefront.previous")}
                </Button>
                <span className="text-xs text-muted-foreground tabular-nums">
                  {t("marketplace:storefront.slideOf", { number: open + 1, count })}
                </span>
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => {
                    move(1)
                  }}
                >
                  {t("marketplace:storefront.next")} <ChevronRight />
                </Button>
              </div>
            )}
          </DialogContent>
        )}
      </Dialog>
    </section>
  )
}
