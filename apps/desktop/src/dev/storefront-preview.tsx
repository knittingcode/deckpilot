// Development only: the Marketplace outside the app shell, against the
// storefront fixtures and a fixed catalog, for review in a plain browser:
//
//   pnpm --filter @deckpilot/desktop dev   →   /?showcase=storefront
//   /?showcase=storefront&fixture=hostile   (default | empty | hostile)
//   &lang=pl, &delay=800 (ms per answer, to see placeholders)
//
// No agent, no Tauri: plugin requests answer from `storefrontCatalog()`
// and change nothing (installing says so). Loaded by main.tsx only when
// `import.meta.env.DEV`.

import { useState } from "react"

import { ThemeProvider } from "@/components/theme-provider"
import { SidebarProvider } from "@/components/ui/sidebar"
import { Toaster } from "@/components/ui/sonner"
import { TooltipProvider } from "@/components/ui/tooltip"
import { TitlebarContext } from "@/hooks/use-titlebar"
import { AppContext, type AppContextValue } from "@/lib/app-context"
import { TitlebarStore } from "@/lib/titlebar"
import { type PluginClient, PluginClientContext, PluginError } from "@/plugins/api"
import { PluginsSettings } from "@/plugins/marketplace"
import { StorefrontClientContext } from "@/plugins/storefront-api"
import { fixtureClient } from "@/plugins/storefront/dev-fixture"
import type { PluginRuntimeStatus } from "@/plugins/types"

import { storefrontCatalog } from "./storefront-catalog"

const RUNTIME: PluginRuntimeStatus = {
  available: true,
  built: true,
  enabled: true,
  executionSupported: true,
  platform: "macos",
  registry: { source: "preview", freshness: "fresh", seq: 42 },
  features: ["storefront"],
}

/** A plugin client answering from the fixed catalog; every change is refused. */
function previewClient(): PluginClient {
  const answers: Partial<Record<keyof PluginClient, unknown>> = {
    getRuntime: () => Promise.resolve(RUNTIME),
    listPlugins: () => Promise.resolve([]),
    listKnownPlugins: () => Promise.resolve([]),
    listModuleMigrations: () => Promise.resolve([]),
    listAvailablePlugins: () =>
      Promise.resolve({ registry: RUNTIME.registry, plugins: storefrontCatalog() }),
    subscribe: () => () => undefined,
  }
  return new Proxy({} as PluginClient, {
    get: (_, key: string) =>
      answers[key as keyof PluginClient] ??
      (() =>
        Promise.reject(
          new PluginError("preview", "This is the storefront preview: nothing is installed."),
        )),
  })
}

function appContext(): AppContextValue {
  const noop = () => undefined
  return {
    profiles: { profiles: [], activeProfileId: null },
    editingProfileId: null,
    setEditingProfileId: noop,
    refresh: noop,
    revision: 0,
    agent: null,
    setAgent: noop,
    updates: null,
    setUpdates: noop,
    navigate: noop,
    openSettings: noop,
    closeSettings: noop,
    modules: [],
    setModules: noop,
  }
}

export function StorefrontPreview() {
  const params = new URLSearchParams(window.location.search)
  const [client] = useState(previewClient)
  const [storefront] = useState(() =>
    fixtureClient(params.get("fixture") ?? "default", Number(params.get("delay") ?? 0)),
  )
  const [titlebar] = useState(() => new TitlebarStore())
  const [context] = useState(appContext)
  return (
    <ThemeProvider>
      <AppContext value={context}>
        <TitlebarContext value={titlebar}>
          <TooltipProvider>
            <SidebarProvider className="h-svh min-h-0">
              <PluginClientContext value={client}>
                <StorefrontClientContext value={storefront}>
                  <div className="flex h-svh w-full flex-col bg-background text-foreground">
                    <PluginsSettings />
                  </div>
                </StorefrontClientContext>
              </PluginClientContext>
            </SidebarProvider>
          </TooltipProvider>
        </TitlebarContext>
      </AppContext>
      <Toaster />
    </ThemeProvider>
  )
}
