import { describe, expect, it } from "vitest"

import {
  hasStorefrontFeature,
  isMediaSha256,
  isStorefrontMediaDataUrl,
  NO_STOREFRONT,
} from "./storefront-api"
import {
  createFixtureStorefrontClient,
  defaultFixture,
  emptyFixture,
  fixtureFromJson,
  HOSTILE_MEDIA,
  hostileFixture,
  hostileMedia,
  MEDIA,
  mediaFixtures,
  storefrontFixture,
} from "./storefront-mocks"
import { SECTION_KINDS, STOREFRONT_FEATURE, STOREFRONT_MEDIA_TYPES } from "./storefront-types"

describe("storefront contract", () => {
  it("accepts only base64 PNG, WebP and JPEG data URLs", () => {
    for (const url of Object.values(mediaFixtures)) expect(isStorefrontMediaDataUrl(url)).toBe(true)
    expect(isStorefrontMediaDataUrl("data:image/jpeg;base64,/9j/4AAQ")).toBe(true)
    for (const url of Object.values(hostileMedia)) expect(isStorefrontMediaDataUrl(url)).toBe(false)
    for (const url of [
      "",
      "data:image/png;base64,",
      "data:image/PNG;base64,AAAA",
      "data:image/gif;base64,R0lGODlh",
      "data:image/png,rawbytes",
      "data:image/png;base64,AAAA\n",
      " data:image/png;base64,AAAA",
      "data:image/png;base64,AA AA",
      "blob:http://localhost/x",
      null,
      42,
    ]) {
      expect(isStorefrontMediaDataUrl(url)).toBe(false)
    }
  })

  it("names media by 64 lowercase hex digits only", () => {
    expect(isMediaSha256(MEDIA.weatherIcon.sha256)).toBe(true)
    for (const bad of [
      "",
      "A".repeat(64),
      "a".repeat(63),
      "a".repeat(65),
      "../" + "a".repeat(61),
    ]) {
      expect(isMediaSha256(bad)).toBe(false)
    }
  })

  it("detects the agent feature", () => {
    expect(STOREFRONT_FEATURE).toBe("storefront")
    expect(hasStorefrontFeature({ features: ["pagedAnswers", "storefront"] })).toBe(true)
    expect(hasStorefrontFeature({ features: ["pagedAnswers"] })).toBe(false)
    expect(hasStorefrontFeature({})).toBe(false)
    expect(hasStorefrontFeature(null)).toBe(false)
  })

  it("has fixtures that use only the contract's kinds and media types", () => {
    const storefront = storefrontFixture
    for (const section of storefront.sections) expect(SECTION_KINDS).toContain(section.kind)
    const refs = [
      ...storefront.sections.flatMap((s) => s.items.map((i) => i.artwork)),
      ...Object.values(defaultFixture.listings).flatMap((l) => [
        ...(l.icon ? [l.icon] : []),
        ...l.screenshots.map((s) => s.media),
        ...(l.promoArt ? [l.promoArt.media] : []),
      ]),
    ]
    for (const ref of refs) {
      expect(isMediaSha256(ref.sha256)).toBe(true)
      expect(STOREFRONT_MEDIA_TYPES).toContain(ref.mediaType)
    }
    // One reference is deliberately undelivered (the placeholder).
    expect(refs.filter((r) => !(r.sha256 in defaultFixture.media)).map((r) => r.sha256)).toEqual(
      Array(refs.filter((r) => r.sha256 === MEDIA.missing.sha256).length).fill(
        MEDIA.missing.sha256,
      ),
    )
  })

  it("serves fixtures and filters hostile media like the real client", async () => {
    const client = createFixtureStorefrontClient(hostileFixture)
    expect((await client.getStorefront())?.seq).toBe(43)
    for (const ref of Object.values(HOSTILE_MEDIA))
      expect(await client.getRegistryMedia(ref.sha256)).toBeNull()
    expect(await client.getRegistryListing("__proto__")).toBeNull()
    expect(await client.getRegistryMedia("../etc/passwd")).toBeNull()
    const raw = createFixtureStorefrontClient(hostileFixture, { unfilteredMedia: true })
    expect(await raw.getRegistryMedia(HOSTILE_MEDIA.svg.sha256)).toContain("svg")
    const good = createFixtureStorefrontClient(defaultFixture)
    expect(await good.getRegistryMedia(MEDIA.weatherIcon.sha256)).toMatch(
      /^data:image\/png;base64,/,
    )
    expect(await good.getRegistryMedia(MEDIA.missing.sha256)).toBeNull()
    expect(await createFixtureStorefrontClient(emptyFixture).getStorefront()).toBeNull()
    expect(await NO_STOREFRONT.getStorefront()).toBeNull()
  })

  it("reads a Studio export", () => {
    const fixture = fixtureFromJson(JSON.parse(JSON.stringify(defaultFixture)))
    expect(fixture.storefront?.sections.length).toBe(storefrontFixture.sections.length)
    expect(fixtureFromJson({ storefront: null }).listings).toEqual({})
    expect(() => fixtureFromJson([])).toThrow()
    expect(() => fixtureFromJson({ storefront: { sections: "x" } })).toThrow()
    expect(fixtureFromJson({ media: { a: 1, b: "data:image/png;base64,AAAA" } }).media).toEqual({
      b: "data:image/png;base64,AAAA",
    })
  })
})
