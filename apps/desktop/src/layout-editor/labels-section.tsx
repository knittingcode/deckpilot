// The Inspector's "Labels" section (design: key labels, §6): further texts
// on a key, in the look of the binding the key shows. The list (front
// first), one label's settings inline, "More", and a stage: the renderer's
// image of the key with the outlines of its labels (`labelBoxes`), where a
// label is selected, dragged or nudged. Label text is never laid out here;
// every change goes to the renderer, which draws it.

import {
  DndContext,
  type DragEndEvent,
  KeyboardSensor,
  PointerSensor,
  closestCenter,
  useSensor,
  useSensors,
} from "@dnd-kit/core"
import {
  SortableContext,
  sortableKeyboardCoordinates,
  useSortable,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable"
import { CSS } from "@dnd-kit/utilities"
import { ChevronDown, ChevronRight, GripVertical, MoreHorizontal, Plus } from "lucide-react"
import { type KeyboardEvent, useEffect, useId, useRef, useState } from "react"

import { KeyImage } from "@/components/key-preview"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { type TranslationKey, useTranslation } from "@/i18n"
import { maxTitleLength, referenceText } from "@/lib/display-templates"
import {
  type FaceLabel,
  type KeyLabel,
  LABEL_DEFAULTS,
  LABEL_LIMITS,
  type LabelBox,
  type LabelPosition,
  type LabelSpot,
} from "@/lib/key-labels"
import { controlKey, controlLabel } from "@/lib/layout-model"
import type { ControlId } from "@/lib/types"
import { cn } from "@/lib/utils"

import { keyColors, shownBinding } from "./appearance"
import { ColorField } from "./color-field"
import { type KeyLook, type LabelSelection, nextSelection } from "./key-look-context"
import {
  LABEL_PRESETS,
  type LabelPreset,
  type LabelReport,
  addLabel,
  copiedLabels,
  defaultPlateOpacity,
  duplicateLabel,
  hasText,
  labelReports,
  labelsOf,
  moveLabel,
  nudge,
  placedAt,
  placedFromBox,
  positionKind,
  removeLabel,
  reorderLabels,
  resized,
  updateLabel,
  withLabels,
} from "./labels"
import { LabelOutlines } from "./label-outlines"
import { NumberField } from "./text-appearance"
import { TitleVariablePicker } from "./title-variables"
import { type ExpectedBoxes, useExpectedBoxes } from "./use-expected-boxes"

const OPEN_KEY = "deckpilot.editor.labelsOpen"

/** Whether the section is open: remembered for the session (chosen default). */
function rememberedOpen(): boolean {
  try {
    return sessionStorage.getItem(OPEN_KEY) === "1"
  } catch {
    return false
  }
}

function rememberOpen(open: boolean) {
  try {
    sessionStorage.setItem(OPEN_KEY, open ? "1" : "0")
  } catch {
    // Not remembered; the section still works.
  }
}

const PRESET_NAMES: Record<LabelPreset, TranslationKey> = {
  top: "editor:labels.preset.top",
  center: "editor:labels.preset.center",
  bottom: "editor:labels.preset.bottom",
  topLeft: "editor:labels.preset.topLeft",
  topRight: "editor:labels.preset.topRight",
  bottomLeft: "editor:labels.preset.bottomLeft",
  bottomRight: "editor:labels.preset.bottomRight",
  ribbon: "editor:labels.preset.ribbon",
}

const SPOT_NAMES: Record<LabelSpot, TranslationKey> = {
  topLeft: "editor:labels.preset.topLeft",
  top: "editor:labels.preset.top",
  topRight: "editor:labels.preset.topRight",
  left: "editor:labels.spot.left",
  center: "editor:labels.preset.center",
  right: "editor:labels.spot.right",
  bottomLeft: "editor:labels.preset.bottomLeft",
  bottom: "editor:labels.preset.bottom",
  bottomRight: "editor:labels.preset.bottomRight",
}

const GRID: LabelSpot[] = [
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

const SIZE_FIELDS = [
  ["width", "editor:labels.width"],
  ["maxWidth", "editor:labels.maxWidth"],
  ["height", "editor:labels.height"],
  ["maxHeight", "editor:labels.maxHeight"],
] as const satisfies readonly (readonly [keyof KeyLabel, TranslationKey])[]

const REPORTS: Record<LabelReport, TranslationKey> = {
  unresolved: "editor:labels.report.unresolved",
  truncated: "editor:labels.report.truncated",
  overlapsTitle: "editor:labels.report.overlapsTitle",
  lowContrast: "editor:labels.report.lowContrast",
  small: "editor:labels.report.small",
}

/** The labels of one key: list, settings and stage. */
export function LabelsSection({ look, control }: { look: KeyLook; control: ControlId }) {
  const { t } = useTranslation()
  const [openState, setOpenState] = useState(rememberedOpen)
  // The selected label is the editor's (shared with the device canvas'
  // outlines) when it provides one, else this section's own.
  const [own, setOwn] = useState<LabelSelection | null>(null)
  const headingId = useId()
  const key = controlKey(control)
  const selection = look.selectLabel ? look.labelSelection : own
  const mine = selection?.key === key ? selection : null
  const selected = mine?.id ?? null
  const editRequest = mine?.edit ?? 0
  const choose = (id: string | null, edit = false) => {
    if (look.selectLabel) look.selectLabel(key, id, edit)
    else setOwn((current) => nextSelection(current, key, id, edit))
  }
  const setSelected = (id: string | null) => {
    choose(id)
  }
  // A label edited from the canvas (a double click) opens the section
  // until it is closed again.
  const [closedAt, setClosedAt] = useState(0)
  const open = openState || (editRequest !== 0 && editRequest !== closedAt)
  const boxes = look.labelBoxes?.get(key) ?? []
  // Where labels moved by a drag or a nudge are, until the renderer's next
  // boxes arrive: the stage and the settings (a mode change) start there.
  const expected = useExpectedBoxes(boxes)
  const binding = shownBinding(look.draft, look.pageId, control)
  if (!binding) return null
  const labels = labelsOf(binding.appearance)
  const shown = look.faces.get(key)?.labels ?? []
  const colors = keyColors(binding.appearance ?? {}, look.defaults)
  const setOpen = (next: boolean) => {
    rememberOpen(next)
    setOpenState(next)
    if (!next) setClosedAt(editRequest)
  }
  const commit = (next: KeyLabel[], group?: string) => {
    look.restyle(withLabels(next), group)
  }
  const add = (preset: LabelPreset) => {
    const added = addLabel(labels, preset)
    if (!added) return
    commit(added.labels)
    setSelected(added.id)
    setOpen(true)
  }
  const current = labels.find((l) => l.id === selected) ?? null

  return (
    <section
      data-slot="key-labels"
      aria-labelledby={headingId}
      className="grid min-w-0 gap-2 border-t pt-2"
    >
      <div className="flex min-w-0 items-center gap-1.5">
        <button
          type="button"
          id={headingId}
          aria-expanded={open}
          className="flex min-w-0 flex-1 items-center gap-1 text-left text-sm font-medium"
          onClick={() => {
            setOpen(!open)
          }}
        >
          {open ? <ChevronDown className="size-4" /> : <ChevronRight className="size-4" />}
          {t("editor:labels.title")}
          {labels.length > 0 && (
            <span
              className="rounded-full bg-muted px-1.5 text-xs text-muted-foreground"
              aria-label={t("editor:labels.count", { count: labels.length })}
            >
              {labels.length}
            </span>
          )}
        </button>
        <AddLabel disabled={labels.length >= LABEL_LIMITS.labels} onAdd={add} />
      </div>
      {open && (
        <>
          {!look.keyLabels && (
            <p className="text-xs text-muted-foreground" role="note">
              {t("editor:labels.agentTooOld")}
            </p>
          )}
          {labels.length > 0 && (
            <LabelStage
              look={look}
              control={control}
              boxes={boxes}
              expected={expected}
              selected={selected}
              onSelect={setSelected}
              onEdit={(id) => {
                choose(id, true)
              }}
              onMove={(id, position) => {
                commit(updateLabel(labels, id, placedAt(position)), `label-move:${key}:${id}`)
              }}
            />
          )}
          <LabelList
            labels={labels}
            shown={shown}
            selected={selected}
            onSelect={(id) => {
              setSelected(selected === id ? null : id)
            }}
            onChange={commit}
            onSelected={setSelected}
          />
          {current && (
            <LabelSettings
              key={current.id}
              look={look}
              label={current}
              editRequest={editRequest}
              box={expected.boxes.find((b) => b.id === current.id)}
              reports={labelReports(
                current,
                boxes.find((b) => b.id === current.id),
                {
                  unresolved: shown.find((s) => s.id === current.id)?.unresolved === true,
                  textColor: colors.text,
                  keyBackground: colors.background,
                },
              )}
              onChange={(patch, group) => {
                commit(updateLabel(labels, current.id, patch), group)
              }}
            />
          )}
        </>
      )}
    </section>
  )
}

function AddLabel({ disabled, onAdd }: { disabled: boolean; onAdd: (p: LabelPreset) => void }) {
  const { t } = useTranslation()
  return (
    <div className="flex">
      <Button
        type="button"
        variant="outline"
        size="sm"
        className="rounded-r-none"
        disabled={disabled}
        title={disabled ? t("editor:labels.full", { count: LABEL_LIMITS.labels }) : undefined}
        onClick={() => {
          onAdd("bottom")
        }}
      >
        <Plus /> {t("editor:labels.add")}
      </Button>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            type="button"
            variant="outline"
            size="icon-sm"
            className="rounded-l-none border-l-0"
            disabled={disabled}
            aria-label={t("editor:labels.addAt")}
          >
            <ChevronDown />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          {LABEL_PRESETS.map((preset) => (
            <DropdownMenuItem
              key={preset}
              onSelect={() => {
                onAdd(preset)
              }}
            >
              {t(PRESET_NAMES[preset])}
            </DropdownMenuItem>
          ))}
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  )
}

/** The list, front first; reordering changes the paint order. */
function LabelList({
  labels,
  shown,
  selected,
  onSelect,
  onSelected,
  onChange,
}: {
  labels: KeyLabel[]
  shown: FaceLabel[]
  selected: string | null
  onSelect: (id: string) => void
  onSelected: (id: string | null) => void
  onChange: (labels: KeyLabel[], group?: string) => void
}) {
  const { t } = useTranslation()
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  )
  if (labels.length === 0) {
    return <p className="text-xs text-muted-foreground">{t("editor:labels.none")}</p>
  }
  // Front first: the reverse of the paint order.
  const rows = [...labels].reverse()
  const onDragEnd = (event: DragEndEvent) => {
    const { active, over } = event
    if (!over || active.id === over.id) return
    const from = labels.findIndex((l) => l.id === active.id)
    const to = labels.findIndex((l) => l.id === over.id)
    if (from >= 0 && to >= 0) onChange(reorderLabels(labels, from, to))
  }
  return (
    <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={onDragEnd}>
      <SortableContext items={rows.map((l) => l.id)} strategy={verticalListSortingStrategy}>
        <ul className="grid min-w-0 gap-1" aria-label={t("editor:labels.list")}>
          {rows.map((label) => (
            <LabelRow
              key={label.id}
              label={label}
              face={shown.find((s) => s.id === label.id)}
              selected={selected === label.id}
              onSelect={() => {
                onSelect(label.id)
              }}
              onEnabled={(enabled) => {
                onChange(updateLabel(labels, label.id, { enabled: enabled ? null : false }))
              }}
              onDuplicate={() => {
                const copy = duplicateLabel(labels, label.id)
                if (copy) {
                  onChange(copy.labels)
                  onSelected(copy.id)
                }
              }}
              onMove={(towards) => {
                onChange(moveLabel(labels, label.id, towards))
              }}
              onRemove={() => {
                onChange(removeLabel(labels, label.id))
                if (selected === label.id) onSelected(null)
              }}
            />
          ))}
        </ul>
      </SortableContext>
    </DndContext>
  )
}

function LabelRow({
  label,
  face,
  selected,
  onSelect,
  onEnabled,
  onDuplicate,
  onMove,
  onRemove,
}: {
  label: KeyLabel
  face: FaceLabel | undefined
  selected: boolean
  onSelect: () => void
  onEnabled: (enabled: boolean) => void
  onDuplicate: () => void
  onMove: (towards: "front" | "back") => void
  onRemove: () => void
}) {
  const { t } = useTranslation()
  const { attributes, listeners, setNodeRef, transform, transition } = useSortable({
    id: label.id,
  })
  const enabled = label.enabled !== false
  // The resolved text as the key shows it; the template while it is not shown.
  const text = face?.text ?? label.text
  const where = positionKind(label.position)
  const chip =
    where.mode === "absolute"
      ? t("editor:labels.mode.absolute")
      : where.spot
        ? t(SPOT_NAMES[where.spot])
        : ""
  return (
    <li
      ref={setNodeRef}
      style={{ transform: CSS.Transform.toString(transform), transition }}
      data-slot="key-label-row"
      data-selected={selected || undefined}
      className={cn(
        "flex min-w-0 items-center gap-1.5 rounded-md border px-1.5 py-1",
        selected && "border-ring bg-accent/50",
      )}
    >
      <button
        type="button"
        className="cursor-grab text-muted-foreground"
        aria-label={t("editor:labels.reorder", { text: label.text })}
        {...attributes}
        {...listeners}
      >
        <GripVertical className="size-4" />
      </button>
      <Switch
        size="sm"
        checked={enabled}
        aria-label={t("editor:labels.enabled", { text: label.text })}
        onCheckedChange={onEnabled}
      />
      <button
        type="button"
        className="grid min-w-0 flex-1 text-left"
        aria-pressed={selected}
        onClick={onSelect}
      >
        <span className={cn("truncate text-sm", !enabled && "text-muted-foreground")}>
          {text || "-"}
          {face?.unresolved && (
            <span
              className="ml-1 inline-block size-1.5 rounded-full bg-amber-500 align-middle"
              role="img"
              aria-label={t("editor:labels.report.unresolved")}
            />
          )}
        </span>
      </button>
      <span className="shrink-0 rounded-sm bg-muted px-1.5 text-xs text-muted-foreground">
        {chip}
      </span>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            aria-label={t("editor:labels.menu", { text: label.text })}
          >
            <MoreHorizontal />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuItem onSelect={onSelect}>{t("editor:labels.edit")}</DropdownMenuItem>
          <DropdownMenuItem onSelect={onDuplicate}>{t("editor:labels.duplicate")}</DropdownMenuItem>
          <DropdownMenuItem
            onSelect={() => {
              onMove("front")
            }}
          >
            {t("editor:labels.moveUp")}
          </DropdownMenuItem>
          <DropdownMenuItem
            onSelect={() => {
              onMove("back")
            }}
          >
            {t("editor:labels.moveDown")}
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem variant="destructive" onSelect={onRemove}>
            {t("editor:labels.remove")}
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </li>
  )
}

/**
 * The renderer's image of the key with outlines where it put the labels:
 * click selects, a drag moves (free position, snapped, one undo step on
 * release), arrows nudge by 1 unit and Shift+arrows by 5, Escape ends.
 */
function LabelStage({
  look,
  control,
  boxes,
  expected,
  selected,
  onSelect,
  onEdit,
  onMove,
}: {
  look: KeyLook
  control: ControlId
  boxes: LabelBox[]
  /** The section's expected boxes (`useExpectedBoxes`), shared with the settings. */
  expected: ExpectedBoxes
  selected: string | null
  onSelect: (id: string | null) => void
  onEdit: (id: string) => void
  onMove: (id: string, position: LabelPosition) => void
}) {
  const { t } = useTranslation()
  const key = controlKey(control)
  const image = look.images.get(key)
  const canvas = boxes[0]?.canvas ?? { width: LABEL_LIMITS.canvas, height: LABEL_LIMITS.canvas }
  // Nudges and drags start from where a label was moved to, until the
  // renderer's next boxes arrive.
  const moveTo = (id: string, position: LabelPosition) => {
    onMove(id, position)
    expected.moved(id, position)
  }
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === "Escape") {
      if (selected) {
        event.stopPropagation()
        onSelect(null)
      }
      return
    }
    if (event.key === "Enter" && selected) {
      event.preventDefault()
      onEdit(selected)
      return
    }
    nudge(event, expected.boxes, selected, moveTo)
  }
  return (
    <div
      role="group"
      tabIndex={0}
      aria-label={t("editor:labels.stage", { control: controlLabel(control) })}
      data-slot="key-label-stage"
      className="relative mx-auto aspect-square w-36 overflow-hidden rounded-lg border border-device-key-border bg-black focus-visible:ring-[3px] focus-visible:ring-ring/50 focus-visible:outline-none"
      style={
        canvas.width !== canvas.height
          ? { aspectRatio: `${canvas.width} / ${canvas.height}` }
          : undefined
      }
      onPointerDown={() => {
        onSelect(null)
      }}
      onKeyDown={onKeyDown}
    >
      {image && <KeyImage image={image} />}
      <LabelOutlines
        boxes={boxes}
        selected={selected}
        onSelect={onSelect}
        onEdit={onEdit}
        onMove={onMove}
        expected={expected}
      />
    </div>
  )
}

/** The opacity field's placeholder: the default's number, or "auto". */
function plateDefault(value: number | "auto" | null, auto: string): string {
  if (value === null) return ""
  return value === "auto" ? auto : String(value)
}

type Patch = { [K in keyof KeyLabel]?: KeyLabel[K] | null }

/** One label's settings: text, position, style, and "More". */
function LabelSettings({
  look,
  label,
  editRequest,
  box,
  reports,
  onChange,
}: {
  look: KeyLook
  label: KeyLabel
  /** Non-zero, and new, when the label was double-clicked: its text field takes the focus. */
  editRequest: number
  box: LabelBox | undefined
  reports: LabelReport[]
  onChange: (patch: Patch, group?: string) => void
}) {
  const { t } = useTranslation()
  const textId = useId()
  const input = useRef<HTMLInputElement>(null)
  const [more, setMore] = useState(false)
  // Text being typed that cannot be kept (empty): the label keeps its last
  // text, and the field says why, until it has text again or loses focus.
  // It belongs to the text it was typed over; a new one (undo) replaces it.
  const [typed, setTyped] = useState<{ for: string; text: string } | null>(null)
  const text = typed?.for === label.text ? typed.text : label.text
  const needsText = !hasText(text)
  useEffect(() => {
    if (editRequest === 0) return
    const field = input.current
    if (!field) return
    field.scrollIntoView({ block: "nearest" })
    field.focus()
    field.select()
  }, [editRequest])
  const position = label.position ?? LABEL_DEFAULTS.position
  const group = (field: string) => `label:${label.id}:${field}`
  const setMode = (mode: LabelPosition["mode"]) => {
    if (mode === position.mode) return
    // The new mode keeps the label where the renderer drew it.
    if (box) {
      onChange(placedFromBox(box, mode))
      return
    }
    onChange(
      placedAt(
        mode === "absolute"
          ? { mode, x: 500, y: 500 }
          : mode === "anchored"
            ? { mode, to: "bottom", inset: 0 }
            : { mode, at: "bottom" },
      ),
    )
  }
  const spot = positionKind(position).spot
  const background = label.background ?? "auto"
  return (
    <div className="grid min-w-0 gap-3 rounded-md border p-2" data-slot="key-label-settings">
      <div className="grid gap-1">
        <label htmlFor={textId} className="text-xs text-muted-foreground">
          {t("editor:labels.text")}
        </label>
        <div className="flex min-w-0 items-center gap-1.5">
          <Input
            id={textId}
            ref={input}
            className="h-8 min-w-0 flex-1"
            maxLength={Math.max(maxTitleLength(text), LABEL_LIMITS.template)}
            value={text}
            aria-invalid={needsText || undefined}
            aria-describedby={needsText ? `${textId}-error` : undefined}
            onChange={(event) => {
              const next = event.target.value
              setTyped({ for: label.text, text: next })
              if (hasText(next)) onChange({ text: next }, group("text"))
            }}
            onBlur={() => {
              setTyped(null)
            }}
          />
          {look.profileId && (
            <TitleVariablePicker
              profileId={look.profileId}
              onPick={(reference) => {
                const start = input.current?.selectionStart ?? text.length
                const end = input.current?.selectionEnd ?? text.length
                const next = text.slice(0, start) + referenceText(reference) + text.slice(end)
                if (Array.from(next).length <= LABEL_LIMITS.template) {
                  setTyped(null)
                  onChange({ text: next })
                }
              }}
            />
          )}
        </div>
        {needsText && (
          <p id={`${textId}-error`} className="text-xs text-destructive" role="alert">
            {t("editor:labels.needsText")}
          </p>
        )}
      </div>

      <div className="grid gap-1.5">
        <span className="text-xs text-muted-foreground">{t("editor:labels.position")}</span>
        <ToggleGroup
          type="single"
          size="sm"
          variant="outline"
          value={position.mode}
          aria-label={t("editor:labels.position")}
          onValueChange={(value) => {
            if (value) setMode(value as LabelPosition["mode"])
          }}
        >
          <ToggleGroupItem value="preset">{t("editor:labels.mode.preset")}</ToggleGroupItem>
          <ToggleGroupItem value="anchored">{t("editor:labels.mode.anchored")}</ToggleGroupItem>
          <ToggleGroupItem value="absolute">{t("editor:labels.mode.absolute")}</ToggleGroupItem>
        </ToggleGroup>
        {position.mode === "absolute" ? (
          <div className="flex flex-wrap gap-2">
            <NumberField
              label={t("editor:labels.x")}
              value={Math.round(position.x / 10)}
              range={[0, 100]}
              placeholder="50"
              onCommit={(value) => {
                onChange({ position: { ...position, x: (value ?? 50) * 10 } })
              }}
            />
            <NumberField
              label={t("editor:labels.y")}
              value={Math.round(position.y / 10)}
              range={[0, 100]}
              placeholder="50"
              onCommit={(value) => {
                onChange({ position: { ...position, y: (value ?? 50) * 10 } })
              }}
            />
          </div>
        ) : (
          <div
            className="grid w-24 grid-cols-3 gap-1"
            role="radiogroup"
            aria-label={t("editor:labels.place")}
          >
            {GRID.map((cell) => {
              const allowed = position.mode === "anchored" || (cell !== "left" && cell !== "right")
              return (
                <button
                  key={cell}
                  type="button"
                  role="radio"
                  aria-checked={spot === cell}
                  aria-label={t(SPOT_NAMES[cell])}
                  disabled={!allowed}
                  className={cn(
                    "size-7 rounded-sm border",
                    spot === cell && "border-ring bg-primary/20",
                    !allowed && "opacity-30",
                  )}
                  onClick={() => {
                    onChange(
                      placedAt(
                        position.mode === "anchored"
                          ? { mode: "anchored", to: cell, inset: position.inset ?? null }
                          : { mode: "preset", at: cell as Exclude<LabelSpot, "left" | "right"> },
                      ),
                    )
                  }}
                />
              )
            })}
          </div>
        )}
        {position.mode === "anchored" && (
          <NumberField
            label={t("editor:labels.inset")}
            value={position.inset ?? undefined}
            range={[0, LABEL_LIMITS.inset]}
            placeholder="0"
            onCommit={(value) => {
              onChange({ position: { ...position, inset: value } })
            }}
          />
        )}
      </div>

      <div className="flex flex-wrap items-end gap-2">
        <NumberField
          label={t("editor:labels.size")}
          value={label.size ?? undefined}
          range={[LABEL_LIMITS.size[0], LABEL_LIMITS.size[1]]}
          placeholder={String(LABEL_DEFAULTS.size)}
          onCommit={(value) => {
            // A smallest size above the new size comes down with it.
            onChange(resized(label, value))
          }}
        />
        <ToggleGroup
          type="single"
          size="sm"
          variant="outline"
          value={label.weight === "bold" ? "bold" : ""}
          onValueChange={(value) => {
            onChange({ weight: value === "bold" ? "bold" : null })
          }}
        >
          <ToggleGroupItem value="bold" aria-label={t("editor:labels.bold")}>
            <span className="font-bold">B</span>
          </ToggleGroupItem>
        </ToggleGroup>
        <ColorField
          compact
          label={t("editor:labels.color")}
          value={label.color ?? undefined}
          onChange={(value, colorGroup) => {
            onChange({ color: value }, colorGroup)
          }}
        />
      </div>

      {reports.length > 0 && (
        <ul className="grid gap-0.5 text-xs" data-slot="key-label-reports">
          {reports.map((report) => (
            <li
              key={report}
              className={cn(
                report === "overlapsTitle"
                  ? "text-muted-foreground"
                  : "text-amber-600 dark:text-amber-400",
              )}
            >
              {t(REPORTS[report])}
            </li>
          ))}
        </ul>
      )}

      <button
        type="button"
        aria-expanded={more}
        className="flex items-center gap-1 text-left text-xs text-muted-foreground"
        onClick={() => {
          setMore(!more)
        }}
      >
        {more ? <ChevronDown className="size-3.5" /> : <ChevronRight className="size-3.5" />}
        {t("editor:labels.more")}
      </button>
      {more && (
        <div className="grid gap-2" data-slot="key-label-more">
          <div className="flex flex-wrap gap-2">
            <NumberField
              label={t("editor:labels.offsetX")}
              value={label.offsetX ?? undefined}
              range={[-LABEL_LIMITS.offset, LABEL_LIMITS.offset]}
              placeholder="0"
              onCommit={(value) => {
                onChange({ offsetX: value })
              }}
            />
            <NumberField
              label={t("editor:labels.offsetY")}
              value={label.offsetY ?? undefined}
              range={[-LABEL_LIMITS.offset, LABEL_LIMITS.offset]}
              placeholder="0"
              onCommit={(value) => {
                onChange({ offsetY: value })
              }}
            />
          </div>
          <div className="flex flex-wrap gap-2">
            {SIZE_FIELDS.map(([field, name]) => (
              <NumberField
                key={field}
                label={t(name)}
                value={label[field] ?? undefined}
                range={[LABEL_LIMITS.percent[0], LABEL_LIMITS.percent[1]]}
                placeholder="-"
                onCommit={(value) => {
                  onChange({ [field]: value })
                }}
              />
            ))}
          </div>
          <Choice
            label={t("editor:labels.align")}
            value={label.align ?? "auto"}
            options={[
              ["auto", t("editor:labels.auto")],
              ["left", t("editor:labels.alignLeft")],
              ["center", t("editor:labels.alignCenter")],
              ["right", t("editor:labels.alignRight")],
            ]}
            onChange={(value) => {
              onChange({ align: value === "auto" ? null : (value as KeyLabel["align"]) })
            }}
          />
          <div className="flex flex-wrap items-end gap-2">
            <Choice
              label={t("editor:labels.background")}
              value={background === "auto" || background === "none" ? background : "color"}
              options={[
                ["auto", t("editor:labels.auto")],
                ["none", t("editor:labels.backgroundNone")],
                ["color", t("editor:labels.backgroundColor")],
              ]}
              onChange={(value) => {
                onChange({
                  background: value === "auto" ? null : value === "none" ? "none" : "#000000",
                  backgroundOpacity: null,
                })
              }}
            />
            {background.startsWith("#") && (
              <ColorField
                compact
                label={t("editor:labels.backgroundColor")}
                value={background}
                onChange={(value, colorGroup) => {
                  onChange({ background: (value as KeyLabel["background"]) ?? null }, colorGroup)
                }}
              />
            )}
            {background !== "none" && (
              <NumberField
                label={t("editor:labels.opacity")}
                value={label.backgroundOpacity ?? undefined}
                range={[0, 100]}
                // The renderer's own default (85 over the title; "auto" while
                // the box does not say why it has a plate): shown, never written.
                placeholder={plateDefault(
                  defaultPlateOpacity(label, box),
                  t("editor:labels.autoShort"),
                )}
                onCommit={(value) => {
                  onChange({ backgroundOpacity: value })
                }}
              />
            )}
          </div>
          <div className="flex flex-wrap gap-2">
            <NumberField
              label={t("editor:labels.padding")}
              value={label.padding ?? undefined}
              range={[LABEL_LIMITS.padding[0], LABEL_LIMITS.padding[1]]}
              placeholder={String(LABEL_DEFAULTS.padding)}
              onCommit={(value) => {
                onChange({ padding: value })
              }}
            />
            <NumberField
              label={t("editor:labels.radius")}
              value={label.radius ?? undefined}
              range={[LABEL_LIMITS.radius[0], LABEL_LIMITS.radius[1]]}
              placeholder={String(LABEL_DEFAULTS.radius)}
              onCommit={(value) => {
                onChange({ radius: value })
              }}
            />
          </div>
          <div className="flex flex-wrap items-end gap-2">
            <Choice
              label={t("editor:labels.overflow")}
              value={label.overflow ?? LABEL_DEFAULTS.overflow}
              options={[
                ["ellipsis", t("editor:labels.overflowEllipsis")],
                ["clip", t("editor:labels.overflowClip")],
                ["wrap", t("editor:labels.overflowWrap")],
                ["shrink", t("editor:labels.overflowShrink")],
              ]}
              onChange={(value) => {
                onChange({
                  overflow:
                    value === LABEL_DEFAULTS.overflow ? null : (value as KeyLabel["overflow"]),
                })
              }}
            />
            {label.overflow === "wrap" && (
              <NumberField
                label={t("editor:labels.maxLines")}
                value={label.maxLines ?? undefined}
                range={[LABEL_LIMITS.lines[0], LABEL_LIMITS.lines[1]]}
                placeholder={String(LABEL_DEFAULTS.lines)}
                onCommit={(value) => {
                  onChange({ maxLines: value })
                }}
              />
            )}
            {label.overflow === "shrink" && (
              <NumberField
                label={t("editor:labels.minSize")}
                value={label.minSize ?? undefined}
                range={[LABEL_LIMITS.size[0], label.size ?? LABEL_DEFAULTS.size]}
                placeholder={String(LABEL_DEFAULTS.minSize)}
                onCommit={(value) => {
                  onChange({ minSize: value })
                }}
              />
            )}
          </div>
          <Choice
            label={t("editor:labels.layer")}
            value={label.layer ?? "auto"}
            options={[
              ["auto", t("editor:labels.auto")],
              ["behindIcon", t("editor:labels.layerBehindIcon")],
              ["overIcon", t("editor:labels.layerOverIcon")],
              ["overTitle", t("editor:labels.layerOverTitle")],
            ]}
            onChange={(value) => {
              onChange({ layer: value === "auto" ? null : (value as KeyLabel["layer"]) })
            }}
          />
          <Choice
            label={t("editor:labels.visible")}
            value={label.visible ?? LABEL_DEFAULTS.visible}
            options={[
              ["always", t("editor:labels.visibleAlways")],
              ["whenResolved", t("editor:labels.visibleWhenResolved")],
              ["whenProject", t("editor:labels.visibleWhenProject")],
            ]}
            onChange={(value) => {
              onChange({
                visible: value === LABEL_DEFAULTS.visible ? null : (value as KeyLabel["visible"]),
              })
            }}
          />
        </div>
      )}
    </div>
  )
}

function Choice({
  label,
  value,
  options,
  onChange,
}: {
  label: string
  value: string
  options: [string, string][]
  onChange: (value: string) => void
}) {
  const id = useId()
  return (
    <div className="grid gap-1">
      <label htmlFor={id} className="text-xs text-muted-foreground">
        {label}
      </label>
      <Select value={value} onValueChange={onChange}>
        <SelectTrigger id={id} size="sm" className="w-44">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {options.map(([option, text]) => (
            <SelectItem key={option} value={option}>
              {text}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  )
}

/**
 * The labels of a multi-selection: no mixed editing; one action copies the
 * primary (first-selected) key's labels to the others, with new ids, in one
 * undo step.
 */
export function LabelsOfSelection({ look, controls }: { look: KeyLook; controls: ControlId[] }) {
  const { t } = useTranslation()
  const primary = controls[0]
  if (!primary) return null
  const source = shownBinding(look.draft, look.pageId, primary)?.appearance
  const labels = labelsOf(source)
  return (
    <div className="flex min-w-0 flex-wrap items-center gap-2" data-slot="key-labels-mixed">
      <span className="text-sm text-muted-foreground">{t("editor:labels.mixed")}</span>
      <Button
        type="button"
        variant="outline"
        size="sm"
        disabled={labels.length === 0}
        onClick={() => {
          look.restyle((appearance) =>
            appearance === source ? appearance : withLabels(copiedLabels(labels))(appearance),
          )
        }}
      >
        {t("editor:labels.copyFrom", { control: controlLabel(primary) })}
      </Button>
    </div>
  )
}
