// @vitest-environment jsdom
// The storefront in the real Marketplace (PluginsSettings): editorial
// Discover, its catalog-only fallback, the sidebar's collections, a
// plugin's page with its listing, hostile content and Polish.
import "@testing-library/jest-dom/vitest"

import { act, cleanup, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { storefrontCatalog } from "@/dev/storefront-catalog"
import { i18n } from "@/i18n"
import { installDomStubs } from "@/test/dom"
import { discoverReady, marketplaceSidebar } from "@/test/marketplace"
import { createFakePluginClient, readyRuntime } from "@/test/plugins"
import { renderWithApp } from "@/test/render"
import { randomStorefront, stubReducedMotion } from "@/test/storefront"
import { resetToasts } from "@/test/toast"

import { PluginClientContext } from "../api"
import { PluginsSettings } from "../marketplace"
import { type StorefrontClient, StorefrontClientContext } from "../storefront-api"
import {
  createFixtureStorefrontClient,
  defaultFixture,
  emptyFixture,
  hostileFixture,
  type StorefrontFixture,
} from "../storefront-mocks"
import { clearMediaCache } from "./media-cache"

vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)
vi.mock("../client", () => ({
  createTauriPluginClient: () => {
    throw new Error("the Tauri client is not used in tests")
  },
}))

beforeAll(installDomStubs)
beforeEach(() => {
  resetToasts()
  clearMediaCache()
  stubReducedMotion(false)
})
afterEach(cleanup)

function renderStorefront(
  storefront: StorefrontClient | StorefrontFixture = defaultFixture,
  { features = ["storefront"] }: { features?: string[] } = {},
) {
  const client = createFakePluginClient({
    runtime: readyRuntime({ features }),
    available: storefrontCatalog(),
  })
  const sf = "getStorefront" in storefront ? storefront : createFixtureStorefrontClient(storefront)
  const spy = vi.spyOn(sf, "getStorefront")
  const result = renderWithApp(
    <PluginClientContext value={client}>
      <StorefrontClientContext value={sf}>
        <PluginsSettings />
      </StorefrontClientContext>
    </PluginClientContext>,
  )
  return { ...result, client, storefront: sf, getStorefront: spy }
}

const sidebarButton = (name: string) => within(marketplaceSidebar()).queryByRole("button", { name })

/** Every catalog tile's name and developer line on screen. */
function tileFacts(): { name: string; text: string }[] {
  return [...document.querySelectorAll<HTMLElement>("li")]
    .filter((li) => li.querySelector("button[aria-describedby]"))
    .map((li) => ({
      name: li.querySelector("button[aria-describedby]")?.textContent ?? "",
      text: li.textContent,
    }))
}

describe("editorial Discover", () => {
  it("shows the storefront's sections in order, built from the catalog", async () => {
    renderStorefront()
    await discoverReady()
    const carousel = await screen.findByRole("region", { name: "Highlights" })
    expect(carousel).toHaveAttribute("aria-roledescription", "carousel")
    const focus = screen.getByRole("list", { name: "Stay focused" })
    // The editorial-only id (deckpilot.break-reminder) is not in the catalog: dropped.
    expect(
      within(focus)
        .getAllByRole("listitem")
        .map((li) => li.textContent),
    ).toEqual([expect.stringContaining("Pomodoro"), expect.stringContaining("Acme Clock")])
    expect(screen.getByRole("heading", { name: "New: Weather" })).toBeInTheDocument()
    expect(screen.getByRole("list", { name: "Developer tools" })).toBeInTheDocument()
    expect(screen.getByRole("list", { name: "Recently added" })).toBeInTheDocument()
    expect(screen.getByRole("list", { name: "Browse by category" })).toHaveTextContent(
      "Watch what matters.",
    )
    expect(screen.getByRole("list", { name: "All plugins" })).toBeInTheDocument()
    // Section headings in the agent's order.
    const order = [
      "Stay focused",
      "New: Weather",
      "Developer tools",
      "Recently added",
      "All plugins",
    ]
    const headings = screen.getAllByRole("heading", { level: 3 }).map((h) => h.textContent)
    const positions = order.map((title) => headings.indexOf(title))
    expect(positions.every((p, i) => p >= 0 && (i === 0 || p > (positions[i - 1] ?? 0)))).toBe(true)
    // Nothing invented.
    expect(screen.queryByText(/popular|downloads|rating|stars/i)).not.toBeInTheDocument()
    // The spotlight shows the named version's notes, from the listing.
    expect(await screen.findByText("Fixes the wind unit in Polish.")).toBeInTheDocument()
  })

  it("leads from a banner to a plugin's page, a collection or a category", async () => {
    const { user } = renderStorefront()
    await screen.findByRole("region", { name: "Highlights" })
    await user.click(screen.getByRole("button", { name: /See plugin/ }))
    expect(await screen.findByRole("heading", { name: "Weather", level: 3 })).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Back to plugins" }))
    await user.click(await screen.findByRole("tab", { name: /Slide 3/ }))
    await user.click(screen.getByRole("button", { name: /Browse/ }))
    // Discover narrowed to the category.
    expect(await screen.findByRole("status")).toHaveTextContent("in Monitoring")
  })

  it("puts the storefront's collections in the sidebar instead of the built-in ones", async () => {
    const { user } = renderStorefront()
    await screen.findByRole("region", { name: "Highlights" })
    expect(sidebarButton("Productivity")).toBeInTheDocument()
    expect(sidebarButton("Development")).toBeInTheDocument()
    expect(sidebarButton("Code")).not.toBeInTheDocument()
    await user.click(sidebarButton("Productivity") as HTMLElement)
    expect(
      await screen.findByRole("heading", { name: "Productivity", level: 2 }),
    ).toBeInTheDocument()
    expect(screen.getByText("Get more done.")).toBeInTheDocument()
    const list = screen.getByRole("list", { name: "Productivity" })
    expect(within(list).getAllByRole("listitem")).toHaveLength(2)
  })

  it("is catalog-only when the agent has no storefront", async () => {
    renderStorefront(emptyFixture)
    await discoverReady()
    expect(await screen.findByRole("list", { name: "Recently added" })).toBeInTheDocument()
    expect(screen.getByRole("heading", { name: "Collections" })).toBeInTheDocument()
    expect(screen.queryByRole("region", { name: "Highlights" })).not.toBeInTheDocument()
    expect(sidebarButton("Code")).toBeInTheDocument()
  })

  it("is catalog-only without the feature, and never asks", async () => {
    const { getStorefront } = renderStorefront(defaultFixture, { features: [] })
    await discoverReady()
    expect(await screen.findByRole("list", { name: "Recently added" })).toBeInTheDocument()
    expect(screen.queryByRole("region", { name: "Highlights" })).not.toBeInTheDocument()
    expect(getStorefront).not.toHaveBeenCalled()
  })

  it("is catalog-only when the request fails", async () => {
    const failing: StorefrontClient = {
      getStorefront: () => Promise.reject(new Error("transport")),
      getRegistryListing: () => Promise.reject(new Error("transport")),
      getRegistryMedia: () => Promise.reject(new Error("transport")),
    }
    renderStorefront(failing)
    expect(await screen.findByRole("heading", { name: "Collections" })).toBeInTheDocument()
  })

  it("renders the hostile storefront inertly and skips what it does not know", async () => {
    renderStorefront(createFixtureStorefrontClient(hostileFixture, { unfilteredMedia: true }))
    await screen.findByRole("list", { name: "Unknown layout" })
    expect(document.querySelector("script, iframe, svg[onload], img[onerror]")).toBeNull()
    expect(screen.queryByText("A section kind this UI does not know")).not.toBeInTheDocument()
    // Template text stays text.
    expect(
      screen.getByRole("heading", { name: "{{constructor.constructor('alert(1)')()}}" }),
    ).toBeInTheDocument()
    // Its banners lead nowhere, so there are none.
    expect(screen.queryByRole("region", { name: "Highlights" })).not.toBeInTheDocument()
    // The `__proto__` collection's title is literal.
    expect(sidebarButton("<b>bold</b>")).toBeInTheDocument()
    expect(document.querySelector("b")).toBeNull()
  })

  it("never changes a card's name, developer, verification or action (random editorial)", async () => {
    const catalog = storefrontCatalog()
    let seed = 7
    const next = () => {
      seed = (seed * 16807) % 2147483647
      return seed / 2147483647
    }
    for (let round = 0; round < 6; round++) {
      const storefront = randomStorefront(next)
      renderStorefront({ storefront, listings: {}, media: {} })
      await discoverReady()
      await waitFor(() => {
        expect(screen.queryByLabelText("Loading Discover")).not.toBeInTheDocument()
      })
      for (const tile of tileFacts()) {
        const plugin = catalog.find((p) => p.name === tile.name)
        expect(plugin, `round ${String(round)}: ${tile.name}`).toBeDefined()
        expect(tile.text).toContain(plugin?.publisherInfo?.displayName ?? plugin?.publisher ?? "")
        expect(tile.text).not.toContain("evil")
      }
      // Install buttons name catalog plugins only; never an editorial title.
      const content = document.querySelector('[data-slot="marketplace-content"]') as HTMLElement
      for (const button of within(content).queryAllByRole("button", { name: /^Install / })) {
        const name = button.getAttribute("aria-label")?.replace(/^Install /, "")
        expect(catalog.map((p) => p.name)).toContain(name)
      }
      // The verification mark is the catalog's, tile by tile.
      for (const li of document.querySelectorAll<HTMLElement>("li")) {
        const name = li.querySelector("button[aria-describedby]")?.textContent
        const plugin = catalog.find((p) => p.name === name)
        if (!plugin) continue
        expect(within(li).queryByRole("img", { name: "Verified publisher" }) !== null).toBe(
          plugin.publisherInfo?.verified === true,
        )
      }
      cleanup()
    }
  })
})

describe("a plugin's page with a listing", () => {
  it("shows the listing's copy, media, highlights, notes and links", async () => {
    const { user } = renderStorefront()
    await screen.findByRole("region", { name: "Highlights" })
    await user.click(screen.getByRole("button", { name: /See plugin/ }))
    const article = await screen.findByRole("article")
    expect(
      await within(article).findByText("Weather on your deck, at a glance"),
    ).toBeInTheDocument()
    const description = article.querySelector('[data-slot="safe-markdown"]') as HTMLElement
    expect(within(description).getByText("7-day").tagName).toBe("STRONG")
    expect(within(article).getByRole("list", { name: "Highlights" })).toHaveTextContent(
      "7-day forecast",
    )
    expect(within(article).getByRole("list", { name: "Screenshots" })).toBeInTheDocument()
    expect(within(article).getByRole("heading", { name: "What’s new" })).toBeInTheDocument()
    expect(article).toHaveTextContent("Fixes the wind unit in Polish.")
    const versions = within(article).getByRole("list", { name: "Versions" })
    // The older version's listing note, as markdown.
    expect(within(versions).getByText("air quality").tagName).toBe("STRONG")
    const links = within(article).getByRole("list", { name: "Links" })
    expect(links).toHaveTextContent("Changelog")
    expect(links).toHaveTextContent("github.com/knittingcode/deckpilot-plugins/issues")
  })

  it("shows a third-party plugin's links in full, and only https ones", async () => {
    const fixture: StorefrontFixture = {
      ...defaultFixture,
      listings: {
        ...defaultFixture.listings,
        "acme.clock": {
          ...(defaultFixture.listings["acme.clock"] as NonNullable<
            (typeof defaultFixture.listings)["acme.clock"]
          >),
          links: {
            support: "https://acme.example/help",
            documentation: "javascript:alert(1)",
            changelog: "http://acme.example/",
          },
        },
      },
    }
    const { user } = renderStorefront(fixture)
    await screen.findByRole("region", { name: "Highlights" })
    const row = screen.getByRole("list", { name: "Stay focused" })
    await user.click(within(row).getByRole("button", { name: "Acme Clock" }))
    const links = await screen.findByRole("list", { name: "Links" })
    expect(links).toHaveTextContent("https://acme.example/help")
    expect(links).not.toHaveTextContent("javascript")
    expect(links).not.toHaveTextContent("http://")
    expect(screen.getByText(/its links are shown in full/)).toBeInTheDocument()
  })
})

describe("in Polish", () => {
  afterEach(async () => {
    await act(() => i18n.changeLanguage("en"))
  })

  it("names the storefront's controls in Polish", async () => {
    await act(() => i18n.changeLanguage("pl"))
    renderStorefront()
    const carousel = await screen.findByRole("region", { name: "Wyróżnione" })
    expect(carousel).toHaveAttribute("aria-roledescription", "karuzela")
    expect(within(carousel).getByRole("group")).toHaveAccessibleName("1 z 3")
    expect(screen.getByRole("button", { name: "Następny" })).toBeInTheDocument()
    expect(screen.getByRole("tablist", { name: "Wybierz slajd" })).toBeInTheDocument()
    expect(screen.getByText("W centrum uwagi")).toBeInTheDocument()
  })
})
