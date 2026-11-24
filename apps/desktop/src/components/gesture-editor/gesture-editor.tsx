import { useDndMonitor } from "@dnd-kit/core"
import { type ReactNode, useState } from "react"
import { toast } from "sonner"

import { ConfirmDialog } from "@/components/dialogs"
import { LookMoveDialog } from "@/layout-editor/look-move-dialog"
import { type LookMove, keepLookOnRemoval, lookTransfer } from "@/layout-editor/labels"
import {
  bindingsFor,
  controlKey,
  controlLabel,
  removeBinding,
  setBinding,
  targetLabel,
  triggerLabel,
} from "@/lib/layout-model"
import { type TranslationKey, useTranslation } from "@/i18n"
import type {
  ActionType,
  BindingTarget,
  ControlBinding,
  ControlId,
  DeviceLayout,
  Gesture,
  GestureTimings,
  Profile,
  Trigger,
} from "@/lib/types"

import {
  GestureCard,
  type CardTiming,
  type GestureCardProps,
  type GestureTargetGroup,
} from "./gesture-card"
import { GestureCarousel } from "./gesture-carousel"
import {
  GESTURE_LABELS,
  GESTURES,
  clickTiming,
  convertToGestures,
  gestureBinding,
  gestureSlotId,
  keyMode,
  keyTimings,
  needsConversion,
  setGesture,
  setKeyTiming,
  releaseSlotId,
  slotGesture,
  triggerFor,
  type ClickTiming,
} from "./model"
import type { MigrationOffer } from "@/components/context-migrations"
import { contextOf, isDisplayOnly } from "@/lib/action-context"

export type { GestureTargetGroup } from "./gesture-card"

export interface GestureEditorProps {
  draft: DeviceLayout
  pageId: string
  control: ControlId
  profile: Pick<Profile, "actions" | "workflows" | "projects">
  /** Targets to offer, encoded with `encode`/`decode`. */
  groups: GestureTargetGroup[]
  /** Further targets only gestures can have (the Work Session controller, G4). */
  gestureGroups?: GestureTargetGroup[]
  encode: (target: BindingTarget | undefined) => string
  decode: (value: string) => BindingTarget | null
  /** Action types, for the parameters an override can replace. */
  types: ActionType[]
  /** This computer's default timings. */
  defaults: GestureTimings
  /** Messages of the last failed save about this control. */
  issues: string[]
  /** Commits a change; `group` merges consecutive edits into one undo step. */
  onChange: (layout: DeviceLayout, group?: string) => void
  /**
   * Makes each gesture a slot that takes what is dragged onto it (inside
   * the layout editor's drag and drop). It turns what was dropped into the
   * target to assign, creating an action for an action type if needed;
   * `null`: it cannot be put on a gesture.
   */
  resolveDrop?: (dragged: unknown) => Promise<BindingTarget | null> | BindingTarget | null
  /**
   * Opens the steps of a step workflow (a Multi Action) a gesture runs;
   * without it, no "Edit steps" is offered.
   */
  onEditWorkflow?: (workflow: string) => void
  /**
   * Why `target` cannot run from this control for `trigger`, in words;
   * `null` when it can. Such targets are offered disabled with the reason
   * (docs/device-controls.md).
   */
  incompatible?: (target: BindingTarget, trigger: Trigger) => string | null
  /** Bindings of the control kept although they cannot run, with the reason. */
  notRun?: { trigger: Trigger; reason: string; migrate?: MigrationOffer | undefined }[]
  /** The placeholder of a binding's label, e.g. the title it shows without one. */
  labelPlaceholder?: (binding: ControlBinding) => string | undefined
  /**
   * Whether the binding's label is the key's visible title, edited in the
   * Inspector's key header (not again in its card).
   */
  titleInHeader?: (binding: ControlBinding) => boolean
  /** The saved action `id` a gesture runs: shared with whom, and its definition. */
  actionSettings?: (id: string) => ReactNode
}

/** The reason a binding of `trigger` is kept but not run, if it is. */
function notRunReason(
  notRun: GestureEditorProps["notRun"],
  trigger: Trigger,
  binding: ControlBinding | undefined,
): string | undefined {
  if (!binding) return undefined
  return notRun?.find((n) => n.trigger.type === trigger.type)?.reason
}

/** What the kept binding of `trigger` can become, if anything. */
function notRunMigrate(
  notRun: GestureEditorProps["notRun"],
  trigger: Trigger,
  binding: ControlBinding | undefined,
): MigrationOffer | undefined {
  if (!binding) return undefined
  return notRun?.find((n) => n.trigger.type === trigger.type)?.migrate
}

/**
 * Listens for drops on the gesture slots of `control`. A component of its
 * own, because the listener only exists inside a drag-and-drop context.
 */
function SlotDrops({
  control,
  resolve,
  onDrop,
}: {
  control: ControlId
  resolve: NonNullable<GestureEditorProps["resolveDrop"]>
  onDrop: (gesture: Gesture | "release", target: BindingTarget) => void
}) {
  useDndMonitor({
    onDragEnd: (event) => {
      const gesture =
        event.over?.id === releaseSlotId(control) ? "release" : slotGesture(control, event.over?.id)
      if (!gesture) return
      void Promise.resolve(resolve(event.active.data.current)).then((target) => {
        if (target) onDrop(gesture, target)
      })
    },
  })
  return null
}

interface PendingConversion {
  gesture: Gesture
  target: BindingTarget
}

const CLICK_NOTES: Record<ClickTiming, TranslationKey> = {
  immediate: "editor:gesture.clickNote.immediate",
  onRelease: "editor:gesture.clickNote.onRelease",
  afterWindow: "editor:gesture.clickNote.afterWindow",
}

/**
 * Click, double click and long press of one key or pressable dial, each
 * with its own target, parameter overrides and label, plus the key's
 * timings (docs/gestures-and-work-session.md, section 10).
 */
export function GestureEditor(props: GestureEditorProps) {
  const { t } = useTranslation()
  const { draft, pageId, control, defaults, onChange } = props
  const [pending, setPending] = useState<PendingConversion | null>(null)
  // A removal that moves a look with labels onto a binding with a look of
  // its own: the user chooses how (keep, replace or merge the labels).
  const [lookChoice, setLookChoice] = useState<{
    after: DeviceLayout
    labels: number
    to: Trigger
  } | null>(null)
  /**
   * Commits a removal, keeping the key's title and look on the binding now
   * shown in the same step; says when labels moved, and asks first when the
   * new binding has a look of its own.
   */
  const removeKeepingLook = (after: DeviceLayout) => {
    const transfer = lookTransfer(draft, after, pageId, control)
    if (transfer?.conflict) {
      setLookChoice({ after, labels: transfer.labels, to: transfer.to.trigger })
      return
    }
    onChange(keepLookOnRemoval(draft, after, pageId, control))
    if (transfer && transfer.labels > 0) {
      toast.info(
        t("editor:labels.moved", {
          count: transfer.labels,
          trigger: triggerLabel(transfer.to.trigger),
        }),
      )
    }
  }
  const chooseLook = (move: LookMove | null) => {
    const choice = lookChoice
    setLookChoice(null)
    if (!choice || move === null) return
    onChange(keepLookOnRemoval(draft, choice.after, pageId, control, move))
  }
  const mode = keyMode(draft, pageId, control)
  const timing = clickTiming(draft, pageId, control)
  const own = keyTimings(draft, pageId, control)
  const release = bindingsFor(draft, pageId, control).find((b) => b.trigger.type === "release")
  const doubleClickMs = own?.doubleClickMs ?? defaults.doubleClickMs
  const base = `gesture-${controlKey(control)}`

  const assign = (gesture: Gesture, target: BindingTarget | null) => {
    // A drop of an action that cannot run from this gesture, or whose
    // plugin cannot run it now, is refused (the library offers neither).
    const refused = target
      ? (props.incompatible?.(target, triggerFor(mode, gesture)) ?? notPlaceable(target))
      : null
    if (refused) {
      toast.error(
        t("editor:gesture.refused", {
          gesture: GESTURE_LABELS[gesture].toLowerCase(),
          control: controlLabel(control),
        }),
        { description: refused },
      )
      return
    }
    if (target && needsConversion(draft, pageId, control, gesture, target)) {
      setPending({ gesture, target })
      return
    }
    const next = setGesture(draft, pageId, control, gesture, target)
    // Removing an action keeps the key's title and look (on the binding now shown).
    if (target) onChange(next)
    else removeKeepingLook(next)
  }

  /**
   * Why `target` cannot be newly assigned (a plugin action whose plugin is
   * missing, a read-only workflow); `null` when it can, and for what the
   * gesture already does.
   */
  /** A press or click bound to an action that only shows something (its contexts). */
  const displayOnlyBinding = (binding: ControlBinding | undefined): boolean => {
    const target = binding?.target
    if (!binding || target?.type !== "action") return false
    if (contextOf({ kind: "binding", trigger: binding.trigger }) !== "key") return false
    const action = props.profile.actions.find((a) => a.id === target.action)
    const type = props.types.find((ty) => ty.id === action?.actionType)
    return type !== undefined && isDisplayOnly(type)
  }
  const notPlaceable = (target: BindingTarget): string | null => {
    const value = props.encode(target)
    if (bindingsFor(draft, pageId, control).some((b) => props.encode(b.target) === value)) {
      return null
    }
    for (const group of props.groups) {
      const option = group.options.find((o) => o.value === value)
      if (option) return option.unavailable ?? null
    }
    return null
  }

  const confirmConversion = () => {
    if (!pending) return
    const converted = convertToGestures(draft, pageId, control).layout
    onChange(setGesture(converted, pageId, control, pending.gesture, pending.target))
    setPending(null)
  }

  /** What the raw release does (keys without gestures); `null` removes it. */
  const assignRelease = (target: BindingTarget | null) => {
    const trigger: Trigger = { type: "release" }
    const refused = !target
      ? null
      : target.type === "stateful"
        ? t("editor:library.gestureOnly")
        : (props.incompatible?.(target, trigger) ?? notPlaceable(target))
    if (refused) {
      toast.error(
        t("editor:gesture.refused", {
          gesture: t("editor:gesture.release").toLowerCase(),
          control: controlLabel(control),
        }),
        { description: refused },
      )
      return
    }
    if (target) {
      onChange(
        setBinding(draft, pageId, {
          control,
          trigger,
          target,
          label: release?.label ?? null,
          ...(release?.appearance ? { appearance: release.appearance } : {}),
        }),
      )
    } else {
      removeKeepingLook(removeBinding(draft, pageId, control, trigger))
    }
  }

  const bound = (gesture: Gesture) =>
    bindingsFor(draft, pageId, control).some((b) => b.trigger.type === gesture)
  const allGroups = [...props.groups, ...(props.gestureGroups ?? [])]

  /** The key's timing a gesture's card edits, once it matters. */
  const timingOf = (gesture: Gesture): CardTiming | undefined => {
    const field =
      gesture === "doubleClick" ? "doubleClickMs" : gesture === "longPress" ? "longPressMs" : null
    if (!field || (!bound(gesture) && own?.[field] === undefined)) return undefined
    const id = `${base}-${field === "doubleClickMs" ? "double" : "long"}-ms`
    return {
      id,
      field,
      value: own?.[field],
      fallback: defaults[field],
      onCommit: (value) => {
        onChange(setKeyTiming(draft, pageId, control, field, value), id)
      },
    }
  }

  // One card per gesture, and the raw release while the key has no gestures.
  const cards: GestureCardProps[] = GESTURES.map((gesture) => {
    const binding = gestureBinding(draft, pageId, control, gesture)
    const trigger = binding?.trigger ?? triggerFor(mode, gesture)
    const title = GESTURE_LABELS[gesture]
    return {
      id: `${base}-${gesture}`,
      title,
      trigger,
      binding,
      groups: allGroups,
      encode: props.encode,
      decode: props.decode,
      incompatible: props.incompatible,
      notRun: notRunReason(props.notRun, trigger, binding),
      migrate: notRunMigrate(props.notRun, trigger, binding),
      displayOnly: displayOnlyBinding(binding),
      slot: props.resolveDrop ? gestureSlotId(control, gesture) : undefined,
      pageId,
      timing: timingOf(gesture),
      onAssign: (target) => {
        assign(gesture, target)
      },
      details: {
        draft,
        pageId,
        profile: props.profile,
        types: props.types,
        inSentence: GESTURE_LABELS[gesture].toLowerCase(),
        onChange,
        onEditWorkflow: props.onEditWorkflow,
        labelPlaceholder: props.labelPlaceholder,
        titleInHeader: props.titleInHeader,
        actionSettings: props.actionSettings,
      },
    }
  })
  if (mode !== "gesture") {
    const trigger: Trigger = { type: "release" }
    cards.push({
      id: `${base}-release`,
      title: t("editor:gesture.release"),
      trigger,
      binding: release,
      groups: props.groups,
      encode: props.encode,
      decode: props.decode,
      incompatible: props.incompatible,
      notRun: notRunReason(props.notRun, trigger, release),
      migrate: notRunMigrate(props.notRun, trigger, release),
      help: t("editor:gesture.releaseHint"),
      slot: props.resolveDrop ? releaseSlotId(control) : undefined,
      pageId,
      onAssign: (target) => {
        assignRelease(target)
      },
    })
  }
  return (
    <section className="grid min-w-0 gap-3" aria-labelledby={`${base}-heading`}>
      <div className="grid gap-1">
        <h3 id={`${base}-heading`} className="text-sm font-medium">
          {t("editor:gesture.heading")}
        </h3>
        <p className="text-xs text-muted-foreground" role="note">
          {mode === "raw"
            ? t("editor:gesture.rawNote")
            : t(CLICK_NOTES[timing], { ms: doubleClickMs })}
        </p>
      </div>

      {props.resolveDrop && (
        <SlotDrops
          control={control}
          resolve={props.resolveDrop}
          onDrop={(gesture, target) => {
            if (gesture === "release") assignRelease(target)
            else assign(gesture, target)
          }}
        />
      )}
      <GestureCarousel
        label={t("editor:gesture.heading")}
        count={cards.length}
        dragAware={props.resolveDrop !== undefined}
        renderCard={(index, visible) => {
          const card = cards[index]
          if (!card) return null
          return <GestureCard key={card.id} {...card} droppable={visible} />
        }}
      />

      {props.issues.length > 0 && (
        <ul className="grid gap-1 text-xs text-destructive" role="alert">
          {props.issues.map((issue) => (
            <li key={issue}>{issue}</li>
          ))}
        </ul>
      )}

      <ConversionDialog
        control={control}
        pending={pending}
        release={release}
        doubleClickMs={doubleClickMs}
        longPressMs={own?.longPressMs ?? defaults.longPressMs}
        draft={draft}
        profile={props.profile}
        onConfirm={confirmConversion}
        onCancel={() => {
          setPending(null)
        }}
      />
      <LookMoveDialog
        open={lookChoice !== null}
        labels={lookChoice?.labels ?? 0}
        trigger={lookChoice ? triggerLabel(lookChoice.to) : ""}
        onChoose={chooseLook}
      />
    </section>
  )
}

function ConversionDialog({
  control,
  pending,
  release,
  doubleClickMs,
  longPressMs,
  draft,
  profile,
  onConfirm,
  onCancel,
}: {
  control: ControlId
  pending: PendingConversion | null
  release: ControlBinding | undefined
  doubleClickMs: number
  longPressMs: number
  draft: DeviceLayout
  profile: GestureEditorProps["profile"]
  onConfirm: () => void
  onCancel: () => void
}) {
  const { t } = useTranslation()
  const name = controlLabel(control)
  const adding = pending ? GESTURE_LABELS[pending.gesture].toLowerCase() : ""

  return (
    <ConfirmDialog
      open={pending !== null}
      onOpenChange={(open) => {
        // Also called after confirming; `onConfirm` has cleared it already.
        if (!open) onCancel()
      }}
      title={t("editor:gesture.convert.title", { control: name })}
      // The switch itself keeps the key's click; only removing a release
      // binding drops data, so only then is it a destructive confirmation.
      destructive={release !== undefined}
      description={
        <span className="grid gap-2">
          {pending?.gesture === "click" ? (
            <span>{t("editor:gesture.convert.click", { control: name })}</span>
          ) : (
            <span>
              {pending?.gesture === "doubleClick"
                ? t("editor:gesture.convert.doubleClick", {
                    control: name,
                    gesture: adding,
                    ms: doubleClickMs,
                  })
                : t("editor:gesture.convert.longPress", {
                    control: name,
                    gesture: adding,
                    ms: longPressMs,
                  })}
            </span>
          )}
          {release && (
            <span>
              {t("editor:gesture.convert.release", {
                target: targetLabel(release.target, profile.actions, profile.workflows, draft),
              })}
            </span>
          )}
          <span>{t("editor:gesture.convert.untilSaved")}</span>
        </span>
      }
      confirmLabel={t("editor:gesture.convert.confirm")}
      onConfirm={onConfirm}
    />
  )
}
