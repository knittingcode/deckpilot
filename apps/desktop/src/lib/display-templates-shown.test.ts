import { describe, expect, it } from "vitest"

import {
  MAX_SHOWN_TITLE_CHARS,
  maxTitleLength,
  offeredOnKey,
  pickerEntries,
  referenceText,
  shownOnKey,
  titleSegments,
} from "./display-templates"
import type { CatalogEntry } from "./types"

describe("what a key shows of a resolved title", () => {
  it("keeps a short title and its markers as they are", () => {
    const shown = shownOnKey({
      text: "Projekt: -",
      markers: [{ start: 9, end: 10, reference: "projects.current.key" }],
    })
    expect(shown).toEqual({
      text: "Projekt: -",
      markers: [{ start: 9, end: 10, reference: "projects.current.key" }],
      cut: false,
    })
    expect(titleSegments(shown)).toEqual([
      { text: "Projekt: " },
      { text: "-", marker: { start: 9, end: 10, reference: "projects.current.key" } },
    ])
  })

  it("cuts at 32 characters and never keeps a marker past the cut", () => {
    // 34 characters: the key keeps 31 of them and adds `…`.
    const text = `${"a".repeat(28)} - - -`
    const shown = shownOnKey({
      text,
      markers: [
        { start: 29, end: 30, reference: "shared.inside" },
        { start: 33, end: 34, reference: "shared.past" },
      ],
    })
    expect(Array.from(shown.text)).toHaveLength(MAX_SHOWN_TITLE_CHARS)
    expect(shown.text.endsWith("…")).toBe(true)
    expect(shown.cut).toBe(true)
    expect(shown.markers.map((m) => m.reference)).toEqual(["shared.inside"])
    for (const m of shown.markers) expect(m.end).toBeLessThanOrEqual(shown.text.length - 1)
  })

  it("leaves out a marker the cut goes through", () => {
    const text = `${"b".repeat(30)}-----`
    const shown = shownOnKey({ text, markers: [{ start: 30, end: 35, reference: "x" }] })
    expect(shown.markers).toEqual([])
  })

  it("counts characters, not UTF-16 units, as the agent does", () => {
    const text = `${"😀".repeat(30)}-xyz`
    const shown = shownOnKey({ text, markers: [{ start: 60, end: 61, reference: "r" }] })
    expect(Array.from(shown.text)).toHaveLength(32)
    expect(shown.markers).toEqual([{ start: 60, end: 61, reference: "r" }])
    expect(titleSegments(shown).find((s) => s.marker)?.text).toBe("-")
  })
})

describe("title limits and insertion", () => {
  it("allows 160 characters only with references", () => {
    expect(maxTitleLength("Plain")).toBe(32)
    expect(maxTitleLength("{{ shared.team.name }}")).toBe(160)
    expect(referenceText("projects.current.key")).toBe("{{ projects.current.key }}")
  })
})

describe("what the title picker offers", () => {
  const entry = (reference: string, extra: Partial<CatalogEntry> = {}): CatalogEntry => ({
    reference,
    label: reference,
    group: "shared",
    valueType: { type: "string" },
    availability: "available",
    ...extra,
  })

  it("offers display-safe text, numbers and booleans", () => {
    expect(offeredOnKey(entry("shared.team.name"))).toBe(true)
    expect(offeredOnKey(entry("shared.count", { valueType: { type: "integer" } }))).toBe(true)
    expect(offeredOnKey(entry("system.profile.current.name"))).toBe(true)
  })

  it("never offers secrets, sensitive values, ids or values of a run or a device", () => {
    expect(offeredOnKey(entry("shared.token", { sensitive: true }))).toBe(false)
    expect(offeredOnKey(entry("shared.key", { valueType: { type: "secretRef" } }))).toBe(false)
    expect(offeredOnKey(entry("shared.blob", { valueType: { type: "json" } }))).toBe(false)
    expect(offeredOnKey(entry("projects.current.id"))).toBe(false)
    expect(offeredOnKey(entry("workSession.projectId"))).toBe(false)
    expect(offeredOnKey(entry("steps.a.output"))).toBe(false)
    expect(offeredOnKey(entry("workflow.input"))).toBe(false)
    expect(offeredOnKey(entry("system.device.name"))).toBe(false)
    expect(offeredOnKey(entry("shared.x", { availability: "runtimeOnly" }))).toBe(false)
    // A name that merely ends in "id" is not an id.
    expect(offeredOnKey(entry("shared.paid"))).toBe(true)
  })

  it("lists each reference once, a legacy alias under its canonical id", () => {
    const listed = pickerEntries([
      entry("plugins.deckpilot.projects.current.key", {
        deprecated: { canonical: "projects.current.key", since: "0.10" },
      }),
      entry("shared.team.name"),
      entry("shared.team.name"),
    ])
    expect(listed.map((e) => e.reference)).toEqual([
      "plugins.deckpilot.projects.current.key",
      "shared.team.name",
    ])
  })
})
