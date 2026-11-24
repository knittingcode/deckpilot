// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, fireEvent, render, screen, within } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { TooltipProvider } from "@/components/ui/tooltip"
import { installDomStubs } from "@/test/dom"
import {
  fakeActions,
  storefrontSnapshot,
  stubReducedMotion,
  withStorefrontClient,
} from "@/test/storefront"

import { createFixtureStorefrontClient } from "../storefront-mocks"
import { CardRow } from "./card-row"
import { clearMediaCache } from "./media-cache"

beforeAll(installDomStubs)
beforeEach(() => {
  clearMediaCache()
  stubReducedMotion(false)
})
afterEach(cleanup)

function renderRow() {
  const snapshot = storefrontSnapshot()
  const actions = fakeActions()
  render(
    withStorefrontClient(
      createFixtureStorefrontClient(),
      <TooltipProvider>
        <CardRow
          title="Stay focused"
          description="Timers and reminders."
          plugins={snapshot.available.slice(0, 4)}
          snapshot={snapshot}
          updates={[]}
          migrations={[]}
          actions={actions}
        />
      </TooltipProvider>,
    ),
  )
  const list = screen.getByRole("list", { name: "Stay focused" })
  return { list, actions }
}

/** Gives the scroller a layout: `width` of content, `client` visible, at `left`. */
function layout(list: HTMLElement, width: number, client: number, left: number) {
  Object.defineProperty(list, "scrollWidth", { configurable: true, value: width })
  Object.defineProperty(list, "clientWidth", { configurable: true, value: client })
  Object.defineProperty(list, "scrollLeft", { configurable: true, value: left })
  fireEvent.scroll(list)
}

describe("card row", () => {
  it("lists the catalog's cards under the row's name", () => {
    const { list } = renderRow()
    expect(within(list).getAllByRole("listitem")).toHaveLength(4)
    expect(screen.getByRole("heading", { name: "Stay focused" })).toBeInTheDocument()
    // Focusable cards: each opens the plugin's page.
    expect(within(list).getByRole("button", { name: "Weather" })).toBeInTheDocument()
  })

  it("has no scroll buttons when everything fits", () => {
    const { list } = renderRow()
    layout(list, 600, 600, 0)
    expect(screen.queryByRole("button", { name: /Scroll Stay focused/ })).not.toBeInTheDocument()
  })

  it("scrolls by about a page, each button disabled at its end", () => {
    const { list } = renderRow()
    const scrollBy = vi.fn()
    list.scrollBy = scrollBy as typeof list.scrollBy
    layout(list, 1200, 400, 0)
    const back = screen.getByRole("button", { name: "Scroll Stay focused back" })
    const forward = screen.getByRole("button", { name: "Scroll Stay focused forward" })
    expect(back).toBeDisabled()
    expect(forward).toBeEnabled()
    fireEvent.click(forward)
    expect(scrollBy).toHaveBeenLastCalledWith({ left: 360, behavior: "smooth" })
    layout(list, 1200, 400, 800)
    expect(back).toBeEnabled()
    expect(forward).toBeDisabled()
    fireEvent.click(back)
    expect(scrollBy).toHaveBeenLastCalledWith({ left: -360, behavior: "smooth" })
  })

  it("scrolls without animation with reduced motion", () => {
    stubReducedMotion(true)
    const { list } = renderRow()
    const scrollBy = vi.fn()
    list.scrollBy = scrollBy as typeof list.scrollBy
    layout(list, 1200, 400, 0)
    fireEvent.click(screen.getByRole("button", { name: "Scroll Stay focused forward" }))
    expect(scrollBy).toHaveBeenLastCalledWith({ left: 360, behavior: "auto" })
  })
})
