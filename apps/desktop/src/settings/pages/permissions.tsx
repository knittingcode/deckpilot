import { lazy, Suspense } from "react"

import { Loading } from "@/components/page"
import {
  InlineStatus,
  SettingsGroup,
  SettingsRow,
  SettingsSection,
} from "@/components/settings-layout"
import { useAppContext } from "@/lib/app-context"
import { CAPABILITIES } from "@/lib/capabilities"
import type { Capability } from "@/lib/types"
import { Trans, useTranslation } from "@/i18n"

/**
 * Capabilities declared in module manifests. They are fixed by each
 * module; disabling a module withdraws all of them.
 */
export function PermissionsSettings() {
  const { t } = useTranslation()
  const { modules, openSettings } = useAppContext()
  const capabilities = Object.keys(CAPABILITIES) as Capability[]
  return (
    <div className="grid gap-8">
      <SettingsSection
        title={t("settings:permissions.modulesTitle")}
        description={
          <Trans
            i18nKey="settings:permissions.intro"
            components={{
              page: (
                <button
                  type="button"
                  className="text-primary underline-offset-2 hover:underline"
                  onClick={() => {
                    openSettings("modules")
                  }}
                />
              ),
            }}
          />
        }
      >
        <SettingsGroup>
          {capabilities.map((capability) => {
            const users = modules.filter((m) => m.capabilities.includes(capability))
            return (
              <SettingsRow
                key={capability}
                align="start"
                label={t(CAPABILITIES[capability].title)}
                description={t(CAPABILITIES[capability].description)}
              >
                {users.length === 0 ? (
                  <p className="w-44 text-caption text-muted-foreground">
                    {t("settings:permissions.unused")}
                  </p>
                ) : (
                  <ul
                    className="grid w-44 gap-1 text-sm"
                    aria-label={t(CAPABILITIES[capability].title)}
                  >
                    {users.map((module) => (
                      <li key={module.id} className="flex items-center justify-between gap-3">
                        <span className="truncate">{module.name}</span>
                        <InlineStatus tone={module.enabled ? "positive" : "neutral"}>
                          {module.enabled
                            ? t("settings:permissions.granted")
                            : t("settings:permissions.disabled")}
                        </InlineStatus>
                      </li>
                    ))}
                  </ul>
                )}
              </SettingsRow>
            )
          })}
        </SettingsGroup>
      </SettingsSection>
      {/* Builds with plugins: every plugin's grants, from the same data as its Marketplace page. */}
      {import.meta.env.VITE_DECKPILOT_PLUGINS === "1" && (
        <SettingsSection
          title={t("settings:permissions.pluginsTitle")}
          description={t("settings:permissions.pluginsIntro")}
        >
          <Suspense fallback={<Loading />}>
            <PluginPermissions />
          </Suspense>
        </SettingsSection>
      )}
    </div>
  )
}

/** Loaded only when it renders, so builds without plugins never load plugin code. */
const PluginPermissions = lazy(() =>
  import("@/plugins/marketplace").then((m) => ({ default: m.PluginPermissions })),
)
