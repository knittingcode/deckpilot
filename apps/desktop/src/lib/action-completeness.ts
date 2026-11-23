// Draft actions (`deckpilot_core::completeness`): an action, a key's or
// gesture's binding, a Multi Action step or a workflow node may be saved
// before its required parameters are filled in. Such a use is *incomplete*:
// it is kept, shown as "Needs configuration", and never runs. Every run is
// refused before any handler is called, with the result code
// `configuration-incomplete` and the missing parameter keys.
//
// Incomplete is derived from the type's parameter schema, never stored: a
// required parameter is missing when it has no value and no declared
// default, or its value is blank (text, code, credential, a list without
// items, an empty project role, a blank template). A reference counts as a
// value until it is resolved at run time. Every other problem (a value of
// the wrong type, an unknown parameter, a value too long, a choice not
// offered) is still refused when saving.

import type {
  ActionInstance,
  ActionParameters,
  ActionResult,
  ActionType,
  ControlId,
  GraphIssue,
  ParameterSpec,
  ParameterValue,
  Trigger,
} from "./types"

/** The result code of a run refused because required parameters are missing. */
export const CONFIGURATION_INCOMPLETE = "configuration-incomplete"

/**
 * Where an incomplete action is used, as `get_profile` (`incompleteUses`),
 * `save_layout` and `save_workflow` (`incomplete`) report it
 * (`IncompleteUse` of deckpilot-core). `missing` is for the use: the
 * instance's values with the binding's or step's overrides; for a node,
 * parameters filled by a mapping are not missing.
 */
export interface IncompleteUse {
  site:
    | { kind: "binding"; layout: string; page: string; control: ControlId; trigger: Trigger }
    | { kind: "step"; workflow: string; index: number }
    | { kind: "node"; workflow: string; node: string }
  action: string
  /** Parameter keys, in the type's order. */
  missing: string[]
}

/**
 * What the app backend adds to its answers. Every field is left out when
 * there is nothing incomplete, so older answers read as complete.
 */
export interface ProfileCompleteness {
  /** Action id → its missing parameter keys (the instance's own values). */
  incompleteActions?: Record<string, string[]>
  incompleteUses?: IncompleteUse[]
}

/** `create_action`, `update_action` and `duplicate_action` answer with this. */
export type SavedAction = ActionInstance & {
  /** Missing parameter keys; left out when the action is complete. */
  incomplete?: string[]
}

/** The missing parameters of an action of `profile`; empty when complete. */
export function incompleteAction(profile: ProfileCompleteness, action: string): string[] {
  return profile.incompleteActions?.[action] ?? []
}

function sameJson(a: unknown, b: unknown): boolean {
  return JSON.stringify(a) === JSON.stringify(b)
}

/** The missing parameters of one binding (a key's trigger or a gesture); empty when complete. */
export function incompleteBinding(
  profile: ProfileCompleteness,
  binding: { layout: string; page: string; control: ControlId; trigger: Trigger },
): string[] {
  const use = profile.incompleteUses?.find(
    (u) =>
      u.site.kind === "binding" &&
      u.site.layout === binding.layout &&
      u.site.page === binding.page &&
      sameJson(u.site.control, binding.control) &&
      sameJson(u.site.trigger, binding.trigger),
  )
  return use?.missing ?? []
}

/** The missing parameters of a Multi Action step (0-based index); empty when complete. */
export function incompleteStep(
  profile: ProfileCompleteness,
  workflow: string,
  index: number,
): string[] {
  const use = profile.incompleteUses?.find(
    (u) => u.site.kind === "step" && u.site.workflow === workflow && u.site.index === index,
  )
  return use?.missing ?? []
}

/** The missing parameters of a workflow node; empty when complete. */
export function incompleteNode(
  profile: ProfileCompleteness,
  workflow: string,
  node: string,
): string[] {
  const use = profile.incompleteUses?.find(
    (u) => u.site.kind === "node" && u.site.workflow === workflow && u.site.node === node,
  )
  return use?.missing ?? []
}

function blank(spec: ParameterSpec, value: ParameterValue): boolean {
  switch (value.type) {
    case "reference":
      return false
    case "template":
      return value.value.trim() === ""
    case "resource":
      return value.value.type === "projectRole" && value.value.role.trim() === ""
    case "text":
      switch (spec.kind.type) {
        case "text":
        case "code":
        case "credential":
          return value.value.trim() === ""
        case "lines":
        case "pairs":
          return value.value.split("\n").every((line) => line.trim() === "")
        default:
          return false
      }
    default:
      return false
  }
}

/**
 * The required parameters `parameters` leave missing, in the type's order:
 * what the backend computes (`ActionType::missing`), for a form that is
 * being edited. The backend's answer wins once saved.
 */
export function missingParameters(
  type: Pick<ActionType, "parameters">,
  parameters: ActionParameters,
): string[] {
  return type.parameters
    .filter((spec) => {
      if (!spec.required) return false
      const value = parameters[spec.key] ?? spec.default
      return value === undefined || blank(spec, value)
    })
    .map((spec) => spec.key)
}

/**
 * The missing parameter keys of a run refused as incomplete, `null` for any
 * other result. `data.missing` holds the keys separated by commas.
 */
export function refusedAsIncomplete(result: Pick<ActionResult, "code" | "data">): string[] | null {
  if (result.code !== CONFIGURATION_INCOMPLETE) return null
  const missing = result.data?.missing ?? ""
  return missing === "" ? [] : missing.split(",")
}

/**
 * Whether a graph issue only says a node is not configured yet: shown on
 * the node as "Needs configuration"; it does not prevent saving, and the
 * workflow does not run until it is fixed.
 */
export function isIncompleteIssue(issue: GraphIssue): boolean {
  return issue.code === CONFIGURATION_INCOMPLETE
}

/** The localized texts (en, pl) of the contract. */
export const COMPLETENESS_TEXT = {
  /** "Needs configuration" / "Wymaga konfiguracji": the badge on a key, step, node or action. */
  badge: "actions:completeness.badge",
  /** "Missing: {{parameters}}" with the parameters' labels joined. */
  missing: "actions:completeness.missing",
  /** "{{name}} needs configuration before it can run." */
  runRefused: "actions:completeness.runRefused",
  /** The result code, as other codes are worded. */
  code: "errors:pluginAction.configuration-incomplete",
} as const
