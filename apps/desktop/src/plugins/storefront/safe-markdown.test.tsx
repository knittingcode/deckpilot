// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, render } from "@testing-library/react"
import { afterEach, describe, expect, it } from "vitest"

import { hostileListings, listingFixtures } from "../storefront-mocks"
import { parseInline, parseMarkdown } from "./markdown"
import { SafeMarkdown } from "./safe-markdown"

afterEach(cleanup)

function renderMd(source: string) {
  const { container } = render(<SafeMarkdown source={source} />)
  return container.querySelector<HTMLElement>('[data-slot="safe-markdown"]') as HTMLElement
}

/** Only these elements may ever come out of the markdown subset. */
const ALLOWED = new Set(["DIV", "P", "UL", "LI", "STRONG", "EM", "CODE", "BR", "SPAN"])

function onlyAllowedElements(root: HTMLElement) {
  for (const el of root.querySelectorAll("*")) {
    expect(ALLOWED.has(el.tagName), el.tagName).toBe(true)
    for (const attr of el.attributes) {
      expect(["class", "data-slot", "dir"]).toContain(attr.name)
    }
  }
}

describe("safe markdown", () => {
  it("renders the subset: paragraphs, breaks, lists, bold, em, code, https links", () => {
    const root = renderMd(listingFixtures["deckpilot.weather"]?.descriptionMd ?? "")
    expect(root.querySelectorAll("p")).toHaveLength(2)
    expect(root.querySelector("strong")).toHaveTextContent("7-day")
    expect(root.querySelectorAll("ul > li")).toHaveLength(2)
    expect(root.querySelector("em")).toHaveTextContent("workflows")
    // A link shows its text and its full URL; nothing navigates.
    expect(root.querySelector("a")).toBeNull()
    expect(root.querySelector('[data-slot="markdown-link"]')).toHaveTextContent(
      "documentation (https://github.com/knittingcode/deckpilot-plugins)",
    )
    expect(renderMd("a\nb").querySelector("br")).not.toBeNull()
    expect(renderMd("`24h` and `12h`").querySelectorAll("code")).toHaveLength(2)
    onlyAllowedElements(root)
  })

  it("keeps the hostile listing's markup, scripts, images and bad links as literal text", () => {
    const source = hostileListings["acme.clock"]?.descriptionMd ?? ""
    const started = performance.now()
    const whole = renderMd(source)
    // The emphasis bomb and the long line stay linear; the input is clipped.
    expect(performance.now() - started).toBeLessThan(1500)
    onlyAllowedElements(whole)
    expect(whole.textContent.length).toBeLessThanOrEqual(8000)
    cleanup()
    // Each hostile paragraph on its own, so none is clipped away.
    const { container } = render(
      <>
        {source.split("\n\n").map((part, i) => (
          <SafeMarkdown key={i} source={part} />
        ))}
      </>,
    )
    const root = container
    for (const md of root.querySelectorAll<HTMLElement>('[data-slot="safe-markdown"]')) {
      onlyAllowedElements(md)
    }
    expect(root.querySelector("script, img, svg, a, iframe, h1, table")).toBeNull()
    const text = root.textContent
    expect(text).toContain("<script>alert(1)</script>")
    expect(text).toContain('<img src=x onerror="alert(1)">')
    expect(text).toContain("[click](javascript:alert(1))")
    expect(text).toContain("[data](data:text/html;base64,")
    expect(text).toContain("[http](http://acme.example/)")
    expect(text).toContain("![image](https://acme.example/x.png)")
    expect(text).toContain("# Heading")
    expect(text).toContain("| a | b |")
    expect(text).toContain("**unclosed _nested `code")
    // The RTL override is removed, so it cannot reorder anything.
    expect(text).not.toContain("\u202E")
    expect(text).toContain("eviltxt.exe")
    // No link came out of any of it.
    expect(root.querySelector('[data-slot="markdown-link"]')).toBeNull()
  })

  it("survives an emphasis bomb and a 6000-character line", () => {
    const bomb = "*".repeat(10000)
    const nodes = parseInline(bomb)
    expect(nodes.every((n) => n.type === "text")).toBe(true)
    expect(nodes.map((n) => (n.type === "text" ? n.text : "")).join("")).toBe(bomb)
    const blocks = parseMarkdown(`${"W".repeat(6000)}\n\n${"_a ".repeat(3000)}`)
    expect(blocks).toHaveLength(2)
    // The input is clipped to a bounded length.
    expect(JSON.stringify(parseMarkdown("x".repeat(50000))).length).toBeLessThan(9000)
  })

  it("does not nest without bound and keeps intraword underscores", () => {
    expect(parseInline("snake_case_name")).toEqual([{ type: "text", text: "snake_case_name" }])
    const nested = parseInline("**a _b_ a**")
    expect(nested[0]?.type === "strong" && nested[0].children.map((n) => n.type)).toEqual([
      "text",
      "em",
      "text",
    ])
    // Past the depth limit, markers are literal.
    expect(parseInline("**x**", undefined, 3)).toEqual([{ type: "text", text: "**x**" }])
    expect(parseInline("****")).toEqual([{ type: "text", text: "****" }])
    expect(parseInline("** spaced **")).toEqual([{ type: "text", text: "** spaced **" }])
  })

  it("renders release notes from the hostile listing literally", () => {
    for (const note of hostileListings["acme.clock"]?.releaseNotes ?? []) {
      const root = renderMd(note.text)
      onlyAllowedElements(root)
      expect(root.querySelector("script")).toBeNull()
    }
  })
})
