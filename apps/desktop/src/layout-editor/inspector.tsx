import type { ReactNode } from "react"
import { Eraser } from "lucide-react"
import { toast } from "sonner"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { GestureEditor, type GestureEditorProps } from "@/components/gesture-editor/gesture-editor"
import { IncompatibleNote } from "@/components/incompatible-note"
import { MissingActionNote } from "@/components/missing-action-note"
import { ContextMigrations, type MigrationOffer } from "@/components/context-migrations"
import { contextOf, isDisplayOnly } from "@/lib/action-context"
import { DEFAULT_TIMINGS, controlIssues } from "@/components/gesture-editor/model"
import {
  acceptsGestures,
  bindingsFor,
  controlKey,
  controlLabel,
  page as findPage,
  removeBinding,
  sameTrigger,
  setBinding,
  supportedTriggers,
  triggerKey,
  triggerLabel,
} from "@/lib/layout-model"
import { useTranslation } from "@/i18n"
import { useActionModules } from "@/lib/app-context"
import { SWITCH_LAYOUT, layoutSwitchOf } from "@/lib/layout-navigation"
import type {
  ActionType,
  BindingTarget,
  ControlBinding,
  ControlId,
  DeviceLayout,
  DeviceModel,
  GestureTimings,
  Profile,
  Trigger,
} from "@/lib/types"

import { isSystemBack } from "@/lib/folder-back"
import { brokenFolderControl, folderControlOf, pressFolderControl } from "@/lib/folder-control"

import { shownBinding, stylable } from "./appearance"
import { FolderControlPanel, FolderSettings } from "./folder-settings"
import { KeyAppearanceEditor, KeysAppearanceEditor } from "./key-appearance-editor"
import { useKeyLook } from "./key-look-context"
import { labelsOf } from "./labels"
import { useLookKeepingRemoval } from "./use-look-keeping-removal"
import { LayoutNavigationPanels, type LayoutNavigationContext } from "./layout-switch-inspector"
import { SystemActionPanels } from "./system-action-inspector"
import {
  decodeTarget,
  encodeTarget,
  WORK_SESSION_MODULE,
  gestureOnlyGroups,
  readOnlyTarget,
  unavailableTarget,
  missingTarget,
  type TargetGroup,
} from "./targets"

interface InspectorProps {
  model: DeviceModel
  draft: DeviceLayout
  pageId: string
  selection: ControlId[]
  focus: ControlId | null
  groups: TargetGroup[]
  folder: string | undefined
  /** For the gesture editor: targets' names and parameter overrides. */
  profile?: Pick<Profile, "actions" | "workflows" | "projects">
  types?: ActionType[]
  /** This computer's default gesture timings. */
  gestureDefaults?: GestureTimings
  /** Messages of the last failed save; those about the control show inline. */
  issues?: string[]
  /** Commits a change; `group` merges consecutive edits into one undo step. */
  onChange: (layout: DeviceLayout, group?: string) => void
  /**
   * Applies an edit to the latest draft (a change decided later, in a
   * dialog); without it, to the draft as last rendered.
   */
  onUpdate?: (edit: (layout: DeviceLayout) => DeviceLayout) => void
  onClear: () => void
  /**
   * Turns what was dragged onto a gesture's slot into the target to
   * assign; see `GestureEditorProps.resolveDrop`.
   */
  resolveDrop?: GestureEditorProps["resolveDrop"]
  /** Opens the steps of a Multi Action a gesture runs. */
  onEditWorkflow?: (workflow: string) => void
  /**
   * Why `target` cannot run from `control` for `trigger`, in words; `null`
   * when it can. Such targets are offered disabled with the reason.
   */
  /** The saved action `id` a binding runs: shared with whom, and its definition. */
  actionSettings?: (id: string) => ReactNode
  incompatible?: (control: ControlId, target: BindingTarget, trigger: Trigger) => string | null
  /**
   * Bindings of the focused control that are kept although their action
   * cannot run from it, with the reason (`preview_layout`).
   */
  notRun?: { trigger: Trigger; reason: string; migrate?: MigrationOffer | undefined }[]
  /** For keys that switch layouts or go back to the previous one. */
  layoutNavigation?: LayoutNavigationContext
}

/**
 * The inspector's frame: a section of the editor's workspace below the
 * device (no card and no visible heading of its own). Its accessible name
 * says what it shows ("Key 15 settings").
 */
function InspectorSection({
  label,
  target,
  children,
}: {
  label: string
  /** Diagnostics: the control shown, `multiple` or `none`. */
  target: string
  children?: ReactNode
}) {
  return (
    <section
      data-slot="inspector"
      data-inspector-target={target}
      aria-label={label}
      className="grid min-h-24 content-start gap-3"
    >
      {children}
    </section>
  )
}

/** Settings of the selected control, or a summary of a multi-selection. */
export function Inspector(props: InspectorProps) {
  const { t } = useTranslation()
  const { selection, focus } = props
  if (selection.length === 0 || !focus) {
    // Nothing selected: an empty inspector (it keeps its size).
    return <InspectorSection label={t("editor:inspector.label")} target="none" />
  }
  if (selection.length > 1) {
    return (
      <InspectorSection
        label={t("editor:inspector.regionMany", { count: selection.length })}
        target="multiple"
      >
        <div className="grid gap-3 text-sm">
          <p className="text-muted-foreground">
            {t("editor:inspector.multiHint", { controls: selection.map(controlLabel).join(", ") })}
          </p>
          <KeysAppearanceEditor controls={selection} />
          <Button variant="outline" size="sm" className="w-fit" onClick={props.onClear}>
            <Eraser /> {t("editor:inspector.clearMany", { count: selection.length })}
          </Button>
        </div>
      </InspectorSection>
    )
  }
  // The folder's protected system Back has nothing to set: an empty
  // inspector with a name of its own. Its protection is in the editing logic.
  if (isSystemBack(props.model, props.draft, props.pageId, focus)) {
    return <InspectorSection label={t("editor:systemBack.region")} target={controlKey(focus)} />
  }
  // A folder key only opens its folder (`lib/folder-control.ts`): the
  // folder's settings and the key's look, no gestures or actions.
  const opens = folderControlOf(props.draft, props.pageId, focus)
  if (opens !== undefined) {
    return (
      <InspectorSection
        label={t("editor:inspector.region", { control: controlLabel(focus) })}
        target={controlKey(focus)}
      >
        <div data-slot="folder-control" className="grid gap-4">
          {/* The key's look and visible title; the folder's name is its own. */}
          <KeyAppearanceEditor
            control={focus}
            defaultTitle={findPage(props.draft, opens)?.name}
            onChange={props.onChange}
          />
          <FolderControlPanel
            model={props.model}
            draft={props.draft}
            pageId={props.pageId}
            control={focus}
            folder={opens}
            onChange={props.onChange}
          />
        </div>
      </InspectorSection>
    )
  }
  return <ControlInspector {...props} control={focus} />
}

/**
 * Why the target a control already uses cannot run now; it stays bound.
 * `readOnly`: it does run, it only cannot be changed here.
 */
export function UnavailableNote({
  reason,
  readOnly = false,
}: {
  reason: string | undefined
  readOnly?: boolean
}) {
  const { t } = useTranslation()
  if (!reason) return null
  return (
    <p
      className={readOnly ? "text-xs text-muted-foreground" : "text-xs text-destructive"}
      role="note"
    >
      {readOnly
        ? t("editor:inspector.readOnly", { reason })
        : t("editor:inspector.unavailable", { reason })}
    </p>
  )
}

function ControlInspector({
  model,
  draft,
  pageId,
  control,
  groups,
  folder,
  onChange: commit,
  onUpdate: update,
  profile,
  types = [],
  gestureDefaults = DEFAULT_TIMINGS,
  issues = [],
  resolveDrop,
  onEditWorkflow,
  incompatible,
  notRun = [],
  layoutNavigation,
  actionSettings,
}: InspectorProps & { control: ControlId }) {
  const { t } = useTranslation()
  // A folder opens from a key of its own (`lib/folder-control.ts`): an edit
  // that puts one next to what the key runs is refused; one that leaves it
  // the key's only binding opens it on the press.
  const onChange = (next: DeviceLayout, group?: string) => {
    if (brokenFolderControl(next, pageId, control)) {
      toast.error(t("editor:folderControl.needsFreeKey"))
      return
    }
    commit(pressFolderControl(next, pageId, control), group)
  }
  const onUpdate = update
    ? (edit: (layout: DeviceLayout) => DeviceLayout) => {
        update((latest) => {
          const next = edit(latest)
          if (next === latest || brokenFolderControl(next, pageId, control)) return latest
          return pressFolderControl(next, pageId, control)
        })
      }
    : undefined
  // Removing a binding keeps the key's title and look (labels included) on
  // the binding shown after, as the gestures' removal does.
  const removal = useLookKeepingRemoval({ draft, pageId, control, onChange, onUpdate })
  const bindings = bindingsFor(draft, pageId, control)
  const gestures = acceptsGestures(model, control)
  // The Work Session controller belongs to the Work Session module: offered
  // while it is enabled, and kept on a control that already uses it.
  const modules = useActionModules()
  const workSession =
    modules.some((m) => m.id === WORK_SESSION_MODULE && m.enabled) ||
    bindings.some((b) => b.target.type === "stateful")
  // Press and release are edited as gestures (single click is the press).
  const triggers = supportedTriggers(model, control).filter(
    (trigger) => !gestures || (trigger.type !== "press" && trigger.type !== "release"),
  )
  // Only what can be chosen here: targets that cannot run now are left
  // out, unless the control already uses them (then they show as chosen).
  const chosen = new Set(bindings.map((b) => encodeTarget(b.target)))
  // Saved actions are not a generic list here (types come from the Action
  // Library): only the ones this control already runs are offered, so a
  // shared saved action stays reachable where it is used.
  const offered = groups
    .map((group) => ({
      ...group,
      options: group.options.filter(
        (o) =>
          (group.id !== "actions" || chosen.has(o.value)) &&
          (o.unavailable === undefined || chosen.has(o.value)),
      ),
    }))
    .filter((group) => group.options.length > 0)
  const page = findPage(draft, pageId)
  const controlMessages = page ? controlIssues(issues, page, control) : []
  /** A press bound to an action that only shows something on the key (its contexts). */
  const displayOnly = (binding: ControlBinding) => {
    const target = binding.target
    if (target.type !== "action") return false
    if (contextOf({ kind: "binding", trigger: binding.trigger }) !== "key") return false
    const action = profile?.actions.find((a) => a.id === target.action)
    const type = types.find((ty) => ty.id === action?.actionType)
    return type !== undefined && isDisplayOnly(type)
  }
  const why = incompatible
    ? (target: BindingTarget, trigger: Trigger) => incompatible(control, target, trigger)
    : undefined
  // The key header edits the visible title: the label of the binding the
  // key shows is not edited a second time below.
  const header = useKeyLook() !== null && stylable(control)
  const shown = header ? shownBinding(draft, pageId, control) : undefined
  const titleInHeader = (binding: ControlBinding) => binding === shown
  // A Switch layout key without a label shows its layout's name.
  const labelPlaceholder = (binding: ControlBinding) => {
    if (binding.target.type !== "action" || !layoutNavigation) return undefined
    const id = binding.target.action
    const action = profile?.actions.find((a) => a.id === id)
    if (action?.actionType !== SWITCH_LAYOUT) return undefined
    const layout = layoutSwitchOf(action, binding.target.overrides).layout
    const name = layoutNavigation.layouts.find((l) => l.id === layout)?.name
    return name
  }
  return (
    <InspectorSection
      label={t("editor:inspector.region", { control: controlLabel(control) })}
      target={controlKey(control)}
    >
      <div className="grid gap-4">
        <KeyAppearanceEditor
          control={control}
          defaultTitle={shown ? labelPlaceholder(shown) : undefined}
          onChange={onChange}
        />
        {gestures && (
          <GestureEditor
            key={`${pageId}:${controlKey(control)}`}
            draft={draft}
            pageId={pageId}
            control={control}
            profile={profile ?? { actions: [], workflows: [], projects: [] }}
            // Every target, so a drop of one that cannot be placed now is
            // refused with its reason (the cards choose nothing themselves).
            groups={groups}
            gestureGroups={gestureOnlyGroups(workSession)}
            encode={encodeTarget}
            decode={decodeTarget}
            types={types}
            defaults={gestureDefaults}
            issues={controlMessages}
            onChange={onChange}
            onUpdate={onUpdate}
            resolveDrop={resolveDrop}
            onEditWorkflow={onEditWorkflow}
            incompatible={why}
            notRun={notRun}
            labelPlaceholder={labelPlaceholder}
            titleInHeader={titleInHeader}
            actionSettings={actionSettings}
          />
        )}
        {!gestures && controlMessages.length > 0 && (
          <ul className="grid gap-1 text-xs text-destructive" role="alert">
            {controlMessages.map((issue) => (
              <li key={issue}>{issue}</li>
            ))}
          </ul>
        )}
        {triggers.map((trigger) => {
          const binding = bindings.find((b) => sameTrigger(b.trigger, trigger))
          const id = `${controlKey(control)}-${triggerKey(trigger)}`
          const lost = binding ? missingTarget(groups, encodeTarget(binding.target)) : undefined
          return (
            <div key={triggerKey(trigger)} className="grid gap-2">
              <Label htmlFor={id}>{triggerLabel(trigger)}</Label>
              <Select
                value={encodeTarget(binding?.target)}
                onValueChange={(value) => {
                  const target = decodeTarget(value)
                  if (target) {
                    onChange(
                      setBinding(draft, pageId, {
                        control,
                        trigger,
                        target,
                        label: binding?.label ?? null,
                        ...(binding?.appearance ? { appearance: binding.appearance } : {}),
                      }),
                    )
                  } else {
                    removal.remove((layout) => removeBinding(layout, pageId, control, trigger))
                  }
                }}
              >
                <SelectTrigger id={id} className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="none">{t("editor:inspector.nothing")}</SelectItem>
                  {offered.map((group) => (
                    <SelectGroup key={group.label}>
                      <SelectLabel>{group.label}</SelectLabel>
                      {group.options.map((option) => {
                        const reason = why?.(option.target, trigger) ?? undefined
                        // Not offered where it cannot run, unless it is what is chosen.
                        if (
                          reason !== undefined &&
                          option.value !== encodeTarget(binding?.target)
                        ) {
                          return null
                        }
                        return (
                          <SelectItem
                            key={option.value}
                            value={option.value}
                            disabled={option.unavailable !== undefined || reason !== undefined}
                            title={reason}
                          >
                            {option.unavailable !== undefined
                              ? t("editor:inspector.optionUnavailable", { option: option.label })
                              : reason !== undefined
                                ? t("editor:inspector.optionNotHere", { option: option.label })
                                : option.label}
                          </SelectItem>
                        )
                      })}
                    </SelectGroup>
                  ))}
                </SelectContent>
              </Select>
              {binding && lost ? (
                <MissingActionNote
                  missing={lost}
                  target={binding.target}
                  actions={profile?.actions ?? []}
                  types={types}
                  projects={profile?.projects ?? []}
                  title={binding.label}
                  onReplace={() => {
                    document.getElementById(id)?.focus()
                  }}
                  onRemove={() => {
                    removal.remove((layout) => removeBinding(layout, pageId, control, trigger))
                  }}
                />
              ) : (
                <UnavailableNote
                  reason={
                    binding ? unavailableTarget(groups, encodeTarget(binding.target)) : undefined
                  }
                  readOnly={binding ? readOnlyTarget(groups, encodeTarget(binding.target)) : false}
                />
              )}
              <IncompatibleNote
                reason={
                  binding
                    ? notRun.find((n) => sameTrigger(n.trigger, binding.trigger))?.reason
                    : undefined
                }
              />
              {binding && (
                <ContextMigrations
                  offer={notRun.find((n) => sameTrigger(n.trigger, binding.trigger))?.migrate}
                />
              )}
              {binding?.target.type === "action" && actionSettings?.(binding.target.action)}
              {binding &&
                (binding.trigger.type === "doubleClick" || binding.trigger.type === "longPress") &&
                labelsOf(binding.appearance).length > 0 && (
                  <p
                    className="text-xs text-muted-foreground"
                    role="note"
                    data-slot="dormant-labels"
                  >
                    {t("editor:labels.dormant", { trigger: triggerLabel(binding.trigger) })}
                  </p>
                )}
              {binding && displayOnly(binding) && (
                <p className="text-xs text-muted-foreground" role="note">
                  {t("editor:context.displayOnly")}
                </p>
              )}
              {binding && !titleInHeader(binding) && (
                <Input
                  aria-label={t("editor:inspector.labelFor", { trigger: triggerLabel(trigger) })}
                  placeholder={labelPlaceholder(binding) ?? t("editor:inspector.labelPlaceholder")}
                  maxLength={32}
                  value={binding.label ?? ""}
                  onChange={(e) => {
                    onChange(
                      setBinding(draft, pageId, { ...binding, label: e.target.value || null }),
                      `label:${id}`,
                    )
                  }}
                />
              )}
            </div>
          )
        })}
        {/*
          The folder this key opens: its settings live here, on the key
          that opens it (Auto Exit; Move Back for a legacy folder).
        */}
        {folder && (
          <FolderSettings
            key={folder}
            model={model}
            draft={draft}
            pageId={folder}
            onChange={onChange}
          />
        )}
        {layoutNavigation && profile && (
          <LayoutNavigationPanels
            draft={draft}
            pageId={pageId}
            control={control}
            actions={profile.actions}
            context={layoutNavigation}
            onChange={onChange}
          />
        )}
        {layoutNavigation && profile && (
          <SystemActionPanels
            draft={draft}
            pageId={pageId}
            control={control}
            actions={profile.actions}
            types={types}
            gestures={gestures}
            context={layoutNavigation}
            onChange={onChange}
          />
        )}
        {removal.dialog}
      </div>
    </InspectorSection>
  )
}
