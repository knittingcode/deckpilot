// Gesture presets in the layout editor (docs/gestures-and-work-session.md
// §3.4): one Action Library entry that puts several existing actions on the
// gestures of one control. Nothing here runs anything or adds a model of its
// own: a preset is resolved to ordinary actions (created or reused) and
// ordinary bindings, validated against the device's descriptor first, and
// applied as one change of the draft (one undo step) or not at all. The
// backend checks the saved layout as always.

import { i18n, type TranslationKey } from "@/i18n"
import {
  bindingIncompatibility,
  controlCapabilities,
  describeIncompatibility,
} from "@/lib/compatibility"
import { bindingsFor, controlLabel, sameControl, triggerLabel } from "@/lib/layout-model"
import { isPluginId, typeAvailability } from "@/lib/plugin-actions"
import type {
  ActionInstance,
  ActionType,
  BindingTarget,
  ControlBinding,
  ControlId,
  DeviceLayout,
  DeviceModel,
  GestureAssignment,
  GesturePreset,
  ModuleInfo,
  PresetGesture,
  Trigger,
} from "@/lib/types"
import type { ActionInput } from "@/lib/api"
import { GESTURE_LABELS, convertToGestures, keyMode } from "@/components/gesture-editor/model"

import { existingPreset, newPresetAction } from "./catalog"

/** The order gestures are listed in (`PresetGesture::ALL`). */
export const PRESET_GESTURES: PresetGesture[] = [
  "click",
  "doubleClick",
  "longPress",
  "release",
  "rotateClockwise",
  "rotateCounterClockwise",
]

/** The id of a preset's library entry among the editor's draggables. */
export function gesturePresetDragId(id: string): string {
  return `library:gesture-preset:${id}`
}

/** A preset with the module that declares it. */
export interface GesturePresetEntry {
  preset: GesturePreset
  module: ModuleInfo
}

/** Every module's presets, in module order; modules that cannot run here are left out. */
export function gesturePresetEntries(modules: ModuleInfo[]): GesturePresetEntry[] {
  return modules
    .filter((module) => module.supported)
    .flatMap((module) => (module.gesturePresets ?? []).map((preset) => ({ preset, module })))
}

// ---------------------------------------------------------------------------
// Texts

/**
 * The translation key of a built-in preset's text. Plugin presets arrive in
 * the app's language from the agent (the plugin's own locales), so only
 * built-in ones are looked up here, falling back to the module's English.
 */
function builtInKey(preset: GesturePreset, path: string): TranslationKey | null {
  const owner = preset.id.slice(0, Math.max(0, preset.id.lastIndexOf(".")))
  if (owner === "" || isPluginId(owner)) return null
  return `actions:gesturePresets.${preset.id.replaceAll(".", "_")}.${path}` as TranslationKey
}

function text(preset: GesturePreset, path: string, fallback: string): string {
  const key = builtInKey(preset, path)
  return key ? i18n.t(key, { defaultValue: fallback }) : fallback
}

/** The preset's name in the window's language. */
export function presetName(preset: GesturePreset): string {
  return text(preset, "name", preset.name)
}

/** What one gesture of the preset does, in the window's language. */
export function assignmentLabel(preset: GesturePreset, assignment: GestureAssignment): string {
  return text(preset, `gestures.${assignment.gesture}`, assignment.label)
}

/** A gesture's name in the window's language ("Double press"). */
export function presetGestureLabel(gesture: PresetGesture): string {
  switch (gesture) {
    case "click":
    case "doubleClick":
    case "longPress":
      return GESTURE_LABELS[gesture]
    case "release":
      return i18n.t("editor:gesture.release")
    case "rotateClockwise":
    case "rotateCounterClockwise":
      return triggerLabel({ type: gesture })
  }
}

/** The gestures of the preset in order, each with what it does: the entry's tooltip. */
export function presetSummary(preset: GesturePreset): string {
  return ordered(preset.gestures)
    .map((a) => `${presetGestureLabel(a.gesture)}: ${assignmentLabel(preset, a)}`)
    .join("\n")
}

function ordered(assignments: GestureAssignment[]): GestureAssignment[] {
  return [...assignments].sort(
    (a, b) => PRESET_GESTURES.indexOf(a.gesture) - PRESET_GESTURES.indexOf(b.gesture),
  )
}

/** "a, b and c" in the window's language. */
function list(items: string[]): string {
  if (items.length <= 1) return items.join("")
  return i18n.t("editor:gesturePreset.list", {
    items: items.slice(0, -1).join(", "),
    last: items.at(-1) ?? "",
  })
}

// ---------------------------------------------------------------------------
// Availability

/**
 * Why the preset cannot be placed now, in words; `null` when it can. Every
 * action it names must be known and runnable: a plugin's that is disabled,
 * missing or waits for a permission, or a built-in module that is turned
 * off, makes the whole preset unavailable. A preset grants nothing.
 */
export function presetUnavailable(
  modules: ModuleInfo[],
  preset: GesturePreset,
  typeOf: (id: string) => ActionType | undefined,
): string | null {
  for (const assignment of preset.gestures) {
    if (!typeOf(assignment.action)) {
      return i18n.t("editor:gesturePreset.unknownAction")
    }
    const availability = typeAvailability(modules, assignment.action)
    if (!availability.available) {
      return (
        availability.reason ??
        i18n.t("editor:gesturePreset.moduleOff", {
          module: availability.owner?.name ?? assignment.action,
        })
      )
    }
  }
  return null
}

// ---------------------------------------------------------------------------
// Planning

/** One gesture that goes on the control. */
export interface PlannedGesture {
  assignment: GestureAssignment
  trigger: Trigger
}

export type PresetPlan =
  | {
      ok: true
      items: PlannedGesture[]
      /** Gestures left out (only with `allowPartial`), each with why. */
      skipped: { gesture: PresetGesture; reason: string }[]
      /** The control's raw press first becomes a click (it gets a double or long press). */
      convert: boolean
    }
  | { ok: false; reason: string }

function needsGestureMode(gesture: PresetGesture): boolean {
  return gesture === "doubleClick" || gesture === "longPress"
}

/** The trigger a preset gesture binds, with the control in gesture mode or raw. */
export function presetTrigger(gesture: PresetGesture, gestures: boolean): Trigger {
  if (gesture === "click") return { type: gestures ? "click" : "press" }
  return { type: gesture }
}

/**
 * What applying `preset` to `control` does, decided from the device's
 * descriptor (never assuming a double press, long press, release, dial or
 * touch exists) and the actions' declared controls. A double or long press
 * puts the control in gesture mode, where a single press is a click and a
 * release cannot be. A gesture the control cannot take refuses the whole
 * preset, unless it allows a partial application; a release that is on
 * the control and could not stay next to the new gestures refuses it too.
 */
export function planPreset({
  preset,
  model,
  layout,
  pageId,
  control,
  typeOf,
}: {
  preset: GesturePreset
  model: DeviceModel
  layout: DeviceLayout
  pageId: string
  control: ControlId
  typeOf: (id: string) => ActionType | undefined
}): PresetPlan {
  const name = controlLabel(control)
  const capabilities = controlCapabilities(model, control)
  if (!capabilities) {
    return { ok: false, reason: i18n.t("editor:gesturePreset.noControl", { control: name }) }
  }
  const mode = keyMode(layout, pageId, control)
  const why = (assignment: GestureAssignment, gestures: boolean): string | null => {
    const type = typeOf(assignment.action)
    if (!type) return i18n.t("editor:gesturePreset.unknownAction")
    const incompatibility = bindingIncompatibility(
      type,
      capabilities,
      presetTrigger(assignment.gesture, gestures),
    )
    return incompatibility ? describeIncompatibility(incompatibility, model, capabilities) : null
  }
  const gestures =
    mode === "gesture" ||
    preset.gestures.some((a) => needsGestureMode(a.gesture) && why(a, true) === null)
  const items: PlannedGesture[] = []
  const skipped: { gesture: PresetGesture; reason: string }[] = []
  for (const assignment of ordered(preset.gestures)) {
    const reason =
      assignment.gesture === "release" && gestures
        ? i18n.t("editor:gesturePreset.releaseWithGestures")
        : why(assignment, gestures)
    if (reason) skipped.push({ gesture: assignment.gesture, reason })
    else items.push({ assignment, trigger: presetTrigger(assignment.gesture, gestures) })
  }
  if (skipped.length > 0 && !preset.allowPartial) {
    const [first] = skipped
    return {
      ok: false,
      reason: i18n.t("editor:gesturePreset.unsupported", {
        control: name,
        gestures: list(skipped.map((s) => presetGestureLabel(s.gesture).toLowerCase())),
        reason: first?.reason ?? "",
      }),
    }
  }
  if (items.length === 0) {
    return { ok: false, reason: i18n.t("editor:gesturePreset.nothing", { control: name }) }
  }
  const convert = gestures && mode === "raw"
  if (convert && bindingsFor(layout, pageId, control).some((b) => b.trigger.type === "release")) {
    return { ok: false, reason: i18n.t("editor:gesturePreset.releaseInTheWay", { control: name }) }
  }
  return { ok: true, items, skipped, convert }
}

/**
 * The draft with the plan applied to `control`: each planned gesture's
 * binding gets its target (a gesture already bound keeps its label and
 * look; only its target changes), every other binding of the control stays
 * as it is. On an empty control the single press is titled with the
 * preset's name. `targets` are in the order of `plan.items`.
 */
export function applyPreset(
  layout: DeviceLayout,
  pageId: string,
  control: ControlId,
  plan: Extract<PresetPlan, { ok: true }>,
  targets: BindingTarget[],
  title: string | null,
): DeviceLayout {
  const empty = keyMode(layout, pageId, control) === "empty"
  const base = plan.convert ? convertToGestures(layout, pageId, control).layout : layout
  return {
    ...base,
    pages: base.pages.map((page) => {
      if (page.id !== pageId) return page
      const bindings = [...page.bindings]
      plan.items.forEach((item, index) => {
        const target = targets[index]
        if (!target) return
        const at = bindings.findIndex(
          (b) => sameControl(b.control, control) && b.trigger.type === item.trigger.type,
        )
        const current = at >= 0 ? bindings[at] : undefined
        if (current) {
          bindings[at] = { ...current, target }
          return
        }
        const shown = item.trigger.type === "click" || item.trigger.type === "press"
        const binding: ControlBinding = {
          control,
          trigger: item.trigger,
          target,
          label: empty && shown ? title : null,
        }
        bindings.push(binding)
      })
      return { ...page, bindings }
    }),
  }
}

/**
 * The action one gesture of the preset stands for: the profile's action that
 * is exactly it (its type, its settings, its name and no label of its own),
 * else what to create, named with the gesture's label in the window's
 * language.
 */
export function presetAction(
  actions: ActionInstance[],
  preset: GesturePreset,
  assignment: GestureAssignment,
  type: ActionType,
): { existing: ActionInstance } | { input: ActionInput } {
  const name = assignmentLabel(preset, assignment)
  const wanted = { name, parameters: assignment.parameters, icon: preset.icon ?? null }
  const existing = existingPreset(
    actions.filter((a) => a.name === name),
    type,
    wanted,
  )
  if (existing) return { existing }
  return { input: newPresetAction(actions, type, wanted) }
}

/** The preset's gestures, labelled, for a toast ("double press and long press"). */
export function gestureList(gestures: PresetGesture[]): string {
  return list(gestures.map((g) => presetGestureLabel(g).toLowerCase()))
}
