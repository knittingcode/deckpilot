// Rendering storefront components in tests: a storefront client in reach
// (as the Marketplace's provider gives one), a catalog snapshot and
// spying Marketplace actions.

import type { ReactNode } from "react"
import { vi } from "vitest"

import { storefrontCatalog } from "@/dev/storefront-catalog"
import type { MarketplaceActions } from "@/plugins/marketplace"
import { MEDIA } from "@/plugins/storefront-mocks"
import type { SectionView, StorefrontView } from "@/plugins/storefront-types"
import type { StorefrontClient } from "@/plugins/storefront-api"
import { catalogIndex } from "@/plugins/storefront/resolve"
import { NONE, StorefrontStateContext } from "@/plugins/storefront/use-storefront"
import type { MarketplaceSnapshot } from "@/plugins/types"

import { readyRuntime } from "./plugins"

/** `node` with `client` as the storefront's (no view: components only). */
export function withStorefrontClient(client: StorefrontClient, node: ReactNode) {
  return <StorefrontStateContext value={{ ...NONE, client }}>{node}</StorefrontStateContext>
}

export function storefrontSnapshot(extra: Partial<MarketplaceSnapshot> = {}): MarketplaceSnapshot {
  return {
    runtime: readyRuntime({ features: ["storefront"] }),
    registry: { source: "/tmp/registry", freshness: "fresh", seq: 42 },
    available: storefrontCatalog(),
    installed: [],
    ...extra,
  }
}

export function fakeActions(): MarketplaceActions {
  return {
    busy: null,
    pending: new Map(),
    failures: new Map(),
    retry: vi.fn(),
    openView: vi.fn(),
    configure: vi.fn(),
    showInstalled: vi.fn(),
    openDiagnostics: vi.fn(),
    blocked: null,
    canRun: true,
    client: {} as MarketplaceActions["client"],
    openDetails: vi.fn(),
    install: vi.fn(),
    installFile: vi.fn(),
    update: vi.fn(),
    setEnabled: vi.fn(),
    setGrant: vi.fn(),
    uninstall: vi.fn(),
    canRestore: false,
    restore: vi.fn(),
    rollback: vi.fn(),
    invoke: vi.fn(),
    refresh: vi.fn(),
    reload: vi.fn(),
  }
}

export { catalogIndex }

/** Stubs `prefers-reduced-motion` (and every other query) to `reduce`. */
export function stubReducedMotion(reduce: boolean) {
  window.matchMedia = vi.fn().mockImplementation((query: string) => ({
    matches: reduce && query.includes("reduce"),
    media: query,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
  }))
}

const section = (s: Partial<SectionView> & Pick<SectionView, "id" | "kind">): SectionView => ({
  items: [],
  plugins: [],
  categories: [],
  ...s,
})

const KINDS = [
  "bannerCarousel",
  "collectionRow",
  "spotlight",
  "recentlyAdded",
  "categoryHighlights",
  "catalog",
  "hologram",
]
const IDS = [
  "deckpilot.weather",
  "deckpilot.pomodoro",
  "acme.clock",
  "acme.github-prs",
  "acme.future",
  "acme.unknown",
  "__proto__",
  "constructor",
  "",
]
const TEXT = [
  "Weather",
  "<script>alert(1)</script>",
  "Official DeckPilot",
  "\u202Eevil",
  "Verified",
]

/** Random editorial, including fields that try to override a plugin's own. */
export function randomStorefront(next: () => number): StorefrontView {
  const pick = <T,>(list: readonly T[]): T => list[Math.floor(next() * list.length)] as T
  const ids = () => Array.from({ length: Math.floor(next() * 6) }, () => pick(IDS))
  const sections = Array.from({ length: 1 + Math.floor(next() * 8) }, (_, i) => {
    const s = section({
      id: `s${String(Math.floor(next() * 6))}-${String(i % 3)}`,
      kind: pick(KINDS),
      title: pick(TEXT),
      description: pick(TEXT),
      plugins: ids(),
      layout: pick(["cards", "compact"] as const),
      limit: Math.floor(next() * 50) - 10,
      version: pick(["0.2.1", "0.2.0", "9.9.9"]),
      categories: [{ id: pick(["monitoring", "productivity", "nope"]), blurb: pick(TEXT) }],
      items: Array.from({ length: Math.floor(next() * 4) }, (_, j) => ({
        id: `b${String(j)}`,
        title: pick(TEXT),
        artwork: MEDIA.launchBanner,
        alt: pick(TEXT),
        target: {
          type: pick(["plugin", "collection", "category"] as const),
          id: pick([...IDS, "productivity", "monitoring"]),
        },
      })),
    })
    // What an editorial must never be able to set on a card.
    Object.assign(s, {
      name: pick(TEXT),
      publisher: "evil",
      verified: true,
      official: true,
      bundledOnly: true,
      permissions: [],
      publisherInfo: { namespace: "evil", login: "evil", verified: true },
    })
    return s
  })
  const collections = Array.from({ length: Math.floor(next() * 4) }, (_, i) => ({
    id: pick(["productivity", "dev", "__proto__", `c${String(i)}`]),
    title: pick(TEXT),
    plugins: ids(),
  }))
  return { seq: 1, sections, collections }
}
