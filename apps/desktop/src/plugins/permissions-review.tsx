// Settings › Permissions, for plugins: every installed plugin's permissions
// as the agent holds them, to review and revoke (or grant) across plugins:
// the only place they are granted and revoked. Each plugin's page in the
// Marketplace shows the same list from the same data, read-only; managing
// anything else stays there.

import { ExternalLink } from "lucide-react"

import { EmptyState } from "@/components/page"
import { Button } from "@/components/ui/button"
import { useTranslation } from "@/i18n"
import { useFormat } from "@/i18n/format"
import { useAppContext } from "@/lib/app-context"

import { pluginDisplayName } from "./catalog"
import { focusPlugin } from "./focus"
import { InstalledPermissions } from "./installed"
import type { MarketplaceActions } from "./marketplace"
import { stateLabel } from "./model"
import { PluginAvatar } from "./plugin-tile"
import type { MarketplaceSnapshot } from "./types"

export function PluginPermissionsReview({
  snapshot,
  actions,
}: {
  snapshot: MarketplaceSnapshot
  actions: MarketplaceActions
}) {
  const { t } = useTranslation()
  const format = useFormat()
  const { navigate } = useAppContext()
  const plugins = [...snapshot.installed].sort((a, b) =>
    format.compare(a.summary.name, b.summary.name),
  )
  if (plugins.length === 0) {
    return <EmptyState variant="plain" title={t("marketplace:installed.empty")} />
  }
  return (
    <ul className="grid gap-6" aria-label={t("marketplace:permissionsReview.list")}>
      {plugins.map((item) => {
        const name = pluginDisplayName(item.summary.id, item.summary.name, snapshot)
        const id = `plugin-permissions-${item.summary.id}`
        return (
          <li key={item.summary.id} aria-labelledby={id} className="grid gap-3">
            <div className="flex flex-wrap items-center gap-3">
              <PluginAvatar name={name} src={item.icon} size="sm" />
              <div className="grid min-w-0 flex-1">
                <span id={id} className="text-sm font-medium">
                  {name}
                </span>
                <span className="text-xs text-muted-foreground">
                  {stateLabel(item.summary.state)}
                </span>
              </div>
              <Button
                variant="ghost"
                size="sm"
                aria-label={t("marketplace:permissionsReview.openNamed", { name })}
                onClick={() => {
                  focusPlugin(item.summary.id)
                  navigate("marketplace")
                }}
              >
                <ExternalLink /> {t("marketplace:permissionsReview.open")}
              </Button>
            </div>
            <InstalledPermissions item={item} actions={actions} />
          </li>
        )
      })}
    </ul>
  )
}
