// The references of a workflow as the app lists them
// (docs/data-model.md, "Workflows").

import type { ProfileView, Usage } from "./types"

/** The references of `workflowId` as the profile view has them. */
export function workflowReferences(view: ProfileView, workflowId: string): Usage[] {
  return view.workflowReferences?.[workflowId] ?? view.workflowUsages[workflowId] ?? []
}

/** References grouped as the dialog lists them. */
export function groupReferences(usages: Usage[]) {
  return {
    keys: usages.filter((u) => u.type === "layoutBinding"),
    actions: usages.filter((u) => u.type === "actionParameter"),
    workflows: usages.filter((u) => u.type === "workflowNode" || u.type === "workflowStep"),
    workSession: usages.filter((u) => u.type === "workSessionSetting"),
  }
}
