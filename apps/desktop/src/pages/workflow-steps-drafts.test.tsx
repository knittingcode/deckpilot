// @vitest-environment jsdom
// A Multi Action step whose action is a draft (required settings not
// filled in, `lib/action-completeness.ts`): kept, marked "Needs
// configuration" with what it lacks, until the action or the step's own
// override fills them in.
import "@testing-library/jest-dom/vitest"

import { cleanup, screen } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { api, resetApi } from "@/test/api-mock"
import { installDomStubs } from "@/test/dom"
import { ACTION_TYPES, action, builtInModules, profileView, stepWorkflow } from "@/test/fixtures"
import { renderWithApp } from "@/test/render"

import { WorkflowsPage } from "./workflows"

vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)
vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)

beforeAll(installDomStubs)
beforeEach(() => {
  resetApi()
  api.listActionTypes.mockResolvedValue(Object.values(ACTION_TYPES))
})
afterEach(cleanup)

const draft = action("draft", "Open docs", ACTION_TYPES.openUrl.id)
const done = action("done", "Open shop", ACTION_TYPES.openUrl.id, {
  parameters: { url: { type: "text", value: "https://shop.example.com" } },
})

function renderSteps(steps: ReturnType<typeof stepWorkflow>) {
  api.getProfile.mockResolvedValue(profileView({ actions: [draft, done], workflows: [steps] }))
  return renderWithApp(<WorkflowsPage />, { modules: builtInModules() })
}

describe("a draft step", () => {
  it("is marked with what it lacks; a complete one is not", async () => {
    renderSteps(stepWorkflow("w-1", "Morning", [draft.id, done.id]))
    await screen.findByRole("combobox", { name: "Action of step 1" })
    const marks = await screen.findAllByText("Needs configuration")
    expect(marks).toHaveLength(1)
    expect(screen.getByText("Missing: URL")).toBeInTheDocument()
  })

  it("is complete when the step's own override fills it in", async () => {
    const steps = stepWorkflow("w-1", "Morning", [draft.id])
    const [first] = steps.steps
    if (first) first.overrides = { url: { type: "text", value: "https://docs.example.com" } }
    renderSteps(steps)
    await screen.findByRole("combobox", { name: "Action of step 1" })
    expect(screen.queryByText("Needs configuration")).not.toBeInTheDocument()
  })
})
