// Variables in key titles: the contract between the editor and the agent
// (docs/variables.md, "Variables in key titles"; protocol 17, capability
// `displayTemplates`; Rust: deckpilot_core::variables::display and
// deckpilot_ipc::display_templates).
//
// The editor never resolves a title itself. The agent resolves every title
// with the one function the device uses (`resolve_display_template`), and
// the editor shows what it answers:
//
// - the picker lists `listDisplayVariables` (display-safe references only:
//   no sensitive variables, no secret references, nothing of a run, a
//   device or the work session) and inserts `{{ <reference> }}`;
// - the title field shows `resolveDisplayTemplates` for the binding's title
//   template, with a non-destructive warning for each `unresolved` entry
//   (the template itself is never changed or cleared);
// - `preview_layout` faces carry the same answer for each key's title as
//   `ControlFace.titleDisplay`, so the Canvas, the Inspector and the device
//   agree.
//
// Syntax: `{{ shared.team.name }}`, `{{ projects.current.key }}` (the
// engine's reference grammar). The older placeholders `{profile}`,
// `{project.key}` and `{project.name}` keep working, resolved through the
// engine too; `{page.number}`, `{page.count}` and `{page.name}` are the
// layout's, as before. A reference without a displayable value shows `-`.
// Text between `{{` and `}}` that is not a reference stays as typed and is
// listed in `invalid`.
//
// Refresh without polling: re-ask on `projectChanged`, `profileChanged`, and
// on `sharedVariablesChanged` whose keys name one of the `dependencies`
// (see `dependsOn`).

import { invoke } from "@tauri-apps/api/core"

import type {
  CatalogEntry,
  CatalogFilter,
  ControlId,
  MissingReason,
  ResolutionContext,
  Trigger,
  VariableType,
} from "./types"

/** What a reference without a displayable value shows on a key. */
export const UNRESOLVED_MARKER = "-"

/** Most templates one `resolveDisplayTemplates` may ask about. */
export const MAX_DISPLAY_TEMPLATES = 200

/** Longest template, in bytes (UTF-8). */
export const MAX_DISPLAY_TEMPLATE_BYTES = 2048

/** Which text of a key a template is for; labels add kinds later. */
export type TextSlot = { kind: "title" }

/** One text of one binding (a binding is its page, control and trigger). */
export interface TextKey {
  /** The page's id. */
  page: string
  control: ControlId
  trigger: Trigger
  slot: TextSlot
}

/** What a resolved text depends on. */
export type DisplayDependency =
  | { kind: "profile" }
  | { kind: "project" }
  /** A variable, by the reference as written (`shared.team.name`). */
  | { kind: "variable"; reference: string }

/** Why a reference shows `-`. Never carries a value. */
export type UnresolvedReason =
  /** Not display-safe, or sensitive, or a secret: it never resolves on a key. */
  | { reason: "notDisplayable" }
  /** No value now (no project, no value set, unknown variable, …). */
  | { reason: "missing"; missing: MissingReason }
  /** Its module or plugin is disabled, unsupported or not installed. */
  | { reason: "unavailable"; owner: string }
  /** Its value is not text (an id, a resource, JSON). */
  | { reason: "notText"; found: VariableType }
  /** It exists only while a workflow runs. */
  | { reason: "runtimeOnly" }

/** A reference of a template shown as `-`: its name as written, and why. */
export type UnresolvedReference = {
  /** Without braces (`shared.team.name`), or a legacy name (`project.key`). */
  reference: string
} & UnresolvedReason

/**
 * Where one `-` marker is in `DisplayText.text`, so the editor can tell it
 * from a value that reads `-`: UTF-16 offsets (JavaScript string indices),
 * `end` exclusive.
 */
export interface MarkerSpan {
  start: number
  end: number
  /** The reference it stands for, as in `UnresolvedReference.reference`. */
  reference: string
}

/** A template resolved for display, as the device shows it. */
export interface DisplayText {
  /** The text the key shows (before `{page.…}` and text layout). */
  text: string
  /** References shown as `-`, in order, each once. Omitted when none. */
  unresolved?: UnresolvedReference[]
  /** Every marker in `text`, in order (a reference used twice has two). Omitted when none. */
  markers?: MarkerSpan[]
  /** Text between `{{ }}` that is not a reference, kept as typed. Omitted when none. */
  invalid?: string[]
  /** Omitted for a static text. */
  dependencies?: DisplayDependency[]
}

/** One answer of `resolveDisplayTemplates`: the template as asked, resolved. */
export interface ResolvedDisplayTemplate extends DisplayText {
  template: string
}

declare module "./types" {
  interface ControlFace {
    /**
     * The key's title template as the agent resolved it for this preview
     * (the binding's label, else its action's), with its diagnostics.
     * Omitted for keys whose title has no `{`, and when no agent of
     * protocol 17 answered (the face's title is then resolved by the core
     * alone: the older placeholders, `system.profile.current.name`,
     * `system.project.current.*` and `system.platform`; other references
     * show `-`).
     */
    titleDisplay?: ResolvedDisplayTemplate
  }
}

/**
 * The references a key title may use in `context`: display-safe only,
 * metadata only. Fails with a `variablesAgentNotRunning` or
 * `variablesAgentTooOld` code like `listVariables`.
 */
export function listDisplayVariables(
  context: ResolutionContext,
  filter?: CatalogFilter,
): Promise<CatalogEntry[]> {
  return invoke<CatalogEntry[]>("list_display_variables", { context, filter })
}

/**
 * What each template shows on a key in `context` (its profile and project;
 * without a project the profile's active one), in the order asked.
 */
export function resolveDisplayTemplates(
  context: ResolutionContext,
  templates: string[],
): Promise<ResolvedDisplayTemplate[]> {
  return invoke<ResolvedDisplayTemplate[]>("resolve_display_templates", { context, templates })
}

/** Whether a `sharedVariablesChanged` key changes a text with `dependencies`. */
export function dependsOn(dependencies: DisplayDependency[] | undefined, key: string): boolean {
  return (dependencies ?? []).some(
    (dependency) =>
      dependency.kind === "variable" &&
      (dependency.reference === key ||
        dependency.reference === `shared.${key}` ||
        legacyAliases(dependency.reference).includes(key)),
  )
}

/** Names the agent may announce a change of `reference` by, besides its own. */
function legacyAliases(reference: string): string[] {
  const project: Record<string, string> = {
    "system.project.current.id": "projects.current.id",
    "system.project.current.key": "projects.current.key",
    "system.project.current.name": "projects.current.name",
  }
  const canonical = project[reference] ?? reference
  if (canonical.startsWith("projects.current.")) {
    return [canonical, `plugins.deckpilot.${canonical}`]
  }
  return [canonical]
}
