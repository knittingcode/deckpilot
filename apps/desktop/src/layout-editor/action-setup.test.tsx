// @vitest-environment jsdom
// The required settings of a draft action in the Inspector follow the stored
// action: a change saved elsewhere meanwhile (the shared editor, a rename)
// is neither shown stale nor written back.
import "@testing-library/jest-dom/vitest"

import { act, cleanup, render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import type { ActionInstance } from "@/lib/types"
import { api, resetApi } from "@/test/api-mock"
import { installDomStubs } from "@/test/dom"
import { ACTION_TYPES, action } from "@/test/fixtures"

import { ActionSetup } from "./action-setup"

vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)

const notify = ACTION_TYPES.notify
const draft = action("draft", "Notify", notify.id)

beforeAll(installDomStubs)
beforeEach(() => {
  resetApi()
  api.updateAction.mockImplementation((_profile: string, saved: ActionInstance) =>
    Promise.resolve(saved),
  )
})
afterEach(cleanup)

function setup(current: ActionInstance) {
  return (
    <ActionSetup
      profileId="p"
      action={current}
      type={notify}
      projects={[]}
      autoFocus={false}
      onSaved={vi.fn()}
      onFocused={vi.fn()}
    />
  )
}

describe("a draft's required settings", () => {
  it("save what is typed on top of the action as it is now, never an older copy", async () => {
    const user = userEvent.setup()
    const { rerender } = render(setup(draft))
    await user.type(screen.getByRole("textbox", { name: /Title/ }), "Hi")
    // Before the delayed save: renamed and given a body elsewhere.
    rerender(
      setup({
        ...draft,
        name: "Renamed",
        parameters: { body: { type: "text", value: "from elsewhere" } },
      }),
    )
    await waitFor(() => {
      expect(api.updateAction).toHaveBeenCalledTimes(1)
    })
    expect(api.updateAction).toHaveBeenCalledWith("p", {
      ...draft,
      name: "Renamed",
      parameters: {
        title: { type: "text", value: "Hi" },
        body: { type: "text", value: "from elsewhere" },
      },
    })
  })

  it("show a value saved elsewhere while nothing is being typed, and save nothing", async () => {
    const { rerender } = render(setup(draft))
    expect(screen.getByRole("textbox", { name: /Title/ })).toHaveValue("")
    rerender(setup({ ...draft, parameters: { title: { type: "text", value: "Set elsewhere" } } }))
    await waitFor(() => {
      expect(screen.getByRole("textbox", { name: /Title/ })).toHaveValue("Set elsewhere")
    })
    await act(() => new Promise((resolve) => setTimeout(resolve, 600)))
    expect(api.updateAction).not.toHaveBeenCalled()
  })
})
