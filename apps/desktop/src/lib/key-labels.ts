// Key labels: the contract between the editor, the app backend and the agent
// (docs/key-text.md, "Labels"; Rust: deckpilot_core::appearance::labels,
// deckpilot_core::FaceLabel, deckpilot_render::LabelBox; capability
// `keyLabels`, protocol 17).
//
// FROZEN for the editor work (WP-b). Every shape here is checked against what
// Rust serializes by `key-labels.contract.json` (written by the Rust test
// `label_contract`, read by `key-labels.contract.test.ts`); change both sides
// together.
//
// Rules the editor must keep:
//
// - Labels live in `KeyAppearance.labels` of the binding the key shows (press,
//   else click, else the first; `shownBinding`). Labels on another trigger's
//   look are dormant (not drawn; `DormantLook` warning). A gesture-editor
//   operation that changes the shown binding moves the whole look, labels
//   included, in the same undo step, explicitly.
// - The renderer is the only place labels are laid out. The editor shows the
//   renderer's image and draws selection outlines and handles from
//   `ControlFace.labelBoxes` (canvas units → percent of the image). It never
//   positions label text with CSS; the CSS `FaceTile` fallback draws no labels.
// - Text is a display template (as titles: `{{ reference }}`, `{profile}`,
//   `{project.key}`, `{project.name}`, `{page.…}`), resolved by the agent; the
//   picker lists `listDisplayVariables` only. A template naming a secret,
//   sensitive or run value is refused on save.
// - Numbers are integers: sizes, offsets, padding, radius and inset in units
//   (points: pixels on a 72 px key), widths and heights in percent of the
//   canvas, absolute positions in per mille. Unset fields are the defaults
//   below; the backend drops values equal to a default when it saves.

import type { MarkerSpan } from "./display-templates"

/** A point of a box (a corner, an edge centre or the centre). */
export type LabelSpot =
  | "topLeft"
  | "top"
  | "topRight"
  | "left"
  | "center"
  | "right"
  | "bottomLeft"
  | "bottom"
  | "bottomRight"

/** The spots a preset may use: top, center, bottom and the corners. */
export type LabelPresetSpot = Exclude<LabelSpot, "left" | "right">

export type LabelPosition =
  /** Inside the key's text area (5 units in from the sides, 6 from top and bottom). */
  | { mode: "preset"; at: LabelPresetSpot }
  /** `x`, `y`: per mille of the canvas; `origin`: the box's point placed there (default center). */
  | { mode: "absolute"; x: number; y: number; origin?: LabelSpot | null }
  /** Against an edge or corner of the key itself, `inset` units in (default 0). */
  | { mode: "anchored"; to: LabelSpot; inset?: number | null }

export type LabelWeight = "regular" | "bold"
/** `"auto"` (default), `"none"` or a colour `#rrggbb`. */
export type LabelBackground = "auto" | "none" | `#${string}`
export type LabelLayer = "behindIcon" | "overIcon" | "overTitle"
export type LabelOverflow = "ellipsis" | "clip" | "wrap" | "shrink"
export type LabelVisibility = "always" | "whenResolved" | "whenProject"
export type LabelAlign = "left" | "center" | "right"

/** How a label looks and where it goes (`KeyLabelLook`); every field optional. */
export interface KeyLabelLook {
  /** Default `{ mode: "preset", at: "bottom" }`. */
  position?: LabelPosition
  /** Units, -72..72. */
  offsetX?: number | null
  offsetY?: number | null
  /** Percent of the canvas, 5..100; `width` (fixed) and `maxWidth` exclude each other. */
  width?: number | null
  maxWidth?: number | null
  /** Percent of the canvas, 5..100; `height` and `maxHeight` exclude each other. */
  height?: number | null
  maxHeight?: number | null
  /** Points, 6..32; default 10. */
  size?: number | null
  weight?: LabelWeight | null
  /** Default by position: left in left corners and edges, right in right ones, else center. */
  align?: LabelAlign | null
  /** Default: the key's text colour (white or black where it does not read on the plate). */
  color?: string | null
  background?: LabelBackground | null
  /** Percent; default 60 (auto), 100 (a colour). */
  backgroundOpacity?: number | null
  /** Units 0..12; default 2. */
  padding?: number | null
  /** Units 0..12; default 2. */
  radius?: number | null
  layer?: LabelLayer | null
  overflow?: LabelOverflow | null
  /** Wrap: 1..4, default 2. */
  maxLines?: number | null
  /** Shrink: points, default 6, at most `size`. */
  minSize?: number | null
}

/** One label of a key's look (`KeyLabel`). */
export interface KeyLabel extends KeyLabelLook {
  /** `[a-z0-9-]{1,16}`, unique on the key; a new label gets 8 random hex digits. */
  id: string
  /** A display template, at most 160 characters. */
  text: string
  /** `false` keeps the label without drawing it. Default: drawn. */
  enabled?: boolean | null
  visible?: LabelVisibility | null
}

/** A label as the key shows it (`KeyFace.labels`): enabled and visible only. */
export interface FaceLabel {
  id: string
  /** Resolved (references and `{page.…}`), at most 120 grapheme clusters. */
  text: string
  spec: KeyLabelLook
  /** Some reference had no displayable value (shown as `-`). */
  unresolved?: boolean
  /**
   * Where each `-` marker is in `text` (after `{page.…}` and the cut), as for
   * titles: UTF-16 offsets, `end` exclusive. Omitted when none.
   */
  markers?: MarkerSpan[]
}

/**
 * Where the renderer put a label (`ControlFace.labelBoxes`), in canvas units:
 * the shorter side of the key is 72 (a 200×100 touch zone is 144×72), origin
 * top left, y down. The same at every pixel size. Draw outlines and handles as
 * `left = x / canvas.width * 100%`, and so on.
 */
export interface LabelBox {
  id: string
  /** The plate (the box with padding), after clamping into the canvas. */
  x: number
  y: number
  width: number
  height: number
  canvas: { width: number; height: number }
  layer: LabelLayer
  /** Text was cut (`…`, clipped, or lines dropped). */
  truncated: boolean
  /** The box meets the title's text block (info, not an error). */
  overlapsTitle: boolean
  /** A plate is drawn behind the text. */
  plate: boolean
  /** The size drawn, in points (smaller than `size` when shrunk). */
  fontPoints: number
  lines: number
}

/** Limits and defaults (`deckpilot_core::appearance::labels`). */
export const LABEL_LIMITS = {
  labels: 8,
  template: 160,
  clusters: 120,
  id: 16,
  lines: [1, 4],
  size: [6, 32],
  padding: [0, 12],
  radius: [0, 12],
  percent: [5, 100],
  offset: 72,
  inset: 36,
  perMille: 1000,
  canvas: 72,
} as const

export const LABEL_DEFAULTS = {
  position: { mode: "preset", at: "bottom" } as LabelPosition,
  size: 10,
  lines: 2,
  minSize: 6,
  padding: 2,
  radius: 2,
  plateOpacity: 60,
  colorPlateOpacity: 100,
  layer: "overIcon" as LabelLayer,
  overflow: "ellipsis" as LabelOverflow,
  visible: "always" as LabelVisibility,
} as const

/** Below this size on a 72 px key the editor reports "may be hard to read". */
export const LABEL_READABLE_POINTS = 8

const SPOTS: readonly string[] = [
  "topLeft",
  "top",
  "topRight",
  "left",
  "center",
  "right",
  "bottomLeft",
  "bottom",
  "bottomRight",
]
const LABEL_ID = /^[a-z0-9-]{1,16}$/
const COLOR = /^#[0-9a-fA-F]{6}$/

/** Characters as Rust counts them (`chars().count()`: code points). */
const codePoints = (text: string) => {
  let count = 0
  for (let i = 0; i < text.length; i++) {
    const unit = text.charCodeAt(i)
    // A high surrogate and its low surrogate are one code point.
    if (unit >= 0xd800 && unit <= 0xdbff && i + 1 < text.length) i++
    count++
  }
  return count
}

const isObject = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value)
const optional = (value: unknown, check: (value: unknown) => boolean) =>
  value === undefined || value === null || check(value)
const integer = (low: number, high: number) => (value: unknown) =>
  typeof value === "number" && Number.isInteger(value) && value >= low && value <= high
const oneOf =
  (...values: string[]) =>
  (value: unknown) =>
    typeof value === "string" && values.includes(value)

export function isLabelPosition(value: unknown): value is LabelPosition {
  if (!isObject(value)) return false
  switch (value.mode) {
    case "preset":
      return (
        typeof value.at === "string" &&
        SPOTS.includes(value.at) &&
        value.at !== "left" &&
        value.at !== "right"
      )
    case "absolute":
      return (
        integer(0, LABEL_LIMITS.perMille)(value.x) &&
        integer(0, LABEL_LIMITS.perMille)(value.y) &&
        optional(value.origin, (o) => typeof o === "string" && SPOTS.includes(o))
      )
    case "anchored":
      return (
        typeof value.to === "string" &&
        SPOTS.includes(value.to) &&
        optional(value.inset, integer(0, LABEL_LIMITS.inset))
      )
    default:
      return false
  }
}

export function isKeyLabelLook(value: unknown): value is KeyLabelLook {
  if (!isObject(value)) return false
  const v = value
  const percent = integer(...LABEL_LIMITS.percent)
  const spacing = integer(...LABEL_LIMITS.padding)
  return (
    (v.position === undefined || isLabelPosition(v.position)) &&
    optional(v.offsetX, integer(-LABEL_LIMITS.offset, LABEL_LIMITS.offset)) &&
    optional(v.offsetY, integer(-LABEL_LIMITS.offset, LABEL_LIMITS.offset)) &&
    optional(v.width, percent) &&
    optional(v.maxWidth, percent) &&
    optional(v.height, percent) &&
    optional(v.maxHeight, percent) &&
    optional(v.size, integer(...LABEL_LIMITS.size)) &&
    optional(v.weight, oneOf("regular", "bold")) &&
    optional(v.align, oneOf("left", "center", "right")) &&
    optional(v.color, (c) => typeof c === "string" && COLOR.test(c)) &&
    optional(
      v.background,
      (b) => typeof b === "string" && (b === "auto" || b === "none" || COLOR.test(b)),
    ) &&
    optional(v.backgroundOpacity, integer(0, 100)) &&
    optional(v.padding, spacing) &&
    optional(v.radius, spacing) &&
    optional(v.layer, oneOf("behindIcon", "overIcon", "overTitle")) &&
    optional(v.overflow, oneOf("ellipsis", "clip", "wrap", "shrink")) &&
    optional(v.maxLines, integer(...LABEL_LIMITS.lines)) &&
    optional(v.minSize, integer(...LABEL_LIMITS.size))
  )
}

export function isKeyLabel(value: unknown): value is KeyLabel {
  return (
    isObject(value) &&
    typeof value.id === "string" &&
    LABEL_ID.test(value.id) &&
    typeof value.text === "string" &&
    codePoints(value.text) <= LABEL_LIMITS.template &&
    optional(value.enabled, (e) => typeof e === "boolean") &&
    optional(value.visible, oneOf("always", "whenResolved", "whenProject")) &&
    isKeyLabelLook(value)
  )
}

export function isFaceLabel(value: unknown): value is FaceLabel {
  return (
    isObject(value) &&
    typeof value.id === "string" &&
    LABEL_ID.test(value.id) &&
    typeof value.text === "string" &&
    isKeyLabelLook(value.spec) &&
    optional(value.unresolved, (u) => typeof u === "boolean") &&
    optional(
      value.markers,
      (markers) =>
        Array.isArray(markers) &&
        markers.every(
          (m) =>
            isObject(m) &&
            integer(0, Number.MAX_SAFE_INTEGER)(m.start) &&
            integer(0, Number.MAX_SAFE_INTEGER)(m.end) &&
            typeof m.reference === "string",
        ),
    )
  )
}

export function isLabelBox(value: unknown): value is LabelBox {
  const number = (n: unknown) => typeof n === "number" && Number.isFinite(n)
  return (
    isObject(value) &&
    typeof value.id === "string" &&
    [value.x, value.y, value.width, value.height, value.fontPoints].every(number) &&
    isObject(value.canvas) &&
    number(value.canvas.width) &&
    number(value.canvas.height) &&
    oneOf("behindIcon", "overIcon", "overTitle")(value.layer) &&
    typeof value.truncated === "boolean" &&
    typeof value.overlapsTitle === "boolean" &&
    typeof value.plate === "boolean" &&
    integer(0, LABEL_LIMITS.lines[1])(value.lines)
  )
}

/** A new label's id: 8 random hex digits, not one of `taken`. */
export function newLabelId(taken: readonly string[] = []): string {
  for (;;) {
    const bytes = new Uint8Array(4)
    crypto.getRandomValues(bytes)
    const id = [...bytes].map((b) => b.toString(16).padStart(2, "0")).join("")
    if (!taken.includes(id)) return id
  }
}
