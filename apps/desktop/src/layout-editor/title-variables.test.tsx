// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"

import type { CatalogEntry } from "@/lib/types"

import { TitlePreview, TitleVariablePicker } from "./title-variables"

// The agent's variable events (`agent-variables`), delivered by hand.
const events = vi.hoisted(() => ({ handler: null as null | ((e: { payload: unknown }) => void) }))
vi.mock("@tauri-apps/api/event", () => ({
  listen: (_: string, handler: (e: { payload: unknown }) => void) => {
    events.handler = handler
    return Promise.resolve(() => {
      events.handler = null
    })
  },
}))

const resolve = vi.hoisted(() => vi.fn())
const list = vi.hoisted(() => vi.fn())
vi.mock("@/lib/display-templates", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/display-templates")>()),
  resolveDisplayTemplates: resolve,
  listDisplayVariables: list,
}))

afterEach(cleanup)

describe("a title with variables", () => {
  it("shows what the key shows, marks only the markers before the cut, and says why", async () => {
    const template = "{{ shared.team.name }} {{ projects.current.key }}"
    resolve.mockResolvedValue([
      {
        template,
        // 34 characters: cut to 31 and `…`; the second marker is past the cut.
        text: `${"T".repeat(28)} - - -`,
        markers: [
          { start: 29, end: 30, reference: "shared.team.name" },
          { start: 33, end: 34, reference: "projects.current.key" },
        ],
        unresolved: [
          { reference: "shared.team.name", reason: "missing", missing: { kind: "noValue" } },
        ],
      },
    ])
    render(<TitlePreview profileId="p" template={template} />)
    const text = await screen.findByText(
      (_, el) => el?.getAttribute("data-slot") === "title-preview-text",
    )
    expect(text.textContent).toBe(`${"T".repeat(28)} - …`)
    const marks = text.querySelectorAll("mark")
    expect([...marks].map((m) => m.getAttribute("data-reference"))).toEqual(["shared.team.name"])
    expect(screen.getByText("The key shows at most 32 characters.")).toBeInTheDocument()
    expect(screen.getByRole("note")).toHaveTextContent(
      "shared.team.name shows “-”: it has no value now.",
    )
    expect(resolve).toHaveBeenCalledWith({ profile: "p" }, [template])
  })

  it("is not resolved when the title is plain", async () => {
    resolve.mockClear()
    render(<TitlePreview profileId="p" template="Mail" />)
    await new Promise((r) => setTimeout(r, 300))
    expect(resolve).not.toHaveBeenCalled()
  })

  it("inserts a display-safe reference from the picker", async () => {
    list.mockResolvedValue([
      {
        reference: "projects.current.key",
        label: "Current project key",
        group: "projects",
        valueType: { type: "string" },
        availability: "available",
      },
    ])
    const onPick = vi.fn()
    const user = userEvent.setup()
    render(<TitleVariablePicker profileId="p" onPick={onPick} />)
    await user.click(screen.getByRole("button", { name: "Insert a variable into the title" }))
    await user.click(await screen.findByText("Current project key"))
    expect(onPick).toHaveBeenCalledWith("projects.current.key")
    expect(list).toHaveBeenCalledWith({ profile: "p" })
  })

  it("offers only display-safe references, each once, whatever the agent lists", async () => {
    const entry = (reference: string, extra: Partial<CatalogEntry> = {}): CatalogEntry => ({
      reference,
      label: reference,
      group: "shared",
      valueType: { type: "string" },
      availability: "available",
      ...extra,
    })
    list.mockResolvedValue([
      entry("shared.team.name"),
      entry("shared.token", { sensitive: true }),
      entry("shared.deploy", { valueType: { type: "secretRef" } }),
      entry("projects.current.id", { valueType: { type: "projectId" } }),
      entry("system.project.current.id"),
      entry("steps.fetch.output"),
      entry("workflow.input"),
      entry("system.device.name"),
      entry("shared.run", { availability: "runtimeOnly" }),
      entry("projects.current.key"),
      // A legacy alias of a listed reference: listed once, under the canonical id.
      entry("plugins.deckpilot.projects.current.key", {
        deprecated: { canonical: "projects.current.key" },
      } as Partial<CatalogEntry>),
    ])
    const user = userEvent.setup()
    render(<TitleVariablePicker profileId="p" onPick={vi.fn()} />)
    await user.click(screen.getByRole("button", { name: "Insert a variable into the title" }))
    const listed = await screen.findByRole("list", { name: "Variables a key can show" })
    const references = [...listed.querySelectorAll("code")].map((c) => c.textContent)
    expect(references).toEqual(["shared.team.name", "projects.current.key"])
  })

  it("sends the older placeholders to the agent, as typed", async () => {
    resolve.mockReset()
    resolve.mockImplementation((_: unknown, [template]: string[]) =>
      Promise.resolve([
        { template, text: "Work · -", markers: [{ start: 7, end: 8, reference: "project.key" }] },
      ]),
    )
    render(<TitlePreview profileId="p" template="{profile} · {project.key}" />)
    const text = await screen.findByText(
      (_, el) => el?.getAttribute("data-slot") === "title-preview-text",
    )
    expect(text.textContent).toBe("Work · -")
    expect(resolve).toHaveBeenCalledWith({ profile: "p" }, ["{profile} · {project.key}"])
  })

  it("is resolved again when the agent reports a change, without polling", async () => {
    resolve.mockReset()
    let value = "Alpha"
    resolve.mockImplementation((_: unknown, [template]: string[]) =>
      Promise.resolve([{ template, text: value }]),
    )
    render(<TitlePreview profileId="p" template="{{ shared.team.name }}" />)
    expect(await screen.findByText("Alpha")).toBeInTheDocument()
    expect(resolve).toHaveBeenCalledTimes(1)
    value = "Beta"
    await waitFor(() => {
      expect(events.handler).not.toBeNull()
    })
    events.handler?.({ payload: { type: "sharedVariablesChanged", keys: ["team.name"] } })
    expect(await screen.findByText("Beta", {}, { timeout: 2000 })).toBeInTheDocument()
    // Another profile's change does not ask again.
    const calls = resolve.mock.calls.length
    events.handler?.({ payload: { type: "profileChanged", profile: "other" } })
    await new Promise((r) => setTimeout(r, 600))
    expect(resolve.mock.calls.length).toBe(calls)
  })
})
