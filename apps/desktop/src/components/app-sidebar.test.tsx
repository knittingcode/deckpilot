// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, screen } from "@testing-library/react"
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest"

import { AppSidebar } from "@/components/app-sidebar"
import { railGroups } from "@/lib/rail"
import { pluginModules } from "@/lib/plugin-actions"
import { projectsPlugin, workflowsPlugin } from "@/test/first-party"
import { installDomStubs } from "@/test/dom"
import { renderWithApp } from "@/test/render"

beforeAll(() => {
  installDomStubs()
})

afterEach(() => {
  cleanup()
  vi.unstubAllEnvs()
})

describe("main navigation", () => {
  it("leads to the device editor and the other workspace pages", async () => {
    const onNavigate = vi.fn()
    const { user } = renderWithApp(<AppSidebar current="layouts" onNavigate={onNavigate} />)
    expect(screen.getByRole("button", { name: "Layouts" })).toHaveAttribute("aria-current", "page")
    // One page marked at a time.
    expect(
      screen.getAllByRole("button").filter((b) => b.getAttribute("aria-current")),
    ).toHaveLength(1)
    // The Dashboard is gone: the Device Editor is where the app starts.
    expect(screen.queryByRole("button", { name: "Dashboard" })).not.toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Device Editor" }))
    expect(onNavigate).toHaveBeenCalledWith("designer")
    for (const name of ["Layouts", "Workflows"]) {
      expect(screen.getByRole("button", { name })).toBeInTheDocument()
    }
    // The Action Library page is gone: saved actions are in the Device
    // Editor's library.
    expect(screen.queryByRole("button", { name: "Action Library" })).not.toBeInTheDocument()
  })

  it("leaves out what moved to the toolbar and the settings window", () => {
    renderWithApp(<AppSidebar current="designer" onNavigate={vi.fn()} />)
    for (const name of ["Device Models", "Profiles", "Settings", "Layout Designer"]) {
      expect(screen.queryByRole("button", { name })).not.toBeInTheDocument()
    }
    expect(screen.queryByRole("button", { name: /Choose the profile/ })).not.toBeInTheDocument()
  })
})

describe("main navigation in a build with plugins", () => {
  it("shows Workflows and Projects only while their plugins are enabled", async () => {
    vi.stubEnv("VITE_DECKPILOT_PLUGINS", "1")
    const onNavigate = vi.fn()
    const { user, rerender, context } = renderWithApp(
      <AppSidebar current="designer" onNavigate={onNavigate} />,
      { modules: [], pluginModules: [] },
    )
    expect(screen.queryByRole("button", { name: "Workflows" })).not.toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "Projects" })).not.toBeInTheDocument()

    rerender(<AppSidebar current="designer" onNavigate={onNavigate} />, {
      ...context,
      pluginModules: pluginModules([workflowsPlugin(), projectsPlugin()]),
    })
    await user.click(screen.getByRole("button", { name: "Workflows" }))
    expect(onNavigate).toHaveBeenCalledWith("workflows")
    await user.click(screen.getByRole("button", { name: "Projects" }))
    expect(onNavigate).toHaveBeenCalledWith("projects/projects")

    rerender(<AppSidebar current="designer" onNavigate={onNavigate} />, {
      ...context,
      pluginModules: pluginModules([workflowsPlugin("disabled"), projectsPlugin("disabled")]),
    })
    expect(screen.queryByRole("button", { name: "Workflows" })).not.toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "Projects" })).not.toBeInTheDocument()
  })

  it("keeps plugin and module pages in a group of their own, apart from the built-in pages", () => {
    vi.stubEnv("VITE_DECKPILOT_PLUGINS", "1")
    renderWithApp(<AppSidebar current="designer" onNavigate={vi.fn()} />, {
      modules: [],
      pluginModules: pluginModules([workflowsPlugin(), projectsPlugin()]),
    })
    // Removing a built-in page (as the Action Library was) cannot take them
    // with it: their group is built from them alone.
    const groups = [...document.querySelectorAll("[data-sidebar='group']")]
    const names = (group: Element) =>
      [...group.querySelectorAll("button")].map((b) => b.textContent.trim())
    const contributed = groups.find((g) => names(g).includes("Workflows"))
    expect(contributed && names(contributed)).toEqual(["Workflows", "Projects"])
    const builtIn = groups.find((g) => names(g).includes("Device Editor"))
    expect(builtIn && names(builtIn)).toEqual(["Device Editor", "Layouts"])
  })
})

describe("the rail's groups", () => {
  const builtIn = [
    { id: "workspace", label: "Workspace", items: ["designer", "layouts", "dashboard", "actions"] },
  ]
  const contributed = { id: "configure", label: "Configure", items: ["workflows", "projects"] }

  it("keeps contributed pages whatever built-in page is removed, also all of them", () => {
    const pages = builtIn[0]?.items ?? []
    const without = [...pages.map((removed) => pages.filter((p) => p !== removed)), []]
    for (const items of without) {
      const groups = railGroups(
        [{ ...builtIn[0], id: "workspace", label: "W", items }],
        contributed,
      )
      expect(groups.at(-1)).toEqual(contributed)
      // Never inside a built-in group.
      for (const group of groups.slice(0, -1)) {
        expect(group.items).not.toContain("workflows")
      }
    }
  })

  it("shows no empty group", () => {
    expect(
      railGroups([{ id: "workspace", label: "W", items: [] }], { ...contributed, items: [] }),
    ).toEqual([])
  })
})
