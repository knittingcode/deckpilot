// The System module's Website, Open, Open Application and Close
// Application actions in the app (docs/system-actions.md): their ids, the
// Website address rules (the same as the core's `paths::website_url`, so
// the form says what saving would refuse), the resource keys the pickers
// bind, and the localized names of the built-in System actions.

import { i18n, type TranslationKey } from "@/i18n"
import type { ActionType, MachineResourceBinding, Os, ResourceKind, ResourceRef } from "@/lib/types"

export const WEBSITE = "system.open-url"
export const OPEN = "system.open-file"
export const OPEN_APPLICATION = "system.open-app"
export const CLOSE_APPLICATION = "system.close-app"
/**
 * Open folder: replaced by Open (which opens folders too) and deprecated,
 * so it is not offered for new keys; saved ones keep running, keep their
 * folder icon and stay editable (docs/system-actions.md).
 */
export const OPEN_FOLDER = "system.open-directory"

/** The longest address that opens (`paths::MAX_URL_LEN`). */
export const MAX_URL_LENGTH = 2048

export type WebsiteProblem =
  | "empty"
  | "tooLong"
  | "control"
  | "backslash"
  | "scheme"
  | "invalid"
  | "credentials"
  | "noHost"
  | "mailBrowser"

/** The message of each problem (`actions:system.website.problems.*`). */
export const WEBSITE_PROBLEMS: Record<WebsiteProblem, TranslationKey> = {
  empty: "actions:system.website.problems.empty",
  tooLong: "actions:system.website.problems.tooLong",
  control: "actions:system.website.problems.control",
  backslash: "actions:system.website.problems.backslash",
  scheme: "actions:system.website.problems.scheme",
  invalid: "actions:system.website.problems.invalid",
  credentials: "actions:system.website.problems.credentials",
  noHost: "actions:system.website.problems.noHost",
  mailBrowser: "actions:system.website.problems.mailBrowser",
}

// eslint-disable-next-line no-control-regex -- control characters are what it finds.
const CONTROL = /[\u0000-\u001f\u007f-\u009f]/

/** A scheme is written: `name:` not followed by a digit (`localhost:3000` is a port). */
function hasScheme(lower: string): boolean {
  return /^[a-z][a-z0-9+.-]*:(?!\d)/.test(lower)
}

function isMail(text: string): boolean {
  return text.trim().toLowerCase().startsWith("mailto:")
}

/**
 * The address a Website action opens, normalised as the agent does
 * (`https://` added when no scheme is written, the URL parser's form), or
 * why it is refused. A saved `mailto:` address passes unchanged, but not
 * with a chosen browser. Text with `{{ … }}` references is checked when it
 * runs.
 */
export function checkWebsite(
  text: string,
  withBrowser = false,
): { url: string; problem: null } | { url: null; problem: WebsiteProblem } {
  const refuse = (problem: WebsiteProblem) => ({ url: null, problem }) as const
  const trimmed = text.trim()
  if (trimmed.includes("{{")) return { url: trimmed, problem: null }
  if (trimmed === "") return refuse("empty")
  if (isMail(trimmed)) return withBrowser ? refuse("mailBrowser") : { url: trimmed, problem: null }
  if (new TextEncoder().encode(trimmed).length > MAX_URL_LENGTH) return refuse("tooLong")
  if (CONTROL.test(trimmed)) return refuse("control")
  if (trimmed.includes("\\")) return refuse("backslash")
  const lower = trimmed.toLowerCase()
  let full = trimmed
  if (hasScheme(lower)) {
    if (!lower.startsWith("http://") && !lower.startsWith("https://")) return refuse("scheme")
  } else {
    full = `https://${trimmed}`
  }
  let url: URL
  try {
    url = new URL(full)
  } catch {
    return refuse("invalid")
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") return refuse("scheme")
  if (url.username !== "" || url.password !== "") return refuse("credentials")
  if (url.hostname === "") return refuse("noHost")
  if (url.href.length > MAX_URL_LENGTH) return refuse("tooLong")
  return { url: url.href, problem: null }
}

/** Lower-case ASCII words of `name`, joined by `-` (`Visual Studio Code` → `visual-studio-code`). */
export function slug(name: string): string {
  const plain = name
    .normalize("NFKD")
    .replace(/[̀-ͯ]/g, "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 40)
    .replace(/-+$/g, "")
  return plain === "" ? "item" : plain
}

/**
 * Whether a resource reference is one the pickers made (a key of this
 * computer: `app.…`, `file.…`, `folder.…`, or an import's unbound
 * `import.…`), shown in the simple mode; a project role or any other key
 * opens in advanced mode. No value is simple.
 */
export function isLocalReference(reference: ResourceRef | null): boolean {
  if (reference === null) return true
  if (reference.type === "projectRole") return reference.role.trim() === ""
  const key = reference.key.trim()
  return key === "" || /^(app|file|folder|import)\./.test(key)
}

/**
 * The native dialog's filter for "Choose another…": application bundles on
 * macOS, programs and shortcuts on Windows, desktop entries on Linux (where
 * a program has no extension, so every file is offered too).
 */
export function applicationFilters(
  os: Os,
  name: string,
): { name: string; extensions: string[] }[] | undefined {
  switch (os) {
    case "macOs":
      return [{ name, extensions: ["app"] }]
    case "windows":
      return [{ name, extensions: ["exe", "lnk"] }]
    case "linux":
      return [
        { name, extensions: ["desktop"] },
        { name: "*", extensions: ["*"] },
      ]
    case "other":
      return undefined
  }
}

/**
 * The secondary parameters of a machine-local one (Open's and Open
 * folder's file or subfolder inside a folder): drawn only on request or
 * when set.
 */
export function isSecondaryParameter(actionType: string | undefined, key: string): boolean {
  return (actionType === OPEN || actionType === OPEN_FOLDER) && key === "path"
}

/** A short random suffix for a new key: 6 lower-case letters or digits. */
export function randomSuffix(): string {
  const alphabet = "abcdefghijklmnopqrstuvwxyz0123456789"
  const bytes = new Uint8Array(6)
  crypto.getRandomValues(bytes)
  return Array.from(bytes, (b) => alphabet[b % alphabet.length]).join("")
}

/**
 * The resource key a picked application, file or folder is bound under on
 * this computer: `app.safari-k3x9q2`, `file.report-pdf-…`, `folder.downloads-…`.
 * A binding that already holds the same value is reused. A new key gets a
 * random suffix, so an imported profile cannot guess the keys this
 * computer uses (docs/system-actions.md, import), and never changes the
 * binding of another action.
 */
export function bindingKeyFor(
  prefix: "app" | "file" | "folder",
  name: string,
  kind: ResourceKind,
  value: string,
  bindings: MachineResourceBinding[],
  suffix: () => string = randomSuffix,
): string {
  const same = bindings.find((b) => b.kind === kind && b.value === value)
  if (same) return same.key
  const taken = new Set(bindings.map((b) => b.key))
  for (;;) {
    const candidate = `${prefix}.${slug(name)}-${suffix()}`
    if (!taken.has(candidate)) return candidate
  }
}

/** The last part of a path, for naming its binding. */
export function baseName(path: string): string {
  const parts = path.split(/[\\/]/).filter((p) => p !== "")
  const last = parts.at(-1) ?? path
  return last.replace(/\.(app|exe|lnk|desktop)$/i, "")
}

const NAMES: Partial<Record<string, { name: TranslationKey; description: TranslationKey }>> = {
  [WEBSITE]: {
    name: "actions:system.names.website",
    description: "actions:system.descriptions.website",
  },
  [OPEN]: { name: "actions:system.names.open", description: "actions:system.descriptions.open" },
  [OPEN_APPLICATION]: {
    name: "actions:system.names.openApplication",
    description: "actions:system.descriptions.openApplication",
  },
  [CLOSE_APPLICATION]: {
    name: "actions:system.names.closeApplication",
    description: "actions:system.descriptions.closeApplication",
  },
  [OPEN_FOLDER]: {
    name: "actions:system.names.openFolder",
    description: "actions:system.descriptions.openFolder",
  },
}

/** A built-in action type's name in the window's language; others as they are. */
export function actionTypeName(type: Pick<ActionType, "id" | "name">): string {
  const names = NAMES[type.id]
  return names ? i18n.t(names.name) : type.name
}

/** A built-in action type's description in the window's language; others as they are. */
export function actionTypeDescription(type: Pick<ActionType, "id" | "description">): string {
  const names = NAMES[type.id]
  return names ? i18n.t(names.description) : type.description
}

type Texts = { label: TranslationKey; description: TranslationKey }

const APPLICATION: Texts = {
  label: "actions:system.parameters.application.label",
  description: "actions:system.parameters.application.description",
}

/** The parameters of the four actions, in the window's language. */
const PARAMETERS: Partial<Record<string, Partial<Record<string, Texts>>>> = {
  [WEBSITE]: {
    url: {
      label: "actions:system.parameters.url.label",
      description: "actions:system.parameters.url.description",
    },
    browser: {
      label: "actions:system.parameters.browser.label",
      description: "actions:system.parameters.browser.description",
    },
  },
  [OPEN]: {
    location: {
      label: "actions:system.parameters.location.label",
      description: "actions:system.parameters.location.description",
    },
    path: {
      label: "actions:system.parameters.path.label",
      description: "actions:system.parameters.path.description",
    },
    application: {
      label: "actions:system.parameters.openWith.label",
      description: "actions:system.parameters.openWith.description",
    },
  },
  [OPEN_APPLICATION]: {
    application: APPLICATION,
    arguments: {
      label: "actions:system.parameters.arguments.label",
      description: "actions:system.parameters.arguments.description",
    },
  },
  [CLOSE_APPLICATION]: {
    application: APPLICATION,
    force: {
      label: "actions:system.parameters.force.label",
      description: "actions:system.parameters.force.description",
    },
  },
}

/** A parameter's label and description in the window's language. */
export function parameterTexts(
  actionType: string | undefined,
  spec: { key: string; label: string; description?: string | null },
): { label: string; description: string | undefined } {
  const texts = actionType ? PARAMETERS[actionType]?.[spec.key] : undefined
  if (!texts) return { label: spec.label, description: spec.description ?? undefined }
  return { label: i18n.t(texts.label), description: i18n.t(texts.description) }
}

/**
 * Which machine-local picker a parameter gets: the applications, the
 * browsers, or files and folders. `null` for every other parameter.
 */
export function machinePick(
  actionType: string | undefined,
  key: string,
): "application" | "browser" | "location" | "folder" | null {
  if (actionType === WEBSITE && key === "browser") return "browser"
  if (actionType === OPEN && key === "location") return "location"
  if (actionType === OPEN_FOLDER && key === "location") return "folder"
  if (
    (actionType === OPEN || actionType === OPEN_APPLICATION || actionType === CLOSE_APPLICATION) &&
    key === "application"
  ) {
    return "application"
  }
  return null
}

/** What a System action's execution error code means (`actions:system.errors.*`). */
export const EXECUTION_ERRORS: Partial<Record<string, TranslationKey>> = {
  "invalid-url": "actions:system.errors.invalidUrl",
  "not-found": "actions:system.errors.notFound",
  "resource-unbound": "actions:system.errors.unbound",
  "invalid-application": "actions:system.errors.invalidApplication",
  "not-identifiable": "actions:system.errors.notIdentifiable",
  "protected-process": "actions:system.errors.protected",
  "close-failed": "actions:system.errors.closeFailed",
  "not-user-initiated": "actions:system.errors.notUserInitiated",
  "not-permitted": "actions:system.errors.notPermitted",
  "program-not-found": "actions:system.errors.programNotFound",
  "open-failed": "actions:system.errors.openFailed",
  interrupted: "actions:system.errors.interrupted",
  elevated: "actions:system.errors.elevated",
}

/** An execution error of a System action in words, or `null` for an unknown code. */
export function executionError(code: string | null | undefined): string | null {
  const key = code ? EXECUTION_ERRORS[code] : undefined
  return key ? i18n.t(key) : null
}
