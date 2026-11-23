// An editorial row in its compact layout: a dense list of plugins, two or
// three columns wide, each with its icon, name, developer and state
// control, all from the catalog's `AvailablePlugin`.

import { useId } from "react"

import { cn } from "@/lib/utils"

import { officialIds, primaryCategory, useEntry } from "../catalog"
import { DeveloperLine, PluginAvatar, StateControl } from "../plugin-tile"
import type { AvailablePlugin } from "../types"
import { type CatalogProps, SectionHeading } from "./shared"
import { clip, LIMITS } from "./text"

export function CompactList({
  title,
  description,
  plugins,
  ...catalog
}: CatalogProps & {
  title: string
  description?: string | undefined
  plugins: AvailablePlugin[]
}) {
  const headingId = `compact-${useId().replace(/:/g, "")}`
  const name = clip(title, LIMITS.title)
  const official = officialIds(catalog.snapshot, catalog.migrations)
  return (
    <section aria-labelledby={headingId} className="grid gap-3" data-slot="compact-list">
      <SectionHeading
        id={headingId}
        title={name}
        description={description && clip(description, LIMITS.body)}
      />
      <ul aria-label={name} className="grid gap-x-6 gap-y-1 @2xl:grid-cols-2 @5xl:grid-cols-3">
        {plugins.map((plugin) => (
          <li key={plugin.id} className="min-w-0">
            <CompactRow plugin={plugin} official={official.has(plugin.id)} {...catalog} />
          </li>
        ))}
      </ul>
    </section>
  )
}

function CompactRow({
  plugin,
  official,
  snapshot,
  updates,
  actions,
}: CatalogProps & { plugin: AvailablePlugin; official: boolean }) {
  const { state, context } = useEntry(plugin, snapshot, updates, actions)
  const muted = state.kind === "incompatible" || state.kind === "withdrawn"
  return (
    <div
      className={cn(
        "group/row relative flex items-center gap-3 rounded-lg px-2 py-2 transition-colors hover:bg-accent/40",
      )}
    >
      <PluginAvatar
        name={plugin.name}
        category={primaryCategory(plugin)}
        src={context.installed?.icon}
        media={plugin.icon}
        size="sm"
        muted={muted}
      />
      <div className="grid min-w-0 flex-1">
        <button
          type="button"
          className="w-fit max-w-full truncate text-left text-sm font-medium outline-none after:absolute after:inset-0 after:rounded-lg focus-visible:after:ring-[3px] focus-visible:after:ring-ring/50"
          onClick={() => {
            actions.openDetails(plugin.id)
          }}
        >
          {plugin.name}
        </button>
        <DeveloperLine plugin={plugin} official={official} className="truncate" />
      </div>
      <div className="relative z-10 shrink-0">
        <StateControl plugin={plugin} state={state} context={context} actions={actions} />
      </div>
    </div>
  )
}
