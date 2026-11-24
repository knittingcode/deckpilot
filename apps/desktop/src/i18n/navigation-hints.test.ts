/// <reference types="node" />
// Texts that send the user somewhere ("Start it in Settings › Background
// Agent", "turn it on in Marketplace › Installed") name a place that
// exists, under the name it is shown with, in both languages.

import { readdirSync, readFileSync } from "node:fs"
import { join } from "node:path"

import { describe, expect, it } from "vitest"

const LOCALES = join(__dirname, "locales")

type Tree = { [key: string]: string | Tree }

function strings(tree: Tree): string[] {
  return Object.values(tree).flatMap((v) => (typeof v === "string" ? [v] : strings(v)))
}

function locale(language: string): { file: string; texts: string[] }[] {
  return readdirSync(join(LOCALES, language))
    .filter((f) => f.endsWith(".json"))
    .map((file) => ({
      file,
      texts: strings(JSON.parse(readFileSync(join(LOCALES, language, file), "utf8")) as Tree),
    }))
}

function json(language: string, file: string): Tree {
  return JSON.parse(readFileSync(join(LOCALES, language, file), "utf8")) as Tree
}

/** The names of the settings pages, and of the Marketplace's sections, in `language`. */
function places(language: string) {
  const settings = json(language, "settings.json")
  const pages = Object.values(settings.pages as Tree)
    .map((page) => (typeof page === "string" ? null : page.title))
    .filter((title): title is string => typeof title === "string")
  const marketplace = Object.values(json(language, "marketplace.json").nav as Tree).filter(
    (name): name is string => typeof name === "string",
  )
  // A module's own settings page is named by the module (e.g. HTTP).
  return { settings: [...pages, "HTTP"], marketplace }
}

const PREFIX = {
  en: { settings: /Settings › /g, marketplace: /Marketplace › /g },
  pl: { settings: /Ustawienia(?:ch)? › /g, marketplace: /Marketplace › /g },
} as const

describe.each(["en", "pl"] as const)("places named in %s texts", (language) => {
  const known = places(language)
  const named = (kind: "settings" | "marketplace") =>
    locale(language).flatMap(({ file, texts }) =>
      texts.flatMap((text) =>
        [...text.matchAll(PREFIX[language][kind])].map((m) => ({
          file,
          rest: text.slice(m.index + m[0].length),
          text,
        })),
      ),
    )

  it("are settings pages that exist", () => {
    const found = named("settings")
    expect(found.length).toBeGreaterThan(0)
    const unknown = found.filter(
      ({ rest }) => !known.settings.some((name) => rest.startsWith(name)),
    )
    expect(unknown.map(({ file, text }) => `${file}: ${text}`)).toEqual([])
  })

  it("are Marketplace sections that exist", () => {
    const unknown = named("marketplace").filter(
      ({ rest }) => !known.marketplace.some((name) => rest.startsWith(name)),
    )
    expect(unknown.map(({ file, text }) => `${file}: ${text}`)).toEqual([])
  })
})
