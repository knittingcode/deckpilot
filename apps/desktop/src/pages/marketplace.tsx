import { useTitlebar } from "@/hooks/use-titlebar"
import { useTranslation } from "@/i18n"
import { PluginsSettings } from "@/plugins/marketplace"

/**
 * The plugin marketplace as a workspace of the main window, opened from
 * the toolbar: the only place plugins are discovered, installed and
 * managed, with its own sidebar beside its content. Only in builds with
 * plugins.
 */
export function MarketplacePage() {
  const { t } = useTranslation()
  useTitlebar({ title: t("marketplace:page.title"), subtitle: t("marketplace:page.description") })
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <PluginsSettings />
    </div>
  )
}
