// The Marketplace's storefront (state and hooks; the provider is
// storefront-provider.tsx): which client answers, the editorial view
// for the registry snapshot shown, and each plugin's listing. One
// `StorefrontProvider` around the Marketplace loads the view once for the
// sidebar (collections), Discover (sections) and a plugin's page.
//
// Which client: the dev build's fixture (`VITE_STOREFRONT_FIXTURE`), else,
// only when the agent has the `storefront` feature, the one of a
// surrounding `StorefrontClientContext` (tests) or the Tauri commands.
// Without the feature every answer is "nothing" (catalog-only Discover).

import { createContext, useContext, useEffect, useState } from "react"

import {
  createTauriStorefrontClient,
  hasStorefrontFeature,
  NO_STOREFRONT,
  type StorefrontClient,
} from "../storefront-api"
import type { ListingView, StorefrontView } from "../storefront-types"
import type { PluginRuntimeStatus } from "../types"
import { devStorefrontClient } from "./dev-fixture"
import type { CatalogIndex, ResolvedCollection, ResolvedSection } from "./resolve"

export interface StorefrontState {
  /** The client to ask (listings, media); `NO_STOREFRONT` when there is none. */
  client: StorefrontClient
  /** `loading` until the view is known; then `ready` (with or without one). */
  status: "loading" | "ready"
  /** The agent's view; `null`: catalog-only. */
  view: StorefrontView | null
  /** What Discover shows, resolved against the catalog. Empty: catalog-only. */
  sections: ResolvedSection[]
  /** The sidebar's editorial collections; empty: the built-in ones. */
  collections: ResolvedCollection[]
  index: CatalogIndex
}

export const NONE: StorefrontState = {
  client: NO_STOREFRONT,
  status: "ready",
  view: null,
  sections: [],
  collections: [],
  index: new Map(),
}

export const StorefrontStateContext = createContext<StorefrontState>(NONE)

let tauriClient: StorefrontClient | undefined

/** The client for an agent with `runtime`'s features. */
export function storefrontClientFor(
  runtime: Pick<PluginRuntimeStatus, "features"> | null | undefined,
  provided: StorefrontClient,
): StorefrontClient {
  const dev = devStorefrontClient()
  if (dev) return dev
  if (!hasStorefrontFeature(runtime)) return NO_STOREFRONT
  if (provided !== NO_STOREFRONT) return provided
  return (tauriClient ??= createTauriStorefrontClient())
}

export function isView(value: unknown): value is StorefrontView {
  return (
    typeof value === "object" && value !== null && Array.isArray((value as StorefrontView).sections)
  )
}

export function useStorefront(): StorefrontState {
  return useContext(StorefrontStateContext)
}

/** A plugin's listing, `null` while loading and when it has none. */
export function useListing(plugin: string): ListingView | null {
  const { client } = useStorefront()
  const [loaded, setLoaded] = useState<{
    client: StorefrontClient
    plugin: string
    listing: ListingView | null
  } | null>(null)
  useEffect(() => {
    if (client === NO_STOREFRONT) return
    let live = true
    client.getRegistryListing(plugin).then(
      (listing) => {
        if (live) setLoaded({ client, plugin, listing: isListing(listing) ? listing : null })
      },
      () => {
        if (live) setLoaded({ client, plugin, listing: null })
      },
    )
    return () => {
      live = false
    }
  }, [client, plugin])
  return loaded && loaded.client === client && loaded.plugin === plugin ? loaded.listing : null
}

/** The outer shape of a listing; the components cope with anything inside. */
function isListing(value: unknown): value is ListingView {
  if (typeof value !== "object" || value === null) return false
  const l = value as Record<string, unknown>
  return (
    Array.isArray(l.highlights) &&
    Array.isArray(l.screenshots) &&
    Array.isArray(l.releaseNotes) &&
    typeof l.links === "object" &&
    l.links !== null
  )
}
