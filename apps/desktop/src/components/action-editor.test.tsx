// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { act, cleanup, render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { DialogProvider } from "@/components/dialogs"

import { ActionEditor } from "./action-editor"
import type { ActionInstance, ActionType, ModuleInfo, Project } from "@/lib/types"
import { i18n } from "@/i18n"
import { pluginModules } from "@/lib/plugin-actions"
import { api, resetApi } from "@/test/api-mock"
import { installDomStubs } from "@/test/dom"
import { projectsPlugin } from "@/test/first-party"

vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)
vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)

beforeAll(installDomStubs)

const { createAction, updateAction } = api

const logType: ActionType = {
  id: "general.log",
  name: "Log message",
  description: "Writes a message to the execution log.",
  category: "General",
  requiresProject: false,
  parameters: [
    {
      key: "message",
      label: "Message",
      kind: { type: "text", multiline: true, maxLength: 1000 },
      required: true,
    },
  ],
}

const selectType: ActionType = {
  id: "projects.select",
  name: "Select project",
  description: "Makes a project of the profile active.",
  category: "Projects",
  requiresProject: false,
  parameters: [{ key: "project_id", label: "Project", kind: { type: "project" }, required: false }],
}

const repositoryType: ActionType = {
  ...selectType,
  id: "projects.repository",
  name: "Project repository",
  description: "Resolves the repository of a project on this computer.",
}

function module(id: string, name: string, actionTypes: ActionType[], enabled = true): ModuleInfo {
  return {
    id,
    name,
    version: "0.1.0",
    description: `${name} module`,
    platforms: [],
    pages: [],
    capabilities: [],
    config: [],
    dependencies: [],
    enabledByDefault: true,
    enabled,
    supported: true,
    configValues: {},
    actionTypes,
  }
}

const projects: Project[] = [
  { id: "p-shop", key: "SHOP", name: "Shop", resources: {} },
  { id: "p-cmr", key: "CMR", name: "Customer Relations", resources: {} },
]

function renderEditor(
  action: ActionInstance | null,
  usages = 0,
  modules: ModuleInfo[] = [module("general", "General", [logType])],
) {
  const onSaved = vi.fn()
  render(
    <ActionEditor
      profileId="profile"
      modules={modules}
      projects={projects}
      action={action}
      references={Array.from({ length: usages }, (_, index) => ({
        site: { type: "step" as const, workflow: "w", index },
        ownerName: "Start Work",
      }))}
      canRun={false}
      onSaved={onSaved}
      onDeleted={vi.fn()}
    />,
  )
  return { onSaved }
}

describe("ActionEditor", () => {
  it("mentions project placeholders only with the Projects plugin", () => {
    vi.stubEnv("VITE_DECKPILOT_PLUGINS", "1")
    try {
      const hint = /to show the active project on the key/
      // Projects not installed (and no built-in module provides them).
      renderEditor(null, 0, [
        module("general", "General", [logType]),
        module("projects", "Projects", []),
      ])
      expect(screen.queryByText(hint)).not.toBeInTheDocument()
      expect(screen.getByLabelText("Name")).not.toHaveAttribute(
        "placeholder",
        expect.stringMatching(/project/i),
      )
      cleanup()
      // Installed but disabled.
      renderEditor(null, 0, [
        module("general", "General", [logType]),
        ...pluginModules([projectsPlugin("disabled")]),
      ])
      expect(screen.queryByText(hint)).not.toBeInTheDocument()
      cleanup()
      renderEditor(null, 0, [
        module("general", "General", [logType]),
        ...pluginModules([projectsPlugin()]),
      ])
      expect(screen.getByText(hint)).toBeInTheDocument()
    } finally {
      vi.unstubAllEnvs()
    }
  })

  beforeEach(() => {
    resetApi()
  })
  afterEach(cleanup)

  it("needs a name, and saves a draft without its required settings", async () => {
    const user = userEvent.setup()
    renderEditor(null)
    // A required setting not filled in: marked, said next to it.
    expect(screen.getByText("Needs configuration")).toBeInTheDocument()
    const message = screen.getByRole("textbox", { name: /Message/ })
    expect(message).toHaveAttribute("aria-invalid", "true")
    expect(message).toHaveAccessibleDescription("Required: fill this in before the action can run.")
    await user.click(screen.getByRole("button", { name: /create action/i }))
    expect(screen.getByText("Enter a name.")).toBeInTheDocument()
    expect(createAction).not.toHaveBeenCalled()
    // With a name: saved as a draft, the setting left out (never blank).
    await user.type(screen.getByLabelText("Name"), "Say hello")
    await user.click(screen.getByRole("button", { name: /create action/i }))
    expect(createAction).toHaveBeenCalledExactlyOnceWith(
      "profile",
      expect.objectContaining({ name: "Say hello", actionType: logType.id, parameters: {} }),
    )
  })

  it("creates an action from the schema-generated form", async () => {
    const user = userEvent.setup()
    const created = { id: "a1", name: "Greet", actionType: logType.id, parameters: {} }
    createAction.mockResolvedValue(created)
    const { onSaved } = renderEditor(null)

    await user.type(screen.getByLabelText("Name"), "Greet")
    await user.type(screen.getByLabelText("Key label"), "Hi {{project.key}")
    await user.type(screen.getByLabelText(/^Message/), "Hello from {{profile}")
    await user.click(screen.getByRole("button", { name: /create action/i }))

    expect(createAction).toHaveBeenCalledWith("profile", {
      name: "Greet",
      description: "",
      icon: null,
      label: "Hi {project.key}",
      actionType: logType.id,
      parameters: { message: { type: "text", value: "Hello from {profile}" } },
    })
    expect(onSaved).toHaveBeenCalledWith(created)
  })

  it("keeps the identifier when editing and asks about references before deleting", async () => {
    const user = userEvent.setup()
    const action: ActionInstance = {
      id: "a1",
      name: "Greet",
      actionType: logType.id,
      parameters: { message: { type: "text", value: "hi" } },
    }
    updateAction.mockResolvedValue(action)
    renderEditor(action, 2)

    expect(screen.getByText("Workflow “Start Work”, step 2")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: /delete/i }))
    const dialog = screen.getByRole("dialog", { name: "Delete “Greet”?" })
    // Nothing is deleted while a reference has no decision.
    expect(within(dialog).getByRole("button", { name: "Apply and delete" })).toBeDisabled()
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }))
    expect(api.deleteAction).not.toHaveBeenCalled()
    expect(api.replaceActionReferences).not.toHaveBeenCalled()
    expect(api.unbindActionReferences).not.toHaveBeenCalled()

    const name = screen.getByLabelText("Name")
    await user.clear(name)
    await user.type(name, "Greet everyone")
    await user.click(screen.getByRole("button", { name: /^save$/i }))
    expect(updateAction).toHaveBeenCalledWith(
      "profile",
      expect.objectContaining({ id: "a1", name: "Greet everyone" }),
    )
  })

  it("creates an action by choosing a module, a type and a project", async () => {
    const user = userEvent.setup()
    createAction.mockResolvedValue({
      id: "a2",
      name: "CMR",
      actionType: selectType.id,
      parameters: {},
    })
    renderEditor(null, 0, [
      module("general", "General", [logType]),
      module("projects", "Projects", [selectType]),
    ])
    expect(screen.getByLabelText("Action type")).toBeDisabled()
    await user.click(screen.getByRole("button", { name: /create action/i }))
    expect(screen.getByText("Choose an action type.")).toBeInTheDocument()

    await user.click(screen.getByLabelText("Module"))
    await user.click(screen.getByRole("option", { name: "Projects" }))
    await user.click(screen.getByLabelText(/^Project/))
    await user.click(screen.getByRole("option", { name: "CMR · Customer Relations" }))
    await user.type(screen.getByLabelText("Name"), "CMR")
    await user.click(screen.getByRole("button", { name: /create action/i }))

    expect(createAction).toHaveBeenCalledWith(
      "profile",
      expect.objectContaining({
        actionType: "projects.select",
        parameters: { project_id: { type: "project", value: "p-cmr" } },
      }),
    )
  })

  it("stores the current project as no project, not as the active one", async () => {
    const user = userEvent.setup()
    createAction.mockResolvedValue({
      id: "a5",
      name: "Repo",
      actionType: repositoryType.id,
      parameters: {},
    })
    renderEditor(null, 0, [module("projects", "Projects", [selectType, repositoryType])])
    await user.click(screen.getByLabelText("Action type"))
    await user.click(screen.getByRole("option", { name: "Project repository" }))
    // The default needs no choice; the dropdown lists this profile's projects.
    await user.click(screen.getByLabelText(/^Project/))
    expect(screen.getByRole("option", { name: "Current project (default)" })).toBeInTheDocument()
    expect(screen.getByRole("option", { name: "SHOP · Shop" })).toBeInTheDocument()
    await user.click(screen.getByRole("option", { name: "Current project (default)" }))
    await user.type(screen.getByLabelText("Name"), "Repo")
    await user.click(screen.getByRole("button", { name: /create action/i }))
    expect(createAction).toHaveBeenCalledWith(
      "profile",
      expect.objectContaining({ actionType: "projects.repository", parameters: {} }),
    )
  })

  it("offers no deprecated types for new actions but still edits saved ones", async () => {
    const user = userEvent.setup()
    const nextType: ActionType = {
      ...selectType,
      id: "projects.next",
      name: "Next project",
      description: "Makes the next project of the profile active.",
      parameters: [],
      deprecated: true,
    }
    const modules = [module("projects", "Projects", [selectType, nextType])]
    // The deprecated type does not count: select is the only type offered.
    renderEditor(null, 0, modules)
    expect(screen.getByLabelText("Action type")).toHaveTextContent("Select project")
    await user.click(screen.getByLabelText("Action type"))
    expect(screen.getByRole("option", { name: "Select project" })).toBeInTheDocument()
    expect(screen.queryByRole("option", { name: "Next project" })).not.toBeInTheDocument()
    cleanup()

    const saved: ActionInstance = {
      id: "a6",
      name: "Old next",
      actionType: nextType.id,
      parameters: {},
    }
    updateAction.mockResolvedValue(saved)
    renderEditor(saved, 0, modules)
    expect(screen.getByText(/Projects › Next project/)).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: /^save$/i }))
    expect(updateAction).toHaveBeenCalledWith(
      "profile",
      expect.objectContaining({ id: "a6", actionType: "projects.next" }),
    )
  })

  it("keeps actions of disabled modules visible but unavailable", () => {
    const action: ActionInstance = {
      id: "a3",
      name: "Next project",
      actionType: selectType.id,
      parameters: { project_id: { type: "project", value: "p-shop" } },
    }
    renderEditor(action, 0, [
      module("general", "General", [logType]),
      module("projects", "Projects", [selectType], false),
    ])
    expect(screen.getByText("Unavailable")).toBeInTheDocument()
    expect(screen.getByText(/The Projects module is disabled/)).toBeInTheDocument()
    expect(screen.getByRole("button", { name: /^save$/i })).toBeDisabled()
    expect(screen.getByRole("button", { name: /duplicate/i })).toBeDisabled()
    expect(screen.getByRole("button", { name: /delete/i })).toBeEnabled()
  })

  it("flags project parameters that refer to another profile", () => {
    const action: ActionInstance = {
      id: "a4",
      name: "Foreign",
      actionType: selectType.id,
      parameters: { project_id: { type: "project", value: "p-elsewhere" } },
    }
    renderEditor(action, 0, [module("projects", "Projects", [selectType])])
    expect(screen.getByText(/not part of this profile/)).toBeInTheDocument()
  })
})

describe("ActionEditor in Polish", () => {
  beforeEach(async () => {
    resetApi()
    await act(async () => {
      await i18n.changeLanguage("pl")
    })
  })
  afterEach(async () => {
    cleanup()
    await act(async () => {
      await i18n.changeLanguage("en")
    })
  })

  it("labels the form and its validation in Polish; parameter labels stay as provided", async () => {
    const user = userEvent.setup()
    renderEditor(null)
    expect(screen.getByLabelText("Nazwa")).toHaveAttribute("placeholder", "Otwórz Terminal")
    expect(screen.getByLabelText("Etykieta klawisza")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Utwórz akcję" }))
    expect(screen.getByText("Wprowadź nazwę.")).toBeInTheDocument()
    // "Message" comes from the module and is not translated; what it needs
    // is said in Polish.
    expect(screen.getByText("Message")).toBeInTheDocument()
    expect(screen.getByText("Wymaga konfiguracji")).toBeInTheDocument()
    expect(
      screen.getByText("Wymagane: uzupełnij, zanim akcja będzie mogła działać."),
    ).toBeInTheDocument()
    expect(createAction).not.toHaveBeenCalled()
  })

  it.each([
    [2, "Akcja jest używana w 2 miejscach.", "2 odwołania wymagają wyboru."],
    [5, "Akcja jest używana w 5 miejscach.", "5 odwołań wymaga wyboru."],
  ])("counts %i references with Polish plurals", async (count, used, undecided) => {
    const user = userEvent.setup()
    const action: ActionInstance = {
      id: "a1",
      name: "Greet",
      actionType: logType.id,
      parameters: { message: { type: "text", value: "hi" } },
    }
    renderEditor(action, count)
    await user.click(screen.getByRole("button", { name: /usuń/i }))
    const dialog = screen.getByRole("dialog", { name: "Usunąć „Greet”?" })
    expect(within(dialog).getByText(new RegExp(`^${used}`))).toBeInTheDocument()
    expect(within(dialog).getByText(new RegExp(undecided))).toBeInTheDocument()
    expect(within(dialog).getByRole("button", { name: "Zastosuj i usuń" })).toBeDisabled()
  })
})

describe("a draft that is run", () => {
  it("says in words that a draft cannot run yet, with what it lacks", async () => {
    const user = userEvent.setup()
    // The agent refuses before any handler runs (`configuration-incomplete`).
    api.runAction.mockResolvedValue({
      status: "failed",
      message: "missing required parameter `message`",
      code: "configuration-incomplete",
      data: { missing: "message" },
    })
    render(
      <ActionEditor
        profileId="profile"
        modules={[module("general", "General", [logType])]}
        projects={projects}
        action={{
          id: "draft",
          name: "Say hello",
          actionType: logType.id,
          parameters: {},
          description: "",
          icon: null,
          label: null,
        }}
        references={[]}
        canRun
        onSaved={vi.fn()}
        onDeleted={vi.fn()}
      />,
    )
    await user.click(screen.getByRole("button", { name: "Run" }))
    const status = await screen.findByRole("status")
    expect(status).toHaveTextContent(
      "Say hello needs configuration before it can run. Missing: Message",
    )
    // Neither the agent's words nor the keys as output.
    expect(status).not.toHaveTextContent("missing required parameter")
    expect(status).not.toHaveTextContent(/^missing/)
  })
})

describe("an edit that leaves a shared action without a required setting", () => {
  afterEach(cleanup)
  const complete: ActionInstance = {
    id: "a-log",
    name: "Write log",
    actionType: logType.id,
    parameters: { message: { type: "text", value: "hello" } },
  }

  function renderShared(usages: number) {
    render(
      <DialogProvider>
        <ActionEditor
          profileId="profile"
          modules={[module("general", "General", [logType])]}
          projects={projects}
          action={complete}
          references={Array.from({ length: usages }, (_, index) => ({
            site: { type: "step" as const, workflow: "w", index },
            ownerName: "Start Work",
          }))}
          canRun={false}
          onSaved={vi.fn()}
          onDeleted={vi.fn()}
        />
      </DialogProvider>,
    )
  }

  beforeEach(() => {
    cleanup()
    resetApi()
    updateAction.mockImplementation((_profile: string, saved: ActionInstance) =>
      Promise.resolve(saved),
    )
  })

  it("asks first, saying how many places stop running, and saves only when confirmed", async () => {
    const user = userEvent.setup()
    renderShared(3)
    await user.clear(screen.getByLabelText(/^Message/))
    await user.click(screen.getByRole("button", { name: "Save" }))
    const ask = await screen.findByRole("alertdialog", {
      name: "Save Write log without its required settings?",
    })
    expect(ask).toHaveTextContent(
      "This action is used in 3 places. Without its required settings it stops running in all of them",
    )
    expect(updateAction).not.toHaveBeenCalled()
    await user.click(within(ask).getByRole("button", { name: "Cancel" }))
    expect(updateAction).not.toHaveBeenCalled()
    await user.click(screen.getByRole("button", { name: "Save" }))
    await user.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Save anyway" }),
    )
    expect(updateAction).toHaveBeenCalledTimes(1)
    expect(updateAction.mock.calls[0]?.[1]).toMatchObject({ parameters: {} })
  })

  it("does not ask for an action used in one place, or when the edit keeps it complete", async () => {
    const user = userEvent.setup()
    renderShared(1)
    await user.clear(screen.getByLabelText(/^Message/))
    await user.click(screen.getByRole("button", { name: "Save" }))
    expect(updateAction).toHaveBeenCalledTimes(1)
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument()
    cleanup()
    updateAction.mockClear()
    renderShared(3)
    await user.type(screen.getByLabelText(/^Message/), " there")
    await user.click(screen.getByRole("button", { name: "Save" }))
    expect(updateAction).toHaveBeenCalledTimes(1)
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument()
  })
})
