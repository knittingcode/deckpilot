// @vitest-environment jsdom
// Plugin actions in the layout editor (key configuration, library, gesture
// editor) and in the step-workflow editor (experimental,
// VITE_DECKPILOT_PLUGINS=1). The plugin modules come from the agent's
// `listPlugins` (here the in-memory stand-in of test/plugins.ts), exactly
// as `usePluginModules` turns them into editor modules.
import "@testing-library/jest-dom/vitest"

import { cleanup, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { LayoutEditor } from "@/layout-editor/editor"
import { targetGroups } from "@/layout-editor/targets"
import { pluginActionUnavailable, pluginModules } from "@/lib/plugin-actions"
import type { DeviceLayout, DeviceModel, ModuleInfo } from "@/lib/types"
import { WorkflowsPage } from "@/pages/workflows"
import { api, resetApi } from "@/test/api-mock"
import { installDomStubs } from "@/test/dom"
import {
  ACTION_TYPES,
  action,
  builtInModules,
  profileView,
  savedByAgent,
  stepWorkflow,
} from "@/test/fixtures"
import { HELLO, createFakePluginClient } from "@/test/plugins"
import { renderWithApp } from "@/test/render"
import { resetToasts } from "@/test/toast"

vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)
vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)

const GREET = `${HELLO}.greet`

beforeAll(() => {
  installDomStubs()
  Element.prototype.hasPointerCapture = () => false
  Element.prototype.releasePointerCapture = () => undefined
  Element.prototype.scrollIntoView = () => undefined
})
beforeEach(() => {
  resetApi()
  resetToasts()
  api.previewLayout.mockResolvedValue([])
  api.listIcons.mockResolvedValue([])
  // The backend's list as it is before the app read the plugins.
  api.listActionTypes.mockResolvedValue([ACTION_TYPES.openUrl])
  api.getGestureDefaults.mockResolvedValue({ doubleClickMs: 350, longPressMs: 800 })
  api.saveLayout.mockImplementation((_, saved) => Promise.resolve(savedByAgent(saved)))
})
afterEach(() => {
  cleanup()
  vi.unstubAllEnvs()
})

/** The plugin modules the app derives from the agent's answer. */
async function helloModules(state: "enabled" | "disabled" | "uninstalled") {
  const client = createFakePluginClient({ helloInstalled: true })
  if (state === "disabled") await client.disablePlugin(HELLO)
  if (state === "uninstalled") await client.uninstallPlugin(HELLO)
  return pluginModules(await client.listPlugins())
}

const greeting = action("a-greet", "Greet Ada", GREET, {
  parameters: { name: { type: "text", value: "Ada" } },
})
const docs = action("a-docs", "Open docs", ACTION_TYPES.openUrl.id)

const mk2: DeviceModel = {
  id: "elgato.streamdeck.mk2",
  vendor: "Elgato",
  name: "Stream Deck MK.2",
  usb: null,
  capabilities: { keys: { rows: 3, columns: 5, image: null }, encoders: null, touch: null },
  support: "verified",
  verification: [],
}

/** Key 1 runs the plugin action on press; the rest is empty. */
const layout: DeviceLayout = {
  id: "layout",
  name: "Main",
  model: mk2.id,
  homePage: "home",
  explicitTree: true,
  pages: [
    {
      id: "home",
      name: "Home",
      bindings: [
        {
          control: { kind: "key", index: 0 },
          trigger: { type: "press" },
          target: { type: "action", action: greeting.id },
          label: "Hi",
        },
      ],
    },
  ],
}

function renderLayout(plugins: ModuleInfo[]) {
  const view = profileView({ actions: [docs, greeting], layouts: [layout] })
  return renderWithApp(
    <LayoutEditor
      profileId="profile-1"
      view={view}
      model={mk2}
      saved={layout}
      onDirtyChange={vi.fn()}
      onSaved={vi.fn()}
    />,
    { modules: builtInModules(), ...(plugins.length > 0 ? { pluginModules: plugins } : {}) },
  )
}

/** The library: its search and sections (types only; no saved actions). */
const library = () =>
  screen.getByRole("searchbox", { name: "Search the library" }).parentElement
    ?.parentElement as HTMLElement

describe("pluginActionUnavailable", () => {
  it("says why a plugin action cannot be placed, and nothing for built-in types", async () => {
    vi.stubEnv("VITE_DECKPILOT_PLUGINS", "1")
    const enabled = [...builtInModules(), ...(await helloModules("enabled"))]
    const disabled = [...builtInModules(), ...(await helloModules("disabled"))]
    const none = builtInModules()
    expect(pluginActionUnavailable(enabled, GREET)).toBeNull()
    expect(pluginActionUnavailable(disabled, GREET)).toBe(
      "Hello Weather (acme.hello-weather): the plugin is disabled",
    )
    expect(pluginActionUnavailable(none, GREET)).toBe(
      "The plugin acme.hello-weather is not installed",
    )
    expect(pluginActionUnavailable(disabled, ACTION_TYPES.openUrl.id)).toBeNull()
  })

  it("is never set without the flag", async () => {
    expect(pluginActionUnavailable(await helloModules("disabled"), GREET)).toBeNull()
    expect(pluginActionUnavailable([], GREET)).toBeNull()
  })

  it("marks only the affected actions among the layout targets", () => {
    const groups = targetGroups(
      { actions: [docs, greeting], workflows: [] },
      layout,
      "home",
      (type) => (type === GREET ? "the plugin is disabled" : null),
    )
    const actions = groups.find((g) => g.label === "My actions")?.options ?? []
    expect(actions.map((o) => [o.label, o.unavailable])).toEqual([
      ["Open docs", undefined],
      ["Greet Ada", "the plugin is disabled"],
    ])
    // Without a callback nothing is marked (the old behaviour).
    const plain = targetGroups({ actions: [docs, greeting], workflows: [] }, layout, "home")
    expect(plain[0]?.options.every((o) => !("unavailable" in o))).toBe(true)
  })
})

/** What a gesture's card shows it does. */
function assignedIn(name: string): HTMLElement {
  const header = screen
    .getByRole("group", { name })
    .querySelector<HTMLElement>("[data-slot=gesture-card-action]")
  if (!header) throw new Error(`nothing assigned to ${name}`)
  return header
}

/** The Inspector's explanation of an assignment whose plugin is missing. */
function missingNote(): HTMLElement {
  const note = document.querySelector<HTMLElement>("[data-slot=missing-action-note]")
  if (!note) throw new Error("no missing-plugin note")
  return note
}

describe("the layout editor with plugins", () => {
  beforeEach(() => {
    vi.stubEnv("VITE_DECKPILOT_PLUGINS", "1")
  })

  it("offers an enabled plugin action like any other, with its parameters on gestures", async () => {
    const { user } = renderLayout(await helloModules("enabled"))
    // The plugin's type, like a built-in one; not the profile's saved action.
    expect(
      within(library()).getByRole("button", { name: "Drag Greet onto a control" }),
    ).toBeInTheDocument()
    expect(
      within(library()).queryByRole("button", { name: "Drag Greet Ada onto a control" }),
    ).toBeNull()
    await user.click(screen.getByRole("button", { name: "Key 1" }))
    // The backend's list has no plugin types yet; the editor knows them
    // from the plugins the app read, so the manifest's parameters show.
    expect(
      await screen.findByRole("button", {
        name: "Override parameters for this key/gesture only",
      }),
    ).toBeInTheDocument()
    expect(screen.queryByText(/^Unavailable:/)).not.toBeInTheDocument()
  })

  it("does not let a disabled plugin's action be placed, and keeps the keys that use it", async () => {
    const { user } = renderLayout(await helloModules("disabled"))
    // A turned-off plugin's types are not offered (as a disabled module's);
    // built-in types are.
    expect(within(library()).queryByRole("button", { name: /Drag Greet/ })).toBeNull()
    expect(within(library()).queryByText("Greet")).not.toBeInTheDocument()
    expect(
      within(library()).getByRole("button", {
        name: `Drag ${ACTION_TYPES.httpRequest.name} onto a control`,
      }),
    ).toBeInTheDocument()

    // The key that uses it keeps it and says why it does not run.
    await user.click(screen.getByRole("button", { name: "Key 1" }))
    // The gesture says why in words (no plugin id, no raw reason) and
    // offers the way back; the assignment stays.
    const note = missingNote()
    expect(note).toHaveAttribute("data-reason", "disabled")
    expect(note).toHaveTextContent("Plugin turned off")
    expect(note).toHaveTextContent(
      "\u201cGreet Ada\u201d comes from Hello Weather, which is turned off.",
    )
    expect(note).not.toHaveTextContent("acme.hello-weather")
    expect(within(note).getByRole("button", { name: "Enable plugin" })).toBeInTheDocument()
    expect(assignedIn("Single press")).toHaveTextContent("Greet Ada")
    // Replacing it is a drop from the library; removing it, its X.
    expect(note).toHaveTextContent("drag it from the Action Library onto this gesture")
    expect(screen.getByRole("button", { name: "Remove Single press" })).toBeInTheDocument()

    // Another key cannot get it: the library does not offer it.
    await user.click(screen.getByRole("button", { name: "Key 2" }))
    expect(within(library()).queryByRole("button", { name: /Drag Greet/ })).toBeNull()

    // Saving keeps the reference as it was.
    await user.click(screen.getByRole("button", { name: /Save/ }))
    await waitFor(() => {
      expect(screen.getByRole("button", { name: /Save/ })).toBeDisabled()
    })
    expect(api.saveLayout).not.toHaveBeenCalled()
  })

  it("keeps a key whose plugin was uninstalled and says so", async () => {
    const { user } = renderLayout(await helloModules("uninstalled"))
    await user.click(screen.getByRole("button", { name: "Key 1" }))
    const note = missingNote()
    expect(note).toHaveAttribute("data-reason", "notInstalled")
    expect(note).toHaveTextContent("Plugin not installed")
    // The plugin's name is not known any more: never its id.
    expect(note).not.toHaveTextContent("acme.hello-weather")
    expect(within(note).getByRole("button", { name: "Install plugin" })).toBeInTheDocument()
    expect(assignedIn("Single press")).toHaveTextContent("Greet Ada")
  })
})

describe("the layout editor without the flag", () => {
  it("is what it was: no plugin reasons, every action placeable", async () => {
    const { user } = renderLayout([])
    // Built-in types are placeable; no plugin, so no plugin type.
    expect(
      within(library()).getByRole("button", {
        name: `Drag ${ACTION_TYPES.httpRequest.name} onto a control`,
      }),
    ).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Key 1" }))
    expect(screen.queryByText(/^Unavailable:/)).not.toBeInTheDocument()
    expect(
      screen.queryByRole("button", { name: "Override parameters for this key/gesture only" }),
    ).toBeNull()
  })
})

function renderSteps(plugins: ModuleInfo[]) {
  api.getProfile.mockResolvedValue(
    profileView({
      actions: [docs, greeting],
      workflows: [stepWorkflow("w-1", "Morning", [greeting.id])],
    }),
  )
  return renderWithApp(<WorkflowsPage />, {
    modules: builtInModules(),
    ...(plugins.length > 0 ? { pluginModules: plugins } : {}),
  })
}

describe("the step-workflow editor with plugins", () => {
  beforeEach(() => {
    vi.stubEnv("VITE_DECKPILOT_PLUGINS", "1")
  })

  it("adds an enabled plugin action as a step, with its parameters", async () => {
    const { user } = renderSteps(await helloModules("enabled"))
    await screen.findByRole("combobox", { name: "Action of step 1" })
    expect(screen.queryByText(/^Unavailable:/)).not.toBeInTheDocument()
    expect(
      screen.getByRole("button", { name: "Override parameters for this step" }),
    ).toBeInTheDocument()
    await user.click(screen.getByRole("combobox", { name: "Action to add" }))
    expect(await screen.findByRole("option", { name: "Greet Ada" })).not.toHaveAttribute(
      "aria-disabled",
    )
  })

  it("keeps a step of a disabled plugin with the reason and does not add new ones", async () => {
    const { user } = renderSteps(await helloModules("disabled"))
    await screen.findByRole("combobox", { name: "Action of step 1" })
    expect(screen.getByRole("combobox", { name: "Action of step 1" })).toHaveTextContent(
      "Greet Ada",
    )
    expect(screen.getByText(/Unavailable: Plugin turned off\. The step stays/)).toBeInTheDocument()
    await user.click(screen.getByRole("combobox", { name: "Action to add" }))
    const option = await screen.findByRole("option", {
      name: "Greet Ada (unavailable: Plugin turned off)",
    })
    expect(option).toHaveAttribute("aria-disabled", "true")
    expect(screen.getByRole("option", { name: "Open docs" })).not.toHaveAttribute("aria-disabled")
  })

  it("says that a step's plugin is not installed", async () => {
    renderSteps(await helloModules("uninstalled"))
    expect(await screen.findByText(/Unavailable: Plugin not installed/)).toBeInTheDocument()
  })
})

describe("the step-workflow editor without the flag", () => {
  it("is what it was", async () => {
    const { user } = renderSteps([])
    await screen.findByRole("combobox", { name: "Action of step 1" })
    expect(screen.queryByText(/^Unavailable:/)).not.toBeInTheDocument()
    await user.click(screen.getByRole("combobox", { name: "Action to add" }))
    // The backend's list has no plugin types: unavailable, in the old words.
    expect(await screen.findByRole("option", { name: "Greet Ada (unavailable)" })).toHaveAttribute(
      "aria-disabled",
      "true",
    )
  })
})
