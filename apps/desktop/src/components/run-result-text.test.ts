import { describe, expect, it } from "vitest"

import { describeFailure } from "@/workflow-builder/run-debug"

import { isIncompleteRefusal, runResultText } from "./run-result-text"

const type = {
  parameters: [
    {
      key: "application",
      label: "Application",
      kind: { type: "resource" as const },
      required: true,
    },
  ],
}

describe("the words of a refused run", () => {
  it("names the action and its missing settings by their labels", () => {
    const refused = {
      code: "configuration-incomplete",
      message: "parameter `application` is required",
      data: { missing: "application" },
    }
    expect(isIncompleteRefusal(refused)).toBe(true)
    expect(runResultText(refused, "Open application", type)).toBe(
      "Open application needs configuration before it can run. Missing: Application",
    )
    // Without the type: the key itself.
    expect(runResultText(refused, "Open application")).toContain("Missing: application")
  })

  it("leaves every other result's message as it is", () => {
    const done = { message: "Opened Safari", data: { application: "Safari" } }
    expect(isIncompleteRefusal(done)).toBe(false)
    expect(runResultText(done, "Open application", type)).toBe("Opened Safari")
  })

  it("explains a workflow node refused for missing configuration", () => {
    const { explanation } = describeFailure({
      code: "action.failed",
      actionCode: "configuration-incomplete",
      message: "parameter `url` is required",
      retryable: false,
    })
    expect(explanation).toBe("the action needs configuration")
  })
})
