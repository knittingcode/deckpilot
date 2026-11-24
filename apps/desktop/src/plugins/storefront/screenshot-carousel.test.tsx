// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { installDomStubs } from "@/test/dom"
import { withStorefrontClient } from "@/test/storefront"

import {
  createFixtureStorefrontClient,
  hostileListings,
  listingFixtures,
} from "../storefront-mocks"
import { clearMediaCache } from "./media-cache"
import { ScreenshotCarousel } from "./screenshot-carousel"

beforeAll(installDomStubs)
beforeEach(clearMediaCache)
afterEach(cleanup)

const shots = listingFixtures["deckpilot.weather"]?.screenshots ?? []

function renderShots(list = shots) {
  render(
    withStorefrontClient(
      createFixtureStorefrontClient(),
      <>
        <button type="button">before</button>
        <ScreenshotCarousel screenshots={list} name="Weather" />
        <button type="button">after</button>
      </>,
    ),
  )
}

describe("screenshot carousel", () => {
  it("shows thumbnails as figures with alternatives and captions", async () => {
    renderShots()
    const list = screen.getByRole("list", { name: "Screenshots" })
    const items = within(list).getAllByRole("listitem")
    expect(items).toHaveLength(3)
    expect(items[0]?.querySelector("figure figcaption")).toHaveTextContent("The forecast page")
    expect(items[1]?.querySelector("figcaption")).toBeNull()
    expect(
      await within(list).findByRole("img", { name: "Six weather keys showing temperatures" }),
    ).toBeInTheDocument()
    // The undelivered one keeps its alternative on the placeholder.
    await vi.waitFor(() => {
      expect(
        within(list).getByRole("img", { name: "A screenshot that does not load" }),
      ).toHaveAttribute("data-state", "fallback")
    })
    expect(
      screen.getByRole("button", {
        name: "Open screenshot 1 of 3: Six weather keys showing temperatures",
      }),
    ).toBeInTheDocument()
  })

  it("opens a lightbox that traps focus, moves with the arrows and returns focus", async () => {
    const user = userEvent.setup()
    renderShots()
    const thumb = screen.getByRole("button", { name: /Open screenshot 2 of 3/ })
    await user.click(thumb)
    const dialog = await screen.findByRole("dialog", { name: "Weather: screenshot 2 of 3" })
    expect(within(dialog).getByRole("img", { name: "The weather settings" })).toBeInTheDocument()
    // Focus stays inside the dialog however far Tab goes.
    for (let i = 0; i < 8; i++) {
      await user.tab()
      expect(dialog.contains(document.activeElement)).toBe(true)
    }
    await user.keyboard("{ArrowRight}")
    expect(screen.getByRole("dialog")).toHaveAccessibleName("Weather: screenshot 3 of 3")
    await user.keyboard("{ArrowRight}")
    expect(screen.getByRole("dialog")).toHaveAccessibleName("Weather: screenshot 1 of 3")
    expect(within(screen.getByRole("dialog")).getByText("The forecast page").tagName).toBe(
      "FIGCAPTION",
    )
    await user.keyboard("{Escape}")
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
    // Back on the thumbnail of the screenshot last shown.
    expect(document.activeElement).toBe(
      screen.getByRole("button", { name: /Open screenshot 1 of 3/ }),
    )
  })

  it("names a screenshot without an alternative by the plugin, and refuses hostile images", async () => {
    renderShots(hostileListings["acme.clock"]?.screenshots)
    const list = screen.getByRole("list", { name: "Screenshots" })
    expect(within(list).getAllByRole("img", { name: /Weather, screenshot 2/ })).toHaveLength(1)
    await vi.waitFor(() => {
      expect(list.querySelectorAll('[data-state="fallback"]')).toHaveLength(2)
    })
    expect(list.querySelector("img")).toBeNull()
  })
})
