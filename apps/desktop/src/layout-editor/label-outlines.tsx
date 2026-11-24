// The outlines of a key's labels over the renderer's image of the key
// (docs/key-text.md, "Labels"; lib/key-labels.ts): where the renderer put
// each label (`ControlFace.labelBoxes`, canvas units), drawn as percent of
// the image, never laid out in CSS. Used by the Inspector's label stage and
// by the selected key on the device canvas.
//
// - a click selects a label, a double click edits it (its text field);
// - a drag moves it (a free position, snapped within 4 units, one undo
//   step on release); until the renderer's next boxes arrive, the outline
//   stays where it was dropped instead of jumping back;
// - arrows nudge the selected label by 1 unit, Shift+arrows by 5
//   (`onKeyDown`, for the element that holds the focus).

import { type PointerEvent, useEffect, useRef, useState } from "react"

import type { LabelBox, LabelPosition } from "@/lib/key-labels"
import { cn } from "@/lib/utils"

import { movedBy } from "./labels"

/** How long a dropped outline waits for the renderer's boxes before it goes back. */
const SETTLE_MS = 1500

interface Drag {
  id: string
  x: number
  y: number
  dx: number
  dy: number
  /** Released: waiting for boxes other than these (the renderer's next ones). */
  settled?: LabelBox[]
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
}: LabelOutlinesProps) {
  const layer = useRef<HTMLDivElement>(null)
  const [held, setDrag] = useState<Drag | null>(null)
  // New boxes from the renderer replace a dropped outline's own position.
  const drag = held?.settled && held.settled !== boxes ? null : held
  const settled = drag?.settled !== undefined
  useEffect(() => {
    if (!settled) return
    const timer = setTimeout(() => {
      setDrag(null)
    }, SETTLE_MS)
    return () => {
      clearTimeout(timer)
    }
  }, [settled])

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
    if (!drag || drag.settled) return
    event.stopPropagation()
    setDrag({ ...drag, dx: units(event.clientX - drag.x), dy: units(event.clientY - drag.y) })
  }
  const onPointerUp = (event: PointerEvent<HTMLDivElement>) => {
    if (!drag || drag.settled) return
    event.stopPropagation()
    const box = boxes.find((b) => b.id === drag.id)
    if (box && (Math.abs(drag.dx) >= 0.5 || Math.abs(drag.dy) >= 0.5)) {
      onMove(drag.id, movedBy(box, drag.dx, drag.dy, true))
      setDrag({ ...drag, settled: boxes })
    } else {
      setDrag(null)
    }
  }
  const percent = (value: number, of: number) => `${(value / of) * 100}%`
  return (
    <div
      ref={layer}
      data-slot="key-label-outlines"
      className="pointer-events-none absolute inset-0"
    >
      {boxes.map((box) => {
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
