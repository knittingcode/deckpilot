// @vitest-environment jsdom
// Gesture presets in the layout editor, end to end in the app (simulated
// Stream Deck MK.2, the agent mocked): one compact library entry; dropped
// on a key it becomes ordinary actions and gesture bindings in one undo
// step, or nothing; dropped on one gesture card only that gesture changes;
// a plugin's preset follows the plugin's availability, and the keys keep
// their bindings while the plugin is gone.
import "@testing-library/jest-dom/vitest"

import { cleanup, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { gestureSlotId, releaseSlotId } from "@/components/gesture-editor/model"
import { LayoutEditor } from "@/layout-editor/editor"
import { pluginModules } from "@/lib/plugin-actions"
import type { ActionInput } from "@/lib/api"
import type {
  ActionInstance,
  ControlBinding,
  DeviceLayout,
  GesturePreset,
  ModuleInfo,
} from "@/lib/types"
import { api, resetApi } from "@/test/api-mock"
import { dnd, drop, resetDnd } from "@/test/dnd"
import { installDomStubs } from "@/test/dom"
import { builtInModules, profileView, savedByAgent } from "@/test/fixtures"
import { BRIGHTNESS, MK2, deckModule } from "@/test/gesture-presets"
import { HELLO, createFakePluginClient } from "@/test/plugins"
import { renderWithApp } from "@/test/render"
import { resetToasts, toast as toasts } from "@/test/toast"

import { gesturePresetDragId } from "./gesture-presets"

vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)
vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)
vi.mock("@dnd-kit/core", async (original) =>
  (await import("@/test/dnd")).dndModule(await original()),
)

const BRIGHTNESS_ENTRY = gesturePresetDragId("deck.brightness-control")
const GREETINGS = `${HELLO}.greetings`

/** Hello Weather's preset: Ada on a single press, Bob loudly on a long press. */
const greetings: GesturePreset = {
  id: GREETINGS,
  name: "Greetings",
  description: "",
  category: "Fun",
  allowPartial: false,
  gestures: [
    {
      gesture: "click",
      action: `${HELLO}.greet`,
      parameters: { name: { type: "text", value: "Ada" } },
      label: "Greet Ada",
    },
    {
      gesture: "longPress",
      action: `${HELLO}.greet`,
      parameters: {
        name: { type: "text", value: "Bob" },
        style: { type: "choice", value: "excited" },
      },
      label: "Greet Bob loudly",
    },
  ],
}

beforeAll(() => {
  installDomStubs()
  Element.prototype.hasPointerCapture = () => false
  Element.prototype.releasePointerCapture = () => undefined
  Element.prototype.scrollIntoView = () => undefined
})

let made: ActionInput[] = []
beforeEach(() => {
  vi.stubEnv("VITE_DECKPILOT_PLUGINS", "1")
  resetApi()
  resetToasts()
  resetDnd()
  made = []
  api.previewLayout.mockResolvedValue([])
  api.listIcons.mockResolvedValue([])
  api.listActionTypes.mockResolvedValue([BRIGHTNESS])
  api.getGestureDefaults.mockResolvedValue({ doubleClickMs: 300, longPressMs: 700 })
  api.saveLayout.mockImplementation((_, saved) => Promise.resolve(savedByAgent(saved)))
  api.createAction.mockImplementation((_profile: string, input: ActionInput) => {
    made.push(input)
    return Promise.resolve({ ...input, id: `made-${String(made.length)}` })
  })
})
afterEach(() => {
  cleanup()
  vi.unstubAllEnvs()
})

function layoutWith(bindings: ControlBinding[]): DeviceLayout {
  return {
    id: "layout",
    name: "Main",
    model: MK2.id,
    homePage: "home",
    explicitTree: true,
    pages: [{ id: "home", name: "Home", bindings }],
  }
}

function editor(layout: DeviceLayout, actions: ActionInstance[] = []) {
  return (
    <LayoutEditor
      profileId="profile-1"
      view={profileView({ actions, layouts: [layout] })}
      model={MK2}
      saved={layout}
      onDirtyChange={vi.fn()}
      onSaved={vi.fn()}
    />
  )
}

async function helloModules(state: "enabled" | "disabled" | "uninstalled") {
  const client = createFakePluginClient({ helloInstalled: true })
  if (state === "disabled") await client.disablePlugin(HELLO)
  if (state === "uninstalled") await client.uninstallPlugin(HELLO)
  return pluginModules(await client.listPlugins()).map((m) =>
    m.id === HELLO && state !== "uninstalled" ? { ...m, gesturePresets: [greetings] } : m,
  )
}

function render(layout: DeviceLayout, plugins: ModuleInfo[] = [], actions: ActionInstance[] = []) {
  return renderWithApp(editor(layout, actions), {
    modules: [...builtInModules(), deckModule()],
    pluginModules: plugins,
  })
}

function cardOf(name: string): HTMLElement {
  return screen.getByRole("group", { name })
}

/** What a gesture's card shows it does; `null` when nothing. */
function assigned(name: string): string | null {
  return (
    cardOf(name).querySelector<HTMLElement>("[data-slot=gesture-card-action]")?.textContent ?? null
  )
}

/** The bindings of key `index` in the last layout the preview was asked for. */
function previewed(index: number) {
  const layout = api.previewLayout.mock.calls.at(-1)?.[1]
  return (layout?.pages[0]?.bindings ?? [])
    .filter((b) => b.control.kind === "key" && b.control.index === index)
    .map((b) => b.trigger.type)
}

describe("the library entry", () => {
  it("is one compact entry under its category, marked as several gestures", () => {
    render(layoutWith([]))
    const section = screen.getByRole("region", { name: "Device" })
    const entry = within(section).getByRole("button", {
      name: "Drag Brightness Control onto a control, or onto one of its gestures",
    })
    expect(entry).toHaveTextContent("Brightness Control")
    expect(within(entry).getByRole("img", { name: "Several gestures" })).toBeInTheDocument()
    // The tooltip says what each gesture does; no action ids, no descriptions.
    expect(entry).toHaveAttribute(
      "title",
      [
        "Single press: Decrease brightness by 10%",
        "Double click: Set brightness to 100%",
        "Long press: Set brightness to 50%",
      ].join("\n"),
    )
    expect(entry.textContent).not.toContain("deck.")
    // The Brightness type itself is still listed next to it.
    expect(within(section).getByText("Brightness")).toBeInTheDocument()
  })

  it("is shown in Polish", async () => {
    const { i18n } = await import("@/i18n")
    await i18n.changeLanguage("pl")
    try {
      render(layoutWith([]))
      const section = screen.getByRole("region", { name: "Urządzenie" })
      expect(within(section).getByText("Sterowanie jasnością")).toBeInTheDocument()
      drop(BRIGHTNESS_ENTRY, "key:0")
      await waitFor(() => {
        expect(made.map((m) => m.name)).toEqual([
          "Zmniejsz jasność o 10%",
          "Ustaw jasność na 100%",
          "Ustaw jasność na 50%",
        ])
      })
    } finally {
      await i18n.changeLanguage("en")
    }
  })
})

describe("dropped on an empty key", () => {
  it("assigns every gesture in one undo step, with readable cards", async () => {
    const { user } = render(layoutWith([]))
    drop(BRIGHTNESS_ENTRY, "key:0")
    await waitFor(() => {
      expect(assigned("Long press")).toBe("Set brightness to 50%")
    })
    // Three Brightness actions with the preset's parameters over the defaults.
    expect(made.map((m) => [m.name, m.actionType, m.parameters])).toEqual([
      [
        "Decrease brightness by 10%",
        "deck.brightness",
        {
          change: { type: "choice", value: "dimmer" },
          level: { type: "integer", value: 100 },
          step: { type: "integer", value: 10 },
        },
      ],
      [
        "Set brightness to 100%",
        "deck.brightness",
        {
          change: { type: "choice", value: "set" },
          level: { type: "integer", value: 100 },
          step: { type: "integer", value: 10 },
        },
      ],
      [
        "Set brightness to 50%",
        "deck.brightness",
        {
          change: { type: "choice", value: "set" },
          level: { type: "integer", value: 50 },
          step: { type: "integer", value: 10 },
        },
      ],
    ])
    expect(assigned("Single press")).toBe("Decrease brightness by 10%")
    expect(assigned("Double click")).toBe("Set brightness to 100%")
    // Gesture mode: no raw release card next to the gestures.
    expect(screen.queryByRole("group", { name: "Release" })).not.toBeInTheDocument()
    // The Canvas and the live preview draw the draft.
    await waitFor(() => {
      expect(previewed(0)).toEqual(["click", "doubleClick", "longPress"])
    })

    // One step back is the empty key; one forward is all three again.
    await user.click(screen.getByRole("button", { name: "Undo" }))
    await waitFor(() => {
      expect(previewed(0)).toEqual([])
    })
    await user.click(screen.getByRole("button", { name: "Redo" }))
    await waitFor(() => {
      expect(previewed(0)).toEqual(["click", "doubleClick", "longPress"])
    })

    // Saved as plain bindings: nothing of the preset is stored.
    await user.click(screen.getByRole("button", { name: "Save" }))
    await waitFor(() => {
      expect(api.saveLayout).toHaveBeenCalledTimes(1)
    })
    const saved = api.saveLayout.mock.calls[0]?.[1] as DeviceLayout
    expect(saved.pages[0]?.bindings).toEqual([
      {
        control: { kind: "key", index: 0 },
        trigger: { type: "click" },
        target: { type: "action", action: "made-1" },
        label: "Brightness Control",
      },
      {
        control: { kind: "key", index: 0 },
        trigger: { type: "doubleClick" },
        target: { type: "action", action: "made-2" },
        label: null,
      },
      {
        control: { kind: "key", index: 0 },
        trigger: { type: "longPress" },
        target: { type: "action", action: "made-3" },
        label: null,
      },
    ])
    expect(JSON.stringify(saved)).not.toContain("brightness-control")
  })

  it("leaves every gesture editable on its own and Revert restores the saved key", async () => {
    const { user } = render(layoutWith([]))
    drop(BRIGHTNESS_ENTRY, "key:0")
    await waitFor(() => {
      expect(assigned("Double click")).toBe("Set brightness to 100%")
    })
    await user.click(within(cardOf("Double click")).getByRole("button", { name: /Remove/ }))
    expect(assigned("Double click")).toBeNull()
    expect(assigned("Single press")).toBe("Decrease brightness by 10%")
    expect(assigned("Long press")).toBe("Set brightness to 50%")
    await user.click(screen.getByRole("button", { name: /Revert/ }))
    await waitFor(() => {
      expect(previewed(0)).toEqual([])
    })
  })

  it("is applied to the latest draft: an edit made while its actions were created stays", async () => {
    // The agent answers only when told to, so the second drop happens while
    // the first one's actions are still being created.
    const pending: (() => void)[] = []
    api.createAction.mockImplementation(
      (_profile: string, input: ActionInput) =>
        new Promise((resolve) => {
          pending.push(() => {
            made.push(input)
            resolve({ ...input, id: `made-${String(made.length)}` })
          })
        }),
    )
    render(layoutWith([]))
    drop(BRIGHTNESS_ENTRY, "key:0")
    drop(BRIGHTNESS_ENTRY, "key:1")
    // Answer every request as it comes, until both drops are done.
    await waitFor(() => {
      while (pending.length > 0) pending.shift()?.()
      expect(previewed(0)).toEqual(["click", "doubleClick", "longPress"])
      expect(previewed(1)).toEqual(["click", "doubleClick", "longPress"])
    })
  })

  it("reuses the actions a second drop needs instead of making new ones", async () => {
    render(layoutWith([]))
    drop(BRIGHTNESS_ENTRY, "key:0")
    await waitFor(() => {
      expect(made).toHaveLength(3)
    })
    drop(BRIGHTNESS_ENTRY, "key:1")
    await waitFor(() => {
      expect(previewed(1)).toEqual(["click", "doubleClick", "longPress"])
    })
    expect(made).toHaveLength(3)
  })

  it("applies nothing when one of its actions cannot be made", async () => {
    let calls = 0
    api.createAction.mockImplementation((_profile: string, input: ActionInput) => {
      calls += 1
      if (calls === 2) return Promise.reject(new Error("the store is locked"))
      return Promise.resolve({ ...input, id: `made-${String(calls)}` })
    })
    render(layoutWith([]))
    drop(BRIGHTNESS_ENTRY, "key:0")
    await waitFor(() => {
      expect(toasts.error).toHaveBeenCalledWith(
        "Brightness Control was not put on Key 1",
        expect.objectContaining({ description: "the store is locked" }),
      )
    })
    expect(previewed(0)).toEqual([])
    expect(screen.getByRole("button", { name: "Undo" })).toBeDisabled()
  })
})

describe("dropped on an occupied key", () => {
  const look = { background: "#203040", textColor: "#ffcc00" }
  const mine: ActionInstance = {
    id: "mine",
    name: "Mine",
    actionType: BRIGHTNESS.id,
    parameters: {},
  }

  it("is refused on a raw key, which keeps its immediate press (D2)", () => {
    const layout = layoutWith([
      {
        control: { kind: "key", index: 2 },
        trigger: { type: "press" },
        target: { type: "action", action: mine.id },
        label: "Lights",
      },
    ])
    render(layout, [], [mine])
    drop(BRIGHTNESS_ENTRY, "key:2")
    expect(toasts.error).toHaveBeenCalledWith(
      "Brightness Control was not put on Key 3",
      expect.objectContaining({
        description: expect.stringContaining(
          "Key 3 acts as soon as it is pressed, and this preset does not change that.",
        ) as unknown,
      }),
    )
    expect(made).toEqual([])
    expect(screen.getByRole("button", { name: "Undo" })).toBeDisabled()
  })

  it("keeps the key's look and its other gestures; replaces only the declared ones", async () => {
    // In gesture mode already: the user switched it in the Inspector.
    const layout = layoutWith([
      {
        control: { kind: "key", index: 2 },
        trigger: { type: "click" },
        target: { type: "action", action: mine.id },
        label: "Lights",
        appearance: look,
      },
    ])
    const { user } = render(layout, [], [mine])
    drop(BRIGHTNESS_ENTRY, "key:2")
    await waitFor(() => {
      expect(previewed(2)).toEqual(["click", "doubleClick", "longPress"])
    })
    await user.click(screen.getByRole("button", { name: "Save" }))
    await waitFor(() => {
      expect(api.saveLayout).toHaveBeenCalled()
    })
    const saved = api.saveLayout.mock.calls[0]?.[1] as DeviceLayout
    expect(saved.pages[0]?.bindings[0]).toEqual({
      control: { kind: "key", index: 2 },
      trigger: { type: "click" },
      target: { type: "action", action: "made-1" },
      label: "Lights",
      appearance: look,
    })
    await user.click(screen.getByRole("button", { name: "Undo" }))
    await waitFor(() => {
      expect(previewed(2)).toEqual(["click", "doubleClick", "longPress"])
    })
  })

  it("is refused, with nothing changed, when a release is in the way", () => {
    const layout = layoutWith([
      {
        control: { kind: "key", index: 2 },
        trigger: { type: "press" },
        target: { type: "action", action: mine.id },
      },
      {
        control: { kind: "key", index: 2 },
        trigger: { type: "release" },
        target: { type: "action", action: mine.id },
      },
    ])
    render(layout, [], [mine])
    drop(BRIGHTNESS_ENTRY, "key:2")
    expect(toasts.error).toHaveBeenCalledWith(
      "Brightness Control was not put on Key 3",
      expect.objectContaining({
        description: expect.stringContaining("has a Release action") as unknown,
      }),
    )
    expect(made).toEqual([])
    expect(screen.getByRole("button", { name: "Undo" })).toBeDisabled()
  })
})

describe("dropped on one gesture card", () => {
  const mine: ActionInstance = {
    id: "mine",
    name: "Mine",
    actionType: BRIGHTNESS.id,
    parameters: {},
  }
  const layout = layoutWith([
    {
      control: { kind: "key", index: 0 },
      trigger: { type: "click" },
      target: { type: "action", action: mine.id },
    },
  ])
  const key = { kind: "key" as const, index: 0 }

  it("assigns only that gesture's action and leaves the others alone", async () => {
    const { user } = render(layout, [], [mine])
    await user.click(screen.getByRole("button", { name: "Key 1" }))
    expect(dnd.droppables.has(gestureSlotId(key, "longPress"))).toBe(true)
    drop(BRIGHTNESS_ENTRY, gestureSlotId(key, "longPress"))
    await waitFor(() => {
      expect(assigned("Long press")).toBe("Set brightness to 50%")
    })
    expect(made.map((m) => m.name)).toEqual(["Set brightness to 50%"])
    expect(assigned("Single press")).toBe("Mine")
    expect(assigned("Double click")).toBeNull()
  })

  it("is refused for a gesture the preset has nothing for", async () => {
    const raw = layoutWith([
      {
        control: { kind: "key", index: 0 },
        trigger: { type: "press" },
        target: { type: "action", action: mine.id },
      },
    ])
    const { user } = render(raw, [], [mine])
    await user.click(screen.getByRole("button", { name: "Key 1" }))
    drop(BRIGHTNESS_ENTRY, releaseSlotId(key))
    await waitFor(() => {
      expect(toasts.error).toHaveBeenCalledWith(
        "Brightness Control has nothing for the release",
        expect.anything(),
      )
    })
    expect(made).toEqual([])
    expect(assigned("Release")).toBeNull()
  })
})

describe("a plugin's preset", () => {
  it("puts the plugin's actions on the key while the plugin can run", async () => {
    render(layoutWith([]), await helloModules("enabled"))
    const section = screen.getByRole("region", { name: "Fun" })
    expect(within(section).getByText("Greetings")).toBeInTheDocument()
    drop(gesturePresetDragId(GREETINGS), "key:4")
    await waitFor(() => {
      expect(previewed(4)).toEqual(["click", "longPress"])
    })
    expect(made.map((m) => [m.name, m.parameters.name])).toEqual([
      ["Greet Ada", { type: "text", value: "Ada" }],
      ["Greet Bob loudly", { type: "text", value: "Bob" }],
    ])
  })

  it("is unavailable, and refused, while the plugin is off or waits for a permission", async () => {
    render(layoutWith([]), await helloModules("disabled"))
    const entry = document.querySelector(`[data-gesture-preset="${GREETINGS}"]`)
    expect(entry).toHaveAttribute("data-compatibility", "unavailable")
    expect(entry).toHaveTextContent("the plugin is disabled")
    cleanup()

    const waiting = (await helloModules("enabled")).map((m) =>
      m.id === HELLO && m.plugin
        ? {
            ...m,
            plugin: {
              ...m.plugin,
              unavailable: { [`${HELLO}.greet`]: "needs the permission `network:api.example.com`" },
            },
          }
        : m,
    )
    render(layoutWith([]), waiting)
    const blocked = document.querySelector(`[data-gesture-preset="${GREETINGS}"]`)
    expect(blocked).toHaveTextContent("needs the permission")
    // A preset grants nothing: no drop, no actions.
    expect(dnd.data.has(gesturePresetDragId(GREETINGS))).toBe(true)
    drop(gesturePresetDragId(GREETINGS), "key:4")
    expect(made).toEqual([])
    expect(toasts.error).toHaveBeenCalledWith(
      "Greetings was not put on Key 5",
      expect.objectContaining({
        description: expect.stringContaining("needs the permission") as unknown,
      }),
    )
  })

  it("is refused atomically when the key cannot take one of its gestures", async () => {
    const turning: GesturePreset = {
      ...greetings,
      gestures: [
        ...greetings.gestures,
        { gesture: "rotateClockwise", action: `${HELLO}.greet`, parameters: {}, label: "Turn" },
      ],
    }
    const plugins = (await helloModules("enabled")).map((m) =>
      m.id === HELLO
        ? {
            ...m,
            gesturePresets: [
              turning,
              { ...turning, id: `${HELLO}.partial`, name: "Partial", allowPartial: true },
            ],
          }
        : m,
    )
    render(layoutWith([]), plugins)
    drop(gesturePresetDragId(GREETINGS), "key:4")
    expect(toasts.error).toHaveBeenCalledWith(
      "Greetings was not put on Key 5",
      expect.objectContaining({ description: expect.stringContaining("turn right") as unknown }),
    )
    expect(made).toEqual([])
    // Allowed to be partial: the rest goes on, and the toast says what was left out.
    drop(gesturePresetDragId(`${HELLO}.partial`), "key:4")
    await waitFor(() => {
      expect(previewed(4)).toEqual(["click", "longPress"])
    })
    expect(toasts.warning).toHaveBeenCalledWith(
      "Partial put on Key 5",
      expect.objectContaining({ description: expect.stringContaining("turn right") as unknown }),
    )
  })

  it("leaves the keys' bindings alone while the plugin is gone, and they work again when it is back", async () => {
    const greetAda: ActionInstance = {
      id: "ada",
      name: "Greet Ada",
      actionType: `${HELLO}.greet`,
      parameters: { name: { type: "text", value: "Ada" } },
    }
    const greetBob: ActionInstance = {
      id: "bob",
      name: "Greet Bob loudly",
      actionType: `${HELLO}.greet`,
      parameters: {
        name: { type: "text", value: "Bob" },
        style: { type: "choice", value: "excited" },
      },
    }
    const applied = layoutWith([
      {
        control: { kind: "key", index: 0 },
        trigger: { type: "click" },
        target: { type: "action", action: "ada" },
        label: "Greetings",
      },
      {
        control: { kind: "key", index: 0 },
        trigger: { type: "longPress" },
        target: { type: "action", action: "bob" },
        label: null,
      },
    ])
    const { user, rerender, context } = render(applied, await helloModules("uninstalled"), [
      greetAda,
      greetBob,
    ])
    await user.click(screen.getByRole("button", { name: "Key 1" }))
    const notes = () => document.querySelectorAll("[data-slot=missing-action-note]")
    expect(notes()).toHaveLength(2)
    expect(assigned("Single press")).toBe("Greet Ada")
    expect(assigned("Long press")).toBe("Greet Bob loudly")
    // Its library entry is gone with it.
    expect(document.querySelector(`[data-gesture-preset="${GREETINGS}"]`)).toBeNull()

    rerender(editor(applied, [greetAda, greetBob]), {
      ...context,
      pluginModules: await helloModules("enabled"),
    })
    await waitFor(() => {
      expect(notes()).toHaveLength(0)
    })
    expect(assigned("Single press")).toBe("Greet Ada")
    // Nothing was rewritten or saved meanwhile.
    expect(api.saveLayout).not.toHaveBeenCalled()
    await waitFor(() => {
      expect(api.previewLayout.mock.calls.at(-1)?.[1]).toEqual(applied)
    })
  })
})
