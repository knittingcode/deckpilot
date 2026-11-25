// The repositories of a project as the project form edits them, and the
// checks it makes before saving (the backend checks the same again).

import type { Project } from "@/lib/types"

/** The repository key grammar (`^[a-z][a-z0-9_]{0,31}$`), as the backend checks it. */
export const REPOSITORY_KEY = /^[a-z][a-z0-9_]{0,31}$/
export const MAX_REPOSITORY_NAME = 64
const MAX_DESCRIPTION = 500

/** One repository as the form edits it. */
export interface RepositoryRow {
  /** The stored id, or a new one made here for a repository not saved yet. */
  id: string
  key: string
  name: string
  description: string
  /** The resource key holding its folder; `null` until a new one is saved. */
  resource: string | null
  /** A folder chosen in this form, bound on this computer when it is saved. */
  pendingPath: string | null
}

/** The rows of `project` (with the repository its role stands for, as shown). */
export function repositoryRows(project: Project | null): RepositoryRow[] {
  return (project?.repositories ?? []).map((r) => ({
    id: r.id,
    key: r.key,
    name: r.name,
    description: r.description ?? "",
    resource: r.resource,
    pendingPath: null,
  }))
}

/** A key from a folder name: lowercase ASCII, `_` between words, unique among `taken`. */
export function proposeKey(folder: string, taken: string[]): string {
  let base = folder
    .normalize("NFD")
    .replace(/[\u0300-\u036f]/g, "")
    // Letters NFD does not take apart.
    .replace(/[łŁ]/g, "l")
    .replace(/[đĐ]/g, "d")
    .replace(/[øØ]/g, "o")
    .replace(/ß/g, "ss")
    .replace(/[æÆ]/g, "ae")
    .replace(/[œŒ]/g, "oe")
    .toLowerCase()
    .replace(/[^a-z0-9_]+/g, "_")
    .replace(/^_+|_+$/g, "")
  if (!/^[a-z]/.test(base)) base = `repo_${base}`.replace(/_+$/, "")
  base = base.slice(0, 32).replace(/_+$/, "")
  if (!taken.includes(base)) return base
  for (let n = 2; ; n += 1) {
    const suffix = `_${n}`
    const candidate = `${base.slice(0, 32 - suffix.length)}${suffix}`
    if (!taken.includes(candidate)) return candidate
  }
}

/** Why a row cannot be saved as it is, per field; empty when it can. */
export function rowProblems(
  rows: RepositoryRow[],
  index: number,
): { key?: "invalid" | "duplicate"; name?: "tooLong"; description?: "tooLong" } {
  const row = rows[index]
  if (!row) return {}
  const problems: ReturnType<typeof rowProblems> = {}
  const key = row.key.trim()
  if (!REPOSITORY_KEY.test(key)) problems.key = "invalid"
  else if (rows.some((other, i) => i !== index && other.key.trim() === key)) {
    problems.key = "duplicate"
  }
  if (Array.from(row.name.trim()).length > MAX_REPOSITORY_NAME) problems.name = "tooLong"
  if (Array.from(row.description.trim()).length > MAX_DESCRIPTION) problems.description = "tooLong"
  return problems
}

/** Whether every row can be saved. */
export function rowsValid(rows: RepositoryRow[]): boolean {
  return rows.every((_, index) => Object.keys(rowProblems(rows, index)).length === 0)
}
