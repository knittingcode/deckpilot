// The outlines of a key's labels over the renderer's image of the key
// (docs/key-text.md, "Labels"; lib/key-labels.ts): where the renderer put
// each label (`ControlFace.labelBoxes`, canvas units), drawn as percent of
// the image, never laid out in CSS. Used by the Inspector's label stage and
// by the selected key on the device canvas.
//
// - a click selects a label, a double click edits it (its text field);
// - a drag moves it (a free position, snapped within 4 units, one undo
//   step on release);
// - arrows nudge the selected label by 1 unit, Shift+arrows by 5 (`nudge`,
//   for the element that holds the focus).
//
// Until the renderer's next boxes arrive, a moved label's outline is where
// the move put it (`useExpectedBoxes`), and the next drag or nudge starts
// from there, so quick moves add up instead of starting from the old box.

import { type PointerEvent, useRef, useState } from "react"

import type { LabelBox, LabelPosition } from "@/lib/key-labels"
import { cn } from "@/lib/utils"

import { movedBy } from "./labels"
import { type ExpectedBoxes, useExpectedBoxes } from "./use-expected-boxes"

interface Drag {
  id: string
  x: number
  y: number
  dx: number
  dy: number
}

export interface LabelOutlinesProps {
  boxes: LabelBox[]
  selected: string | null
  onSelect: (id: string) => void
  /** A label was double-clicked: edit it. */
  onEdit?: (id: string) => void
  onMove: (id: string, position: LabelPosition) => void
  /** Outlines are drawn smaller on the device canvas. */
  compact?: boolean
  /** A name for each outline, e.g. "Label SHOP"; outlines without one are hidden from assistive technology. */
  describe?: (box: LabelBox) => string | undefined
  /**
   * Where moved labels are expected, when the parent also moves them (the
   * stage's nudges); without it the outlines keep their own.
   */
  expected?: ExpectedBoxes
}

/**
 * The outlines, filling their positioned parent (which shows the key's
 * image edge to edge). The layer itself lets clicks through; only the
 * outlines take them.
 */
export function LabelOutlines({
  boxes,
  selected,
  onSelect,
  onEdit,
  onMove,
  compact = false,
  describe,
  expected,
}: LabelOutlinesProps) {
  const layer = useRef<HTMLDivElement>(null)
  const [drag, setDrag] = useState<Drag | null>(null)
  const own = useExpectedBoxes(boxes)
  const placed = expected ?? own
  const shownBoxes = placed.boxes

  const canvas = boxes[0]?.canvas ?? { width: 72, height: 72 }
  const units = (pixels: number) => {
    const width = layer.current?.getBoundingClientRect().width ?? 0
    return width > 0 ? (pixels / width) * canvas.width : 0
  }
  const onPointerDown = (box: LabelBox) => (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return
    // Neither the key under it (a drag of the key, its selection) nor the
    // stage's background (deselection) sees this pointer.
    event.stopPropagation()
    event.preventDefault()
    onSelect(box.id)
    event.currentTarget.setPointerCapture(event.pointerId)
    setDrag({ id: box.id, x: event.clientX, y: event.clientY, dx: 0, dy: 0 })
  }
  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    if (!drag) return
    event.stopPropagation()
    setDrag({ ...drag, dx: units(event.clientX - drag.x), dy: units(event.clientY - drag.y) })
  }
  const onPointerUp = (event: PointerEvent<HTMLDivElement>) => {
    if (!drag) return
    event.stopPropagation()
    // From where the label is now: a move the renderer has not drawn yet counts.
    const box = shownBoxes.find((b) => b.id === drag.id)
    if (box && (Math.abs(drag.dx) >= 0.5 || Math.abs(drag.dy) >= 0.5)) {
      const position = movedBy(box, drag.dx, drag.dy, true)
      onMove(drag.id, position)
      placed.moved(drag.id, position)
    }
    setDrag(null)
  }
  const percent = (value: number, of: number) => `${(value / of) * 100}%`
  return (
    <div
      ref={layer}
      data-slot="key-label-outlines"
      className="pointer-events-none absolute inset-0"
    >
      {shownBoxes.map((box) => {
        const moving = drag?.id === box.id ? drag : null
        const name = describe?.(box)
        return (
          <div
            key={box.id}
            data-slot="key-label-box"
            data-label={box.id}
            data-selected={selected === box.id ? "" : undefined}
            aria-hidden={name === undefined ? true : undefined}
            aria-label={name}
            role={name === undefined ? undefined : "img"}
            className={cn(
              "pointer-events-auto absolute cursor-move touch-none rounded-sm outline-white/60 outline-dashed",
              compact && selected !== box.id && "outline-white/40",
              selected === box.id && "outline-2 outline-sky-400 outline-solid",
            )}
            style={{
              left: percent(box.x + (moving?.dx ?? 0), box.canvas.width),
              top: percent(box.y + (moving?.dy ?? 0), box.canvas.height),
              width: percent(box.width, box.canvas.width),
              height: percent(box.height, box.canvas.height),
            }}
            onPointerDown={onPointerDown(box)}
            onPointerMove={onPointerMove}
            onPointerUp={onPointerUp}
            onPointerCancel={() => {
              setDrag(null)
            }}
            onClick={(event) => {
              event.stopPropagation()
            }}
            onDoubleClick={(event) => {
              // Not the key's double click (entering a folder).
              event.stopPropagation()
              onSelect(box.id)
              onEdit?.(box.id)
            }}
          />
        )
      })}
    </div>
  )
}
