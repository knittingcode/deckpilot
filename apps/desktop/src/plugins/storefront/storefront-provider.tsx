// One provider around the Marketplace loads the storefront view once for
// the sidebar (collections), Discover (sections) and a plugin's page, for
// the registry snapshot shown (see use-storefront.ts).

import { type ReactNode, useContext, useEffect, useMemo, useState } from "react"

import { NO_STOREFRONT, type StorefrontClient, StorefrontClientContext } from "../storefront-api"
import type { StorefrontView } from "../storefront-types"
import type { AvailablePlugin, PluginRuntimeStatus } from "../types"
import { catalogIndex, resolveCollections, resolveSections } from "./resolve"
import {
  isView,
  NONE,
  type StorefrontState,
  StorefrontStateContext,
  storefrontClientFor,
} from "./use-storefront"

export function StorefrontProvider({
  runtime,
  available,
  seq,
  children,
}: {
  runtime: Pick<PluginRuntimeStatus, "features">
  /** The catalog every placement is checked against. */
  available: AvailablePlugin[]
  /** The registry snapshot shown: a new one loads the view again. */
  seq: number | undefined
  children: ReactNode
}) {
  const provided = useContext(StorefrontClientContext)
  const client = storefrontClientFor(runtime, provided)
  const [loaded, setLoaded] = useState<{
    client: StorefrontClient
    seq: number | undefined
    view: StorefrontView | null
  } | null>(null)

  useEffect(() => {
    if (client === NO_STOREFRONT) return
    let live = true
    client.getStorefront().then(
      (view) => {
        if (live) setLoaded({ client, seq, view: isView(view) ? view : null })
      },
      () => {
        // A transport failure: catalog-only, like no storefront.
        if (live) setLoaded({ client, seq, view: null })
      },
    )
    return () => {
      live = false
    }
  }, [client, seq])

  const current = loaded && loaded.client === client && loaded.seq === seq ? loaded : null
  const state = useMemo<StorefrontState>(() => {
    const index = catalogIndex(available)
    if (client === NO_STOREFRONT) return { ...NONE, index }
    if (!current) return { ...NONE, client, status: "loading", index }
    const collections = resolveCollections(current.view, index)
    return {
      client,
      status: "ready",
      view: current.view,
      sections: resolveSections(current.view, index, collections),
      collections,
      index,
    }
  }, [available, client, current])

  return <StorefrontStateContext value={state}>{children}</StorefrontStateContext>
}
