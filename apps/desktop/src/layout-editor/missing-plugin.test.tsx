// @vitest-environment jsdom
// Keys whose action's plugin is missing (docs/key-text.md, "Keys whose
// plugin is missing"): the assignment is kept as it is, the key shows the
// compact missing-plugin face (drawn by the agent's renderer; the editor
// only asks for it again when the plugins change), and the Inspector says
// why in words, per gesture, with the way back.
import "@testing-library/jest-dom/vitest"

import { cleanup, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { FaceTile } from "@/components/key-face"
import { LayoutEditor } from "@/layout-editor/editor"
import { catalogSignature, missingAction } from "@/lib/action-availability"
import type { PluginDetails } from "@/plugins/types"
import { pluginModules } from "@/lib/plugin-actions"
import type { DeviceLayout, DeviceModel, ModuleInfo } from "@/lib/types"
import type { ModuleMigration } from "@/plugins/types"
import { takeFocusedPlugin } from "@/plugins/focus"
import { api, resetApi } from "@/test/api-mock"
import { installDomStubs } from "@/test/dom"
import { ACTION_TYPES, action, builtInModules, profileView, savedByAgent } from "@/test/fixtures"
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
  vi.stubEnv("VITE_DECKPILOT_PLUGINS", "1")
  resetApi()
  resetToasts()
  api.previewLayout.mockResolvedValue([])
  api.listIcons.mockResolvedValue([])
  api.listActionTypes.mockResolvedValue([ACTION_TYPES.openUrl])
  api.getGestureDefaults.mockResolvedValue({ doubleClickMs: 350, longPressMs: 800 })
  api.saveLayout.mockImplementation((_, saved) => Promise.resolve(savedByAgent(saved)))
  takeFocusedPlugin()
})
afterEach(() => {
  cleanup()
  vi.unstubAllEnvs()
})

async function helloModules(state: "enabled" | "disabled" | "uninstalled") {
  const client = createFakePluginClient({ helloInstalled: true })
  if (state === "disabled") await client.disablePlugin(HELLO)
  if (state === "uninstalled") await client.uninstallPlugin(HELLO)
  return pluginModules(await client.listPlugins())
}

/** Hello Weather enabled, with `greet` waiting for a permission. */
async function waitingForPermission(): Promise<ModuleInfo[]> {
  const modules = await helloModules("enabled")
  return modules.map((m) =>
    m.id === HELLO && m.plugin
      ? {
          ...m,
          plugin: {
            ...m.plugin,
            unavailable: { [GREET]: "the permission `network:api.example.com` is not granted" },
            // What `usePluginModules` reads from the plugin's grants.
            grantable: { [GREET]: ["network:api.example.com"] },
          },
        }
      : m,
  )
}

/** A plugin module in `state` (as `pluginModules` makes it). */
async function inState(state: string): Promise<ModuleInfo[]> {
  const modules = await helloModules("enabled")
  return modules.map((m) =>
    m.id === HELLO && m.plugin ? { ...m, plugin: { ...m.plugin, state } } : m,
  )
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

/** Key 1: single press greets (plugin), long press opens docs (core). */
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
          trigger: { type: "click" },
          target: { type: "action", action: greeting.id },
          label: "Hi",
          appearance: { textColor: "#ffcc00" },
        },
        {
          control: { kind: "key", index: 0 },
          trigger: { type: "longPress" },
          target: { type: "action", action: docs.id },
        },
      ],
    },
  ],
}

function editor() {
  const view = profileView({ actions: [docs, greeting], layouts: [layout] })
  return (
    <LayoutEditor
      profileId="profile-1"
      view={view}
      model={mk2}
      saved={layout}
      onDirtyChange={vi.fn()}
      onSaved={vi.fn()}
    />
  )
}

function renderLayout(plugins: ModuleInfo[]) {
  return renderWithApp(editor(), { modules: builtInModules(), pluginModules: plugins })
}

const notes = () => [...document.querySelectorAll<HTMLElement>("[data-slot=missing-action-note]")]

function firstNote(): HTMLElement {
  const [note] = notes()
  if (!note) throw new Error("no missing-plugin note")
  return note
}

/** What a gesture's card shows it does. */
function assignedIn(name: string): HTMLElement {
  const header = cardOf(name).querySelector<HTMLElement>("[data-slot=gesture-card-action]")
  if (!header) throw new Error(`nothing assigned to ${name}`)
  return header
}

function cardOf(name: string): HTMLElement {
  const card = screen.getByRole("group", { name })
  return card
}

describe("missingAction", () => {
  it("tells the states apart: not installed, disabled, permission, integrity, revoked, busy, incompatible", async () => {
    const reason = (modules: ModuleInfo[], type = GREET) =>
      missingAction([...builtInModules(), ...modules], type)?.reason ?? null
    expect(reason(await helloModules("enabled"))).toBeNull()
    expect(reason(await helloModules("disabled"))).toBe("disabled")
    expect(reason(await helloModules("uninstalled"))).toBe("notInstalled")
    expect(reason([])).toBe("notInstalled")
    // A permission not granted is not "uninstalled".
    expect(reason(await waitingForPermission())).toBe("permission")
    expect(reason(await inState("broken"))).toBe("integrity")
    expect(reason(await inState("quarantined"))).toBe("revoked")
    expect(reason(await inState("updating"))).toBe("busy")
    // The installed version no longer has the action.
    expect(reason(await helloModules("enabled"), `${HELLO}.gone`)).toBe("incompatible")
    // Built-in actions are not plugin actions.
    expect(reason(await helloModules("disabled"), ACTION_TYPES.openUrl.id)).toBeNull()
  })

  it("offers the recovery the reason calls for, and names the plugin by name only", async () => {
    const of = async (modules: Promise<ModuleInfo[]> | ModuleInfo[]) =>
      missingAction([...builtInModules(), ...(await modules)], GREET)
    expect(await of(helloModules("disabled"))).toEqual({
      reason: "disabled",
      owner: HELLO,
      ownerName: "Hello Weather",
      recovery: "enable",
    })
    expect((await of(helloModules("uninstalled")))?.recovery).toBe("install")
    expect((await of(waitingForPermission()))?.recovery).toBe("permissions")
    expect((await of(inState("broken")))?.recovery).toBe("update")
    expect((await of(inState("updating")))?.recovery).toBeNull()
    // Unknown here: no name rather than the id.
    expect((await of([]))?.ownerName).toBeNull()
  })

  it("follows the plugin that serves a legacy action type", () => {
    const migration: ModuleMigration = {
      module: "projects",
      moduleName: "Projects",
      plugin: "deckpilot.projects",
      pluginName: "Projects",
      phase: "cutOver",
      attempts: 0,
      maxAttempts: 3,
      canRetry: false,
      route: "plugin",
      aliases: [{ legacy: "projects.next", target: "deckpilot.projects.next" }],
      updatedAt: 0,
    }
    const plugin = (available: boolean) => ({
      id: "deckpilot.projects",
      name: "Projects",
      publisher: "deckpilot",
      version: "1.0.0",
      state: "enabled" as const,
      origin: { type: "bundled" as const },
      actions: [
        {
          action: "next",
          actionType: "deckpilot.projects.next",
          name: "Next project",
          available,
          ...(available
            ? {}
            : { unavailableReason: "needs the permission `projectContext:select`" }),
          definition: { ...ACTION_TYPES.openUrl, id: "deckpilot.projects.next" },
        },
      ],
    })
    // The plugin's grants: `next` needs an optional permission not granted.
    const grants = (summary: ReturnType<typeof plugin>) =>
      ({
        summary,
        permissions: [
          {
            key: "projectContext:select",
            kind: "projectContext",
            tier: "medium",
            status: "notGranted",
            scope: null,
            optional: true,
            actions: ["next"],
          },
        ],
      }) as unknown as PluginDetails
    const modules = (plugins: ReturnType<typeof plugin>[]) => [
      ...builtInModules(),
      ...pluginModules(plugins, [], [migration], plugins.map(grants)),
    ]
    expect(missingAction(modules([plugin(true)]), "projects.next")).toBeNull()
    expect(missingAction(modules([plugin(false)]), "projects.next")?.reason).toBe("permission")
    expect(missingAction(modules([]), "projects.next")).toEqual({
      reason: "notInstalled",
      owner: "deckpilot.projects",
      ownerName: "Projects",
      recovery: "install",
    })
  })

  it("changes the catalog signature when a plugin comes back", async () => {
    const off = catalogSignature([...builtInModules(), ...(await helloModules("disabled"))])
    const on = catalogSignature([...builtInModules(), ...(await helloModules("enabled"))])
    const waiting = catalogSignature([...builtInModules(), ...(await waitingForPermission())])
    expect(new Set([off, on, waiting]).size).toBe(3)
  })
})

describe("a key whose plugin is missing", () => {
  it("shows the missing state on the affected gesture only, and keeps everything", async () => {
    const { user } = renderLayout(await helloModules("disabled"))
    await user.click(screen.getByRole("button", { name: "Key 1" }))
    const shown = notes()
    expect(shown).toHaveLength(1)
    const note = firstNote()
    expect(cardOf("Single press")).toContainElement(note)
    expect(cardOf("Long press")).not.toContainElement(note)
    expect(note).toHaveTextContent("Plugin turned off")
    expect(note).toHaveTextContent("Title “Hi”")
    expect(note).toHaveTextContent("Name: Ada")
    expect(note).not.toHaveTextContent(HELLO)
    expect(note).not.toHaveTextContent("Unavailable")
    // The assignment is still the one it was.
    expect(assignedIn("Single press")).toHaveTextContent("Greet Ada")
    expect(assignedIn("Long press")).toHaveTextContent("Open docs")
  })

  it("removes only that gesture's assignment, undoably, and Revert brings it back", async () => {
    const { user } = renderLayout(await helloModules("uninstalled"))
    await user.click(screen.getByRole("button", { name: "Key 1" }))
    await user.click(within(firstNote()).getByRole("button", { name: "Remove assignment" }))
    expect(notes()).toHaveLength(0)
    expect(assignedIn("Long press")).toHaveTextContent("Open docs")
    await user.click(screen.getByRole("button", { name: "Undo" }))
    expect(notes()).toHaveLength(1)
    await user.click(screen.getByRole("button", { name: "Redo" }))
    expect(notes()).toHaveLength(0)
    await user.click(screen.getByRole("button", { name: /Revert/ }))
    await waitFor(() => {
      expect(notes()).toHaveLength(1)
    })
    expect(api.saveLayout).not.toHaveBeenCalled()
  })

  it("opens the Marketplace at the plugin, to install it or to manage it", async () => {
    const installed = renderLayout(await helloModules("uninstalled"))
    await installed.user.click(screen.getByRole("button", { name: "Key 1" }))
    await installed.user.click(within(firstNote()).getByRole("button", { name: "Install plugin" }))
    expect(installed.context.navigate).toHaveBeenCalledWith("marketplace")
    expect(takeFocusedPlugin()).toBe(HELLO)
    cleanup()

    const waiting = renderLayout(await waitingForPermission())
    await waiting.user.click(screen.getByRole("button", { name: "Key 1" }))
    const note = firstNote()
    expect(note).toHaveAttribute("data-reason", "permission")
    expect(note).toHaveTextContent("Permission needed")
    await waiting.user.click(within(note).getByRole("button", { name: "Review permissions" }))
    // The plugin's page in the Marketplace manages its permissions.
    expect(waiting.context.navigate).toHaveBeenCalledWith("marketplace")
    expect(takeFocusedPlugin()).toBe(HELLO)
    expect(waiting.context.openSettings).not.toHaveBeenCalled()
    // Nothing is granted from here.
    expect(api.saveLayout).not.toHaveBeenCalled()
  })

  it("asks for the key faces again when the plugin comes back", async () => {
    const { rerender, context } = renderLayout(await helloModules("disabled"))
    await waitFor(() => {
      expect(api.previewLayout).toHaveBeenCalled()
    })
    const before = api.previewLayout.mock.calls.length
    rerender(editor(), { ...context, pluginModules: await helloModules("enabled") })
    await waitFor(() => {
      expect(api.previewLayout.mock.calls.length).toBeGreaterThan(before)
    })
    // The layout it asks for is the same: nothing was rewritten.
    const last = api.previewLayout.mock.calls.at(-1)
    expect(last?.[1]).toEqual(layout)
  })

  it("is explained in Polish too", async () => {
    const { i18n } = await import("@/i18n")
    await i18n.changeLanguage("pl")
    try {
      const { user } = renderLayout(await helloModules("disabled"))
      await user.click(screen.getByRole("button", { name: "Klawisz 1" }))
      const note = firstNote()
      expect(note).toHaveTextContent("Wtyczka jest wyłączona")
      expect(within(note).getByRole("button", { name: "Włącz wtyczkę" })).toBeInTheDocument()
    } finally {
      await i18n.changeLanguage("en")
    }
  })
})

describe("the face drawn until the device's image arrives", () => {
  it("is the plug with the key's title, no warning text", () => {
    render(<FaceTile face={{ style: "notice", title: "Repo SHOP", unavailable: true }} />)
    const tile = document.querySelector("[data-slot=key-face]")
    expect(tile).toHaveAttribute("data-missing-plugin", "true")
    expect(tile).toHaveAttribute("data-face", "action")
    expect(tile).toHaveTextContent("Repo SHOP")
    expect(tile).not.toHaveTextContent("Unavailable")
    expect(tile?.querySelector(".lucide-plug")).not.toBeNull()
  })
})
