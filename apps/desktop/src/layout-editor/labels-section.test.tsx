// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, fireEvent, render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import type { KeyLabel, LabelBox } from "@/lib/key-labels"
import type { ControlBinding, DeviceLayout, KeyAppearance } from "@/lib/types"

import { BUILTIN_APPEARANCE, type KeyLookChange } from "./appearance"
import type { KeyLook } from "./key-look-context"
import { LabelsOfSelection, LabelsSection } from "./labels-section"
import { LookMoveDialog } from "./look-move-dialog"

Element.prototype.hasPointerCapture = () => false
Element.prototype.releasePointerCapture = () => undefined
Element.prototype.scrollIntoView = () => undefined

const key0 = { kind: "key", index: 0 } as const
const key1 = { kind: "key", index: 1 } as const

function layoutWith(appearances: (KeyAppearance | undefined)[]): DeviceLayout {
  const bindings: ControlBinding[] = appearances.map((appearance, index) => ({
    control: { kind: "key", index },
    trigger: { type: "press" },
    target: { type: "back" },
    label: null,
    ...(appearance ? { appearance } : {}),
  }))
  return {
    id: "l",
    name: "L",
    model: "m",
    homePage: "h",
    pages: [{ id: "h", name: "H", bindings }],
  }
}

const box = (id: string): LabelBox => ({
  id,
  x: 20,
  y: 50,
  width: 32,
  height: 12,
  canvas: { width: 72, height: 72 },
  layer: "overIcon",
  truncated: false,
  overlapsTitle: false,
  plate: true,
  fontPoints: 10,
  lines: 1,
})

let restyle: ReturnType<typeof vi.fn<(change: KeyLookChange, group?: string) => void>>

function lookOf(draft: DeviceLayout, extra: Partial<KeyLook> = {}): KeyLook {
  return {
    draft,
    pageId: "h",
    defaults: BUILTIN_APPEARANCE,
    images: new Map(),
    faces: new Map([
      [
        "key:0",
        {
          style: "action",
          title: "",
          labels: [
            { id: "a", text: "SHOP", spec: {} },
            { id: "b", text: "-", spec: {}, unresolved: true },
          ],
        },
      ],
    ]),
    labelBoxes: new Map([["key:0", [box("a"), box("b")]]]),
    keyLabels: true,
    icons: {} as KeyLook["icons"],
    library: {} as KeyLook["library"],
    typeOf: () => undefined,
    restyle,
    ...extra,
  }
}

/** What the last change does to `appearance`. */
const applied = (appearance: KeyAppearance) => {
  const change = restyle.mock.lastCall?.[0]
  return typeof change === "function" ? change(appearance) : change
}

beforeEach(() => {
  restyle = vi.fn()
  sessionStorage.clear()
})
afterEach(cleanup)

const labels: KeyLabel[] = [
  { id: "a", text: "{{ projects.current.key }}" },
  { id: "b", text: "Team", position: { mode: "anchored", to: "topRight", inset: 2 } },
]

describe("the labels section", () => {
  it("adds a bottom label with the project's name and remembers being open", async () => {
    const user = userEvent.setup()
    render(<LabelsSection look={lookOf(layoutWith([undefined]))} control={key0} />)
    expect(screen.getByRole("button", { name: "Labels" })).toHaveAttribute("aria-expanded", "false")
    await user.click(screen.getByRole("button", { name: /Add label/ }))
    const next = applied({})
    expect(next).toEqual({
      labels: [
        {
          id: expect.any(String) as string,
          text: "{{ projects.current.name }}",
          position: { mode: "preset", at: "bottom" },
        },
      ],
    })
    expect(sessionStorage.getItem("deckpilot.editor.labelsOpen")).toBe("1")
  })

  it("lists labels front first with their resolved text, a mark for a missing value and the position", async () => {
    const user = userEvent.setup()
    render(<LabelsSection look={lookOf(layoutWith([{ labels }]))} control={key0} />)
    await user.click(screen.getByRole("button", { name: /Labels/ }))
    const rows = screen.getAllByRole("listitem")
    expect(rows.map((r) => r.textContent)).toEqual([
      expect.stringContaining("-") as string,
      expect.stringContaining("SHOP") as string,
    ])
    expect(within(rows[0] as HTMLElement).getByText("Top right")).toBeInTheDocument()
    expect(
      within(rows[0] as HTMLElement).getByRole("img", { name: /has no value now/ }),
    ).toBeInTheDocument()
    // Hiding a label keeps it.
    await user.click(screen.getByRole("switch", { name: "Show the label “Team”" }))
    expect(applied({ labels })?.labels?.[1]).toMatchObject({ id: "b", enabled: false })
  })

  it("moves a selected label by a unit with the arrow keys, as a free position", async () => {
    const user = userEvent.setup()
    sessionStorage.setItem("deckpilot.editor.labelsOpen", "1")
    render(<LabelsSection look={lookOf(layoutWith([{ labels }]))} control={key0} />)
    await user.click(screen.getByRole("button", { name: /SHOP/ }))
    const stage = screen.getByRole("group", { name: /Labels on Key 1/ })
    fireEvent.keyDown(stage, { key: "ArrowRight" })
    expect(applied({ labels })?.labels?.[0]).toEqual({
      id: "a",
      text: "{{ projects.current.key }}",
      position: { mode: "absolute", x: 514, y: 778 },
    })
  })

  it("says when the agent does not draw labels", async () => {
    const user = userEvent.setup()
    render(
      <LabelsSection
        look={lookOf(layoutWith([{ labels }]), { keyLabels: false })}
        control={key0}
      />,
    )
    await user.click(screen.getByRole("button", { name: /Labels/ }))
    expect(screen.getByRole("note")).toHaveTextContent("does not draw labels yet")
  })
})

describe("the plate's opacity in the editor", () => {
  it("shows the renderer's default over the title (85) and never writes it unless changed", async () => {
    const user = userEvent.setup()
    sessionStorage.setItem("deckpilot.editor.labelsOpen", "1")
    const look = lookOf(layoutWith([{ labels }]), {
      labelBoxes: new Map([["key:0", [{ ...box("a"), overlapsTitle: true }, box("b")]]]),
    })
    render(<LabelsSection look={look} control={key0} />)
    await user.click(screen.getByRole("button", { name: /SHOP/ }))
    await user.click(screen.getByRole("button", { name: "More" }))
    const opacity = screen.getByLabelText("Opacity (%)")
    expect(opacity).toHaveValue(null)
    expect(opacity).toHaveAttribute("placeholder", "85")
    // Another change writes no opacity.
    await user.type(screen.getByLabelText("Size (pt)"), "12")
    expect(applied({ labels })?.labels?.[0]).not.toHaveProperty("backgroundOpacity")
    // Only typing one does.
    await user.type(opacity, "70")
    expect(applied({ labels })?.labels?.[0]).toMatchObject({ backgroundOpacity: 70 })
  })

  it("shows 60 where the plate covers neither the title nor a mark", async () => {
    const user = userEvent.setup()
    sessionStorage.setItem("deckpilot.editor.labelsOpen", "1")
    render(<LabelsSection look={lookOf(layoutWith([{ labels }]))} control={key0} />)
    await user.click(screen.getByRole("button", { name: /SHOP/ }))
    await user.click(screen.getByRole("button", { name: "More" }))
    expect(screen.getByLabelText("Opacity (%)")).toHaveAttribute("placeholder", "60")
  })
})

describe("labels of a multi-selection", () => {
  it("copies the primary key's labels to the others with new ids", async () => {
    const user = userEvent.setup()
    const source: KeyAppearance = { labels }
    render(
      <LabelsOfSelection
        look={lookOf(layoutWith([source, { bold: true }]))}
        controls={[key0, key1]}
      />,
    )
    await user.click(screen.getByRole("button", { name: "Copy labels from Key 1" }))
    expect(applied(source)).toBe(source)
    const copied = applied({ bold: true })
    expect(copied?.bold).toBe(true)
    expect(copied?.labels?.map((l) => l.text)).toEqual(labels.map((l) => l.text))
    expect(copied?.labels?.map((l) => l.id)).not.toContain("a")
  })
})

describe("moving a look with labels", () => {
  it("offers keep, replace and merge, or cancel", async () => {
    const user = userEvent.setup()
    const onChoose = vi.fn()
    render(<LookMoveDialog open labels={2} trigger="Long press" onChoose={onChoose} />)
    expect(screen.getByRole("alertdialog")).toHaveTextContent("has 2 labels, and Long press")
    await user.click(screen.getByRole("button", { name: "Merge the labels" }))
    expect(onChoose).toHaveBeenLastCalledWith("merge")
    await user.click(screen.getByRole("button", { name: "Keep its look" }))
    expect(onChoose).toHaveBeenLastCalledWith("keep")
  })
})
