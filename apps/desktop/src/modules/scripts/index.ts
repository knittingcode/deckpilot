import { lazy } from "react"

/**
 * The Scripts screen, a host view the `deckpilot.scripts` plugin activates
 * (plugin API 1.13, `hostViews`; `modules/host-views.ts`). Its editor
 * (CodeMirror) loads with the page only.
 */
export const SCRIPTS_PAGE = "scripts/scripts"

export const ScriptsPageView = lazy(() =>
  import("./scripts-page").then((m) => ({ default: m.ScriptsPage })),
)
