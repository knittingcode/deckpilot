import { describe, expect, it } from "vitest"

import { matchesWords, searchWords } from "./search"

describe("the app's one search rule", () => {
  it("finds a text holding every word, in any order and any case", () => {
    expect(matchesWords("open url", "Open URL")).toBe(true)
    expect(matchesWords("URL open", "Open URL")).toBe(true)
    expect(matchesWords("  docker   STATUS ", "Docker status", "Docker")).toBe(true)
    // Across the texts of an entry: its name and its category.
    expect(matchesWords("status system", "Docker status", "System")).toBe(true)
    expect(matchesWords("open folder", "Open URL")).toBe(false)
  })

  it("finds everything for an empty query, and skips missing texts", () => {
    expect(matchesWords("", "anything")).toBe(true)
    expect(matchesWords("   ")).toBe(true)
    expect(matchesWords("mail", undefined, null, "", "Mail")).toBe(true)
    expect(matchesWords("mail", undefined, null)).toBe(false)
  })

  it("lowers the case of every language", () => {
    expect(searchWords("ŹRÓDŁO Łódź")).toEqual(["źródło", "łódź"])
    expect(matchesWords("łódź", "Biuro ŁÓDŹ")).toBe(true)
  })
})
