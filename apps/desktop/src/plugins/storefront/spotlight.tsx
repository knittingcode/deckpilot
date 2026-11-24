// One plugin, large (storefront design §7): the listing's promo art, the
// editorial headline and body, then the plugin as the catalog knows it
// (name, developer, verification, state control) and, when the spotlight
// names a published version, that version's release notes.

import { useId } from "react"

import { useTranslation } from "@/i18n"

import { officialIds, primaryCategory, useEntry } from "../catalog"
import { DeveloperLine, FailureText, PluginAvatar, StateControl } from "../plugin-tile"
import type { AvailablePlugin } from "../types"
import { SafeMarkdown } from "./safe-markdown"
import type { CatalogProps } from "./shared"
import { clip, LIMITS } from "./text"
import { useListing } from "./use-storefront"
import { VerifiedImage } from "./verified-image"

export function Spotlight({
  title,
  description,
  plugin,
  version,
  ...catalog
}: CatalogProps & {
  title?: string | undefined
  description?: string | undefined
  plugin: AvailablePlugin
  version?: string | undefined
}) {
  const { t } = useTranslation()
  const listing = useListing(plugin.id)
  const { state, context } = useEntry(plugin, catalog.snapshot, catalog.updates, catalog.actions)
  const headingId = `spotlight-${useId().replace(/:/g, "")}`
  const official = officialIds(catalog.snapshot, catalog.migrations).has(plugin.id)
  const note = version ? listing?.releaseNotes.find((n) => n.version === version) : undefined
  const category = primaryCategory(plugin)
  const art = listing?.promoArt
  return (
    <section
      aria-labelledby={headingId}
      data-slot="spotlight"
      className="grid overflow-hidden rounded-2xl bg-card ring-1 ring-border @3xl:grid-cols-[minmax(0,1.1fr)_minmax(0,1fr)]"
    >
      <VerifiedImage
        media={art?.media}
        alt={clip(art?.alt, LIMITS.alt)}
        aspect={2}
        className="w-full"
        fallback={
          <div className="flex size-full items-center justify-center bg-gradient-to-br from-muted via-secondary to-card">
            <PluginAvatar name={plugin.name} category={category} media={plugin.icon} size="lg" />
          </div>
        }
      />
      <div className="grid content-center gap-3 p-5 @3xl:p-6">
        <p className="text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
          {t("marketplace:storefront.spotlight")}
        </p>
        <h3
          id={headingId}
          dir="auto"
          className="line-clamp-2 text-xl leading-tight font-semibold tracking-tight [overflow-wrap:anywhere]"
        >
          {title ? clip(title, LIMITS.title) : plugin.name}
        </h3>
        {description && (
          <p
            dir="auto"
            className="line-clamp-3 text-sm [overflow-wrap:anywhere] text-foreground/80"
          >
            {clip(description, LIMITS.body)}
          </p>
        )}
        <div className="flex items-center gap-3 pt-1">
          <PluginAvatar
            name={plugin.name}
            category={category}
            src={context.installed?.icon}
            media={plugin.icon}
            size="md"
          />
          <div className="grid min-w-0 flex-1 gap-0.5">
            <button
              type="button"
              className="w-fit max-w-full truncate text-left text-sm font-semibold underline-offset-2 outline-none hover:underline focus-visible:ring-[3px] focus-visible:ring-ring/50"
              onClick={() => {
                catalog.actions.openDetails(plugin.id)
              }}
            >
              {plugin.name}
            </button>
            <DeveloperLine plugin={plugin} official={official} />
          </div>
          <StateControl plugin={plugin} state={state} context={context} actions={catalog.actions} />
        </div>
        <FailureText state={state} />
        {note && (
          <div className="grid gap-1 border-t pt-3">
            <p className="text-xs font-medium text-muted-foreground">
              {t("marketplace:storefront.whatsNewIn", {
                version: clip(note.version, LIMITS.version),
              })}
            </p>
            <SafeMarkdown source={note.text} className="line-clamp-4 text-xs leading-5" />
          </div>
        )}
      </div>
    </section>
  )
}
