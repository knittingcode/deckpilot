// Nothing that renders storefront or listing data turns text into HTML,
// navigates the webview or opens anything outside it (storefront design
// §4 and R2): a source check of the storefront folder and of every
// Marketplace file that renders its data, so a later change cannot slip
// one in.

import { readdirSync, readFileSync } from "node:fs"
import path from "node:path"

import { describe, expect, it } from "vitest"

const DIR = import.meta.dirname
/** Marketplace files outside the folder that render storefront or listing data. */
const RENDERERS = [
  "../details.tsx",
  "../plugin-tile.tsx",
  "../home.tsx",
  "../marketplace-sidebar.tsx",
  "../marketplace.tsx",
]
const FORBIDDEN: [RegExp, string][] = [
  [/dangerouslySetInnerHTML/, "dangerouslySetInnerHTML"],
  [/\.innerHTML\b|\.outerHTML\s*=/, "innerHTML / outerHTML"],
  [/insertAdjacentHTML|createContextualFragment|DOMParser|document\.write/, "HTML parsing"],
  [/\beval\s*\(|new Function\s*\(/, "eval"],
  [/<a\s|<iframe|<object|<embed/, "a navigating element"],
  [/window\.open\s*\(|location\.(href|assign|replace)/, "navigation"],
  [/@tauri-apps\/plugin-(opener|shell)|@tauri-apps\/api\/shell/, "the Tauri opener or shell"],
  [/\bshell\s*\.\s*open\s*\(|\bopener\s*\.\s*\w+\s*\(/, "the shell or opener object"],
  [/["'`]plugin:(opener|shell)\|/, "the opener or shell plugin's commands"],
  [
    /(?<![\w.$])open\s*\(|\bopenUrl\s*\(|\bopenPath\s*\(|revealItemInDir/,
    "opening outside the app",
  ],
]

describe("storefront sources", () => {
  const files = [
    ...readdirSync(DIR).filter(
      (f) => /\.(ts|tsx)$/.test(f) && !f.endsWith(".test.ts") && !f.endsWith(".test.tsx"),
    ),
    ...RENDERERS,
  ]

  it("are checked", () => {
    expect(files.length).toBeGreaterThan(10)
  })

  for (const file of files) {
    it(`${file} has no HTML injection or navigation`, () => {
      // Code only: a comment may name what it forbids.
      const source = readFileSync(path.join(DIR, file), "utf8")
        .replace(/\/\*[\s\S]*?\*\//g, "")
        .replace(/^\s*\/\/.*$/gm, "")
      for (const [pattern, what] of FORBIDDEN) {
        expect(pattern.test(source), `${file}: ${what}`).toBe(false)
      }
    })
  }

  it("the check itself catches an offender", () => {
    expect(FORBIDDEN.some(([p]) => p.test("<div dangerouslySetInnerHTML={{ __html: x }} />"))).toBe(
      true,
    )
    expect(FORBIDDEN.some(([p]) => p.test("el.innerHTML = text"))).toBe(true)
    expect(
      FORBIDDEN.some(([p]) => p.test('import { openUrl } from "@tauri-apps/plugin-opener"')),
    ).toBe(true)
    expect(FORBIDDEN.some(([p]) => p.test('import { open } from "@tauri-apps/plugin-shell"'))).toBe(
      true,
    )
    expect(FORBIDDEN.some(([p]) => p.test("void open(url)"))).toBe(true)
    expect(FORBIDDEN.some(([p]) => p.test("await shell.open(url)"))).toBe(true)
    expect(FORBIDDEN.some(([p]) => p.test("opener.openUrl(url)"))).toBe(true)
    expect(FORBIDDEN.some(([p]) => p.test('invoke("plugin:opener|open_url", { url })'))).toBe(true)
    expect(FORBIDDEN.some(([p]) => p.test("invoke('plugin:shell|open', { path })"))).toBe(true)
    expect(FORBIDDEN.some(([p]) => p.test("invoke(`plugin:shell|execute`)"))).toBe(true)
    // A state setter is not an opener.
    expect(FORBIDDEN.some(([p]) => p.test("setOpen(!open)"))).toBe(false)
  })
})
