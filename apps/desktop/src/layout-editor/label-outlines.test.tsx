// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { act, cleanup, fireEvent, render } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

import type { LabelBox } from "@/lib/key-labels"

import { LabelOutlines } from "./label-outlines"
import { expectedBox, movedBy } from "./labels"

Element.prototype.setPointerCapture = () => undefined

const box = (id: string, canvas = { width: 72, height: 72 }): LabelBox => ({
  id,
  x: 20,
  y: 50,
  width: 32,
  height: 12,
  canvas,
  layer: "overIcon",
  truncated: false,
  overlapsTitle: false,
  plate: true,
  fontPoints: 10,
  lines: 1,
})

/** The outlines in a layer `pixels` wide (jsdom lays nothing out). */
function renderAt(
  pixels: number,
  boxes: LabelBox[],
  props: Partial<Parameters<typeof LabelOutlines>[0]> = {},
) {
  const onMove = vi.fn()
  const onSelect = vi.fn()
  const onEdit = vi.fn()
  const result = render(
    <div style={{ position: "relative" }}>
      <LabelOutlines
        boxes={boxes}
        selected={null}
        onSelect={onSelect}
        onEdit={onEdit}
        onMove={onMove}
        {...props}
      />
    </div>,
  )
  const layer = result.container.querySelector<HTMLElement>("[data-slot=key-label-outlines]")
  if (!layer) throw new Error("no layer")
  layer.getBoundingClientRect = () =>
    ({ width: pixels, height: pixels, left: 0, top: 0, right: pixels, bottom: pixels }) as DOMRect
  const outline = (id: string) => {
    const element = result.container.querySelector<HTMLElement>(`[data-label="${id}"]`)
    if (!element) throw new Error(`no outline ${id}`)
    return element
  }
  return { ...result, onMove, onSelect, onEdit, outline }
}

function drag(element: HTMLElement, dx: number, dy: number) {
  fireEvent.pointerDown(element, { button: 0, pointerId: 1, clientX: 10, clientY: 10 })
  fireEvent.pointerMove(element, { pointerId: 1, clientX: 10 + dx, clientY: 10 + dy })
  fireEvent.pointerUp(element, { pointerId: 1, clientX: 10 + dx, clientY: 10 + dy })
}

afterEach(cleanup)

describe("label outlines", () => {
  it("are placed in percent of the canvas from the renderer's boxes, never laid out in CSS", () => {
    const { outline } = renderAt(144, [box("a")])
    expect(parseFloat(outline("a").style.left)).toBeCloseTo((20 / 72) * 100, 3)
    expect(parseFloat(outline("a").style.top)).toBeCloseTo((50 / 72) * 100, 3)
    expect(parseFloat(outline("a").style.width)).toBeCloseTo((32 / 72) * 100, 3)
    // A touch zone's canvas is 144 × 72 units.
    cleanup()
    const zone = renderAt(200, [box("z", { width: 144, height: 72 })])
    expect(parseFloat(zone.outline("z").style.left)).toBeCloseTo((20 / 144) * 100, 3)
    expect(parseFloat(zone.outline("z").style.top)).toBeCloseTo((50 / 72) * 100, 3)
  })

  it("move a label by the same units whatever size the key is shown at", () => {
    // 36 px on a 144 px key and 18 px on a 72 px one are both 18 units.
    const big = renderAt(144, [box("a")])
    drag(big.outline("a"), 36, 0)
    cleanup()
    const small = renderAt(72, [box("a")])
    drag(small.outline("a"), 18, 0)
    expect(big.onMove).toHaveBeenCalledTimes(1)
    expect(small.onMove.mock.calls[0]).toEqual(big.onMove.mock.calls[0])
    expect(big.onMove.mock.calls[0]?.[1]).toEqual(movedBy(box("a"), 18, 0, true))
  })

  it("select on press, ask to edit on a double click, and ignore a click without a move", () => {
    const { outline, onSelect, onEdit, onMove } = renderAt(144, [box("a")])
    drag(outline("a"), 0, 0)
    expect(onSelect).toHaveBeenCalledWith("a")
    expect(onMove).not.toHaveBeenCalled()
    fireEvent.doubleClick(outline("a"))
    expect(onEdit).toHaveBeenCalledWith("a")
  })

  it("keep a dropped outline where it was dropped until the renderer's new boxes arrive", () => {
    vi.useFakeTimers()
    try {
      const boxes = [box("a")]
      const view = renderAt(144, boxes)
      drag(view.outline("a"), 36, 0)
      // 18 units further right (snapped to the text area's edge) while the
      // preview is drawn again: where the renderer is expected to draw it.
      const dropped = expectedBox(box("a"), movedBy(box("a"), 18, 0, true))
      expect(parseFloat(view.outline("a").style.left)).toBeCloseTo((dropped.x / 72) * 100, 3)
      // The renderer answers: its box is used.
      const moved = { ...box("a"), x: 38 }
      view.rerender(
        <div style={{ position: "relative" }}>
          <LabelOutlines boxes={[moved]} selected="a" onSelect={vi.fn()} onMove={vi.fn()} />
        </div>,
      )
      expect(parseFloat(view.outline("a").style.left)).toBeCloseTo((38 / 72) * 100, 3)
      // Without an answer, it goes back after a moment.
      drag(view.outline("a"), 36, 0)
      act(() => {
        vi.advanceTimersByTime(2000)
      })
      expect(parseFloat(view.outline("a").style.left)).toBeCloseTo((38 / 72) * 100, 3)
    } finally {
      vi.useRealTimers()
    }
  })

  it("start a quick second drag from where the first put the label", () => {
    const view = renderAt(144, [box("a")])
    // 10 px on 144 px: 5 units across each time, no guide within reach
    // (down, the text area's bottom edge is: centre 60).
    drag(view.outline("a"), 10, 0)
    drag(view.outline("a"), 10, 0)
    expect(view.onMove.mock.calls.map((call) => call[1] as unknown)).toEqual([
      { mode: "absolute", x: 569, y: 833 },
      { mode: "absolute", x: 638, y: 833 },
    ])
  })

  it("drag a label on a 144 × 72 canvas (a Stream Deck+ strip) in its own units", () => {
    const wide = box("z", { width: 144, height: 72 })
    // 200 px across: 50 px is 36 units across, -10 px is 7.2 units up.
    const view = renderAt(200, [wide])
    drag(view.outline("z"), 50, -10)
    // Centre (72, 48.8): on the strip's centre line, in per mille of 144 and 72.
    expect(view.onMove).toHaveBeenCalledWith("z", { mode: "absolute", x: 500, y: 678 })
    expect(view.onMove.mock.calls[0]?.[1]).toEqual(movedBy(wide, 36, -7.2, true))
    // Until the renderer answers, the outline is there.
    expect(parseFloat(view.outline("z").style.left)).toBeCloseTo(((72 - 16) / 144) * 100, 3)
  })

  it("stop the pointer from reaching the key under them", () => {
    const onKeyDown = vi.fn()
    const onDoubleClick = vi.fn()
    const { container } = render(
      <div onPointerDown={onKeyDown} onDoubleClick={onDoubleClick}>
        <LabelOutlines boxes={[box("a")]} selected={null} onSelect={vi.fn()} onMove={vi.fn()} />
      </div>,
    )
    const element = container.querySelector<HTMLElement>("[data-label=a]")
    if (!element) throw new Error("no outline")
    fireEvent.pointerDown(element, { button: 0, pointerId: 1 })
    fireEvent.doubleClick(element)
    expect(onKeyDown).not.toHaveBeenCalled()
    expect(onDoubleClick).not.toHaveBeenCalled()
  })
})
