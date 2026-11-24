// Where moved labels are, until the renderer draws them (label-outlines.tsx):
// a drag or a nudge commits a position, and the renderer's next boxes take
// a moment. Meanwhile a moved label's box is where the move put it
// (`expectedBox`), so the next drag or nudge starts from there.

import { useEffect, useMemo, useState } from "react"

import type { LabelBox, LabelPosition } from "@/lib/key-labels"

import { expectedBox } from "./labels"

/** How long a moved outline waits for the renderer's boxes before it goes back. */
const SETTLE_MS = 1500

/** Where moved labels are expected, for the renderer's `boxes` they were moved from. */
interface Expected {
  for: LabelBox[]
  boxes: ReadonlyMap<string, LabelBox>
  /** New for each move: restarts the wait. */
  at: number
}

export interface ExpectedBoxes {
  /** The renderer's boxes, with the moved labels where they are expected. */
  boxes: LabelBox[]
  /** Label `id` was moved to `position`: its box is expected there until new boxes arrive. */
  moved: (id: string, position: LabelPosition) => void
}

let moves = 0

/**
 * The boxes to show and to move labels from: the renderer's `boxes`, with
 * labels moved since then where they are expected. New boxes from the
 * renderer replace them; without an answer they go back after a moment.
 */
export function useExpectedBoxes(boxes: LabelBox[]): ExpectedBoxes {
  const [held, setExpected] = useState<Expected | null>(null)
  const expected = held?.for === boxes ? held : null
  const at = expected?.at
  useEffect(() => {
    if (at === undefined) return
    const timer = setTimeout(() => {
      setExpected(null)
    }, SETTLE_MS)
    return () => {
      clearTimeout(timer)
    }
  }, [at])
  const shown = useMemo(
    () => (expected ? boxes.map((b) => expected.boxes.get(b.id) ?? b) : boxes),
    [boxes, expected],
  )
  const moved = (id: string, position: LabelPosition) => {
    setExpected((previous) => {
      const current = previous?.for === boxes ? previous.boxes : new Map<string, LabelBox>()
      const box = current.get(id) ?? boxes.find((b) => b.id === id)
      if (!box) return previous
      return {
        for: boxes,
        boxes: new Map(current).set(id, expectedBox(box, position)),
        at: ++moves,
      }
    })
  }
  return { boxes: shown, moved }
}
