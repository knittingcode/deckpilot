import { describe, expect, it } from "vitest"

import "@/i18n"

import {
  byRecency,
  COLLECTIONS,
  categoryCounts,
  collectionPlugins,
  inCollection,
  listingOf,
  categoryName,
  type EntryContext,
  entryState,
  firstPublished,
  isBundledOnly,
  isFiltering,
  matchesSearch,
  officialIds,
  pluginDisplayName,
} from "./catalog"
import type { AvailablePlugin, MarketplaceSnapshot, PluginDetails } from "./types"

const plugin = (id: string, extra: Partial<AvailablePlugin> = {}): AvailablePlugin => ({
  id,
  name: id.split(".")[1] ?? id,
  summary: "A plugin.",
  publisher: id.split(".")[0] ?? id,
  latest: "1.0.0",
  versions: [{ version: "1.0.0", permissions: [] }],
  ...extra,
})

const installed = (
  id: string,
  extra: Partial<PluginDetails["summary"]> = {},
  manifest: unknown = {},
): PluginDetails => ({
  summary: {
    id,
    name: id,
    publisher: "acme",
    version: "1.0.0",
    state: "enabled",
    origin: { type: "registry", registry: "https://example" },
    actions: [],
    ...extra,
  },
  description: "",
  license: "MIT",
  permissions: [],
  manifest,
  audit: [],
})

const context = (extra: Partial<EntryContext> = {}): EntryContext => ({
  installed: undefined,
  update: undefined,
  pending: undefined,
  failure: undefined,
  restorable: false,
  canRun: true,
  ...extra,
})

const snapshot = (extra: Partial<MarketplaceSnapshot> = {}): MarketplaceSnapshot => ({
  runtime: {
    available: true,
    built: true,
    enabled: true,
    executionSupported: true,
    platform: "macos",
  },
  registry: null,
  available: [],
  installed: [],
  ...extra,
})

describe("entry state", () => {
  it("offers Install for an installable version, else says why not", () => {
    expect(entryState(plugin("acme.a"), context()).kind).toBe("install")
    expect(
      entryState(
        plugin("acme.b", {
          versions: [{ version: "1.0.0", permissions: [], incompatible: ["x"] }],
        }),
        context(),
      ).kind,
    ).toBe("incompatible")
    expect(
      entryState(
        plugin("acme.c", { versions: [{ version: "1.0.0", permissions: [], withdrawn: "bad" }] }),
        context(),
      ),
    ).toEqual({ kind: "withdrawn", reason: "bad" })
  })

  it("never offers a bundled-only plugin, restoring DeckPilot's copy when it can", () => {
    expect(entryState(plugin("deckpilot.git"), context())).toEqual({
      kind: "bundled",
      restorable: false,
    })
    expect(entryState(plugin("deckpilot.workflows"), context({ restorable: true }))).toEqual({
      kind: "bundled",
      restorable: true,
    })
    expect(entryState(plugin("acme.x", { bundledOnly: true }), context()).kind).toBe("bundled")
    expect(isBundledOnly(plugin("deckpilot.github"))).toBe(false)
  })

  it("shows a request in flight, then a failure, before anything else", () => {
    const failure = {
      operation: "install" as const,
      error: { title: "No", message: "no", reason: "x" },
    }
    expect(entryState(plugin("acme.a"), context({ pending: "install" })).kind).toBe("installing")
    expect(entryState(plugin("acme.a"), context({ pending: "update" })).kind).toBe("updating")
    expect(entryState(plugin("acme.a"), context({ failure })).kind).toBe("failed")
  })

  it("drops a failed install once installed, a failed update once its version is reached", () => {
    const error = { title: "No", message: "no", reason: "x" }
    const install = { operation: "install" as const, error }
    expect(entryState(plugin("acme.a"), context({ failure: install })).kind).toBe("failed")
    // Installed since (another window, the agent retried): nothing failed.
    expect(
      entryState(plugin("acme.a"), context({ failure: install, installed: installed("acme.a") }))
        .kind,
    ).toBe("installed")
    const update = { operation: "update" as const, error, version: "1.1.0" }
    const at = (version: string) =>
      entryState(
        plugin("acme.a"),
        context({ failure: update, installed: installed("acme.a", { version }) }),
      ).kind
    expect(at("1.0.0")).toBe("failed")
    expect(at("1.1.0")).toBe("installed")
    expect(at("1.2.0")).toBe("installed")
    // An update failure without the version it tried stays.
    expect(
      entryState(
        plugin("acme.a"),
        context({
          failure: { operation: "update", error },
          installed: installed("acme.a", { version: "9.0.0" }),
        }),
      ).kind,
    ).toBe("failed")
  })

  it("is Installed only for what the agent lists as installed", () => {
    const item = installed("acme.a")
    expect(entryState(plugin("acme.a"), context()).kind).toBe("install")
    expect(entryState(plugin("acme.a"), context({ installed: item })).kind).toBe("installed")
    expect(
      entryState(
        plugin("acme.a"),
        context({ installed: installed("acme.a", { state: "installing" }) }),
      ).kind,
    ).toBe("installing")
  })

  it("opens a view, configures, enables or updates an installed plugin", () => {
    const view = { id: "main", label: "Main", view: { id: "v", kind: "actions" } }
    expect(
      entryState(
        plugin("acme.a"),
        context({ installed: installed("acme.a", { navigation: [view] }) }),
      ),
    ).toEqual({ kind: "open", entry: view })
    const configurable = installed(
      "acme.a",
      {},
      {
        contributes: { settings: { fields: [{ key: "city", label: "City" }] } },
      },
    )
    expect(entryState(plugin("acme.a"), context({ installed: configurable })).kind).toBe(
      "configure",
    )
    const off = installed("acme.a", { state: "disabled" })
    expect(entryState(plugin("acme.a"), context({ installed: off })).kind).toBe("enable")
    // Where plugins cannot run, there is nothing to enable.
    expect(entryState(plugin("acme.a"), context({ installed: off, canRun: false })).kind).toBe(
      "installed",
    )
    const broken = installed("acme.a", { state: "broken" })
    expect(entryState(plugin("acme.a"), context({ installed: broken })).kind).toBe("attention")
    const update = { kind: "available" as const, plugin: off, version: "2.0.0" }
    expect(entryState(plugin("acme.a"), context({ installed: off, update })).kind).toBe("update")
  })
})

describe("catalog sections", () => {
  it("orders the catalog by first publication, undated plugins last by name", () => {
    const dated = (id: string, ...dates: number[]) =>
      plugin(id, {
        versions: dates.map((publishedAt, i) => ({
          version: `1.${String(i)}.0`,
          permissions: [],
          publishedAt,
        })),
      })
    const plugins = [
      plugin("acme.zeta"),
      dated("acme.old", 100, 900),
      plugin("acme.alpha"),
      dated("acme.new", 500),
    ]
    expect(firstPublished(plugins[1] as AvailablePlugin)).toBe(100)
    expect(byRecency(plugins).map((p) => p.id)).toEqual([
      "acme.new",
      "acme.old",
      "acme.alpha",
      "acme.zeta",
    ])
  })

  it("counts and names the categories", () => {
    const plugins = [
      plugin("acme.gh", { categories: ["developer-tools"] }),
      plugin("acme.chat", { categories: ["communication", "developer-tools"] }),
      plugin("acme.weird", { categories: ["weather"] }),
    ]
    expect(categoryCounts(plugins)).toEqual([
      { id: "communication", count: 1 },
      { id: "developer-tools", count: 2 },
      { id: "weather", count: 1 },
    ])
    expect(categoryName("smart-home")).toBe("Smart home")
    // An unknown category, from its id.
    expect(categoryName("home-office")).toBe("Home office")
  })

  it("filters only when there is a search or a category", () => {
    expect(isFiltering({ query: "  ", category: null })).toBe(false)
    expect(isFiltering({ query: "git", category: null })).toBe(true)
    expect(isFiltering({ query: "", category: "media" })).toBe(true)
  })

  it("searches names, descriptions, developers and categories in words", () => {
    const p = plugin("acme.gh", {
      name: "GitHub",
      description: "Pull requests and checks.",
      categories: ["developer-tools"],
      publisherInfo: {
        namespace: "acme",
        displayName: "Acme Tools",
        login: "acme",
        verified: true,
      },
    })
    expect(matchesSearch(p, "checks")).toBe(true)
    expect(matchesSearch(p, "developer tools")).toBe(true)
    expect(matchesSearch(p, "acme tools github")).toBe(true)
    expect(matchesSearch(p, "gitlab")).toBe(false)
  })
})

describe("official plugins and names", () => {
  it("knows official plugins from the registry flag, the bundle and migrations only", () => {
    const ids = officialIds(
      snapshot({
        available: [plugin("acme.flagged", { official: true }), plugin("acme.other")],
        installed: [installed("deckpilot.notifications", { origin: { type: "bundled" } })],
        known: [
          {
            id: "deckpilot.projects",
            name: "Projects",
            publisher: "DeckPilot",
            namespace: "deckpilot",
            lastVersion: "0.1.0",
            installed: false,
            firstSeenAt: 0,
            lastSeenAt: 0,
            restorable: true,
          },
        ],
      }),
      [
        {
          module: "github",
          moduleName: "GitHub",
          plugin: "deckpilot.github",
          phase: "cutOver",
          attempts: 0,
          maxAttempts: 3,
          canRetry: false,
          route: "plugin",
          aliases: [],
          updatedAt: 0,
        },
      ],
    )
    expect(ids.has("acme.flagged")).toBe(true)
    expect(ids.has("deckpilot.notifications")).toBe(true)
    expect(ids.has("deckpilot.projects")).toBe(true)
    expect(ids.has("deckpilot.github")).toBe(true)
    expect(ids.has("acme.other")).toBe(false)
  })

  it("never shows a manifest's raw locale key as a plugin's name", () => {
    const s = snapshot({
      installed: [installed("deckpilot.docker", { name: "Docker" })],
      available: [plugin("deckpilot.github", { name: "GitHub" })],
    })
    expect(pluginDisplayName("deckpilot.docker", "t:plugin.name", s)).toBe("Docker")
    expect(pluginDisplayName("deckpilot.github", "t:plugin.name", s)).toBe("GitHub")
    expect(pluginDisplayName("deckpilot.jira", "t:plugin.name", s)).toBe("deckpilot.jira")
    expect(pluginDisplayName("deckpilot.jira", "Jira", s)).toBe("Jira")
  })
})

describe("collections", () => {
  it("come from the category map in one place, or the registry's own field, never from names", () => {
    expect(COLLECTIONS.code.categories).toEqual(["developer-tools"])
    expect(COLLECTIONS.work.categories).toEqual(["productivity", "communication"])
    expect(COLLECTIONS.monitor.categories).toEqual(["monitoring"])
    // `other` belongs to no collection.
    expect(Object.values(COLLECTIONS).flatMap((c) => c.categories)).not.toContain("other")
    expect(inCollection(plugin("acme.gh", { categories: ["developer-tools"] }), "code")).toBe(true)
    // A name alone decides nothing: "Game Center" is not in Play.
    expect(
      inCollection(plugin("acme.games", { name: "Game Center", categories: ["other"] }), "play"),
    ).toBe(false)
    // Membership is by category only: placements come from the storefront.
    expect(inCollection(plugin("acme.memory", { categories: ["other"] }), "play")).toBe(false)
    expect(
      collectionPlugins(
        [
          plugin("acme.b", { categories: ["monitoring"] }),
          plugin("acme.a", { categories: ["monitoring"] }),
        ],
        "monitor",
      ).map((p) => p.id),
    ).toEqual(["acme.a", "acme.b"])
  })

  it("gives an installed plugin the registry does not list a page from its manifest", () => {
    const item = installed(
      "deckpilot.docker",
      { name: "Docker", version: "0.1.0", origin: { type: "bundled" } },
      {
        categories: ["developer-tools"],
        engines: { deckpilot: ">=0.9.0 <1.0.0" },
        repository: { type: "git", url: "https://github.com/knittingcode/deckpilot-plugins" },
        permissions: [{ kind: "storage", quotaKb: 256, justification: "Keeps state." }],
      },
    )
    const listing = listingOf({ ...item, description: "Containers on your keys." })
    expect(listing).toMatchObject({
      id: "deckpilot.docker",
      name: "Docker",
      installed: "0.1.0",
      categories: ["developer-tools"],
      sourceRepository: "knittingcode/deckpilot-plugins",
      versions: [{ version: "0.1.0", deckpilot: ">=0.9.0 <1.0.0" }],
    })
    expect(listing.versions[0]?.permissions).toHaveLength(1)
  })
})
