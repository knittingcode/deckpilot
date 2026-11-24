// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, fireEvent, render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { type TranslationKey, i18n } from "@/i18n"
import type { KeyLabel, LabelBox } from "@/lib/key-labels"
import type { ControlBinding, DeviceLayout, KeyAppearance } from "@/lib/types"

import { BUILTIN_APPEARANCE, type KeyLookChange } from "./appearance"
import type { KeyLook } from "./key-look-context"
import { LabelsOfSelection, LabelsSection } from "./labels-section"
import { placedAt, updateLabel, withLabels } from "./labels"
import { LookMoveDialog } from "./look-move-dialog"

// The variable picker inserts the reference it is given (its own list is
// tested in title-variables.test.tsx).
vi.mock("./title-variables", () => ({
  TitleVariablePicker: ({ onPick }: { onPick: (reference: string) => void }) => (
    <button
      type="button"
      onClick={() => {
        onPick("projects.current.key")
      }}
    >
      Insert a variable
    </button>
  ),
}))

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

  it("shows auto where the box has a plate but does not say why", async () => {
    const user = userEvent.setup()
    sessionStorage.setItem("deckpilot.editor.labelsOpen", "1")
    render(<LabelsSection look={lookOf(layoutWith([{ labels }]))} control={key0} />)
    await user.click(screen.getByRole("button", { name: /SHOP/ }))
    await user.click(screen.getByRole("button", { name: "More" }))
    expect(screen.getByLabelText("Opacity (%)")).toHaveAttribute("placeholder", "auto")
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
    await user.click(screen.getByRole("button", { name: "Keep its look and delete 2 labels" }))
    expect(onChoose).toHaveBeenLastCalledWith("keep")
  })
})

describe("merging labels beyond the limit", () => {
  it("is not offered when 5 and 5 labels would make 10, and says why", async () => {
    const user = userEvent.setup()
    const onChoose = vi.fn()
    render(<LookMoveDialog open labels={5} merged={10} trigger="Long press" onChoose={onChoose} />)
    const merge = screen.getByRole("button", { name: "Merge the labels" })
    expect(merge).toBeDisabled()
    expect(screen.getByRole("note")).toHaveTextContent(
      "Merging would give the key 10 labels; a key has at most 8.",
    )
    await user.click(merge)
    expect(onChoose).not.toHaveBeenCalled()
    // Keeping says what it deletes.
    await user.click(screen.getByRole("button", { name: "Keep its look and delete 5 labels" }))
    expect(onChoose).toHaveBeenLastCalledWith("keep")
  })
})

describe("label counts in Polish", () => {
  it("use the one, few and many forms", () => {
    const pl = i18n.getFixedT("pl")
    const forms = (key: TranslationKey, extra: Record<string, unknown> = {}) =>
      [1, 2, 5, 12, 22, 25].map((count) => pl(key, { count, ...extra }))
    expect(forms("editor:labels.count")).toEqual([
      "1 etykieta",
      "2 etykiety",
      "5 etykiet",
      "12 etykiet",
      "22 etykiety",
      "25 etykiet",
    ])
    expect(forms("editor:labels.moved", { trigger: "Długie naciśnięcie" })).toEqual([
      "Wygląd klawisza (1 etykieta) przeniesiono na Długie naciśnięcie.",
      "Wygląd klawisza (2 etykiety) przeniesiono na Długie naciśnięcie.",
      "Wygląd klawisza (5 etykiet) przeniesiono na Długie naciśnięcie.",
      "Wygląd klawisza (12 etykiet) przeniesiono na Długie naciśnięcie.",
      "Wygląd klawisza (22 etykiety) przeniesiono na Długie naciśnięcie.",
      "Wygląd klawisza (25 etykiet) przeniesiono na Długie naciśnięcie.",
    ])
    expect(
      forms("editor:labels.move.description", { trigger: "X" }).map(
        (text) => /ma (.+?), a X/.exec(text)?.[1],
      ),
    ).toEqual(["1 etykietę", "2 etykiety", "5 etykiet", "12 etykiet", "22 etykiety", "25 etykiet"])
    expect(forms("editor:labels.move.keep")).toEqual([
      "Zachowaj jego wygląd i usuń 1 etykietę",
      "Zachowaj jego wygląd i usuń 2 etykiety",
      "Zachowaj jego wygląd i usuń 5 etykiet",
      "Zachowaj jego wygląd i usuń 12 etykiet",
      "Zachowaj jego wygląd i usuń 22 etykiety",
      "Zachowaj jego wygląd i usuń 25 etykiet",
    ])
  })
})

describe("a label's text", () => {
  it("cannot be emptied: the label keeps its last text and the field says why", async () => {
    const user = userEvent.setup()
    sessionStorage.setItem("deckpilot.editor.labelsOpen", "1")
    render(<LabelsSection look={lookOf(layoutWith([{ labels }]))} control={key0} />)
    await user.click(screen.getByRole("button", { name: /SHOP/ }))
    const text = screen.getByRole("textbox", { name: "Text" })
    await user.clear(text)
    await user.type(text, "   ")
    // Nothing without text was committed.
    expect(restyle).not.toHaveBeenCalled()
    expect(text).toHaveAttribute("aria-invalid", "true")
    expect(screen.getByRole("alert")).toHaveTextContent("A label needs text")
    // Leaving the field shows the text the label kept.
    await user.tab()
    expect(text).toHaveValue("{{ projects.current.key }}")
    expect(screen.queryByRole("alert")).toBeNull()
    // Text again is committed.
    await user.clear(text)
    await user.type(text, "X")
    expect(applied({ labels })?.labels?.[0]).toMatchObject({ id: "a", text: "X" })
  })

  it("takes a variable at the cursor, within 160 characters", async () => {
    const user = userEvent.setup()
    sessionStorage.setItem("deckpilot.editor.labelsOpen", "1")
    // 134 characters: with the reference's 26, exactly 160.
    const long = `A${"b".repeat(133)}`
    const look = lookOf(layoutWith([{ labels: [{ id: "a", text: long }] }]), { profileId: "p" })
    const view = render(<LabelsSection look={look} control={key0} />)
    await user.click(screen.getByRole("button", { name: /SHOP/ }))
    const text = screen.getByRole<HTMLInputElement>("textbox", { name: "Text" })
    text.setSelectionRange(1, 1)
    await user.click(screen.getByRole("button", { name: "Insert a variable" }))
    const inserted = applied({ labels: [{ id: "a", text: long }] })?.labels?.[0]?.text
    expect(inserted).toBe(`A{{ projects.current.key }}${"b".repeat(133)}`)
    expect(inserted ? Array.from(inserted).length : 0).toBe(160)
    // One character more would pass 160: nothing is inserted.
    view.unmount()
    restyle.mockClear()
    const longer = `${long}c`
    render(
      <LabelsSection
        look={lookOf(layoutWith([{ labels: [{ id: "a", text: longer }] }]), { profileId: "p" })}
        control={key0}
      />,
    )
    await user.click(screen.getByRole("button", { name: /SHOP/ }))
    screen.getByRole<HTMLInputElement>("textbox", { name: "Text" }).setSelectionRange(1, 1)
    await user.click(screen.getByRole("button", { name: "Insert a variable" }))
    expect(restyle).not.toHaveBeenCalled()
  })
})

describe("a label's sizes", () => {
  it("lowers the smallest size when the size goes below it", async () => {
    const user = userEvent.setup()
    sessionStorage.setItem("deckpilot.editor.labelsOpen", "1")
    const shrinking: KeyLabel[] = [
      { id: "a", text: "Long name", size: 14, overflow: "shrink", minSize: 12 },
    ]
    render(<LabelsSection look={lookOf(layoutWith([{ labels: shrinking }]))} control={key0} />)
    await user.click(screen.getByRole("button", { name: /SHOP/ }))
    const size = screen.getByLabelText("Size (pt)")
    await user.clear(size)
    await user.type(size, "9")
    expect(applied({ labels: shrinking })?.labels?.[0]).toMatchObject({ size: 9, minSize: 9 })
  })
})

describe("moving labels before the renderer answers", () => {
  it("adds up two nudges made before the boxes refresh", async () => {
    const user = userEvent.setup()
    sessionStorage.setItem("deckpilot.editor.labelsOpen", "1")
    render(<LabelsSection look={lookOf(layoutWith([{ labels }]))} control={key0} />)
    await user.click(screen.getByRole("button", { name: /SHOP/ }))
    const stage = screen.getByRole("group", { name: /Labels on Key 1/ })
    fireEvent.keyDown(stage, { key: "ArrowRight" })
    expect(applied({ labels })?.labels?.[0]?.position).toEqual({ mode: "absolute", x: 514, y: 778 })
    // The same boxes still: the second step starts where the first put it.
    fireEvent.keyDown(stage, { key: "ArrowRight" })
    expect(applied({ labels })?.labels?.[0]?.position).toEqual({ mode: "absolute", x: 528, y: 778 })
  })

  it("puts a label against the edges from where a nudge left it, before the boxes refresh", async () => {
    const user = userEvent.setup()
    sessionStorage.setItem("deckpilot.editor.labelsOpen", "1")
    // Without a move: the mode change places it from the renderer's box.
    const still = render(<LabelsSection look={lookOf(layoutWith([{ labels }]))} control={key0} />)
    await user.click(screen.getByRole("button", { name: /SHOP/ }))
    await user.click(screen.getByRole("radio", { name: "Edge" }))
    const fromBox = applied({ labels })?.labels?.[0]
    still.unmount()
    render(<LabelsSection look={lookOf(layoutWith([{ labels }]))} control={key0} />)
    await user.click(screen.getByRole("button", { name: /SHOP/ }))
    const stage = screen.getByRole("group", { name: /Labels on Key 1/ })
    fireEvent.keyDown(stage, { key: "ArrowUp", shiftKey: true })
    // The same boxes still: Edge starts where the nudge put the label, so
    // the nudge is not undone.
    await user.click(screen.getByRole("radio", { name: "Edge" }))
    const fromNudge = applied({ labels })?.labels?.[0]
    expect(fromNudge?.position).toMatchObject({ mode: "anchored" })
    expect(fromNudge).not.toEqual(fromBox)
  })

  it("keeps a corner label where it is when it is put against the edges", async () => {
    const user = userEvent.setup()
    sessionStorage.setItem("deckpilot.editor.labelsOpen", "1")
    const look = lookOf(layoutWith([{ labels }]), {
      labelBoxes: new Map([["key:0", [{ ...box("a"), x: 4 }, box("b")]]]),
    })
    render(<LabelsSection look={look} control={key0} />)
    await user.click(screen.getByRole("button", { name: /SHOP/ }))
    await user.click(screen.getByRole("radio", { name: "Edge" }))
    expect(applied({ labels })?.labels?.[0]).toEqual({
      id: "a",
      text: "{{ projects.current.key }}",
      position: { mode: "anchored", to: "bottomLeft", inset: 4 },
      offsetY: -6,
    })
  })
})

describe("editing a label from its outline", () => {
  it("edits the text of a label double-clicked on the stage, opening the section", async () => {
    Element.prototype.setPointerCapture = () => undefined
    const user = userEvent.setup()
    sessionStorage.setItem("deckpilot.editor.labelsOpen", "1")
    render(<LabelsSection look={lookOf(layoutWith([{ labels }]))} control={key0} />)
    const outline = document.querySelector<HTMLElement>(
      "[data-slot=key-label-stage] [data-label=b]",
    )
    if (!outline) throw new Error("no outline")
    fireEvent.doubleClick(outline)
    const text = await screen.findByRole("textbox", { name: "Text" })
    expect(text).toHaveValue("Team")
    expect(text).toHaveFocus()
    await user.keyboard("O")
    // The whole text was selected: typing replaces it.
    expect(applied({ labels })?.labels?.[1]).toMatchObject({ id: "b", text: "O" })
  })

  it("follows the editor's shared selection (the device canvas' outlines)", async () => {
    const selectLabel = vi.fn()
    render(
      <LabelsSection
        look={lookOf(layoutWith([{ labels }]), {
          labelSelection: { key: "key:0", id: "a", edit: 7 },
          selectLabel,
        })}
        control={key0}
      />,
    )
    // An edit request opens the section and focuses the label's text.
    const text = await screen.findByRole("textbox", { name: "Text" })
    expect(text).toHaveValue("{{ projects.current.key }}")
    expect(text).toHaveFocus()
    fireEvent.keyDown(screen.getByRole("group", { name: /Labels on Key 1/ }), { key: "Escape" })
    expect(selectLabel).toHaveBeenCalledWith("key:0", null, false)
  })
})

describe("anchored labels", () => {
  it("set their inset from the edge in units", async () => {
    const user = userEvent.setup()
    sessionStorage.setItem("deckpilot.editor.labelsOpen", "1")
    render(<LabelsSection look={lookOf(layoutWith([{ labels }]))} control={key0} />)
    // The row's own button (not its grip or menu).
    const row = screen.getAllByRole("listitem")[0] as HTMLElement
    // Label b shows "-" (its face is unresolved).
    await user.click(within(row).getByText("-"))
    const inset = screen.getByRole("spinbutton", { name: "Inset from the edge" })
    expect(inset).toHaveValue(2)
    await user.clear(inset)
    await user.type(inset, "6{Enter}")
    expect(applied({ labels })?.labels?.[1]).toMatchObject({
      position: { mode: "anchored", to: "topRight", inset: 6 },
    })
  })
})

describe("labels from older and newer versions", () => {
  it("keep fields this version does not know when moved, and leave no empty list", () => {
    const newer = [
      { id: "a", text: "A", futureField: { kept: true } } as unknown as KeyLabel,
      { id: "b", text: "B" },
    ]
    const moved = applyLook(
      { labels: newer },
      withLabels(updateLabel(newer, "a", placedAt({ mode: "absolute", x: 100, y: 900 }))),
    )
    expect(moved.labels?.[0]).toEqual({
      id: "a",
      text: "A",
      futureField: { kept: true },
      position: { mode: "absolute", x: 100, y: 900 },
    })
    expect(moved.labels?.[1]).toBe(newer[1])
    // A key without labels is stored as before: no `labels` field at all.
    expect("labels" in applyLook({ background: "#000000", labels: newer }, withLabels([]))).toBe(
      false,
    )
  })
})

const applyLook = (appearance: KeyAppearance, change: (a: KeyAppearance) => KeyAppearance) =>
  change(appearance)
