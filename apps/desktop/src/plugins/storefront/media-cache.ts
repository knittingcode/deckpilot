// The in-memory cache of verified storefront images (data URLs by sha256)
// and the one place they are requested and checked.

import { isStorefrontMediaDataUrl, type StorefrontClient } from "../storefront-api"
import type { MediaRef } from "../storefront-types"

/** How much decoded `data:` text the cache keeps (the design's 32 MiB). */
export const MEDIA_CACHE_BYTES = 32 * 1024 * 1024

/** A least-recently-used cache of data URLs by sha256, bounded by their total length. */
export class MediaCache {
  private entries = new Map<string, string>()
  private total = 0
  readonly budget: number

  constructor(budget: number = MEDIA_CACHE_BYTES) {
    this.budget = budget
  }

  get(key: string): string | undefined {
    const value = this.entries.get(key)
    if (value === undefined) return undefined
    // Most recently used last.
    this.entries.delete(key)
    this.entries.set(key, value)
    return value
  }

  set(key: string, value: string): void {
    if (value.length > this.budget) return
    const old = this.entries.get(key)
    if (old !== undefined) {
      this.total -= old.length
      this.entries.delete(key)
    }
    this.entries.set(key, value)
    this.total += value.length
    for (const [k, v] of this.entries) {
      if (this.total <= this.budget) break
      this.entries.delete(k)
      this.total -= v.length
    }
  }

  get bytes(): number {
    return this.total
  }

  get size(): number {
    return this.entries.size
  }

  clear(): void {
    this.entries.clear()
    this.total = 0
  }
}

/** The app-wide cache. */
export const mediaCache = new MediaCache()
const inFlight = new Map<string, Promise<string | null>>()

/** How many failed lookups are remembered (oldest forgotten first). */
export const MEDIA_FAILURES_MAX = 256

/**
 * Images the backend definitively would not show (no image, or an answer
 * that is not an acceptable data URL), so a placeholder is not
 * re-requested on every mount. A transport or IPC failure is not
 * remembered (the next mount tries again). Forgotten when the storefront
 * client or the registry snapshot (`seq`) changes: a new snapshot may
 * deliver them.
 */
const failures = new Set<string>()
let failuresOf: { client: StorefrontClient; seq: number | undefined } | null = null

function sameScope(client: StorefrontClient, seq: number | undefined): boolean {
  return failuresOf?.client === client && failuresOf.seq === seq
}

function rememberFailure(client: StorefrontClient, seq: number | undefined, sha256: string): void {
  if (!sameScope(client, seq)) {
    failures.clear()
    failuresOf = { client, seq }
  }
  failures.add(sha256)
  if (failures.size > MEDIA_FAILURES_MAX) {
    const oldest = failures.values().next().value
    if (oldest !== undefined) failures.delete(oldest)
  }
}

/** Empties the in-memory caches (tests). */
export function clearMediaCache(): void {
  mediaCache.clear()
  inFlight.clear()
  failures.clear()
  failuresOf = null
}

/**
 * Whether `url` may be rendered for `ref`: an allowed base64 `data:` URL
 * whose media type is the one the reference declares.
 */
export function acceptableMediaUrl(url: unknown, ref: MediaRef): url is string {
  return isStorefrontMediaDataUrl(url) && url.startsWith(`data:${ref.mediaType};base64,`)
}

/** The data URL of `ref`, from the cache or the client; `null` when it cannot be shown. */
export function loadMedia(
  client: StorefrontClient,
  ref: MediaRef,
  /** The registry snapshot shown; a new one retries what failed before. */
  seq?: number,
): Promise<string | null> {
  const cached = mediaCache.get(ref.sha256)
  if (cached !== undefined) return Promise.resolve(cached)
  if (sameScope(client, seq) && failures.has(ref.sha256)) return Promise.resolve(null)
  const pending = inFlight.get(ref.sha256)
  if (pending) return pending
  const request = client
    .getRegistryMedia(ref.sha256)
    .then(
      (url) => {
        if (!acceptableMediaUrl(url, ref)) {
          rememberFailure(client, seq, ref.sha256)
          return null
        }
        mediaCache.set(ref.sha256, url)
        return url
      },
      // A transport failure: not remembered.
      () => null,
    )
    .finally(() => {
      inFlight.delete(ref.sha256)
    })
  inFlight.set(ref.sha256, request)
  return request
}
