// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { withStorefrontClient } from "@/test/storefront"

import type { StorefrontClient } from "../storefront-api"
import {
  createFixtureStorefrontClient,
  HOSTILE_MEDIA,
  hostileFixture,
  MEDIA,
} from "../storefront-mocks"
import type { MediaRef } from "../storefront-types"
import {
  acceptableMediaUrl,
  clearMediaCache,
  loadMedia,
  MEDIA_FAILURES_MAX,
  MediaCache,
} from "./media-cache"
import { VerifiedImage } from "./verified-image"

beforeEach(clearMediaCache)
afterEach(cleanup)

const PNG = "data:image/png;base64,iVBORw0KGgo="

/** A client answering `url` for every image; `media` spies on the requests. */
function clientAnswering(url: unknown) {
  const media = vi.fn<(sha256: string) => Promise<string | null>>(() =>
    Promise.resolve(url as string | null),
  )
  const client: StorefrontClient = {
    getStorefront: () => Promise.resolve(null),
    getRegistryListing: () => Promise.resolve(null),
    getRegistryMedia: media,
  }
  return Object.assign(client, { media })
}

function renderImage(client: StorefrontClient, media: MediaRef | undefined = MEDIA.weatherIcon) {
  const { container } = render(
    withStorefrontClient(
      client,
      <VerifiedImage media={media} alt="Weather icon" fallback={<span>W</span>} />,
    ),
  )
  return container.querySelector<HTMLElement>('[data-slot="verified-image"]') as HTMLElement
}

describe("verified image", () => {
  it("accepts only base64 PNG, WebP and JPEG data URLs of the declared type", () => {
    const png = MEDIA.weatherIcon
    expect(acceptableMediaUrl(PNG, png)).toBe(true)
    expect(acceptableMediaUrl("data:image/webp;base64,UklGRg==", png)).toBe(false)
    for (const url of Object.values(hostileFixture.media)) {
      expect(acceptableMediaUrl(url, png)).toBe(false)
    }
    for (const url of [
      "data:image/svg+xml;base64,PHN2Zz4=",
      "https://evil.example/x.png",
      "data:image/gif;base64,R0lGODlh",
      "blob:http://localhost/x",
      "javascript:alert(1)",
      `${PNG}"onerror="alert(1)`,
      null,
    ]) {
      expect(acceptableMediaUrl(url, png)).toBe(false)
    }
  })

  it("shows a verified image with its alternative, in a box of the reference's aspect", async () => {
    const box = renderImage(createFixtureStorefrontClient(), MEDIA.forecastShot)
    expect(box.style.aspectRatio).toMatch(/^1\.6( \/ 1)?$/)
    expect(box).toHaveAttribute("data-state", "loading")
    const img = await screen.findByRole("img", { name: "Weather icon" })
    await vi.waitFor(() => {
      expect(box).toHaveAttribute("data-state", "loaded")
    })
    expect(box.querySelector("img")).toHaveAttribute(
      "src",
      expect.stringMatching(/^data:image\/webp;base64,/),
    )
    expect(img).toBeInTheDocument()
  })

  it("never renders what a compromised backend sends", async () => {
    for (const ref of Object.values(HOSTILE_MEDIA)) {
      clearMediaCache()
      const raw = createFixtureStorefrontClient(hostileFixture, { unfilteredMedia: true })
      const box = renderImage(raw, ref)
      await vi.waitFor(() => {
        expect(box).toHaveAttribute("data-state", "fallback")
      })
      expect(box.querySelector("img")).toBeNull()
      expect(box).toHaveTextContent("W")
      // The huge claim does not stretch the box.
      expect(parseFloat(box.style.aspectRatio)).toBeLessThanOrEqual(6)
      cleanup()
    }
    // A type other than the reference's is refused too.
    const box = renderImage(clientAnswering("data:image/webp;base64,UklGRg=="))
    await vi.waitFor(() => {
      expect(box).toHaveAttribute("data-state", "fallback")
    })
  })

  it("shows the fallback for an invalid reference without asking", () => {
    const client = clientAnswering(PNG)
    const box = renderImage(client, { ...MEDIA.weatherIcon, sha256: "../../etc/passwd" })
    expect(box).toHaveAttribute("data-state", "fallback")
    expect(client.media).not.toHaveBeenCalled()
  })

  it("asks once per image and keeps it in a bounded LRU cache", async () => {
    const client = clientAnswering(PNG)
    await Promise.all([loadMedia(client, MEDIA.weatherIcon), loadMedia(client, MEDIA.weatherIcon)])
    await loadMedia(client, MEDIA.weatherIcon)
    expect(client.media).toHaveBeenCalledTimes(1)
    // A refused or missing image is remembered for the session: not asked again.
    const bad = clientAnswering("https://evil.example/x.png")
    expect(await loadMedia(bad, MEDIA.settingsShot)).toBeNull()
    expect(await loadMedia(bad, MEDIA.settingsShot)).toBeNull()
    expect(bad.media).toHaveBeenCalledTimes(1)
    // No image at all is definitive too.
    const none = clientAnswering(null)
    await loadMedia(none, MEDIA.focusBanner, 42)
    await loadMedia(none, MEDIA.focusBanner, 42)
    expect(none.media).toHaveBeenCalledTimes(1)
    // A new registry snapshot (seq) with the same client: asked again.
    await loadMedia(none, MEDIA.focusBanner, 43)
    expect(none.media).toHaveBeenCalledTimes(2)
    await loadMedia(none, MEDIA.focusBanner, 43)
    expect(none.media).toHaveBeenCalledTimes(2)
    // Another client may have it: asked again.
    const other = clientAnswering(null)
    await loadMedia(other, MEDIA.focusBanner, 43)
    expect(other.media).toHaveBeenCalledTimes(1)
    // A transport failure is not remembered: the next mount tries again.
    const failing = clientAnswering(null)
    failing.media.mockImplementation(() => Promise.reject(new Error("transport")))
    expect(await loadMedia(failing, MEDIA.launchBanner, 43)).toBeNull()
    expect(await loadMedia(failing, MEDIA.launchBanner, 43)).toBeNull()
    expect(failing.media).toHaveBeenCalledTimes(2)

    const cache = new MediaCache(10)
    cache.set("a", "1234")
    cache.set("b", "1234")
    cache.get("a")
    cache.set("c", "1234")
    // `b` was least recently used.
    expect([cache.get("a"), cache.get("b"), cache.get("c")]).toEqual(["1234", undefined, "1234"])
    expect(cache.bytes).toBe(8)
    cache.set("huge", "x".repeat(11))
    expect(cache.get("huge")).toBeUndefined()
    expect(new MediaCache().budget).toBe(32 * 1024 * 1024)
  })

  it("remembers at most a bounded number of failures, oldest forgotten first", async () => {
    const client = clientAnswering(null)
    const ref = (n: number) => ({
      ...MEDIA.weatherIcon,
      sha256: (0x1000 + n).toString(16).padStart(64, "0"),
    })
    for (let n = 0; n <= MEDIA_FAILURES_MAX; n++) await loadMedia(client, ref(n))
    expect(client.media).toHaveBeenCalledTimes(MEDIA_FAILURES_MAX + 1)
    // The newest is remembered; the oldest was forgotten and is asked again.
    await loadMedia(client, ref(MEDIA_FAILURES_MAX))
    expect(client.media).toHaveBeenCalledTimes(MEDIA_FAILURES_MAX + 1)
    await loadMedia(client, ref(0))
    expect(client.media).toHaveBeenCalledTimes(MEDIA_FAILURES_MAX + 2)
  })
})
