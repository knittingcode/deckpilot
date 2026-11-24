// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, screen } from "@testing-library/react"
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest"

import { SettingsSidebar } from "@/components/settings-sidebar"
import {
  findSettingsPage,
  groupedSettings,
  searchSettings,
  settingsPages,
} from "@/settings/registry"
import { installDomStubs } from "@/test/dom"
import { renderWithApp } from "@/test/render"

// Records whether any plugin code is loaded at all.
const loaded = vi.hoisted(() => new Set<string>())
vi.mock("@/plugins/marketplace", () => {
  loaded.add("marketplace")
  return { PluginsSettings: () => null }
})
vi.mock("@/plugins/client", () => {
  loaded.add("client")
  return {}
})

beforeAll(installDomStubs)
afterEach(() => {
  cleanup()
  vi.unstubAllEnvs()
})

describe("plugin marketplace flag", () => {
  it("is off unless the build sets VITE_DECKPILOT_PLUGINS=1", () => {
    expect(import.meta.env.VITE_DECKPILOT_PLUGINS).toBeUndefined()
  })

  it("leaves the marketplace out of settings, search and routes when off", () => {
    for (const value of [undefined, "", "0", "true"]) {
      if (value !== undefined) vi.stubEnv("VITE_DECKPILOT_PLUGINS", value)
      const pages = settingsPages([])
      expect(pages.map((p) => p.id)).not.toContain("plugins")
      // The navigation only opens pages the registry knows.
      expect(findSettingsPage([], "plugins")).toBeUndefined()
      expect(searchSettings(pages, "marketplace")).toEqual([])
      expect(
        groupedSettings(pages)
          .find((g) => g.id === "extensions")
          ?.pages.map((p) => p.title),
      ).toEqual(["Modules", "Permissions"])
    }
    renderWithApp(<SettingsSidebar current="modules" />)
    expect(screen.queryByRole("button", { name: "Plugins" })).not.toBeInTheDocument()
    expect(screen.queryByText(/marketplace/i)).not.toBeInTheDocument()
    expect(loaded.size).toBe(0)
  })

  it("has no Settings › Plugins page when on either: the Marketplace manages plugins", () => {
    vi.stubEnv("VITE_DECKPILOT_PLUGINS", "1")
    expect(findSettingsPage([], "plugins")).toBeUndefined()
    const pages = settingsPages([])
    expect(
      groupedSettings(pages)
        .find((g) => g.id === "extensions")
        ?.pages.map((p) => p.title),
    ).toEqual(["Modules", "Permissions"])
    // Plugin diagnostics and permissions are sections of existing pages.
    expect(searchSettings(pages, "migration").map((p) => p.id)).toContain("diagnostics")
    expect(searchSettings(pages, "plugins").map((p) => p.id)).toEqual(
      expect.arrayContaining(["permissions", "diagnostics"]),
    )
    renderWithApp(<SettingsSidebar current="modules" />)
    expect(screen.queryByRole("button", { name: "Plugins" })).not.toBeInTheDocument()
    // Nothing of the plugins is loaded before such a page renders.
    expect(loaded.size).toBe(0)
  })
})
