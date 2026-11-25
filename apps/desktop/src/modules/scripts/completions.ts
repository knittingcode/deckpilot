// The script API a completion offers (members of `deckpilot`). It mirrors
// the Scripts plugin's `official/scripts/deckpilot.d.ts` in the plugins
// repository; a member listed here that a script cannot use only misses a
// completion, it grants nothing.

export const DECKPILOT_MEMBERS: Readonly<Record<string, readonly string[]>> = {
  "": [
    "platform",
    "run",
    "sleep",
    "log",
    "projects",
    "repositories",
    "path",
    "fs",
    "process",
    "git",
  ],
  log: ["debug", "info", "warn", "error"],
  projects: ["current", "list"],
  repositories: ["current", "default", "get"],
  path: [
    "sep",
    "join",
    "normalize",
    "resolve",
    "dirname",
    "basename",
    "extname",
    "relative",
    "isAbsolute",
    "toPosix",
    "fromPosix",
  ],
  fs: [
    "exists",
    "stat",
    "list",
    "mkdir",
    "readFile",
    "writeFile",
    "copy",
    "move",
    "remove",
    "removeContents",
  ],
  process: ["run", "available"],
  git: ["available", "run"],
  run: ["id", "origin", "userInitiated", "inputs", "signal"],
}

/**
 * Characters that make code read other than it runs (bidirectional
 * controls, zero-width and other invisible characters): the editor draws
 * each one as a visible mark, so a review sees them ("Trojan Source").
 */
export const HIDDEN_CHARS =
  // The class lists single code points on purpose (variation selectors
  // included), never a combined character.
  // eslint-disable-next-line no-misleading-character-class
  /[\u00AD\u061C\u180E\u200B-\u200F\u2028-\u202E\u2060-\u206F\uFE00-\uFE0F\uFEFF\uFFF9-\uFFFB]/

/** Whether `source` holds any of [`HIDDEN_CHARS`]. */
export function hasHiddenChars(source: string): boolean {
  return HIDDEN_CHARS.test(source)
}
