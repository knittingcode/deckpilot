// The layout editor's drag and drop as it is wired, for tests (as
// `layout-editor/key-logic.test.tsx` does it inline): the drag data of the
// real draggables, the editor's drop handler and every listener the
// components registered for drops. A test mocks `@dnd-kit/core` with
// `dndModule` and drops with `drop`:
//
//   vi.mock("@dnd-kit/core", async (original) =>
//     (await import("@/test/dnd")).dndModule(await original()),
//   )

import { act } from "@testing-library/react"
import type { DragEndEvent } from "@dnd-kit/core"
import { useEffect, useRef, type ReactNode } from "react"
import { expect } from "vitest"

type DndCore = typeof import("@dnd-kit/core")

export const dnd = {
  onDragEnd: undefined as ((event: DragEndEvent) => void) | undefined,
  monitors: new Set<{ onDragEnd?: (event: DragEndEvent) => void }>(),
  data: new Map<string, unknown>(),
  droppables: new Set<string>(),
}

/** `@dnd-kit/core` with the hooks recording into `dnd`. */
export function dndModule(actual: DndCore): DndCore {
  const Context = actual.DndContext
  return {
    ...actual,
    DndContext: (props: Parameters<DndCore["DndContext"]>[0] & { children?: ReactNode }) => {
      dnd.onDragEnd = (event) => props.onDragEnd?.(event)
      return <Context {...props} />
    },
    useDraggable: (args: Parameters<DndCore["useDraggable"]>[0]) => {
      dnd.data.set(String(args.id), args.data)
      return actual.useDraggable(args)
    },
    useDroppable: (args: Parameters<DndCore["useDroppable"]>[0]) => {
      const id = String(args.id)
      if (args.disabled) dnd.droppables.delete(id)
      else dnd.droppables.add(id)
      // A drop target that is gone takes no drops.
      useEffect(
        () => () => {
          dnd.droppables.delete(id)
        },
        [id],
      )
      return actual.useDroppable(args)
    },
    useDndMonitor: (listener: Parameters<DndCore["useDndMonitor"]>[0]) => {
      // The newest listener of the component, as dnd-kit keeps it.
      const current = useRef(listener)
      current.current = listener
      useEffect(() => {
        const entry = { onDragEnd: (event: DragEndEvent) => current.current.onDragEnd?.(event) }
        dnd.monitors.add(entry)
        return () => {
          dnd.monitors.delete(entry)
        }
      }, [])
      actual.useDndMonitor(listener)
    },
  } as DndCore
}

/** Forgets what the previous test registered. */
export function resetDnd() {
  dnd.onDragEnd = undefined
  dnd.data.clear()
  dnd.droppables.clear()
  dnd.monitors.clear()
}

/** Drops the draggable `id` on the drop target `over`, as dnd-kit does. */
export function drop(id: string, over: string) {
  expect(dnd.data.has(id), `a draggable ${id}`).toBe(true)
  expect(dnd.droppables.has(over), `a drop target ${over}`).toBe(true)
  const event = {
    active: { id, data: { current: dnd.data.get(id) } },
    over: { id: over },
  } as unknown as DragEndEvent
  act(() => {
    dnd.onDragEnd?.(event)
    for (const monitor of dnd.monitors) monitor.onDragEnd?.(event)
  })
}
