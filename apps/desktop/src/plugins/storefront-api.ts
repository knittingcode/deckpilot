// The storefront client: the Tauri commands of
// `src-tauri/src/commands/storefront.rs` (storefront design §6).
//
// Every method resolves to `null` rather than failing when there is
// nothing to show: an agent without the `storefront` feature, no
// storefront published, a storefront the agent refused, a plugin without a
// listing, an image that failed verification. The UI then falls back to
// the catalog-only Discover, the monogram, or the placeholder. Only a
// transport failure rejects.

import { invoke } from "@tauri-apps/api/core"
import { createContext, useContext } from "react"

import type { ListingView, StorefrontView } from "./storefront-types"

export interface StorefrontClient {
  /** `getStorefront`: the localized, filtered editorial view; `null`: catalog-only. */
  getStorefront(): Promise<StorefrontView | null>
  /** `getRegistryListing`: one plugin's listing; `null` when it has none. */
  getRegistryListing(plugin: string): Promise<ListingView | null>
  /**
   * `getRegistryMedia`: a verified image as a `data:image/(png|webp|jpeg);base64,`
   * URL (re-checked by the backend and here); `null` when it cannot be shown.
   */
  getRegistryMedia(sha256: string): Promise<string | null>
}

/** 64 lowercase hex digits: the only media key the agent accepts. */
export function isMediaSha256(text: string): boolean {
  return /^[0-9a-f]{64}$/.test(text)
}

const MEDIA_DATA_URL = /^data:image\/(png|webp|jpeg);base64,[A-Za-z0-9+/]+={0,2}$/

/**
 * Whether `url` is a storefront image the webview may load: a base64
 * `data:` URL of PNG, WebP or JPEG and nothing else (no SVG, no http, no
 * parameters). Every image a storefront component renders passes this.
 */
export function isStorefrontMediaDataUrl(url: unknown): url is string {
  return typeof url === "string" && MEDIA_DATA_URL.test(url)
}

/** Whether the agent serves the storefront (`PluginRuntimeStatus.features`). */
export function hasStorefrontFeature(runtime: { features?: string[] } | null | undefined): boolean {
  return runtime?.features?.includes("storefront") === true
}

export function createTauriStorefrontClient(): StorefrontClient {
  return {
    getStorefront: () => invoke<StorefrontView | null>("get_storefront"),
    getRegistryListing: (plugin) => invoke<ListingView | null>("get_registry_listing", { plugin }),
    getRegistryMedia: async (sha256) => {
      if (!isMediaSha256(sha256)) return null
      const url = await invoke<string | null>("get_registry_media", { sha256 })
      return isStorefrontMediaDataUrl(url) ? url : null
    },
  }
}

/** A client whose every answer is "nothing": catalog-only Discover. */
export const NO_STOREFRONT: StorefrontClient = {
  getStorefront: () => Promise.resolve(null),
  getRegistryListing: () => Promise.resolve(null),
  getRegistryMedia: () => Promise.resolve(null),
}

export const StorefrontClientContext = createContext<StorefrontClient>(NO_STOREFRONT)

export function useStorefrontClient(): StorefrontClient {
  return useContext(StorefrontClientContext)
}
