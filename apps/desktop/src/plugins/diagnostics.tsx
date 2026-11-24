// The Marketplace's Diagnostics: where DeckPilot's built-in modules stand in
// their move to plugins (with consent review and retry), what DeckPilot's
// functionality is made of, and the registry this computer verified. The
// technical facts live here, out of the catalog.

import { RefreshCw } from "lucide-react"

import { Button } from "@/components/ui/button"
import { useTranslation } from "@/i18n"
import { useAppContext } from "@/lib/app-context"

import type { MarketplaceActions } from "./marketplace"
import { MigrationNotice } from "./migration"
import { classifyPlugins, formatTime } from "./model"
import type { MarketplaceSnapshot, ModuleMigration } from "./types"

/**
 * What DeckPilot's functionality is made of: core functionality (the
 * built-in modules, always there and configured in Settings › Modules),
 * system plugins bundled with DeckPilot (origin `bundled`), the optional
 * plugins installed here, and the registry's plugins that are not installed.
 */
function Classification({ snapshot }: { snapshot: MarketplaceSnapshot }) {
  const { t } = useTranslation()
  const { modules } = useAppContext()
  const classes = classifyPlugins(snapshot, modules)
  const rows: { title: string; text: string }[] = [
    {
      title: t("marketplace:classes.core.title"),
      text:
        classes.core.length > 0
          ? t("marketplace:classes.core.list", {
              modules: classes.core.map((m) => m.name).join(", "),
            })
          : t("marketplace:classes.core.none"),
    },
    ...(classes.bundled.length > 0
      ? [
          {
            title: t("marketplace:classes.bundled.title"),
            text: t("marketplace:classes.bundled.text", { count: classes.bundled.length }),
          },
        ]
      : []),
    {
      title: t("marketplace:classes.installed.title"),
      text:
        classes.installed.length === 0
          ? t("marketplace:classes.installed.none")
          : t("marketplace:classes.installed.text", { count: classes.installed.length }),
    },
    {
      title: t("marketplace:classes.available.title"),
      text:
        classes.available.length === 0
          ? t("marketplace:classes.available.none")
          : t("marketplace:classes.available.text", { count: classes.available.length }),
    },
  ]
  return (
    <section aria-labelledby="diagnostics-classes" className="grid gap-2">
      <h3 id="diagnostics-classes" className="text-sm font-medium">
        {t("marketplace:classes.label")}
      </h3>
      <dl className="grid gap-x-6 gap-y-1 border-y py-2.5 text-caption sm:grid-cols-[max-content_1fr]">
        {rows.map((row) => (
          <div key={row.title} className="contents">
            <dt className="text-muted-foreground">{row.title}</dt>
            <dd>{row.text}</dd>
          </div>
        ))}
      </dl>
    </section>
  )
}

const FRESHNESS = {
  fresh: "marketplace:diagnostics.freshness.fresh",
  stale: "marketplace:diagnostics.freshness.stale",
  neverVerified: "marketplace:diagnostics.freshness.neverVerified",
} as const

/** The registry this computer reads, as the agent verified it. */
function RegistryFacts({
  snapshot,
  actions,
}: {
  snapshot: MarketplaceSnapshot
  actions: MarketplaceActions
}) {
  const { t } = useTranslation()
  const registry = snapshot.registry
  const facts: [string, string][] = registry
    ? [
        [t("marketplace:diagnostics.source"), registry.source],
        [t("marketplace:diagnostics.state"), t(FRESHNESS[registry.freshness])],
        ...(registry.seq !== undefined
          ? [[t("marketplace:diagnostics.seq"), String(registry.seq)] as [string, string]]
          : []),
        ...(registry.expires
          ? [
              [
                t("marketplace:diagnostics.expires"),
                formatTime(Date.parse(registry.expires) / 1000),
              ] as [string, string],
            ]
          : []),
        ...(registry.offline
          ? [[t("marketplace:diagnostics.offline"), registry.offline] as [string, string]]
          : []),
        ...(snapshot.registryError
          ? [
              [t("marketplace:diagnostics.error"), snapshot.registryError.message] as [
                string,
                string,
              ],
            ]
          : []),
        [
          t("marketplace:diagnostics.platform"),
          `${snapshot.runtime.platform}${(snapshot.runtime.features ?? []).length > 0 ? ` · ${(snapshot.runtime.features ?? []).join(", ")}` : ""}`,
        ],
      ]
    : []
  return (
    <section aria-labelledby="diagnostics-registry" className="grid gap-2">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h3 id="diagnostics-registry" className="text-sm font-medium">
          {t("marketplace:diagnostics.registry")}
        </h3>
        <Button
          variant="ghost"
          size="sm"
          disabled={actions.busy !== null}
          onClick={actions.refresh}
        >
          <RefreshCw /> {t("marketplace:registry.check")}
        </Button>
      </div>
      {registry ? (
        <dl className="grid grid-cols-[max-content_1fr] gap-x-6 gap-y-1 border-y py-2.5 text-sm">
          {facts.map(([term, value]) => (
            <div key={term} className="contents">
              <dt className="text-muted-foreground">{term}</dt>
              <dd className="font-mono text-xs break-all select-text">{value}</dd>
            </div>
          ))}
        </dl>
      ) : (
        <p className="text-sm text-muted-foreground">{t("marketplace:registry.none.text")}</p>
      )}
    </section>
  )
}

export function DiagnosticsView({
  snapshot,
  migrations,
  actions,
  onReviewMigration,
  onRetryMigration,
}: {
  snapshot: MarketplaceSnapshot
  migrations: ModuleMigration[]
  actions: MarketplaceActions
  onReviewMigration: (migration: ModuleMigration) => void
  onRetryMigration: (migration: ModuleMigration) => void
}) {
  const { t } = useTranslation()
  return (
    <div className="grid gap-8">
      <p className="text-sm text-muted-foreground">{t("marketplace:diagnostics.intro")}</p>
      {migrations.length > 0 ? (
        <MigrationNotice
          migrations={migrations}
          busy={actions.busy}
          onReview={onReviewMigration}
          onRetry={onRetryMigration}
        />
      ) : (
        <section aria-labelledby="diagnostics-no-migrations" className="grid gap-1">
          <h3 id="diagnostics-no-migrations" className="text-sm font-medium">
            {t("plugins:migration.notice.title")}
          </h3>
          <p className="text-sm text-muted-foreground">
            {t("marketplace:diagnostics.noMigrations")}
          </p>
        </section>
      )}
      <Classification snapshot={snapshot} />
      <RegistryFacts snapshot={snapshot} actions={actions} />
    </div>
  )
}
