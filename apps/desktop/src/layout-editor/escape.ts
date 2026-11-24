// Escape in the Device Editor (docs/editor-navigation-and-simulation.md,
// "Escape"): one effect per press, in this order.
//
// 1. An open menu, popover, picker or dialog closes (Radix handles Escape
//    itself; the editor then does nothing).
// 2. A drag in progress is cancelled (dnd-kit's sensors handle it).
// 3. In a text field (the key's title, a folder or page name, a search, a
//    parameter) Escape leaves the field: the focus goes out of it, what was
//    typed stays (it is already in the draft, one undo step). A field that
//    handles Escape itself (renaming a page cancels the rename) keeps doing
//    so. The next Escape deselects.
// 4. Otherwise the selection is cleared: the Inspector shows its empty
//    state. Deselecting is not an edit: the draft, Undo/Redo, Save/Revert,
//    the page, the Canvas's zoom and pan and the live preview stay.

/** What an Escape press does in the editor. */
export type EscapeEffect = "deselect" | "leaveField" | null

/** Overlays (Radix menus, popovers, selects, dialogs, alert dialogs). */
const OVERLAY =
  '[data-radix-popper-content-wrapper], [role="dialog"], [role="alertdialog"], [role="menu"], [role="listbox"]'
/** Overlays that are open now, wherever the focus is. */
const OPEN_OVERLAY =
  '[data-radix-popper-content-wrapper], [role="dialog"][data-state="open"], [role="alertdialog"][data-state="open"]'

/** Elements where Escape leaves the field instead of deselecting. */
function isTextField(element: Element | null): element is HTMLElement {
  if (!(element instanceof HTMLElement)) return false
  if (element.isContentEditable) return true
  if (element instanceof HTMLTextAreaElement) return true
  if (element instanceof HTMLInputElement) {
    return !["checkbox", "radio", "range", "button", "submit", "reset", "color", "file"].includes(
      element.type,
    )
  }
  return ["combobox", "spinbutton", "searchbox", "textbox"].includes(
    element.getAttribute("role") ?? "",
  )
}

/**
 * What Escape does for `event`: nothing when something else already took it
 * (an overlay, a field that handles it, a drag), leave the field, or
 * deselect.
 */
export function escapeEffect(
  event: { key: string; defaultPrevented: boolean; target: EventTarget | null },
  dragging: boolean,
  doc: Document = document,
): EscapeEffect {
  if (event.key !== "Escape" || event.defaultPrevented || dragging) return null
  const target = event.target instanceof Element ? event.target : null
  // An overlay is open (or Escape comes from inside one): it closes, nothing else.
  if (target?.closest(OVERLAY) || doc.querySelector(OPEN_OVERLAY)) return null
  if (isTextField(target)) return "leaveField"
  return "deselect"
}
