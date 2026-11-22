// Storefront fixtures and a fixture-backed client (storefront design §6
// and §8): for tests, Storybook-style previews and the dev build's "true
// preview" of a Marketplace Studio export. Plugin ids match the agent's
// test registries (`acme.*`) and DeckPilot's own plugins (`deckpilot.*`).
//
// `hostileStorefront`, `hostileListings` and `hostileMedia` are what a
// broken or malicious pipeline could try to put in front of the UI. The
// agent and the backend refuse most of it before it gets here (raw HTML,
// bidi controls, SVG, non-https links); the UI must still render all of it
// as inert literal text, refuse every image that is not a PNG, WebP or
// JPEG `data:` URL, and skip what it does not know.

import type { StorefrontClient } from "./storefront-api"
import { isMediaSha256, isStorefrontMediaDataUrl } from "./storefront-api"
import type {
  ListingView,
  MediaRef,
  SectionView,
  StorefrontMediaType,
  StorefrontView,
} from "./storefront-types"

/** Everything a fixture client answers. */
export interface StorefrontFixture {
  storefront: StorefrontView | null
  listings: Record<string, ListingView>
  /** sha256 → data URL; a missing sha256 answers `null` (the placeholder). */
  media: Record<string, string>
}

// 1×1 images (real, decodable).
const PNG_1X1 =
  "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=="
const WEBP_1X1 =
  "data:image/webp;base64,UklGRiQAAABXRUJQVlA4IBgAAAAwAQCdASoBAAEAAwA0JaQAA3AA/vuUAAA="

function sha(n: number): string {
  return n.toString(16).padStart(64, "0")
}

function media(n: number, mediaType: StorefrontMediaType, width: number, height: number): MediaRef {
  return { sha256: sha(n), mediaType, width, height }
}

export const MEDIA = {
  weatherIcon: media(1, "image/png", 512, 512),
  pomodoroIcon: media(2, "image/webp", 256, 256),
  launchBanner: media(3, "image/webp", 2400, 800),
  focusBanner: media(4, "image/png", 1800, 600),
  forecastShot: media(5, "image/webp", 2560, 1600),
  settingsShot: media(6, "image/png", 1920, 1080),
  weatherPromo: media(7, "image/jpeg", 1600, 800),
  /** Referenced but never delivered: the placeholder must show. */
  missing: media(8, "image/png", 1280, 800),
} as const

export const storefrontFixture: StorefrontView = {
  seq: 42,
  sections: [
    {
      id: "launch",
      kind: "bannerCarousel",
      items: [
        {
          id: "weather-banner",
          title: "Know before you go",
          body: "Weather and air quality on your keys.",
          cta: "See plugin",
          artwork: MEDIA.launchBanner,
          alt: "Rain clouds over a Stream Deck",
          target: { type: "plugin", id: "deckpilot.weather" },
        },
        {
          id: "focus-banner",
          title: "Stay in the zone",
          artwork: MEDIA.focusBanner,
          alt: "",
          decorative: true,
          target: { type: "collection", id: "productivity" },
        },
        {
          id: "monitoring-banner",
          title: "Watch what matters",
          cta: "Browse",
          artwork: MEDIA.missing,
          alt: "Dashboards on keys",
          target: { type: "category", id: "monitoring" },
        },
      ],
      plugins: [],
      categories: [],
    },
    {
      id: "focus",
      kind: "collectionRow",
      layout: "cards",
      title: "Stay focused",
      description: "Timers and reminders.",
      items: [],
      plugins: ["deckpilot.pomodoro", "deckpilot.break-reminder", "acme.clock"],
      categories: [],
    },
    {
      id: "spot-weather",
      kind: "spotlight",
      title: "New: Weather",
      description: "Forecasts on a key.",
      items: [],
      plugins: ["deckpilot.weather"],
      categories: [],
      version: "0.2.1",
    },
    {
      id: "tools",
      kind: "collectionRow",
      layout: "compact",
      title: "Developer tools",
      items: [],
      plugins: ["acme.clock", "acme.github-prs"],
      categories: [],
    },
    {
      id: "new",
      kind: "recentlyAdded",
      title: "Recently added",
      items: [],
      plugins: [],
      categories: [],
      limit: 8,
    },
    {
      id: "cats",
      kind: "categoryHighlights",
      title: "Browse by category",
      items: [],
      plugins: [],
      categories: [{ id: "monitoring", blurb: "Watch what matters." }, { id: "developer-tools" }],
    },
    { id: "all", kind: "catalog", title: "All plugins", items: [], plugins: [], categories: [] },
  ],
  collections: [
    {
      id: "productivity",
      title: "Productivity",
      description: "Get more done.",
      plugins: ["deckpilot.pomodoro", "deckpilot.weather"],
    },
    { id: "dev", title: "Development", plugins: ["acme.github-prs"] },
  ],
}

export const listingFixtures: Record<string, ListingView> = {
  "deckpilot.weather": {
    promo: "Weather on your deck, at a glance",
    descriptionMd:
      "Current conditions and a **7-day** forecast.\n\n- Per-key city\n- Variables for _workflows_\n\nSee the [documentation](https://github.com/knittingcode/deckpilot-plugins).",
    highlights: [
      { title: "7-day forecast", text: "Daily highs, lows and rain chance." },
      { title: "Per-key city", text: "Each key can show another place." },
    ],
    icon: MEDIA.weatherIcon,
    screenshots: [
      {
        media: MEDIA.forecastShot,
        alt: "Six weather keys showing temperatures",
        caption: "The forecast page",
      },
      { media: MEDIA.settingsShot, alt: "The weather settings" },
      { media: MEDIA.missing, alt: "A screenshot that does not load" },
    ],
    promoArt: { media: MEDIA.weatherPromo, alt: "Weather icons over a sky gradient" },
    links: {
      documentation:
        "https://github.com/knittingcode/deckpilot-plugins/tree/main/first-party/weather",
      support: "https://github.com/knittingcode/deckpilot-plugins/issues",
      changelog:
        "https://github.com/knittingcode/deckpilot-plugins/blob/main/first-party/weather/CHANGELOG.md",
    },
    releaseNotes: [
      { version: "0.2.1", text: "Fixes the wind unit in Polish." },
      { version: "0.2.0", text: "Adds **air quality**." },
    ],
  },
  "deckpilot.pomodoro": {
    promo: "Focus in 25-minute sprints",
    highlights: [],
    icon: MEDIA.pomodoroIcon,
    screenshots: [],
    links: {},
    releaseNotes: [],
  },
  // A third-party listing: links are shown in full before opening.
  "acme.clock": {
    promo: "A clock on a key",
    descriptionMd: "Shows the time. Supports `24h` and `12h`.",
    highlights: [],
    screenshots: [],
    links: { support: "https://acme.example/help" },
    releaseNotes: [],
  },
}

export const mediaFixtures: Record<string, string> = {
  [MEDIA.weatherIcon.sha256]: PNG_1X1,
  [MEDIA.pomodoroIcon.sha256]: WEBP_1X1,
  [MEDIA.launchBanner.sha256]: WEBP_1X1,
  [MEDIA.focusBanner.sha256]: PNG_1X1,
  [MEDIA.forecastShot.sha256]: WEBP_1X1,
  [MEDIA.settingsShot.sha256]: PNG_1X1,
  [MEDIA.weatherPromo.sha256]: PNG_1X1,
}

export const defaultFixture: StorefrontFixture = {
  storefront: storefrontFixture,
  listings: listingFixtures,
  media: mediaFixtures,
}

/** No storefront published (or the agent refused it): catalog-only Discover. */
export const emptyFixture: StorefrontFixture = { storefront: null, listings: {}, media: {} }

// ---------------------------------------------------------------- hostile

const LONG = "W".repeat(6000)
const BOMB = "*".repeat(10000)

export const HOSTILE_MEDIA = {
  svg: media(0xbad1, "image/png", 512, 512),
  http: media(0xbad2, "image/png", 512, 512),
  html: media(0xbad3, "image/png", 512, 512),
  /** Claims an enormous size (the agent caps real images at 8 Mi pixels). */
  huge: media(0xbad4, "image/png", 100000, 100000),
} as const

const hostileSection = (
  section: Partial<SectionView> & Pick<SectionView, "id" | "kind">,
): SectionView => ({
  items: [],
  plugins: [],
  categories: [],
  ...section,
})

export const hostileStorefront: StorefrontView = {
  seq: 43,
  sections: [
    hostileSection({
      id: "xss",
      kind: "bannerCarousel",
      items: [
        {
          id: "script",
          title: "<script>alert(1)</script>",
          body: '<img src=x onerror="alert(1)">',
          cta: "javascript:alert(1)",
          artwork: HOSTILE_MEDIA.svg,
          alt: '"><svg onload=alert(1)>',
          target: { type: "plugin", id: "acme.does-not-exist" },
        },
        {
          id: "rtl",
          title: "evil‮txt.exe",
          body: "zero​width and ⁦isolate⁩",
          artwork: HOSTILE_MEDIA.http,
          alt: "x",
          target: { type: "collection", id: "missing-collection" },
        },
        {
          id: "long",
          title: LONG,
          body: BOMB,
          artwork: HOSTILE_MEDIA.huge,
          alt: LONG,
          target: { type: "category", id: "../../etc" },
        },
      ],
    }),
    hostileSection({
      id: "future",
      kind: "hologram",
      title: "A section kind this UI does not know",
    }),
    hostileSection({
      id: "dupes",
      kind: "collectionRow",
      layout: "cards",
      title: "{{constructor.constructor('alert(1)')()}}",
      plugins: ["acme.clock", "acme.clock", "__proto__", "acme.unknown"],
    }),
    hostileSection({
      id: "badlayout",
      // An unknown layout falls back to cards.
      layout: "hero" as unknown as "cards",
      kind: "collectionRow",
      title: "Unknown layout",
      plugins: ["acme.clock"],
    }),
    hostileSection({ id: "spot", kind: "spotlight", title: "No plugin", plugins: [] }),
    hostileSection({ id: "limit", kind: "recentlyAdded", limit: 100000 }),
  ],
  collections: [
    { id: "__proto__", title: "<b>bold</b>", plugins: ["acme.clock"] },
    { id: "empty", title: "Empty", plugins: [] },
  ],
}

export const hostileListings: Record<string, ListingView> = {
  "acme.clock": {
    promo: "<script>alert(1)</script>",
    descriptionMd: [
      "<script>alert(1)</script>",
      '<img src=x onerror="alert(1)">',
      "[click](javascript:alert(1))",
      "[data](data:text/html;base64,PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0Pg==)",
      "[http](http://acme.example/)",
      "![image](https://acme.example/x.png)",
      "# Heading",
      "| a | b |",
      BOMB,
      LONG,
      "evil‮txt.exe",
      "**unclosed _nested `code",
    ].join("\n\n"),
    highlights: Array.from({ length: 50 }, (_, i) => ({ title: `Highlight ${i}`, text: LONG })),
    icon: HOSTILE_MEDIA.svg,
    screenshots: [
      { media: HOSTILE_MEDIA.html, alt: "<img src=x onerror=alert(1)>", caption: LONG },
      { media: HOSTILE_MEDIA.huge, alt: "" },
    ],
    links: {
      documentation: "javascript:alert(1)",
      support: "http://acme.example/",
      changelog: "https://user:pw@acme.example:8443/‮gnp.exe",
    },
    releaseNotes: [
      { version: "<script>", text: "<script>alert(1)</script>" },
      { version: "1.0.0", text: LONG },
    ],
  },
}

/**
 * What a compromised backend could hand over as "data URLs": each must be
 * refused by `isStorefrontMediaDataUrl` and never reach an `<img>`.
 */
export const hostileMedia: Record<string, string> = {
  [HOSTILE_MEDIA.svg.sha256]:
    "data:image/svg+xml;base64,PHN2ZyB4bWxucz0naHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmcnIG9ubG9hZD0nYWxlcnQoMSknLz4=",
  [HOSTILE_MEDIA.http.sha256]: "https://evil.example/tracker.png",
  [HOSTILE_MEDIA.html.sha256]: "data:text/html;base64,PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0Pg==",
  [HOSTILE_MEDIA.huge.sha256]: "data:image/png;charset=utf-8;base64,AAAA",
}

export const hostileFixture: StorefrontFixture = {
  storefront: hostileStorefront,
  listings: hostileListings,
  media: hostileMedia,
}

// ----------------------------------------------------------------- client

export interface FixtureClientOptions {
  /** Delay every answer (ms), to see placeholders. */
  delayMs?: number
  /**
   * Pass media through unfiltered (only to test that components filter
   * themselves); by default the fixture client filters like the real one.
   */
  unfilteredMedia?: boolean
}

function later<T>(value: T, delayMs: number | undefined): Promise<T> {
  if (!delayMs) return Promise.resolve(value)
  return new Promise((resolve) => {
    setTimeout(() => {
      resolve(value)
    }, delayMs)
  })
}

/** A `StorefrontClient` answering from `fixture`. */
export function createFixtureStorefrontClient(
  fixture: StorefrontFixture = defaultFixture,
  options: FixtureClientOptions = {},
): StorefrontClient {
  return {
    getStorefront: () => later(fixture.storefront, options.delayMs),
    getRegistryListing: (plugin) =>
      later(
        Object.hasOwn(fixture.listings, plugin) ? (fixture.listings[plugin] ?? null) : null,
        options.delayMs,
      ),
    getRegistryMedia: (sha256) => {
      const url =
        isMediaSha256(sha256) && Object.hasOwn(fixture.media, sha256)
          ? (fixture.media[sha256] ?? null)
          : null
      if (options.unfilteredMedia) return later(url, options.delayMs)
      return later(isStorefrontMediaDataUrl(url) ? url : null, options.delayMs)
    },
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value)
}

/**
 * A fixture from JSON (a Marketplace Studio export, `VITE_STOREFRONT_FIXTURE`
 * in the dev build). Only the outer shape is checked here; the components
 * must cope with anything inside, as with the hostile fixtures.
 */
export function fixtureFromJson(value: unknown): StorefrontFixture {
  if (!isRecord(value)) throw new Error("a storefront fixture is an object")
  const storefront = value.storefront ?? null
  if (storefront !== null && (!isRecord(storefront) || !Array.isArray(storefront.sections))) {
    throw new Error("storefront must be null or have sections")
  }
  const listings = value.listings ?? {}
  const mediaMap = value.media ?? {}
  if (!isRecord(listings) || !isRecord(mediaMap)) {
    throw new Error("listings and media are objects")
  }
  const mediaOut: Record<string, string> = {}
  for (const [key, url] of Object.entries(mediaMap)) {
    if (typeof url === "string") mediaOut[key] = url
  }
  return {
    storefront: storefront as StorefrontView | null,
    listings: listings as Record<string, ListingView>,
    media: mediaOut,
  }
}
