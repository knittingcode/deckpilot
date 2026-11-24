import { RefreshCw } from "lucide-react"

import { EmptyState } from "@/components/page"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { useTranslation } from "@/i18n"

import type { MarketplaceActions } from "./marketplace"
import { primaryCategory } from "./catalog"
import type { UpdateCandidate } from "./model"
import { PluginAvatar } from "./plugin-tile"
import type { MarketplaceSnapshot } from "./types"

/**
 * Newer versions of installed plugins. The agent compares permissions: an
 * update that asks for more is staged and applied only after consent.
 */
export function UpdatesView({
  updates,
  actions,
  snapshot,
}: {
  updates: UpdateCandidate[]
  actions: MarketplaceActions
  /** For the new versions' release notes and the plugins' icons, when known. */
  snapshot?: MarketplaceSnapshot
}) {
  const { t } = useTranslation()
  return (
    <>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <p className="text-sm text-muted-foreground">{t("marketplace:updates.intro")}</p>
        <Button
          variant="outline"
          size="sm"
          disabled={actions.busy !== null}
          onClick={actions.refresh}
        >
          <RefreshCw /> {t("marketplace:updates.check")}
        </Button>
      </div>
      {actions.blocked && <p className="text-sm text-muted-foreground">{actions.blocked}</p>}
      {updates.length === 0 ? (
        <EmptyState variant="plain" title={t("marketplace:updates.none")}>
          {t("marketplace:updates.noneText")}
        </EmptyState>
      ) : (
        <ul className="grid divide-y border-y" aria-label={t("marketplace:updates.list")}>
          {updates.map((update) => {
            const { summary } = update.plugin
            const to = update.kind === "staged" ? update.pending.version : update.version
            const more =
              update.kind === "staged"
                ? update.pending.changes.added.length + update.pending.changes.broadened.length
                : 0
            const listed = snapshot?.available.find((p) => p.id === summary.id)
            const notes = listed?.versions.find((v) => v.version === to)?.releaseNotes
            const pending = actions.pending.get(summary.id) === "update"
            return (
              <li
                key={summary.id}
                className="flex flex-wrap items-center gap-3 py-3"
                aria-label={t("marketplace:updates.item", {
                  name: summary.name,
                  from: summary.version,
                  to,
                })}
              >
                <PluginAvatar
                  name={summary.name}
                  category={listed ? primaryCategory(listed) : undefined}
                  src={update.plugin.icon}
                  size="md"
                />
                <div className="grid min-w-0 flex-1 gap-1">
                  <span className="text-sm font-medium">{summary.name}</span>
                  <span className="text-xs text-muted-foreground tabular-nums">
                    {summary.version} → {to}
                  </span>
                  {notes && (
                    <p className="mt-1 max-w-prose text-xs whitespace-pre-line">
                      <span className="font-medium">{t("marketplace:details.whatsNew")}: </span>
                      {notes}
                    </p>
                  )}
                </div>
                {update.kind === "staged" && (
                  <Badge variant="outline">
                    {t("marketplace:updates.waiting", { count: more })}
                  </Badge>
                )}
                <Button
                  size="sm"
                  disabled={pending || actions.busy !== null || actions.blocked !== null}
                  onClick={() => {
                    actions.update(update)
                  }}
                >
                  {pending
                    ? t("marketplace:state.updating")
                    : update.kind === "staged"
                      ? t("marketplace:updates.review")
                      : t("marketplace:updates.update")}
                </Button>
              </li>
            )
          })}
        </ul>
      )}
    </>
  )
}
