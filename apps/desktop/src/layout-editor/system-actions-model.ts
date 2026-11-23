// Pure logic of the inspector's System action panels
// (system-action-inspector.tsx): the long press of an Open Application key
// as an ordinary LongPress binding of Close Application, set with the
// gesture editor's own functions (no second gesture model).

import {
  convertToGestures,
  gestureBinding,
  needsConversion,
  setGesture,
} from "@/components/gesture-editor/model"
import { CLOSE_APPLICATION } from "@/lib/system-actions"
import type {
  ActionInstance,
  ControlBinding,
  ControlId,
  DeviceLayout,
  ParameterValue,
} from "@/lib/types"

/** What the key's long press does, for Open Application. */
export type LongPress = "none" | "quit" | "other"

/** The long press of `control`: nothing, quitting `application`, or something else. */
export function longPressOf(
  draft: DeviceLayout,
  pageId: string,
  control: ControlId,
  actions: ActionInstance[],
  application: ParameterValue | undefined,
): LongPress {
  const binding = gestureBinding(draft, pageId, control, "longPress")
  if (!binding) return "none"
  if (binding.target.type !== "action") return "other"
  const id = binding.target.action
  const action = actions.find((a) => a.id === id)
  const quits =
    action?.actionType === CLOSE_APPLICATION &&
    JSON.stringify(action.parameters.application) === JSON.stringify(application)
  return quits ? "quit" : "other"
}

/**
 * Binds the long press of `control` to `quit` (a Close Application action
 * of the same application), or removes it. A raw key is turned into a
 * gesture key first, exactly as the gesture editor does; the bindings that
 * cannot stay (a release) are returned.
 */
export function withLongPress(
  draft: DeviceLayout,
  pageId: string,
  control: ControlId,
  quit: ActionInstance | null,
): { layout: DeviceLayout; removed: ControlBinding[] } {
  if (!quit) return { layout: setGesture(draft, pageId, control, "longPress", null), removed: [] }
  const target = { type: "action" as const, action: quit.id }
  let layout = draft
  let removed: ControlBinding[] = []
  if (needsConversion(draft, pageId, control, "longPress", target)) {
    const converted = convertToGestures(draft, pageId, control)
    layout = converted.layout
    removed = converted.removed
  }
  return { layout: setGesture(layout, pageId, control, "longPress", target), removed }
}
