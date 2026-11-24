import { describe, expect, it } from "vitest"

import { storefrontCatalog } from "@/dev/storefront-catalog"
import { randomStorefront } from "@/test/storefront"

import { hostileStorefront, MEDIA, storefrontFixture } from "../storefront-mocks"
import type { SectionView, StorefrontView } from "../storefront-types"
import type { AvailablePlugin } from "../types"
import {
  CAPS,
  catalogIndex,
  editorialCollectionId,
  editorialSection,
  isEditorialSection,
  noteFor,
  type ResolvedSection,
  resolveCollections,
  resolveSections,
  validMediaRef,
} from "./resolve"

const section = (s: Partial<SectionView> & Pick<SectionView, "id" | "kind">): SectionView => ({
  items: [],
  plugins: [],
  categories: [],
  ...s,
})

const view = (sections: SectionView[], collections: StorefrontView["collections"] = []) => ({
  seq: 1,
  sections,
  collections,
})

function pluginsOf(resolved: ResolvedSection[]): AvailablePlugin[] {
  return resolved.flatMap((s) =>
    s.kind === "collectionRow" ? s.plugins : s.kind === "spotlight" ? [s.plugin] : [],
  )
}

describe("storefront resolution", () => {
  const index = catalogIndex(storefrontCatalog())

  it("keeps the agent's order and the kinds it knows", () => {
    const resolved = resolveSections(storefrontFixture, index)
    expect(resolved.map((s) => s.kind)).toEqual([
      "bannerCarousel",
      "collectionRow",
      "spotlight",
      "collectionRow",
      "recentlyAdded",
      "categoryHighlights",
      "catalog",
    ])
  })

  it("skips a section of a kind it does not know", () => {
    const resolved = resolveSections(
      view([
        section({ id: "future", kind: "hologram", plugins: ["acme.clock"] }),
        section({ id: "all", kind: "catalog" }),
      ]),
      index,
    )
    expect(resolved.map((s) => s.id)).toEqual(["all"])
    expect(resolveSections(hostileStorefront, index).map((s) => s.id)).not.toContain("future")
  })

  it("drops an editorial-only id the catalog does not have, and repeats", () => {
    const [focus] = resolveSections(
      view([
        section({
          id: "focus",
          kind: "collectionRow",
          title: "Stay focused",
          plugins: ["deckpilot.pomodoro", "deckpilot.break-reminder", "acme.clock", "acme.clock"],
        }),
      ]),
      index,
    )
    expect(focus?.kind === "collectionRow" && focus.plugins.map((p) => p.id)).toEqual([
      "deckpilot.pomodoro",
      "acme.clock",
    ])
    // A row with nothing placeable is no row at all.
    expect(
      resolveSections(
        view([section({ id: "x", kind: "collectionRow", title: "X", plugins: ["acme.unknown"] })]),
        index,
      ),
    ).toEqual([])
  })

  it("drops a plugin that runs nowhere here and a spotlight version that is not published", () => {
    const resolved = resolveSections(
      view([
        section({
          id: "a",
          kind: "collectionRow",
          title: "A",
          plugins: ["acme.future", "acme.clock"],
        }),
        section({ id: "b", kind: "spotlight", plugins: ["deckpilot.weather"], version: "9.9.9" }),
        section({ id: "c", kind: "spotlight", plugins: ["acme.future"] }),
      ]),
      index,
    )
    expect(resolved.map((s) => s.id)).toEqual(["a", "b"])
    expect(resolved[0]?.kind === "collectionRow" && resolved[0].plugins.map((p) => p.id)).toEqual([
      "acme.clock",
    ])
    expect(resolved[1]?.kind === "spotlight" && resolved[1].version).toBeUndefined()
  })

  it("drops banners whose targets lead nowhere (the hostile carousel entirely)", () => {
    expect(resolveSections(hostileStorefront, index).map((s) => s.id)).toEqual([
      "dupes",
      "badlayout",
      "limit",
    ])
    const [banners] = resolveSections(storefrontFixture, index)
    // `productivity` is a collection, `monitoring` a category of the catalog.
    expect(banners?.kind === "bannerCarousel" && banners.items.map((i) => i.id)).toEqual([
      "weather-banner",
      "focus-banner",
      "monitoring-banner",
    ])
  })

  it("clamps the hostile limits and layouts and dedupes ids", () => {
    const resolved = resolveSections(hostileStorefront, index)
    const dupes = resolved.find((s) => s.id === "dupes")
    expect(dupes?.kind === "collectionRow" && dupes.plugins.map((p) => p.id)).toEqual([
      "acme.clock",
    ])
    const bad = resolved.find((s) => s.id === "badlayout")
    expect(bad?.kind === "collectionRow" && bad.layout).toBe("cards")
    const limit = resolved.find((s) => s.id === "limit")
    expect(limit?.kind === "recentlyAdded" && limit.limit).toBe(CAPS.recentMax)
    const twice = resolveSections(
      view([section({ id: "all", kind: "catalog" }), section({ id: "all", kind: "catalog" })]),
      index,
    )
    expect(twice).toHaveLength(1)
  })

  it("resolves the sidebar's collections, none empty, `__proto__` a plain id", () => {
    expect(resolveCollections(storefrontFixture, index).map((c) => c.id)).toEqual([
      "productivity",
      "dev",
    ])
    const hostile = resolveCollections(hostileStorefront, index)
    expect(hostile.map((c) => c.id)).toEqual(["__proto__"])
    expect(hostile[0]?.title).toBe("<b>bold</b>")
    expect(resolveCollections(null, index)).toEqual([])
    expect(isEditorialSection(editorialSection("__proto__"))).toBe(true)
    expect(editorialCollectionId(editorialSection("a:b"))).toBe("a:b")
    expect(isEditorialSection("code")).toBe(false)
  })

  it("accepts only valid media references", () => {
    expect(validMediaRef(MEDIA.weatherIcon)).toBe(true)
    expect(validMediaRef({ ...MEDIA.weatherIcon, mediaType: "image/svg+xml" })).toBe(false)
    expect(validMediaRef({ ...MEDIA.weatherIcon, sha256: "../x" })).toBe(false)
    expect(validMediaRef({ ...MEDIA.weatherIcon, width: 0 })).toBe(false)
    expect(validMediaRef({ ...MEDIA.weatherIcon, height: Number.NaN })).toBe(false)
    expect(validMediaRef(null)).toBe(false)
  })

  it("finds a version's release note", () => {
    const notes = [{ version: "0.2.1", text: "Fixes." }]
    expect(noteFor(notes, "0.2.1")?.text).toBe("Fixes.")
    expect(noteFor(notes, "0.2.0")).toBeUndefined()
    expect(noteFor(undefined, "0.2.1")).toBeUndefined()
  })
})

// ------------------------------------------------------------- property test

/** A small deterministic PRNG (mulberry32), so failures reproduce. */
function rng(seed: number) {
  let a = seed
  return () => {
    a |= 0
    a = (a + 0x6d2b79f5) | 0
    let t = Math.imul(a ^ (a >>> 15), 1 | a)
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

describe("editorial cannot alter the catalog (property)", () => {
  it("never changes, adds or invents a plugin, whatever the editorial says", () => {
    for (let seed = 1; seed <= 300; seed++) {
      const catalog = storefrontCatalog()
      const before = structuredClone(catalog)
      const index = catalogIndex(catalog)
      const storefront = randomStorefront(rng(seed))
      const collections = resolveCollections(storefront, index)
      const sections = resolveSections(storefront, index, collections)
      // The catalog is untouched.
      expect(catalog, `seed ${String(seed)}`).toEqual(before)
      // Every placed plugin is the catalog's own object, never a copy or a mix.
      for (const p of [...pluginsOf(sections), ...collections.flatMap((c) => c.plugins)]) {
        expect(index.get(p.id), `seed ${String(seed)}`).toBe(p)
        expect(p.id).not.toBe("acme.future")
        expect(p.id).not.toBe("acme.unknown")
      }
      // Banners lead only to what exists.
      for (const s of sections) {
        if (s.kind !== "bannerCarousel") continue
        for (const item of s.items) {
          if (item.target.type === "plugin") expect(index.has(item.target.id)).toBe(true)
          if (item.target.type === "collection")
            expect(collections.some((c) => c.id === item.target.id)).toBe(true)
        }
      }
      expect(sections.length).toBeLessThanOrEqual(CAPS.sections)
      expect(sections.map((s) => s.kind)).not.toContain("hologram")
    }
  })
})
