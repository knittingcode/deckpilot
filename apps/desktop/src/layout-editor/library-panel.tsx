import { useDraggable } from "@dnd-kit/core"
import {
  ArrowLeft,
  ChevronDown,
  ChevronRight,
  Folder,
  FolderPlus,
  Layers,
  ListOrdered,
  ListPlus,
  Loader2,
  Timer,
} from "lucide-react"
import { useState, type ReactNode } from "react"

import { ActionIcon } from "@/components/action-icon"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import { useTranslation } from "@/i18n"
import { actionCategoryId, actionCategoryLabel } from "@/lib/action-category"
import type { ActionType } from "@/lib/types"
import { cn } from "@/lib/utils"

import {
  catalogDragId,
  filterCatalog,
  presetDragId,
  type CatalogEntry,
  type CatalogGroup,
} from "./catalog"
import type { ActionPreset, DragSource } from "./editing"
import { gesturePresetDragId } from "./gesture-presets"
import type { TargetGroup, TargetOption } from "./targets"
import { actionTypeDescription, actionTypeName } from "@/lib/system-actions"

/** A 16px icon in a small neutral square, the same for every row. */
function EntryIcon({ children }: { children: ReactNode }) {
  return (
    <span
      data-slot="library-icon"
      className="flex size-6 shrink-0 items-center justify-center rounded-md border bg-muted/60 text-muted-foreground [&_svg]:size-4 [&_svg]:shrink-0"
    >
      {children}
    </span>
  )
}

function OptionIcon({ option }: { option: TargetOption }) {
  if (option.kind === "action") return <ActionIcon name={option.icon} />
  if (option.kind === "workflow") return <ListOrdered aria-hidden />
  if (option.kind === "stateful") return <Timer aria-hidden />
  if (option.target.type === "back") return <ArrowLeft aria-hidden />
  return <Folder aria-hidden />
}

/** A compact row of the library, about 32px high: icon, then name. */
const ENTRY =
  "flex h-8 min-w-0 cursor-grab items-center gap-2 rounded-md px-1 text-sm transition-colors select-none hover:bg-accent hover:text-accent-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none active:cursor-grabbing"

/** A row that cannot be dragged: muted, with why under the name. */
const DISABLED_ENTRY = "grid min-w-0 gap-0.5 rounded-md px-1 py-1 text-sm text-muted-foreground"

function LibraryEntry({ option }: { option: TargetOption }) {
  const { t } = useTranslation()
  const source: DragSource = { type: "target", target: option.target }
  const { attributes, listeners, setNodeRef, isDragging } = useDraggable({
    id: `library:${option.value}`,
    data: source,
    // Plugin actions that cannot run now cannot be newly placed, nor
    // actions that cannot run from the selected control.
    disabled: option.unavailable !== undefined || option.incompatible !== undefined,
  })
  if (option.incompatible !== undefined && option.unavailable === undefined) {
    return (
      <IncompatibleEntry
        label={option.label}
        reason={option.incompatible}
        icon={<OptionIcon option={option} />}
      />
    )
  }
  if (option.unavailable !== undefined) {
    return (
      <li
        className={DISABLED_ENTRY}
        aria-label={t("editor:library.unavailable", {
          name: option.label,
          reason: option.unavailable,
        })}
        aria-disabled
        data-compatibility="unavailable"
      >
        <span className="flex h-6 min-w-0 items-center gap-2">
          <EntryIcon>
            <OptionIcon option={option} />
          </EntryIcon>
          <span className="min-w-0 flex-1 truncate" title={option.label}>
            {option.label}
          </span>
        </span>
        <span className="pl-8 text-xs">{option.unavailable}</span>
      </li>
    )
  }
  return (
    <li
      ref={setNodeRef}
      className={cn(ENTRY, isDragging && "opacity-60")}
      {...attributes}
      {...listeners}
      aria-label={t("editor:library.drag", { name: option.label })}
    >
      <EntryIcon>
        <OptionIcon option={option} />
      </EntryIcon>
      <span className="min-w-0 flex-1 truncate" title={option.label}>
        {option.label}
      </span>
    </li>
  )
}

/**
 * An entry that cannot go on the selected control: shown, not draggable,
 * with the reason (docs/device-controls.md).
 */
function IncompatibleEntry({
  label,
  reason,
  icon,
}: {
  label: string
  reason: string
  icon: ReactNode
}) {
  const { t } = useTranslation()
  return (
    <li
      className={DISABLED_ENTRY}
      aria-label={t("editor:library.incompatible", { name: label, reason })}
      aria-disabled
      data-compatibility="incompatible"
    >
      <span className="flex h-6 min-w-0 items-center gap-2 opacity-70">
        <EntryIcon>{icon}</EntryIcon>
        <span className="min-w-0 flex-1 truncate" title={label}>
          {label}
        </span>
      </span>
      <span className="pl-8 text-xs">{reason}</span>
    </li>
  )
}

/** The id of the "New folder" entry among the editor's draggables. */
export const NEW_FOLDER_DRAG_ID = "library:new-folder"

/**
 * "New folder": dropped on an empty control, it creates a folder in the
 * open page and makes the control open it.
 */
function NewFolderEntry() {
  const { t } = useTranslation()
  const source: DragSource = { type: "newFolder" }
  const { attributes, listeners, setNodeRef, isDragging } = useDraggable({
    id: NEW_FOLDER_DRAG_ID,
    data: source,
  })
  return (
    <li
      ref={setNodeRef}
      className={cn(ENTRY, isDragging && "opacity-60")}
      {...attributes}
      {...listeners}
      aria-label={t("editor:library.newFolderDrag")}
      title={t("editor:library.newFolderTitle")}
    >
      <EntryIcon>
        <FolderPlus aria-hidden />
      </EntryIcon>
      <span className="min-w-0 flex-1 truncate">{t("editor:library.newFolder")}</span>
    </li>
  )
}

/** The id of the "Multi Action" entry among the editor's draggables. */
export const NEW_MULTI_ACTION_DRAG_ID = "library:new-multi-action"

/**
 * "Multi Action": dropped on a control or a gesture, it creates an empty
 * step workflow, binds it and opens its steps.
 */
function NewMultiActionEntry() {
  const { t } = useTranslation()
  const source: DragSource = { type: "newMultiAction" }
  const { attributes, listeners, setNodeRef, isDragging } = useDraggable({
    id: NEW_MULTI_ACTION_DRAG_ID,
    data: source,
  })
  return (
    <li
      ref={setNodeRef}
      className={cn(ENTRY, isDragging && "opacity-60")}
      {...attributes}
      {...listeners}
      aria-label={t("editor:library.multiActionDrag")}
      title={t("editor:library.multiActionTitle")}
    >
      <EntryIcon>
        <ListPlus aria-hidden />
      </EntryIcon>
      <span className="min-w-0 flex-1 truncate">{t("editor:library.multiAction")}</span>
    </li>
  )
}

/**
 * An action type: dropped on a control, an action of it with its default
 * settings is created (or the one that exists is reused) and bound.
 */
function CatalogItem({ entry, incompatible }: { entry: CatalogEntry; incompatible?: string }) {
  const { t } = useTranslation()
  const source: DragSource = { type: "actionType", actionType: entry.type.id }
  const { attributes, listeners, setNodeRef, isDragging } = useDraggable({
    id: catalogDragId(entry.type.id),
    data: source,
    disabled: incompatible !== undefined || entry.unavailable !== undefined,
  })
  // A plugin's type it cannot run now (e.g. a permission it lacks): listed,
  // not placeable, with why.
  if (entry.unavailable !== undefined && entry.grant) {
    return <GrantEntry entry={entry} grant={entry.grant} />
  }
  if (entry.unavailable !== undefined) {
    return (
      <li
        className={DISABLED_ENTRY}
        aria-label={t("editor:library.unavailable", {
          name: entry.type.name,
          reason: entry.unavailable,
        })}
        aria-disabled
        data-compatibility="unavailable"
      >
        <span className="flex h-6 min-w-0 items-center gap-2">
          <EntryIcon>
            <ActionIcon name={entry.type.node?.icon} />
          </EntryIcon>
          <span className="min-w-0 flex-1 truncate" title={entry.type.name}>
            {entry.type.name}
          </span>
        </span>
        <span className="pl-8 text-xs">{entry.unavailable}</span>
      </li>
    )
  }
  if (incompatible !== undefined) {
    return (
      <IncompatibleEntry
        label={actionTypeName(entry.type)}
        reason={incompatible}
        icon={<ActionIcon name={entry.type.node?.icon} />}
      />
    )
  }
  return (
    <li
      ref={setNodeRef}
      className={cn(ENTRY, isDragging && "opacity-60")}
      {...attributes}
      {...listeners}
      aria-label={t("editor:library.drag", { name: actionTypeName(entry.type) })}
      title={t("editor:library.typeTitle", {
        description: actionTypeDescription(entry.type),
        module: entry.module,
      })}
    >
      <EntryIcon>
        <ActionIcon name={entry.type.node?.icon} />
      </EntryIcon>
      <span className="min-w-0 flex-1 truncate">{actionTypeName(entry.type)}</span>
    </li>
  )
}

/** Stops an event at the Grant button: no drag, no selection, nothing else. */
const isolate = (event: { stopPropagation: () => void }) => {
  event.stopPropagation()
}

/**
 * A type that only waits for permissions the user can grant: listed, not
 * placeable, with "Permission required" and a Grant button (the permission
 * dialog asks before anything is granted). Permission ids are not shown.
 */
function GrantEntry({
  entry,
  grant,
}: {
  entry: CatalogEntry
  grant: NonNullable<CatalogEntry["grant"]>
}) {
  const { t } = useTranslation()
  return (
    <li
      className={DISABLED_ENTRY}
      aria-label={t("editor:library.unavailable", {
        name: entry.type.name,
        reason: entry.unavailable,
      })}
      data-compatibility="unavailable"
      data-slot="grant-entry"
    >
      <span className="flex h-6 min-w-0 items-center gap-2">
        <EntryIcon>
          <ActionIcon name={entry.type.node?.icon} />
        </EntryIcon>
        <span className="min-w-0 flex-1 truncate" title={entry.type.name}>
          {entry.type.name}
        </span>
        <Button
          variant="outline"
          size="xs"
          className="shrink-0"
          aria-label={grant.label}
          disabled={grant.pending}
          onPointerDown={isolate}
          onMouseDown={isolate}
          onKeyDown={isolate}
          onClick={(event) => {
            event.stopPropagation()
            grant.onGrant()
          }}
        >
          {grant.pending && <Loader2 className="animate-spin" aria-hidden />}
          {t("editor:grant.button")}
        </Button>
      </span>
      <span className="pl-8 text-xs">{entry.unavailable}</span>
    </li>
  )
}

/**
 * A ready-made action of a type in the library, e.g. "Switch to Teams": the
 * profile's action that is exactly it, else a new one, is put on the control.
 */
export interface PresetEntry {
  /** Unique among the presets. */
  key: string
  label: string
  description: string
  /** The catalog group it is listed in, e.g. "Navigation". */
  category: string
  type: ActionType
  preset: ActionPreset
}

function PresetItem({ entry, incompatible }: { entry: PresetEntry; incompatible?: string }) {
  const { t } = useTranslation()
  const source: DragSource = { type: "actionType", actionType: entry.type.id, preset: entry.preset }
  const { attributes, listeners, setNodeRef, isDragging } = useDraggable({
    id: presetDragId(entry.key),
    data: source,
    disabled: incompatible !== undefined,
  })
  const icon = <ActionIcon name={entry.preset.icon ?? entry.type.node?.icon} />
  if (incompatible !== undefined) {
    return <IncompatibleEntry label={entry.label} reason={incompatible} icon={icon} />
  }
  return (
    <li
      ref={setNodeRef}
      className={cn(ENTRY, isDragging && "opacity-60")}
      {...attributes}
      {...listeners}
      aria-label={t("editor:library.drag", { name: entry.label })}
      title={entry.description}
    >
      <EntryIcon>{icon}</EntryIcon>
      <span className="min-w-0 flex-1 truncate">{entry.label}</span>
    </li>
  )
}

/**
 * A gesture preset in the library: one compact entry for several gestures
 * of a control (`layout-editor/gesture-presets.ts`). Its texts are in the
 * window's language already; it never shows an action id.
 */
export interface LibraryGesturePreset {
  /** `GesturePreset.id`. */
  id: string
  label: string
  /** Each gesture and what it does, one per line: the tooltip. */
  summary: string
  /** The catalog group it is listed in, e.g. "Device". */
  category: string
  icon?: string | null
  /** Why it cannot be placed now (a plugin it needs is off, say). */
  unavailable?: string
  /** Why it cannot go on the selected control. */
  incompatible?: string
}

function GesturePresetItem({ entry }: { entry: LibraryGesturePreset }) {
  const { t } = useTranslation()
  const source: DragSource = {
    type: "gesturePreset",
    preset: entry.id,
    name: entry.label,
    icon: entry.icon ?? null,
  }
  const { attributes, listeners, setNodeRef, isDragging } = useDraggable({
    id: gesturePresetDragId(entry.id),
    data: source,
    disabled: entry.unavailable !== undefined || entry.incompatible !== undefined,
  })
  const icon = entry.icon ? <ActionIcon name={entry.icon} /> : <Layers aria-hidden />
  // The subtle mark of a multi-gesture entry; the tooltip says what each does.
  const mark = (
    <Layers
      data-slot="gesture-preset-mark"
      className="size-3.5 shrink-0 text-muted-foreground"
      role="img"
      aria-label={t("editor:gesturePreset.multi")}
    />
  )
  if (entry.unavailable !== undefined) {
    return (
      <li
        className={DISABLED_ENTRY}
        aria-label={t("editor:library.unavailable", {
          name: entry.label,
          reason: entry.unavailable,
        })}
        aria-disabled
        data-compatibility="unavailable"
        data-gesture-preset={entry.id}
      >
        <span className="flex h-6 min-w-0 items-center gap-2">
          <EntryIcon>{icon}</EntryIcon>
          <span className="min-w-0 flex-1 truncate" title={entry.summary}>
            {entry.label}
          </span>
          {mark}
        </span>
        <span className="pl-8 text-xs">{entry.unavailable}</span>
      </li>
    )
  }
  if (entry.incompatible !== undefined) {
    return <IncompatibleEntry label={entry.label} reason={entry.incompatible} icon={icon} />
  }
  return (
    <li
      ref={setNodeRef}
      className={cn(ENTRY, isDragging && "opacity-60")}
      {...attributes}
      {...listeners}
      aria-label={t("editor:gesturePreset.drag", { name: entry.label })}
      title={entry.summary}
      data-gesture-preset={entry.id}
    >
      <EntryIcon>{icon}</EntryIcon>
      <span className="min-w-0 flex-1 truncate">{entry.label}</span>
      {mark}
    </li>
  )
}

/** A group of the library that can be folded away. */
function Section({
  label,
  count,
  open,
  onToggle,
  children,
}: {
  label: string
  count: number
  open: boolean
  onToggle: () => void
  children: ReactNode
}) {
  return (
    <section className="grid gap-0.5 border-t pt-2 first:border-t-0 first:pt-0" aria-label={label}>
      <h3>
        <button
          type="button"
          className="flex h-6 w-full items-center gap-1 rounded-sm px-1 text-left text-[11px] font-medium tracking-wide text-muted-foreground uppercase hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
          aria-expanded={open}
          onClick={onToggle}
        >
          {open ? (
            <ChevronDown className="size-3" aria-hidden />
          ) : (
            <ChevronRight className="size-3" aria-hidden />
          )}
          <span className="flex-1 truncate">{label}</span>
          <span className="tabular-nums" aria-hidden>
            {count}
          </span>
        </button>
      </h3>
      {open && <ul className="grid gap-px">{children}</ul>}
    </section>
  )
}

const matches = (text: string, needle: string) => text.toLowerCase().includes(needle)

/**
 * What can be dragged onto controls, in one searchable list: the
 * profile's actions and workflows, navigation (pages, Back, a new folder,
 * and the navigation action types), sequences (Multi Action) and the
 * action types that need no settings, under their category's name in the
 * window's language (`lib/action-category.ts`), one section per category.
 */
export function LibraryPanel({
  groups,
  catalog = [],
  presets = [],
  gesturePresets = [],
  multiAction = false,
  variant = "card",
  compatibility,
  displays,
}: {
  groups: TargetGroup[]
  /** Action types that can be placed directly; see `catalogGroups`. */
  catalog?: CatalogGroup[]
  /** Ready-made actions listed with the catalog's group of their category. */
  presets?: PresetEntry[]
  /** Gesture presets, listed with the catalog's group of their category too. */
  gesturePresets?: LibraryGesturePreset[]
  /** Offers "Multi Action", which puts a new step workflow on a key. */
  multiAction?: boolean
  /**
   * `card`: a card of its own. `plain`: the content only, for a container
   * that has the title and scrolls (the editor's library sidebar).
   */
  variant?: "card" | "plain"
  /**
   * The control the library is for (docs/device-controls.md): entries that
   * cannot go on it stay listed, disabled with the reason (`incompatible`
   * of the groups' options, `catalog` for action types).
   */
  compatibility?: {
    /** The selected control, e.g. "Key 3"; `null` when none is. */
    control: string | null
    catalog: (type: ActionType) => string | null
  }
  /**
   * Actions and types that only show something on a key (their press does
   * nothing; `lib/action-context.ts`), listed in a section of their own.
   */
  displays?: { options: TargetOption[]; catalog: CatalogEntry[] }
}) {
  const { t } = useTranslation()
  const [query, setQuery] = useState("")
  const whyNot = (type: ActionType) =>
    compatibility?.control ? (compatibility.catalog(type) ?? undefined) : undefined
  const [closed, setClosed] = useState<Set<string>>(new Set())
  const needle = query.trim().toLowerCase()
  const searching = needle !== ""
  const shown = groups
    .map((group) => ({
      ...group,
      options: group.options.filter((o) => matches(`${o.label} ${group.label}`, needle)),
    }))
    .filter((group) => group.options.length > 0 || (group.id === "navigation" && !searching))
  const types = filterCatalog(catalog, query, actionCategoryLabel).filter(
    (group) => group.entries.length > 0,
  )
  const shownPresets = presets.filter((p) =>
    matches(`${p.label} ${p.description} ${p.category} ${actionCategoryLabel(p.category)}`, needle),
  )
  const presetsOf = (category: string) =>
    shownPresets.filter((p) => actionCategoryId(p.category) === actionCategoryId(category))
  const shownGesturePresets = gesturePresets.filter(
    (p) =>
      matches(`${p.label} ${p.summary} ${p.category} ${actionCategoryLabel(p.category)}`, needle) &&
      !(hiding && p.incompatible !== undefined),
  )
  const gesturePresetsOf = (category: string) =>
    shownGesturePresets.filter((p) => actionCategoryId(p.category) === actionCategoryId(category))
  const entriesOf = (category: string) =>
    presetsOf(category).length + gesturePresetsOf(category).length
  for (const category of new Set([
    ...shownPresets.map((p) => p.category),
    ...shownGesturePresets.map((p) => p.category),
  ])) {
    if (!types.some((group) => actionCategoryId(group.label) === actionCategoryId(category)))
      types.push({ label: category, entries: [] })
  }
  // The navigation action types are listed with the pages and Back, in one
  // Navigation section; every other category is a section of its own.
  const isNavigation = (category: string) => actionCategoryId(category) === "navigation"
  const navigationTypes = types.find((group) => isNavigation(group.label))
  const otherTypes = types.filter((group) => !isNavigation(group.label))
  // Found by the English words too, whatever the language.
  const folder =
    !searching ||
    matches(`new folder folders navigation ${t("editor:library.searchFolder")}`, needle)
  const multi =
    multiAction &&
    (!searching ||
      matches(
        `new multi action sequence steps workflow ${t("editor:library.searchMulti")}`,
        needle,
      ))
  const hasItems = groups.some((g) => g.id !== "navigation")
  const navigation = shown.find((group) => group.id === "navigation")
  // A search shows what it found, whatever is folded.
  const isOpen = (id: string) => searching || !closed.has(id)
  const toggle = (id: string) => {
    setClosed((current) => {
      const next = new Set(current)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }
  const navigationCount =
    (navigation?.options.length ?? 0) +
    (folder ? 1 : 0) +
    (navigationTypes ? navigationTypes.entries.length + entriesOf(navigationTypes.label) : 0)
  const shownDisplays = {
    options: (displays?.options ?? []).filter((o) => matches(o.label, needle)),
    catalog: (displays?.catalog ?? []).filter((e) =>
      matches(`${e.type.name} ${e.type.description} ${e.type.category}`, needle),
    ),
  }
  const displayCount = shownDisplays.options.length + shownDisplays.catalog.length
  const nothing =
    searching && shown.length === 0 && types.length === 0 && displayCount === 0 && !folder && !multi

  /** A category's action types and presets. */
  const catalogEntries = (group: CatalogGroup) => (
    <>
      {group.entries.map((entry) => (
        <CatalogItem key={entry.type.id} entry={entry} incompatible={whyNot(entry.type)} />
      ))}
      {presetsOf(group.label).map((entry) => (
        <PresetItem key={entry.key} entry={entry} incompatible={whyNot(entry.type)} />
      ))}
      {gesturePresetsOf(group.label).map((entry) => (
        <GesturePresetItem key={entry.id} entry={entry} />
      ))}
    </>
  )

  const content = (
    <>
      <Input
        type="search"
        className="h-8"
        placeholder={t("editor:library.search")}
        aria-label={t("editor:library.search")}
        value={query}
        onChange={(event) => {
          setQuery(event.target.value)
        }}
      />
      <div className={cn("grid gap-2", variant === "card" && "max-h-[28rem] overflow-y-auto")}>
        {!hasItems && !searching && catalog.length === 0 && (
          <p className="text-sm text-muted-foreground">{t("editor:library.empty")}</p>
        )}
        {nothing && (
          <p className="text-sm text-muted-foreground">{t("editor:library.noMatches")}</p>
        )}
        {displayCount > 0 && (
          <Section
            label={t("editor:library.displays")}
            count={displayCount}
            open={isOpen("displays")}
            onToggle={() => {
              toggle("displays")
            }}
          >
            {shownDisplays.options.map((option) => (
              <LibraryEntry key={option.value} option={option} />
            ))}
            {shownDisplays.catalog.map((entry) => (
              <CatalogItem key={entry.type.id} entry={entry} incompatible={whyNot(entry.type)} />
            ))}
          </Section>
        )}
        {navigationCount > 0 && (
          <Section
            label={t("editor:library.navigation")}
            count={navigationCount}
            open={isOpen("navigation")}
            onToggle={() => {
              toggle("navigation")
            }}
          >
            {folder && <NewFolderEntry />}
            {navigation?.options.map((option) => (
              <LibraryEntry key={option.value} option={option} />
            ))}
            {navigationTypes && catalogEntries(navigationTypes)}
          </Section>
        )}
        {multi && (
          <Section
            label={t("editor:library.sequences")}
            count={1}
            open={isOpen("sequences")}
            onToggle={() => {
              toggle("sequences")
            }}
          >
            <NewMultiActionEntry />
          </Section>
        )}
        {otherTypes.map((group) => {
          const id = `category:${actionCategoryId(group.label)}`
          return (
            <Section
              key={id}
              label={actionCategoryLabel(group.label)}
              count={group.entries.length + entriesOf(group.label)}
              open={isOpen(id)}
              onToggle={() => {
                toggle(id)
              }}
            >
              {catalogEntries(group)}
            </Section>
          )
        })}
        {/* What is not a type comes after the types: workflows and the like. */}
        {shown
          .filter((group) => group.id !== "navigation")
          .map((group) => (
            <Section
              key={group.id}
              label={group.label}
              count={group.options.length}
              open={isOpen(group.id)}
              onToggle={() => {
                toggle(group.id)
              }}
            >
              {group.options.map((option) => (
                <LibraryEntry key={option.value} option={option} />
              ))}
            </Section>
          ))}
      </div>
    </>
  )
  // The search first, then the sections: no instructions above them.
  if (variant === "plain") {
    return <div className="grid gap-3">{content}</div>
  }
  return (
    <Card className="h-fit">
      <CardHeader>
        <CardTitle className="text-sm">{t("editor:library.title")}</CardTitle>
      </CardHeader>
      <CardContent className="grid gap-3">{content}</CardContent>
    </Card>
  )
}
