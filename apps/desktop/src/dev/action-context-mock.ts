/**
 * Development only: the contexts the action-context audit proposes
 * (inventory.md, "Proposed"), for reviewing the editors' filters before an
 * agent sends real declarations. This branch's agent sends none, which
 * means "everywhere". Off unless switched on in Settings › Dialog showcase;
 * referenced only behind `import.meta.env.DEV`, so builds contain neither
 * this table nor its use. Production never derives contexts from ids:
 * they come from the agent.
 */

import { useSyncExternalStore } from "react"

import type { ActionContext } from "@/lib/action-context"
import type { ModuleInfo } from "@/lib/types"

const K: ActionContext = "key"
const S: ActionContext = "sequence"
const W: ActionContext = "workflow"
const D: ActionContext = "display"
const G: ActionContext = "gesture"

/** inventory.md's "Proposed" column: built-ins, official and first-party plugins. */
export const PROPOSED_CONTEXTS: Readonly<Record<string, ActionContext[]>> = {
  "general.log": [S, W],
  "general.wait": [S, W],
  "general.delay": [S, W],
  "deck.next-page": [K, G],
  "deck.previous-page": [K, G],
  "deck.page-indicator": [D],
  "deck.switch-profile": [K, G],
  "deck.brightness": [K, G],
  "deck.random": [K, G],
  "deck.multi-switch": [K, D, G],
  "deck.switch-layout": [K, G],
  "deck.previous-layout": [K, G],
  "system.open-app": [K, S, W, G],
  "system.open-url": [K, S, W, G],
  "system.open-file": [K, S, W, G],
  "system.open-directory": [K, S, W, G],
  "system.run-command": [K, S, W, G],
  "system.process-status": [S, W],
  "system.cpu": [D, W],
  "system.memory": [D, W],
  "system.network": [D, W],
  "system.disk": [D, W],
  "http.request": [K, S, W, G],
  "http.health-check": [S, W],
  "notifications.show": [K, S, W, G],
  "notifications.workflow-summary": [W],
  "deckpilot.docker.status": [K, D, W, G],
  "deckpilot.docker.containers": [K, D, W, G],
  "deckpilot.docker.compose-status": [K, D, W, G],
  "deckpilot.docker.compose-up": [K, S, W, D, G],
  "deckpilot.docker.compose-down": [K, S, W, D, G],
  "deckpilot.docker.compose-restart": [K, S, W, D, G],
  "deckpilot.docker.logs": [W],
  "deckpilot.git.status": [K, D, W, G],
  "deckpilot.git.branch": [K, D, W, G],
  "deckpilot.git.changes": [W],
  "deckpilot.git.last-commit": [W],
  "deckpilot.git.remote-status": [W],
  "deckpilot.git.open-repository": [K, S, W, G],
  "deckpilot.github.current-user": [W],
  "deckpilot.github.repository": [W],
  "deckpilot.github.pull-requests": [W],
  "deckpilot.github.review-requests": [W],
  "deckpilot.github.checks": [W],
  "deckpilot.github.open": [K, S, W, G],
  "deckpilot.gitlab.current-user": [W],
  "deckpilot.gitlab.project": [W],
  "deckpilot.gitlab.merge-requests": [W],
  "deckpilot.gitlab.review-requests": [W],
  "deckpilot.gitlab.pipeline": [W],
  "deckpilot.gitlab.open": [K, S, W, G],
  "deckpilot.jira.myself": [W],
  "deckpilot.jira.issue": [W],
  "deckpilot.jira.issues": [W],
  "deckpilot.jira.open": [K, S, W, G],
  "deckpilot.notifications.show": [K, S, W, G],
  "deckpilot.notifications.workflow-summary": [W],
  "deckpilot.projects.cycle": [K, S, W, G],
  "deckpilot.projects.select": [K, S, W, G],
  "deckpilot.projects.current": [W],
  "deckpilot.projects.repository": [W],
  "deckpilot.projects.documentation": [W],
  "deckpilot.projects.resolve-resource": [W],
  "deckpilot.projects.next": [K, S, W, G],
  "deckpilot.projects.previous": [K, S, W, G],
  "deckpilot.projects.active": [W],
  "deckpilot.workspace.open-in-editor": [K, S, W, G],
  "deckpilot.workspace.open-urls": [K, S, W, G],
  "deckpilot.workspace.open-project": [K, S, W, G],
  "deckpilot.air-quality.current": [K, D, W, G],
  "deckpilot.air-quality.find-station": [W],
  "deckpilot.air-quality.select-station": [K, S, W],
  "deckpilot.air-quality.forget-station": [K, S, W],
  "deckpilot.break-reminder.tick": [W],
  "deckpilot.break-reminder.snooze": [K, S, W, G],
  "deckpilot.break-reminder.skip": [K, S, W, G],
  "deckpilot.break-reminder.restart": [K, S, W, G],
  "deckpilot.break-reminder.pause": [K, S, W, G],
  "deckpilot.break-reminder.resume": [K, S, W, G],
  "deckpilot.break-reminder.toggle-pause": [K, S, W, G],
  "deckpilot.break-reminder.status": [K, D, W, G],
  "deckpilot.markets.rate": [K, D, W, G],
  "deckpilot.markets.change": [W],
  "deckpilot.markets.check-alert": [K, S, W],
  "deckpilot.markets.forget": [K, S, W],
  "deckpilot.memory.new-game": [K, G],
  "deckpilot.memory.flip": [K, G],
  "deckpilot.memory.card": [D],
  "deckpilot.memory.score": [D],
  "deckpilot.pomodoro.tick": [W],
  "deckpilot.pomodoro.start": [K, S, W, G],
  "deckpilot.pomodoro.pause": [K, S, W, G],
  "deckpilot.pomodoro.resume": [K, S, W, G],
  "deckpilot.pomodoro.toggle": [K, S, W, G],
  "deckpilot.pomodoro.skip": [K, S, W, G],
  "deckpilot.pomodoro.reset": [K, S, W, G],
  "deckpilot.pomodoro.status": [K, D, W, G],
  "deckpilot.template.count": [K, S, W],
  "deckpilot.template.reset": [K, S, W, G],
  "deckpilot.weather.current": [K, D, W, G],
  "deckpilot.weather.forecast": [W],
  "deckpilot.weather.find-location": [S, W],
  "deckpilot.weather.forget-location": [K, S, W],
}

/**
 * The modules with the proposed contexts on their types, as an agent that
 * declares them would send them. Types the agent already declares keep
 * their own; types not in the table keep none (everywhere).
 */
export function withProposedContexts(modules: ModuleInfo[]): ModuleInfo[] {
  return modules.map((module) => ({
    ...module,
    actionTypes: module.actionTypes.map((type) => {
      // A legacy id inherits its plugin target's contexts (design §2.4),
      // through the alias data, never through its name.
      const target = module.legacy?.[type.id]?.target
      const proposed =
        PROPOSED_CONTEXTS[type.id] ?? (target ? PROPOSED_CONTEXTS[target] : undefined)
      return type.contexts || !proposed ? type : { ...type, contexts: proposed }
    }),
  }))
}

// ---- The switch (per computer, development only)

const KEY = "deckpilot.dev.mockActionContexts"
const listeners = new Set<() => void>()

function read(): boolean {
  try {
    return localStorage.getItem(KEY) === "1"
  } catch {
    return false
  }
}

export function setMockContexts(on: boolean) {
  try {
    if (on) localStorage.setItem(KEY, "1")
    else localStorage.removeItem(KEY)
  } catch {
    // This session only.
  }
  for (const listener of listeners) listener()
}

/** Whether the editors see the proposed contexts. */
export function useMockContexts(): boolean {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener)
      return () => listeners.delete(listener)
    },
    read,
    () => false,
  )
}
