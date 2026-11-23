// A listing's highlights: short title and text pairs, plain text.

import { useTranslation } from "@/i18n"

import type { HighlightView } from "../storefront-types"
import { clip, LIMITS } from "./text"

/** At most this many (the design's limit). */
const MAX_HIGHLIGHTS = 6

export function Highlights({ highlights }: { highlights: HighlightView[] }) {
  const { t } = useTranslation()
  const shown = highlights
    .filter((h) => typeof h.title === "string" && h.title.trim() !== "")
    .slice(0, MAX_HIGHLIGHTS)
  if (shown.length === 0) return null
  return (
    <section
      aria-labelledby="plugin-detail-highlights"
      className="grid gap-2"
      data-slot="highlights"
    >
      <h4 id="plugin-detail-highlights" className="text-base font-semibold">
        {t("marketplace:storefront.highlights")}
      </h4>
      <ul
        aria-label={t("marketplace:storefront.highlights")}
        className="grid gap-3 @xl:grid-cols-2 @4xl:grid-cols-3"
      >
        {shown.map((h, i) => (
          <li
            key={i}
            className="grid content-start gap-1 rounded-xl bg-card p-4 ring-1 ring-border"
          >
            <span
              dir="auto"
              className="line-clamp-2 text-sm font-semibold [overflow-wrap:anywhere]"
            >
              {clip(h.title, LIMITS.highlightTitle)}
            </span>
            <span
              dir="auto"
              className="line-clamp-4 text-xs [overflow-wrap:anywhere] text-muted-foreground"
            >
              {clip(h.text, LIMITS.highlightText)}
            </span>
          </li>
        ))}
      </ul>
    </section>
  )
}
