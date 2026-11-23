// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { installDomStubs } from "@/test/dom"
import { stubReducedMotion, withStorefrontClient } from "@/test/storefront"

import {
  createFixtureStorefrontClient,
  hostileFixture,
  storefrontFixture,
} from "../storefront-mocks"
import type { BannerView } from "../storefront-types"
import { BannerCarousel } from "./banner-carousel"
import { clearMediaCache } from "./media-cache"

beforeAll(installDomStubs)
beforeEach(() => {
  clearMediaCache()
  stubReducedMotion(false)
})
afterEach(() => {
  cleanup()
  vi.useRealTimers()
})

const banners = storefrontFixture.sections[0]?.items ?? []

function renderCarousel(items: BannerView[] = banners, fixture = createFixtureStorefrontClient()) {
  const onTarget = vi.fn()
  render(withStorefrontClient(fixture, <BannerCarousel items={items} onTarget={onTarget} />))
  return { onTarget, carousel: screen.getByRole("region", { name: "Highlights" }) }
}

const selectedTab = () =>
  screen.getAllByRole("tab").findIndex((t) => t.getAttribute("aria-selected") === "true")

describe("banner carousel", () => {
  it("has the carousel's roles and names, one slide shown", async () => {
    const { carousel } = renderCarousel()
    expect(carousel).toHaveAttribute("aria-roledescription", "carousel")
    // Only the current slide is in the accessibility tree.
    const slides = within(carousel).getAllByRole("group")
    expect(slides).toHaveLength(1)
    expect(slides[0]).toHaveAttribute("aria-roledescription", "slide")
    expect(slides[0]).toHaveAccessibleName("1 of 3")
    expect(within(carousel).getAllByRole("group", { hidden: true })).toHaveLength(3)
    const tablist = screen.getByRole("tablist", { name: "Choose a slide" })
    const tabs = within(tablist).getAllByRole("tab")
    expect(tabs.map((t) => t.getAttribute("aria-label"))).toEqual([
      "Slide 1: Know before you go",
      "Slide 2: Stay in the zone",
      "Slide 3: Watch what matters",
    ])
    expect(tabs[0]).toHaveAttribute("aria-controls", slides[0]?.id)
    // Roving focus: only the selected tab is in the tab order.
    expect(tabs.map((t) => t.tabIndex)).toEqual([0, -1, -1])
    // The artwork has the editorial alternative; decorative art has none.
    // (While it loads, its placeholder carries the same name.)
    const art = await within(slides[0] as HTMLElement).findByRole("img", {
      name: "Rain clouds over a Stream Deck",
    })
    await vi.waitFor(() => {
      expect(within(slides[0] as HTMLElement).getByRole("img").tagName).toBe("IMG")
    })
    expect(art).toBeTruthy()
    expect(within(slides[0] as HTMLElement).getByRole("img")).toHaveAccessibleName(
      "Rain clouds over a Stream Deck",
    )
  })

  it("moves with Previous, Next and the dots, wrapping around", async () => {
    const user = userEvent.setup()
    renderCarousel()
    await user.click(screen.getByRole("button", { name: "Next" }))
    expect(selectedTab()).toBe(1)
    expect(screen.getByRole("group")).toHaveAccessibleName("2 of 3")
    await user.click(screen.getByRole("button", { name: "Previous" }))
    await user.click(screen.getByRole("button", { name: "Previous" }))
    expect(selectedTab()).toBe(2)
    await user.click(screen.getByRole("tab", { name: "Slide 1: Know before you go" }))
    expect(selectedTab()).toBe(0)
  })

  it("moves with the arrow keys, Home and End, keeping focus on the dots", () => {
    renderCarousel()
    const tabs = screen.getAllByRole("tab")
    tabs[0]?.focus()
    fireEvent.keyDown(tabs[0] as HTMLElement, { key: "ArrowRight" })
    expect(selectedTab()).toBe(1)
    expect(document.activeElement).toBe(screen.getAllByRole("tab")[1])
    fireEvent.keyDown(document.activeElement as HTMLElement, { key: "End" })
    expect(selectedTab()).toBe(2)
    fireEvent.keyDown(document.activeElement as HTMLElement, { key: "ArrowRight" })
    expect(selectedTab()).toBe(0)
    fireEvent.keyDown(document.activeElement as HTMLElement, { key: "ArrowLeft" })
    expect(selectedTab()).toBe(2)
    fireEvent.keyDown(document.activeElement as HTMLElement, { key: "Home" })
    expect(selectedTab()).toBe(0)
  })

  it("never advances by itself", () => {
    vi.useFakeTimers()
    renderCarousel()
    act(() => {
      vi.advanceTimersByTime(10 * 60 * 1000)
    })
    expect(selectedTab()).toBe(0)
    expect(vi.getTimerCount()).toBe(0)
  })

  it("has no transition with reduced motion", () => {
    stubReducedMotion(true)
    const { carousel } = renderCarousel()
    expect(carousel).toHaveAttribute("data-motion", "reduce")
    expect(within(carousel).getByRole("group").className).not.toMatch(/animate|transition/)
    cleanup()
    stubReducedMotion(false)
    const full = renderCarousel().carousel
    expect(full).toHaveAttribute("data-motion", "full")
    expect(within(full).getByRole("group").className).toMatch(/animate-in/)
  })

  it("leads inside the Marketplace with its call to action", async () => {
    const user = userEvent.setup()
    const { onTarget } = renderCarousel()
    await user.click(screen.getByRole("button", { name: /See plugin/ }))
    expect(onTarget).toHaveBeenCalledWith({ type: "plugin", id: "deckpilot.weather" })
    await user.click(screen.getByRole("button", { name: "Next" }))
    // Without an editorial label, a neutral one.
    await user.click(screen.getByRole("button", { name: /Learn more/ }))
    expect(onTarget).toHaveBeenLastCalledWith({ type: "collection", id: "productivity" })
  })

  it("shows decorative art with an empty alternative", async () => {
    const decorative = banners.filter((b) => b.decorative)
    renderCarousel(decorative)
    await vi.waitFor(() => {
      expect(document.querySelector("img")).not.toBeNull()
    })
    expect(document.querySelector("img")).toHaveAttribute("alt", "")
    // One slide: no controls.
    expect(screen.queryByRole("tablist")).not.toBeInTheDocument()
  })

  it("renders hostile copy as inert text and refuses hostile images", async () => {
    const items = hostileFixture.storefront?.sections[0]?.items ?? []
    const raw = createFixtureStorefrontClient(hostileFixture, { unfilteredMedia: true })
    const { carousel } = renderCarousel(items, raw)
    expect(carousel.querySelector("script, svg[onload], iframe")).toBeNull()
    expect(screen.getByRole("heading", { level: 3 })).toHaveTextContent("<script>alert(1)</script>")
    fireEvent.click(screen.getByRole("button", { name: "Next" }))
    expect(screen.getByRole("heading", { level: 3 }).textContent).toBe("eviltxt.exe")
    fireEvent.click(screen.getByRole("button", { name: "Next" }))
    // The 6000-character title is clipped.
    expect(screen.getByRole("heading", { level: 3 }).textContent.length).toBeLessThanOrEqual(120)
    await vi.waitFor(() => {
      expect(carousel.querySelectorAll('[data-state="fallback"]')).toHaveLength(3)
    })
    expect(carousel.querySelector("img")).toBeNull()
  })
})
