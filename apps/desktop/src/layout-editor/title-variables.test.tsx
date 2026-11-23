// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"

import { TitlePreview, TitleVariablePicker } from "./title-variables"

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
        valueType: "string",
        availability: { state: "available" },
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
})
