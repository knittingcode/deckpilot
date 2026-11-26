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
 * controls, zero-width and other invisible characters, tag characters and
 * variation selectors that can hide a payload, Hangul fillers that make
 * invisible identifiers, and control characters other than tab and line
 * breaks): the editor draws each one as a visible mark, so a review sees
 * them ("Trojan Source"). The same set as the core's name check
 * (`deckpilot_core::script::is_hidden`), and the controls.
 */
export const HIDDEN_CHARS =
  // The class lists single code points on purpose (variation selectors
  // included), never a combined character.
  // eslint-disable-next-line no-misleading-character-class, no-control-regex
  /[\u{0}-\u{8}\u{B}\u{C}\u{E}-\u{1F}\u{7F}-\u{9F}\u{AD}\u{34F}\u{61C}\u{115F}\u{1160}\u{180E}\u{200B}-\u{200F}\u{2028}-\u{202E}\u{2060}-\u{206F}\u{3164}\u{FE00}-\u{FE0F}\u{FEFF}\u{FFA0}\u{FFF9}-\u{FFFB}\u{E0000}-\u{E007F}\u{E0100}-\u{E01EF}]/u

/** Whether `source` holds any of [`HIDDEN_CHARS`]. */
export function hasHiddenChars(source: string): boolean {
  return HIDDEN_CHARS.test(source)
}
