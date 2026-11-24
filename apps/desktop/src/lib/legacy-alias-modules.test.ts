import { describe, expect, it } from "vitest"

import { moduleInfo } from "@/lib/test-fixtures"
import type { ActionType, LegacyAlias, ModuleInfo } from "@/lib/types"

import { withLegacyAliasModules } from "./plugin-actions"

// The legacy ids the review profile's saved actions use (projects, git,
// docker: 13 actions), each routed by the agent to its plugin's type
// (`list_module_migrations` aliases, read from the running review agent).
const ROUTED: [string, string][] = [
  ["projects.current", "deckpilot.projects.current"],
  ["projects.cycle", "deckpilot.projects.cycle"],
  ["projects.documentation", "deckpilot.projects.documentation"],
  ["projects.repository", "deckpilot.projects.repository"],
  ["projects.select", "deckpilot.projects.select"],
  ["git.changes", "deckpilot.git.changes"],
  ["git.status", "deckpilot.git.status"],
  ["docker.status", "deckpilot.docker.status"],
  ["docker.compose-status", "deckpilot.docker.compose-status"],
  ["docker.compose-up", "deckpilot.docker.compose-up"],
  ["docker.compose-down", "deckpilot.docker.compose-down"],
  ["docker.compose-restart", "deckpilot.docker.compose-restart"],
]

const pluginType = (id: string): ActionType => ({
  id,
  name: `${id} name`,
  description: "",
  category: "Plugin",
  requiresProject: false,
  parameters: [
    {
      key: "field",
      label: `${id} field`,
      kind: { type: "text", multiline: false },
      required: false,
    },
  ],
})

function plugins(unavailable: Record<string, string> = {}): ModuleInfo[] {
  const byPlugin = new Map<string, [string, string][]>()
  for (const pair of ROUTED) {
    const plugin = `deckpilot.${pair[0].split(".")[0] ?? ""}`
    byPlugin.set(plugin, [...(byPlugin.get(plugin) ?? []), pair])
  }
  return [...byPlugin].map(([plugin, pairs]) =>
    moduleInfo({
      id: plugin,
      name: plugin,
      enabled: true,
      actionTypes: pairs.map(([, target]) => pluginType(target)),
      plugin: {
        state: "enabled",
        unavailable,
        aliases: pairs.map(([legacy, target]): LegacyAlias => ({
          legacy,
          target,
          module: legacy.split(".")[0] ?? "",
          plugin,
          pluginName: plugin,
          route: "plugin",
        })),
      },
    }),
  )
}

const native = moduleInfo({ id: "general", actionTypes: [pluginType("general.log")] })

describe("legacy ids a plugin replaces", () => {
  it("each has the plugin's fields under its own id, marked deprecated", () => {
    const modules = withLegacyAliasModules([native, ...plugins()])
    for (const [legacy, target] of ROUTED) {
      const owner = modules.find((m) => m.actionTypes.some((t) => t.id === legacy))
      const type = owner?.actionTypes.find((t) => t.id === legacy)
      expect(type, legacy).toBeDefined()
      expect(type?.deprecated).toBe(true)
      expect(type?.parameters.map((p) => p.label)).toEqual([`${target} field`])
      // Routed as the agent routes it, never by name.
      expect(owner?.legacy?.[legacy]?.target).toBe(target)
      expect(owner?.enabled).toBe(true)
    }
  })

  it("carries the plugin's reason when its type cannot run now", () => {
    const modules = withLegacyAliasModules([
      ...plugins({ "deckpilot.docker.compose-up": "needs the permission `tools:docker:modify`" }),
    ])
    const owner = modules.find((m) => m.legacy?.["docker.compose-up"])
    expect(owner?.legacy?.["docker.compose-up"]?.unavailable).toMatch(/tools:docker:modify/)
    expect(owner?.legacy?.["docker.status"]?.unavailable).toBeNull()
  })

  it("changes nothing without aliases, and keeps listed types as they are", () => {
    const modules = [native]
    expect(withLegacyAliasModules(modules)).toBe(modules)
    // A legacy id a listed module already has keeps that module's definition.
    const listed = moduleInfo({ id: "git", actionTypes: [pluginType("git.status")] })
    const result = withLegacyAliasModules([listed, ...plugins()])
    const owners = result.filter((m) => m.actionTypes.some((t) => t.id === "git.status"))
    expect(owners).toHaveLength(1)
    expect(owners[0]?.actionTypes.find((t) => t.id === "git.status")).toEqual(listed.actionTypes[0])
    // Its other legacy id joins the same module, not a second one of that id.
    expect(result.filter((m) => m.id === "git")).toHaveLength(1)
    expect(owners[0]?.legacy?.["git.changes"]?.target).toBe("deckpilot.git.changes")
  })
})
