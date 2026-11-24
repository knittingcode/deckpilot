// Key labels in the Device Editor: the pure model behind the Inspector's
// "Labels" section (lib/key-labels.ts is the contract; docs/key-text.md,
// "Labels"). Labels live in the look of the binding the key shows; the array
// is the paint order (the last one is drawn in front), and the section lists
// it front first. The renderer alone lays labels out: positions here are only
// computed from the boxes it reports (`LabelBox`), never measured in CSS.

import type { KeyboardEvent } from "react"

import {
  type KeyLabel,
  type KeyLabelLook,
  LABEL_DEFAULTS,
  LABEL_LIMITS,
  type LabelBox,
  type LabelPosition,
  type LabelPresetSpot,
  type LabelSpot,
  newLabelId,
} from "@/lib/key-labels"

import { sameTrigger } from "@/lib/layout-model"
import type { ControlBinding, ControlId, DeviceLayout, KeyAppearance } from "@/lib/types"

import { contrastRatio, shownBinding } from "./appearance"

/** The text a new label starts with (chosen default; the owner may change it). */
export const NEW_LABEL_TEXT = "{{ projects.current.name }}"

/** Below this WCAG contrast ratio a label is reported as hard to read. */
export const LABEL_MIN_CONTRAST = 3

/** Within this many units a dragged label snaps to a guide. */
export const SNAP_UNITS = 4

/** The presets of "Add label": the seven preset spots and the ribbon. */
export type LabelPreset = LabelPresetSpot | "ribbon"

export const LABEL_PRESETS: readonly LabelPreset[] = [
  "top",
  "center",
  "bottom",
  "topLeft",
  "topRight",
  "bottomLeft",
  "bottomRight",
  "ribbon",
]

/** The look a preset gives a new label. */
export function presetLook(preset: LabelPreset): KeyLabelLook {
  if (preset === "ribbon") {
    // A band across the bottom of the key's icon area, 1 unit in, on the
    // automatic plate.
    return {
      position: { mode: "anchored", to: "bottom", inset: 1 },
      width: 100,
      background: "auto",
    }
  }
  return { position: { mode: "preset", at: preset } }
}

/** The labels of a look, in paint order. */
export function labelsOf(appearance: KeyAppearance | null | undefined): KeyLabel[] {
  return appearance?.labels ?? []
}

/** A look change that sets the labels (none: the field is dropped). */
export function withLabels(labels: KeyLabel[]) {
  return (appearance: KeyAppearance): KeyAppearance => {
    const next: KeyAppearance = { ...appearance }
    if (labels.length > 0) next.labels = labels
    else delete next.labels
    return next
  }
}

/** Whether another label may be added. */
export function canAddLabel(labels: readonly KeyLabel[]): boolean {
  return labels.length < LABEL_LIMITS.labels
}

/** Adds a label from `preset` in front of the others; `null` when the key has the most it may. */
export function addLabel(
  labels: readonly KeyLabel[],
  preset: LabelPreset,
  text: string = NEW_LABEL_TEXT,
): { labels: KeyLabel[]; id: string } | null {
  if (!canAddLabel(labels)) return null
  const id = newLabelId(labels.map((l) => l.id))
  return { labels: [...labels, { id, text, ...presetLook(preset) }], id }
}

/** A copy of label `id` with a new id, right in front of it. */
export function duplicateLabel(
  labels: readonly KeyLabel[],
  id: string,
): { labels: KeyLabel[]; id: string } | null {
  const index = labels.findIndex((l) => l.id === id)
  const label = labels[index]
  if (!label || !canAddLabel(labels)) return null
  const copy = { ...structuredClone(label), id: newLabelId(labels.map((l) => l.id)) }
  return { labels: [...labels.slice(0, index + 1), copy, ...labels.slice(index + 1)], id: copy.id }
}

export function removeLabel(labels: readonly KeyLabel[], id: string): KeyLabel[] {
  return labels.filter((l) => l.id !== id)
}

/** Changes label `id`: a value sets a field, `null` resets it to its default. */
export function updateLabel(
  labels: readonly KeyLabel[],
  id: string,
  patch: { [K in keyof KeyLabel]?: KeyLabel[K] | null },
): KeyLabel[] {
  return labels.map((label) => {
    if (label.id !== id) return label
    const merged: Record<string, unknown> = { ...label, ...patch }
    // `width`/`maxWidth` and `height`/`maxHeight` exclude each other.
    if (patch.width != null) merged.maxWidth = null
    if (patch.maxWidth != null) merged.width = null
    if (patch.height != null) merged.maxHeight = null
    if (patch.maxHeight != null) merged.height = null
    // `null` resets a field to its default: it is left out.
    return Object.fromEntries(
      Object.entries(merged).filter(([, value]) => value !== null && value !== undefined),
    ) as unknown as KeyLabel
  })
}

/**
 * The change that sets a label's size (`null`: the default), lowering its
 * smallest size with it: the backend refuses a smallest size above the size.
 */
export function resized(label: KeyLabel, size: number | null) {
  const points = size ?? LABEL_DEFAULTS.size
  return label.minSize != null && label.minSize > points ? { size, minSize: points } : { size }
}

/** Whether a label's text may be kept: the backend drops a label without text. */
export function hasText(text: string): boolean {
  return text.trim() !== ""
}

/** Moves label `id` one step towards the front (drawn later) or the back. */
export function moveLabel(
  labels: readonly KeyLabel[],
  id: string,
  towards: "front" | "back",
): KeyLabel[] {
  const index = labels.findIndex((l) => l.id === id)
  const to = towards === "front" ? index + 1 : index - 1
  if (index < 0 || to < 0 || to >= labels.length) return [...labels]
  return reorderLabels(labels, index, to)
}

/** Moves the label at `from` to `to` (array indices, paint order). */
export function reorderLabels(labels: readonly KeyLabel[], from: number, to: number): KeyLabel[] {
  const next = [...labels]
  const [moved] = next.splice(from, 1)
  if (moved) next.splice(to, 0, moved)
  return next
}

/** Labels copied to another key: the same, each with a new id. */
export function copiedLabels(labels: readonly KeyLabel[]): KeyLabel[] {
  const taken: string[] = []
  return labels.map((label) => {
    const id = newLabelId(taken)
    taken.push(id)
    return { ...structuredClone(label), id }
  })
}

// ---------------------------------------------------------------------------
// Positions, from the renderer's boxes

/** Where a label is, in words, for its row: the preset spot, an edge, or free. */
export function positionKind(position: LabelPosition | undefined): {
  mode: LabelPosition["mode"]
  spot: LabelSpot | null
} {
  if (!position) return { mode: "preset", spot: "bottom" }
  if (position.mode === "preset") return { mode: "preset", spot: position.at }
  if (position.mode === "anchored") return { mode: "anchored", spot: position.to }
  return { mode: "absolute", spot: null }
}

const perMille = (value: number, of: number) =>
  Math.min(LABEL_LIMITS.perMille, Math.max(0, Math.round((value / of) * LABEL_LIMITS.perMille)))

/** The spot of the canvas nearest to `box`'s centre (by thirds). */
function nearestSpot(box: LabelBox): LabelSpot {
  const cx = (box.x + box.width / 2) / box.canvas.width
  const cy = (box.y + box.height / 2) / box.canvas.height
  const column = cx < 1 / 3 ? 0 : cx > 2 / 3 ? 2 : 1
  const row = cy < 1 / 3 ? 0 : cy > 2 / 3 ? 2 : 1
  const spots: LabelSpot[][] = [
    ["topLeft", "top", "topRight"],
    ["left", "center", "right"],
    ["bottomLeft", "bottom", "bottomRight"],
  ]
  return spots[row]?.[column] ?? "center"
}

/** How far `box` is from the edges of its `spot`, in units (the inset of an anchored label). */
function insetFor(box: LabelBox, spot: LabelSpot): number {
  const left = box.x
  const right = box.canvas.width - box.x - box.width
  const top = box.y
  const bottom = box.canvas.height - box.y - box.height
  const distances: number[] = []
  if (spot.startsWith("top")) distances.push(top)
  if (spot.startsWith("bottom")) distances.push(bottom)
  if (spot.endsWith("Left") || spot === "left") distances.push(left)
  if (spot.endsWith("Right") || spot === "right") distances.push(right)
  const inset = distances.length ? Math.min(...distances) : 0
  return Math.min(LABEL_LIMITS.inset, Math.max(0, Math.round(inset)))
}

/**
 * The position of mode `mode` that keeps a label where the renderer drew it
 * (`box`), with its offsets folded in (the caller clears them).
 */
export function positionFromBox(box: LabelBox, mode: LabelPosition["mode"]): LabelPosition {
  if (mode === "absolute") {
    return {
      mode: "absolute",
      x: perMille(box.x + box.width / 2, box.canvas.width),
      y: perMille(box.y + box.height / 2, box.canvas.height),
    }
  }
  const spot = nearestSpot(box)
  if (mode === "anchored") return { mode: "anchored", to: spot, inset: insetFor(box, spot) }
  // Presets have no middle of the left and right edges.
  const at: LabelPresetSpot = spot === "left" || spot === "right" ? "center" : spot
  return { mode: "preset", at }
}

/** The change that puts a label at `position`, clearing its offsets. */
export function placedAt(position: LabelPosition) {
  return { position, offsetX: null, offsetY: null }
}

const offset = (value: number) => {
  const rounded = Math.min(LABEL_LIMITS.offset, Math.max(-LABEL_LIMITS.offset, Math.round(value)))
  return rounded === 0 ? null : rounded
}

/**
 * The change to mode `mode` that keeps a label where the renderer drew it
 * (`box`). An edge label has one inset for its edges: what the box is
 * further from the other edge of a corner, or off the middle of an edge,
 * becomes its offsets, so it does not jump.
 */
export function placedFromBox(box: LabelBox, mode: LabelPosition["mode"]) {
  const position = positionFromBox(box, mode)
  if (position.mode !== "anchored") return placedAt(position)
  const inset = position.inset ?? 0
  const spot = position.to
  const left = box.x
  const right = box.canvas.width - box.x - box.width
  const top = box.y
  const bottom = box.canvas.height - box.y - box.height
  const across =
    spot.endsWith("Left") || spot === "left"
      ? left - inset
      : spot.endsWith("Right") || spot === "right"
        ? inset - right
        : box.x + box.width / 2 - box.canvas.width / 2
  const down = spot.startsWith("top")
    ? top - inset
    : spot.startsWith("bottom")
      ? inset - bottom
      : box.y + box.height / 2 - box.canvas.height / 2
  return { position, offsetX: offset(across), offsetY: offset(down) }
}

/**
 * The box the renderer is expected to draw a label in once it is at
 * `position`, for moves made before its next boxes arrive: an absolute
 * position puts the box's centre there; other positions keep `box`.
 */
export function expectedBox(box: LabelBox, position: LabelPosition): LabelBox {
  if (position.mode !== "absolute") return box
  return {
    ...box,
    x: (position.x / LABEL_LIMITS.perMille) * box.canvas.width - box.width / 2,
    y: (position.y / LABEL_LIMITS.perMille) * box.canvas.height - box.height / 2,
  }
}

/**
 * A label whose box centre is moved by `dx`, `dy` units, as an absolute
 * position (a nudge or a drag; free from then on), snapped to the centre
 * lines and the text area's edges when `snap`.
 */
export function movedBy(box: LabelBox, dx: number, dy: number, snap = false): LabelPosition {
  let cx = box.x + box.width / 2 + dx
  let cy = box.y + box.height / 2 + dy
  if (snap) {
    cx = snapped(cx, guides(box.canvas.width, box.width, 5))
    cy = snapped(cy, guides(box.canvas.height, box.height, 6))
  }
  return {
    mode: "absolute",
    x: perMille(cx, box.canvas.width),
    y: perMille(cy, box.canvas.height),
  }
}

/** Centres a box of `size` snaps to on an axis of `length`: the middle and the text area's edges. */
function guides(length: number, size: number, margin: number): number[] {
  return [length / 2, margin + size / 2, length - margin - size / 2]
}

function snapped(value: number, targets: number[]): number {
  let best = value
  let distance = SNAP_UNITS + 1
  for (const target of targets) {
    const d = Math.abs(target - value)
    if (d <= SNAP_UNITS && d < distance) {
      best = target
      distance = d
    }
  }
  return best
}

// ---------------------------------------------------------------------------
// Reports and moving the look

export type LabelReport = "truncated" | "overlapsTitle" | "lowContrast" | "small" | "unresolved"

/**
 * What the Inspector says about a label: from its box (cut, over the title,
 * small), its resolution, and the contrast of its colour on its own colour
 * background or, without a plate, the key's.
 */
export function labelReports(
  label: KeyLabel,
  box: LabelBox | undefined,
  options: { unresolved: boolean; textColor: string; keyBackground: string },
): LabelReport[] {
  const reports: LabelReport[] = []
  if (options.unresolved) reports.push("unresolved")
  if (box?.truncated) reports.push("truncated")
  if (box?.overlapsTitle) reports.push("overlapsTitle")
  if (box && box.fontPoints < 8) reports.push("small")
  // The automatic plate is chosen for contrast; a colour or none is checked.
  const background = label.background ?? "auto"
  if (background !== "auto") {
    const behind = background === "none" ? options.keyBackground : background
    const color = label.color ?? options.textColor
    if (/^#[0-9a-f]{6}$/i.test(behind) && /^#[0-9a-f]{6}$/i.test(color)) {
      if (contrastRatio(color, behind) < LABEL_MIN_CONTRAST) reports.push("lowContrast")
    }
  }
  return reports
}

/** How a look moves onto a binding that already has one of its own. */
export type LookMove = "keep" | "replace" | "merge"

/** The labels of `to` with `from`'s merged in by id (`from`'s win, in front). */
function mergedLabels(
  from: KeyAppearance | null | undefined,
  to: KeyAppearance | null | undefined,
): KeyLabel[] {
  const incoming = labelsOf(from)
  const ids = new Set(incoming.map((l) => l.id))
  return [...labelsOf(to).filter((l) => !ids.has(l.id)), ...incoming]
}

/** How many labels merging `from`'s into `to`'s gives. */
export function mergedLabelCount(
  from: KeyAppearance | null | undefined,
  to: KeyAppearance | null | undefined,
): number {
  return mergedLabels(from, to).length
}

/**
 * The look `to` gets when the key's shown binding changes from `from`:
 * nothing to move keeps `to`; `to` without a look takes `from`'s whole;
 * otherwise `move` decides: keep `to`'s, replace it with `from`'s, or merge
 * the labels by id (`from`'s win) and keep `to`'s title, icon and colours.
 * A merge never cuts labels: it is offered only within the limit
 * (`LookTransfer.mergeable`).
 */
export function movedLook(
  from: KeyAppearance | null | undefined,
  to: KeyAppearance | null | undefined,
  move: LookMove,
): KeyAppearance | null {
  if (!from) return to ?? null
  if (!to) return from
  if (move === "keep") return to
  if (move === "replace") return from
  return withLabels(mergedLabels(from, to))(to)
}

/** Whether moving `from`'s look onto `to` needs the user's choice. */
export function lookConflict(from: ControlBinding | undefined, to: ControlBinding | undefined) {
  return Boolean(from?.appearance && to?.appearance && labelsOf(from.appearance).length > 0)
}

/** What happens to the key's look when a binding was removed (`before` → `after`). */
export interface LookTransfer {
  /** The binding shown before, and the one shown now (another trigger). */
  from: ControlBinding
  to: ControlBinding
  /** Both have a look and `from`'s has labels: the user chooses (`LookMove`). */
  conflict: boolean
  /** How many labels move. */
  labels: number
  /** How many labels a merge gives the key. */
  merged: number
  /** Whether that stays within the most a key may have. */
  mergeable: boolean
}

/** The look transfer a removal causes, `null` when the shown binding stays or nothing moves. */
export function lookTransfer(
  before: DeviceLayout,
  after: DeviceLayout,
  pageId: string,
  control: ControlId,
): LookTransfer | null {
  const from = shownBinding(before, pageId, control)
  const to = shownBinding(after, pageId, control)
  if (!from || !to || sameTrigger(from.trigger, to.trigger) || !from.appearance) return null
  const merged = mergedLabelCount(from.appearance, to.appearance)
  return {
    from,
    to,
    conflict: lookConflict(from, to),
    labels: labelsOf(from.appearance).length,
    merged,
    mergeable: merged <= LABEL_LIMITS.labels,
  }
}

/**
 * After a binding was removed, keeps the key's title and look on the
 * binding now shown (labels included), in the same change: the look moves
 * as `move` says (`movedLook`), and the title the new binding lacks comes
 * with it.
 */
export function keepLookOnRemoval(
  before: DeviceLayout,
  after: DeviceLayout,
  pageId: string,
  control: ControlId,
  move: LookMove = "keep",
): DeviceLayout {
  const was = shownBinding(before, pageId, control)
  const now = shownBinding(after, pageId, control)
  if (!was || !now || sameTrigger(now.trigger, was.trigger)) return after
  const appearance = movedLook(was.appearance, now.appearance, move)
  const label = now.label ?? was.label
  if (appearance === (now.appearance ?? null) && label === now.label) return after
  const next: ControlBinding = { ...now, label: label ?? null }
  if (appearance) next.appearance = appearance
  else delete next.appearance
  return {
    ...after,
    pages: after.pages.map((p) =>
      p.id === pageId ? { ...p, bindings: p.bindings.map((b) => (b === now ? next : b)) } : p,
    ),
  }
}

/**
 * An automatic plate over the title or a mark: the contract's
 * `coverPlateOpacity` (85, #423); this fallback stands only until that
 * contract is merged.
 */
const COVER_PLATE_OPACITY =
  (LABEL_DEFAULTS as { coverPlateOpacity?: number }).coverPlateOpacity ?? 85

/**
 * The plate opacity a label gets when it sets none, as the renderer decides
 * it: a colour 100; the automatic plate 85 over the title (`box`), and
 * `"auto"` when the box does not say why it has a plate (over a mark it is
 * 85, over a picture or for contrast 60: the box does not report which);
 * `null` without a plate. Shown as the field's placeholder only: the editor
 * never writes it, so a key keeps the renderer's choice (a written 60 would
 * keep the plate at 60 over the title).
 */
export function defaultPlateOpacity(
  label: KeyLabel,
  box: LabelBox | undefined,
): number | "auto" | null {
  const background = label.background ?? "auto"
  if (background === "none") return null
  if (background !== "auto") return LABEL_DEFAULTS.colorPlateOpacity
  return box?.overlapsTitle ? COVER_PLATE_OPACITY : "auto"
}

/**
 * Nudges the selected label with the arrow keys; `true` when the key was
 * used. `boxes` are where the labels are now: the renderer's, or the
 * expected ones of moves it has not drawn yet (`expectedBox`), so quick
 * nudges add up.
 */
export function nudge(
  event: KeyboardEvent,
  boxes: LabelBox[],
  selected: string | null,
  onMove: (id: string, position: LabelPosition) => void,
): boolean {
  const box = boxes.find((b) => b.id === selected)
  if (!box) return false
  const step = event.shiftKey ? 5 : 1
  const moves: Record<string, [number, number]> = {
    ArrowLeft: [-step, 0],
    ArrowRight: [step, 0],
    ArrowUp: [0, -step],
    ArrowDown: [0, step],
  }
  const move = moves[event.key]
  if (!move) return false
  event.preventDefault()
  event.stopPropagation()
  onMove(box.id, movedBy(box, move[0], move[1]))
  return true
}
