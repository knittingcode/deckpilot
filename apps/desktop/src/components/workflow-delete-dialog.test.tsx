// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { Button } from "@/components/ui/button"
import type { ProfileView, Trigger, Usage } from "@/lib/types"

import { WorkflowDeleteDialog } from "./workflow-delete-dialog"

const api = { deleteWorkflow: vi.fn() }

vi.mock("@/lib/api", () => ({
  api: new Proxy({}, { get: (_, name: string) => api[name as keyof typeof api] }),
  errorMessage: (error: unknown) => String(error),
  errorCode: (error: unknown) => (error as { code?: string }).code,
}))

const key = (index: number, trigger?: Trigger): Usage => ({
  type: "layoutBinding",
  layout: "l-main",
  layoutName: "Main",
  page: "p-home",
  pageName: "Home",
  control: { kind: "key", index },
  ...(trigger ? { trigger } : {}),
})

const references: Usage[] = [
  key(0, { type: "press" }),
  key(1, { type: "longPress" }),
  { type: "workflowNode", workflow: "w-release", workflowName: "Release", node: "call" },
  { type: "actionParameter", action: "a-random", actionName: "Pick one" },
]

function view(usages: Usage[] = references, fingerprint = "f-1"): ProfileView {
  return {
    profile: {
      id: "profile-1",
      name: "Work",
      description: "",
      actions: [],
      workflows: [{ id: "w-deploy", name: "Deploy", steps: [] }],
      projects: [],
      layouts: [],
    },
    activeProjectId: null,
    issues: [],
    actionUsages: {},
    workflowUsages: { "w-deploy": usages.filter((u) => u.type === "layoutBinding") },
    workflowReferences: { "w-deploy": usages },
    workflowEntries: [
      {
        id: "w-deploy",
        name: "Deploy",
        kind: "graph",
        hasTriggers: false,
        block: null,
        references: usages.length,
        fingerprint,
      },
    ],
  }
}

function dialog(profile: ProfileView, onDeleted = vi.fn()) {
  render(
    <WorkflowDeleteDialog
      trigger={<Button>Delete</Button>}
      profileId="profile-1"
      view={profile}
      workflow={{ id: "w-deploy", name: "Deploy" }}
      title="Delete “Deploy”?"
      description="The workflow is removed."
      onDeleted={onDeleted}
    />,
  )
  return onDeleted
}

beforeEach(() => {
  api.deleteWorkflow.mockReset()
})
afterEach(cleanup)

describe("WorkflowDeleteDialog", () => {
  it("lists every reference, grouped, and keeps the keys unless asked", async () => {
    const user = userEvent.setup()
    api.deleteWorkflow.mockResolvedValue({ cleared: 0, agentReload: "applied" })
    const onDeleted = dialog(view())
    // Delete is never disabled: it opens the confirmation.
    await user.click(screen.getByRole("button", { name: "Delete" }))
    const shown = screen.getByRole("alertdialog")
    expect(within(shown).getByText(/4 places use this workflow/)).toBeInTheDocument()
    const list = within(shown).getByTestId("workflow-references")
    expect(within(list).getByText("Keys and gestures (layout › page › control)")).toBeVisible()
    expect(within(list).getByText(/Layout “Main”, page “Home”, Key 1$/)).toBeVisible()
    expect(within(list).getByText(/Key 2 \(Long press\)/)).toBeVisible()
    expect(within(list).getByText(/Workflow “Release”, a Sub-workflow or Loop node/)).toBeVisible()
    expect(within(list).getByText(/Action “Pick one”/)).toBeVisible()
    expect(within(list).getByText(/stay and show “Workflow missing”/)).toBeVisible()
    await user.click(within(shown).getByRole("button", { name: "Delete" }))
    await waitFor(() => {
      expect(onDeleted).toHaveBeenCalledWith(0)
    })
    expect(api.deleteWorkflow).toHaveBeenCalledWith("profile-1", "w-deploy", {
      fingerprint: "f-1",
      clearBindings: false,
      clearWorkSession: false,
    })
  })

  it("clears the keys when the user opts in", async () => {
    const user = userEvent.setup()
    api.deleteWorkflow.mockResolvedValue({ cleared: 2, agentReload: "applied" })
    const onDeleted = dialog(view())
    await user.click(screen.getByRole("button", { name: "Delete" }))
    const shown = screen.getByRole("alertdialog")
    await user.click(within(shown).getByLabelText("Also clear these 2 keys"))
    await user.click(within(shown).getByRole("button", { name: "Delete" }))
    await waitFor(() => {
      expect(onDeleted).toHaveBeenCalledWith(2)
    })
    expect(api.deleteWorkflow).toHaveBeenCalledWith("profile-1", "w-deploy", {
      fingerprint: "f-1",
      clearBindings: true,
      clearWorkSession: false,
    })
  })

  it("shows the new references when they changed before the confirmation", async () => {
    const user = userEvent.setup()
    const newer = [...references, key(5)]
    api.deleteWorkflow
      .mockRejectedValueOnce({
        code: "invalid.workflowReferencesChanged",
        message: "changed",
        params: { count: newer.length, usages: newer, fingerprint: "f-2" },
      })
      .mockResolvedValueOnce({ cleared: 0, agentReload: "applied" })
    const onDeleted = dialog(view())
    await user.click(screen.getByRole("button", { name: "Delete" }))
    const shown = screen.getByRole("alertdialog")
    await user.click(within(shown).getByRole("button", { name: "Delete" }))
    // Still open, with the newer list and the reason.
    expect(await within(shown).findByText(/changed meanwhile/)).toBeVisible()
    expect(within(shown).getByText(/5 places use this workflow/)).toBeVisible()
    expect(within(shown).getByText(/Key 6$/)).toBeVisible()
    expect(onDeleted).not.toHaveBeenCalled()
    // Confirming again sends the new fingerprint.
    await user.click(within(shown).getByRole("button", { name: "Delete" }))
    await waitFor(() => {
      expect(onDeleted).toHaveBeenCalled()
    })
    expect(api.deleteWorkflow).toHaveBeenLastCalledWith("profile-1", "w-deploy", {
      fingerprint: "f-2",
      clearBindings: false,
      clearWorkSession: false,
    })
  })

  it("asks plainly for a workflow nothing uses", async () => {
    const user = userEvent.setup()
    api.deleteWorkflow.mockResolvedValue({ cleared: 0, agentReload: "applied" })
    const onDeleted = dialog(view([], "f-0"))
    await user.click(screen.getByRole("button", { name: "Delete" }))
    const shown = screen.getByRole("alertdialog")
    expect(within(shown).getByText("The workflow is removed.")).toBeVisible()
    expect(within(shown).queryByTestId("workflow-references")).toBeNull()
    await user.click(within(shown).getByRole("button", { name: "Delete" }))
    await waitFor(() => {
      expect(onDeleted).toHaveBeenCalledWith(0)
    })
    expect(api.deleteWorkflow).toHaveBeenCalledWith("profile-1", "w-deploy", {
      fingerprint: "f-0",
      clearBindings: false,
      clearWorkSession: false,
    })
  })

  it("deletes a Work Session workflow only after its setting is explicitly cleared", async () => {
    const user = userEvent.setup()
    api.deleteWorkflow.mockResolvedValue({
      cleared: 0,
      clearedWorkSession: 1,
      agentReload: "applied",
    })
    const usages: Usage[] = [key(0), { type: "workSessionSetting", setting: "startWorkflow" }]
    const onDeleted = dialog(view(usages, "f-ws"))
    await user.click(screen.getByRole("button", { name: "Delete" }))
    const shown = screen.getByRole("alertdialog")
    expect(within(shown).getByText("Work Session: start workflow")).toBeVisible()
    // Not ticked: refused here, nothing sent.
    await user.click(within(shown).getByRole("button", { name: "Delete" }))
    expect(await within(shown).findByText(/Tick “Also clear”/)).toBeVisible()
    expect(api.deleteWorkflow).not.toHaveBeenCalled()
    // Ticked: cleared with the delete, the keys kept.
    await user.click(within(shown).getByLabelText("Also clear this Work Session setting"))
    await user.click(within(shown).getByRole("button", { name: "Delete" }))
    await waitFor(() => {
      expect(onDeleted).toHaveBeenCalled()
    })
    expect(api.deleteWorkflow).toHaveBeenCalledWith("profile-1", "w-deploy", {
      fingerprint: "f-ws",
      clearBindings: false,
      clearWorkSession: true,
    })
  })
})
