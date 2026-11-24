// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

// Removing a binding in the Inspector's trigger list (a touch zone's tap)
// keeps the key's look and labels on the binding shown after, as removing
// a gesture does: "Nothing" and a missing action's "Remove assignment".

import { cleanup, render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { type ReactNode, useEffect, useState } from "react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import type {
  ControlBinding,
  ControlId,
  DeviceLayout,
  DeviceModel,
  KeyAppearance,
} from "@/lib/types"
import { installDomStubs } from "@/test/dom"
import { resetToasts } from "@/test/toast"

import { Inspector } from "./inspector"
import type { TargetGroup } from "./targets"

vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)
vi.mock("@/lib/app-context", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/app-context")>()),
  useActionModules: () => [],
  useAppContext: () => ({ profiles: { activeProfileId: null, profiles: [] }, refresh: vi.fn() }),
}))

const strip: DeviceModel = {
  id: "elgato.streamdeck.plus",
  vendor: "Elgato",
  name: "Stream Deck +",
  usb: null,
  capabilities: {
    keys: { rows: 2, columns: 4, image: null },
    encoders: null,
    touch: { zones: 1, image: null, gestures: ["tap", "longPress"] },
  },
  support: "verified",
  verification: [],
} as unknown as DeviceModel

const ZONE: ControlId = { kind: "touchZone", index: 0 }

const look: KeyAppearance = {
  background: "#112233",
  labels: [{ id: "a", text: "SHOP" }],
}

function layoutWith(tapTarget: string): DeviceLayout {
  const bindings: ControlBinding[] = [
    {
      control: ZONE,
      trigger: { type: "touch", gesture: "tap" },
      target: { type: "action", action: tapTarget },
      label: "Shop",
      appearance: look,
    },
    {
      control: ZONE,
      trigger: { type: "touch", gesture: "longPress" },
      target: { type: "action", action: "mail" },
      label: null,
    },
  ]
  return {
    id: "layout",
    name: "Main",
    model: strip.id,
    homePage: "home",
    pages: [{ id: "home", name: "Home", bindings }],
  }
}

const groups: TargetGroup[] = [
  {
    id: "actions",
    label: "Actions",
    options: [
      {
        value: "action:shop",
        label: "Shop",
        kind: "action",
        target: { type: "action", action: "shop" },
      },
      {
        value: "action:mail",
        label: "Mail",
        kind: "action",
        target: { type: "action", action: "mail" },
      },
      {
        value: "action:ghost",
        label: "Ghost",
        kind: "action",
        target: { type: "action", action: "ghost" },
        unavailable: "Its plugin is not installed.",
        missing: {
          reason: "notInstalled",
          owner: "acme.ghost",
          ownerName: "Ghost",
          recovery: null,
        },
      },
    ],
  },
]

const state = { draft: null as DeviceLayout | null }

function Harness({ initial }: { initial: DeviceLayout }): ReactNode {
  const [draft, setDraft] = useState(initial)
  useEffect(() => {
    state.draft = draft
  }, [draft])
  return (
    <Inspector
      model={strip}
      draft={draft}
      pageId="home"
      selection={[ZONE]}
      focus={ZONE}
      groups={groups}
      folder={undefined}
      onChange={(next) => {
        setDraft(next)
      }}
      onUpdate={(edit) => {
        setDraft((latest) => edit(latest))
      }}
      onClear={() => undefined}
    />
  )
}

const zoneBindings = () => state.draft?.pages[0]?.bindings ?? []

beforeAll(() => {
  installDomStubs()
})
beforeEach(() => {
  state.draft = null
  resetToasts()
})
afterEach(cleanup)

describe("removing a binding in the Inspector", () => {
  it("keeps the look and labels when the tap is set to Nothing", async () => {
    const user = userEvent.setup()
    render(<Harness initial={layoutWith("shop")} />)
    await user.click(screen.getByRole("combobox", { name: "Tap" }))
    await user.click(within(screen.getByRole("listbox")).getByRole("option", { name: "Nothing" }))
    expect(zoneBindings()).toEqual([
      expect.objectContaining({
        trigger: { type: "touch", gesture: "longPress" },
        label: "Shop",
        appearance: look,
      }),
    ])
  })

  it("keeps the look and labels when a missing action's assignment is removed", async () => {
    const user = userEvent.setup()
    render(<Harness initial={layoutWith("ghost")} />)
    await user.click(screen.getByRole("button", { name: "Remove assignment" }))
    expect(zoneBindings()).toEqual([
      expect.objectContaining({
        trigger: { type: "touch", gesture: "longPress" },
        label: "Shop",
        appearance: look,
      }),
    ])
  })

  it("asks how the look moves when the long press has a look of its own", async () => {
    const user = userEvent.setup()
    const initial = layoutWith("ghost")
    const own: KeyAppearance = { background: "#ffffff", labels: [{ id: "c", text: "C" }] }
    const withOwn: DeviceLayout = {
      ...initial,
      pages: initial.pages.map((p) => ({
        ...p,
        bindings: p.bindings.map((b) =>
          b.trigger.type === "touch" && b.trigger.gesture === "longPress"
            ? { ...b, appearance: own }
            : b,
        ),
      })),
    }
    render(<Harness initial={withOwn} />)
    await user.click(screen.getByRole("button", { name: "Remove assignment" }))
    expect(screen.getByRole("alertdialog")).toHaveTextContent("has 1 label, and Long press")
    await user.click(screen.getByRole("button", { name: "Merge the labels" }))
    expect(zoneBindings()).toEqual([
      expect.objectContaining({
        appearance: { background: "#ffffff", labels: [own.labels?.[0], look.labels?.[0]] },
      }),
    ])
  })
})
