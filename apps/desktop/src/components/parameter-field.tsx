import { CodeField, CredentialField, ListField, PairsField } from "@/components/structured-fields"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { ReferenceDiagnostics } from "@/components/variable-picker/reference-diagnostics"
import { ReferenceInput } from "@/components/variable-picker/reference-input"
import { ReferenceValueField } from "@/components/variable-picker/reference-value"
import { VariablePicker } from "@/components/variable-picker/variable-picker"
import { useTranslation } from "@/i18n"
import { MachineResourceField } from "@/components/machine-resource-field"
import { initialValue } from "@/lib/parameters"
import {
  WEBSITE,
  WEBSITE_PROBLEMS,
  checkWebsite,
  isSecondaryParameter,
  machinePick,
  parameterTexts,
} from "@/lib/system-actions"
import { PROJECTS_PLUGIN, useIsPluginActive } from "@/lib/plugin-features"
import { PROFILE_PARAMETER, ProfileField, SWITCH_PROFILE_TYPE } from "@/modules/deck/profile-field"
import { CANDIDATES_PARAMETER, RANDOM_TYPE } from "@/modules/deck/candidates"
import { CandidatesField } from "@/modules/deck/candidates-field"
import { MULTI_SWITCH_TYPE, STATES_PARAMETER } from "@/modules/deck/switch-states"
import { SwitchStatesField } from "@/modules/deck/switch-states-field"
import type {
  ActionInstance,
  ParameterSpec,
  ParameterValue,
  Project,
  ResourceRef,
  Workflow,
} from "@/lib/types"
import { TEMPLATE_KIND, templateReferences } from "@/lib/variables"

/**
 * Form control for one action parameter, chosen from its declared kind.
 * `projects` are the projects of the profile the action belongs to; project
 * parameters can only refer to those.
 *
 * Every editor that draws parameters uses this field, so all of them get
 * the variable picker: text takes `{{ … }}` references at the cursor (a
 * `template` value), other kinds a whole-value `reference` of a compatible
 * type. The reference is what is saved, never its previewed value.
 *
 * `actionType` is the type the parameter belongs to, when the caller knows
 * it: a parameter whose text names something of the application (the
 * profile of "Switch profile") is then drawn as a list of what exists.
 * `targets` are the profile's actions and workflows, for the candidates of
 * a Random Action and the states of a Multi Action Switch.
 */
export function ParameterField({
  spec,
  value,
  projects = [],
  actionType,
  targets,
  onChange,
  idPrefix = "parameter",
  error,
}: {
  spec: ParameterSpec
  value: ParameterValue
  projects?: Project[]
  actionType?: string
  /** Makes the field's ids unique when several forms are on screen. */
  idPrefix?: string
  /** Shown under the control, which is marked invalid and described by it. */
  error?: string | undefined
  targets?: {
    actions: Pick<ActionInstance, "id" | "name" | "actionType">[]
    workflows: Pick<Workflow, "id" | "name">[]
  }
  onChange: (value: ParameterValue) => void
}) {
  const { t } = useTranslation()
  const id = `${idPrefix}-${spec.key}`
  const errorId = error ? `${id}-error` : undefined
  const labelId = `${id}-label`
  const profile =
    actionType === SWITCH_PROFILE_TYPE && spec.key === PROFILE_PARAMETER && value.type === "text"
  const candidates =
    targets !== undefined &&
    actionType === RANDOM_TYPE &&
    spec.key === CANDIDATES_PARAMETER &&
    value.type === "text"
  const states =
    targets !== undefined &&
    actionType === MULTI_SWITCH_TYPE &&
    spec.key === STATES_PARAMETER &&
    value.type === "text"
  // The System actions' machine-local values: picked, bound on this
  // computer, never typed as paths.
  const pick = value.type === "resource" ? machinePick(actionType, spec.key) : null
  const texts = parameterTexts(actionType, spec)
  const website =
    actionType === WEBSITE && spec.key === "url" && value.type === "text"
      ? checkWebsite(value.value)
      : null
  // An empty address is reported as a missing required value.
  const websiteError =
    website?.problem && website.problem !== "empty" ? t(WEBSITE_PROBLEMS[website.problem]) : null
  // Open's file inside a folder: only on request, or when set.
  const secondary =
    isSecondaryParameter(actionType, spec.key) && value.type === "text" && value.value === ""
  const field = (
    <div
      className="grid gap-2"
      data-invalid={error ? true : undefined}
      // The control inside is marked once rendered (see `useInvalid`).
      ref={(element) => {
        markInvalid(element, id, errorId, spec.required)
      }}
    >
      <Label id={labelId} htmlFor={id}>
        {texts.label}
        {spec.required && <span className="text-destructive">*</span>}
      </Label>
      {pick && value.type === "resource" ? (
        <MachineResourceField
          id={id}
          pick={pick}
          optional={!spec.required}
          value={resourceOrNull(value.value)}
          onChange={(reference) => {
            onChange({
              type: "resource",
              value: reference ?? { type: "projectRole", role: "" },
            })
          }}
          advanced={<ResourceControl id={`${id}-key`} value={value.value} onChange={onChange} />}
        />
      ) : profile ? (
        <ProfileField
          id={id}
          value={value.value}
          onChange={(profileId) => {
            onChange({ type: "text", value: profileId })
          }}
        />
      ) : candidates ? (
        <CandidatesField
          id={id}
          value={value.value}
          actions={targets.actions}
          workflows={targets.workflows}
          onChange={(text) => {
            onChange({ type: "text", value: text })
          }}
        />
      ) : states ? (
        <SwitchStatesField
          id={id}
          value={value.value}
          actions={targets.actions}
          workflows={targets.workflows}
          onChange={(text) => {
            onChange({ type: "text", value: text })
          }}
        />
      ) : value.type === "reference" ? (
        <ReferenceValueField
          id={id}
          labelId={labelId}
          fieldLabel={spec.label}
          reference={value.value}
          kind={spec.kind}
          onReference={(reference) => {
            onChange({ type: "reference", value: reference })
          }}
          onFixedValue={() => {
            onChange(initialValue(spec))
          }}
        />
      ) : spec.kind.type === "text" ? (
        <Control id={id} spec={spec} value={value} projects={projects} onChange={onChange} />
      ) : (
        <div className="flex min-w-0 items-start gap-1">
          <div className="min-w-0 flex-1">
            <Control id={id} spec={spec} value={value} projects={projects} onChange={onChange} />
          </div>
          <VariablePicker
            kind={spec.kind}
            fieldLabel={spec.label}
            label={t("actions:parameters.useVariable", { label: spec.label })}
            onPick={(reference) => {
              onChange({ type: "reference", value: reference })
            }}
          />
        </div>
      )}
      {error && (
        <p id={errorId} className="text-xs text-destructive" data-slot="field-error">
          {error}
        </p>
      )}
      {websiteError && (
        <p className="text-xs text-destructive" role="alert">
          {websiteError}
        </p>
      )}
      {website?.url && website.url !== (value.type === "text" ? value.value.trim() : "") && (
        <p className="text-xs text-muted-foreground" role="status">
          {t("actions:system.website.opens", { url: website.url })}
        </p>
      )}
      {texts.description && <p className="text-xs text-muted-foreground">{texts.description}</p>}
    </div>
  )
  if (!secondary) return field
  return (
    <details className="grid gap-2 text-sm">
      <summary className="cursor-pointer text-xs text-muted-foreground">
        {t("actions:system.picker.inside", { label: texts.label })}
      </summary>
      <div className="pt-2">{field}</div>
    </details>
  )
}

/** A resource reference that names something, else `null`. */
function resourceOrNull(reference: ResourceRef): ResourceRef | null {
  const name = reference.type === "fixed" ? reference.key : reference.role
  return name.trim() === "" ? null : reference
}

function Control({
  id,
  spec,
  value,
  projects,
  onChange,
}: {
  id: string
  spec: ParameterSpec
  value: ParameterValue
  projects: Project[]
  onChange: (value: ParameterValue) => void
}) {
  const { t } = useTranslation()
  const kind = spec.kind
  if (kind.type === "text" && (value.type === "text" || value.type === "template")) {
    const template = value.type === "template"
    const { references, unclosed } = template
      ? templateReferences(value.value)
      : { references: [], unclosed: false }
    return (
      <>
        <ReferenceInput
          id={id}
          value={value.value}
          fieldLabel={spec.label}
          multiline={kind.multiline}
          maxLength={kind.maxLength}
          aria-invalid={unclosed || undefined}
          onChange={(text, inserted) => {
            // Only a value that is a template is read for references;
            // `{{` typed into plain text stays literal text.
            const asTemplate = (template || inserted) && text.includes("{{")
            onChange({ type: asTemplate ? "template" : "text", value: text })
          }}
        />
        <ReferenceDiagnostics
          references={references}
          kind={TEMPLATE_KIND}
          unclosed={unclosed}
          literalBraces={!template && value.value.includes("{{")}
        />
      </>
    )
  }
  if (kind.type === "integer" && value.type === "integer") {
    return (
      <Input
        id={id}
        type="number"
        min={kind.min}
        max={kind.max}
        value={value.value}
        onChange={(e) => {
          onChange({ type: "integer", value: Math.trunc(Number(e.target.value)) })
        }}
      />
    )
  }
  if (kind.type === "boolean" && value.type === "boolean") {
    return (
      <input
        id={id}
        type="checkbox"
        className="size-4"
        checked={value.value}
        onChange={(e) => {
          onChange({ type: "boolean", value: e.target.checked })
        }}
      />
    )
  }
  if (kind.type === "choice" && value.type === "choice") {
    return (
      <Select
        value={value.value}
        onValueChange={(choice) => {
          onChange({ type: "choice", value: choice })
        }}
      >
        <SelectTrigger id={id} className="w-56">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {kind.options.map((option) => (
            <SelectItem key={option} value={option}>
              {option}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    )
  }
  if (kind.type === "resource" && value.type === "resource") {
    return <ResourceControl id={id} value={value.value} onChange={onChange} />
  }
  if (kind.type === "project" && value.type === "project") {
    return (
      <ProjectControl
        id={id}
        optional={!spec.required}
        value={value.value}
        projects={projects}
        onChange={(project) => {
          onChange({ type: "project", value: project })
        }}
      />
    )
  }
  if (kind.type === "lines" && value.type === "text") {
    return (
      <ListField
        id={id}
        value={value.value}
        itemLabel={kind.itemLabel}
        maxItems={kind.maxItems}
        maxLength={kind.maxLength}
        onChange={(text) => {
          onChange({ type: "text", value: text })
        }}
      />
    )
  }
  if (kind.type === "pairs" && value.type === "text") {
    return (
      <PairsField
        id={id}
        value={value.value}
        separator={kind.separator}
        nameLabel={kind.nameLabel}
        valueLabel={kind.valueLabel}
        maxItems={kind.maxItems}
        caseInsensitive={kind.caseInsensitive}
        noSecrets={kind.noSecrets}
        onChange={(text) => {
          onChange({ type: "text", value: text })
        }}
      />
    )
  }
  if (kind.type === "code" && value.type === "text") {
    return (
      <CodeField
        id={id}
        value={value.value}
        language={kind.language}
        maxLength={kind.maxLength}
        onChange={(text) => {
          onChange({ type: "text", value: text })
        }}
      />
    )
  }
  if (kind.type === "credential" && value.type === "text") {
    return (
      <CredentialField
        id={id}
        value={value.value}
        optional={!spec.required}
        onChange={(text) => {
          onChange({ type: "text", value: text })
        }}
      />
    )
  }
  return <p className="text-sm text-destructive">{t("actions:parameters.unsupported")}</p>
}

/** Select value standing for "no project set", i.e. the current project. */
const CURRENT_PROJECT = "current"

/**
 * Chooses a project of the profile. Optional project parameters offer
 * “Current project”, which is saved as no value (resolved when the action
 * runs), never as the id of the project that happens to be active now.
 */
function ProjectControl({
  id,
  optional,
  value,
  projects,
  onChange,
}: {
  id: string
  optional: boolean
  value: string
  projects: Project[]
  onChange: (project: string) => void
}) {
  const { t } = useTranslation()
  // Projects are the Projects plugin's: without it the value is kept and
  // shown, but cannot be changed (the action does not run either).
  const active = useIsPluginActive(PROJECTS_PLUGIN)
  const known = projects.some((p) => p.id === value)
  if (!active) {
    const current = projects.find((p) => p.id === value)
    return (
      <div className="grid gap-1">
        <p id={id} className="text-sm">
          {current
            ? `${current.key} · ${current.name}`
            : value === ""
              ? t("actions:parameters.project.current")
              : t("actions:parameters.project.notInProfile")}
        </p>
        <p className="text-xs text-muted-foreground" role="note">
          {t("actions:parameters.project.pluginOff")}
        </p>
      </div>
    )
  }
  if (projects.length === 0 && !optional) {
    return (
      <p id={id} className="text-sm text-muted-foreground">
        {t("actions:parameters.project.none")}
      </p>
    )
  }
  const selected = known ? value : value === "" && optional ? CURRENT_PROJECT : ""
  return (
    <>
      <Select
        value={selected}
        onValueChange={(choice) => {
          onChange(choice === CURRENT_PROJECT ? "" : choice)
        }}
      >
        <SelectTrigger id={id} className="w-64">
          <SelectValue placeholder={t("actions:parameters.project.choose")} />
        </SelectTrigger>
        <SelectContent>
          {optional && (
            <SelectItem value={CURRENT_PROJECT}>
              {t("actions:parameters.project.current")}
            </SelectItem>
          )}
          {projects.map((project) => (
            <SelectItem key={project.id} value={project.id}>
              {project.key} · {project.name}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      {value !== "" && !known && (
        <p className="text-xs text-destructive">{t("actions:parameters.project.notInProfile")}</p>
      )}
    </>
  )
}

function ResourceControl({
  id,
  value,
  onChange,
}: {
  id: string
  value: ResourceRef
  onChange: (value: ParameterValue) => void
}) {
  const { t } = useTranslation()
  const text = value.type === "fixed" ? value.key : value.role
  const set = (reference: ResourceRef) => {
    onChange({ type: "resource", value: reference })
  }
  return (
    // Side by side where there is room, the field under the source in a
    // narrow column (the Inspector), never squeezed.
    <div className="flex min-w-0 flex-wrap gap-2">
      <Select
        value={value.type}
        onValueChange={(type) => {
          set(type === "fixed" ? { type: "fixed", key: text } : { type: "projectRole", role: text })
        }}
      >
        <SelectTrigger
          aria-label={t("actions:parameters.resource.source")}
          className="w-56 max-w-full shrink-0"
        >
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="projectRole">
            {t("actions:parameters.resource.projectRole")}
          </SelectItem>
          <SelectItem value="fixed">{t("actions:parameters.resource.fixed")}</SelectItem>
        </SelectContent>
      </Select>
      <Input
        id={id}
        className="min-w-40 flex-1"
        value={text}
        placeholder={value.type === "fixed" ? "shop.repository" : "repository"}
        onChange={(e) => {
          set(
            value.type === "fixed"
              ? { type: "fixed", key: e.target.value }
              : { type: "projectRole", role: e.target.value },
          )
        }}
      />
    </div>
  )
}

/**
 * Marks the field's control (`#id`, whatever kind it is) as required and,
 * with an error, invalid and described by it: one place for every control
 * kind (inputs, selects, pickers).
 */
function markInvalid(
  container: HTMLElement | null,
  id: string,
  errorId: string | undefined,
  required: boolean,
) {
  const control = container?.querySelector<HTMLElement>(`[id="${id}"]`)
  if (!control) return
  if (required) control.setAttribute("aria-required", "true")
  if (errorId) {
    control.setAttribute("aria-invalid", "true")
    control.setAttribute("aria-describedby", errorId)
  } else if (control.getAttribute("aria-describedby")?.endsWith("-error")) {
    control.removeAttribute("aria-invalid")
    control.removeAttribute("aria-describedby")
  }
}
