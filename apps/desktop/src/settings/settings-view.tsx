import { Page } from "@/components/page"
import { SettingsPage } from "@/components/settings-layout"
import { useTranslation } from "@/i18n"
import { useAppContext } from "@/lib/app-context"
import { DEFAULT_SETTINGS_PAGE } from "@/lib/navigation"
import { ModuleSettingsForm } from "@/settings/pages/module-settings"
import { categoryLabel, findSettingsPage } from "@/settings/registry"

/** The settings page `id`; the navigation only opens pages that exist. */
export function SettingsView({ id }: { id: string }) {
  const { t } = useTranslation()
  const { modules, openSettings } = useAppContext()
  const page = findSettingsPage(modules, id) ?? findSettingsPage(modules, DEFAULT_SETTINGS_PAGE)
  if (!page) return null
  const View = page.view
  return (
    <Page
      // The page shows its title itself (SettingsPage); the toolbar shows
      // only where it is, so the title is not there twice.
      title=""
      breadcrumbs={[
        {
          label: t("settings:title"),
          onSelect: () => {
            openSettings(DEFAULT_SETTINGS_PAGE)
          },
        },
        { label: categoryLabel(page.category) },
      ]}
    >
      <SettingsPage title={page.title} description={page.description}>
        {View ? (
          <View />
        ) : (
          page.module && <ModuleSettingsForm key={page.module.id} module={page.module} />
        )}
      </SettingsPage>
    </Page>
  )
}
