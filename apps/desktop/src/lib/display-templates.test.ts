import { describe, expect, it } from "vitest"

import { dependsOn, type DisplayDependency } from "./display-templates"

describe("dependsOn", () => {
  const dependencies: DisplayDependency[] = [
    { kind: "project" },
    { kind: "variable", reference: "shared.team.name" },
    { kind: "variable", reference: "system.project.current.key" },
    { kind: "variable", reference: "plugins.acme.weather.city" },
  ]

  it("matches the keys sharedVariablesChanged names", () => {
    expect(dependsOn(dependencies, "team.name")).toBe(true)
    expect(dependsOn(dependencies, "projects.current.key")).toBe(true)
    expect(dependsOn(dependencies, "plugins.deckpilot.projects.current.key")).toBe(true)
    expect(dependsOn(dependencies, "plugins.acme.weather.city")).toBe(true)
  })

  it("ignores unrelated keys and static texts", () => {
    expect(dependsOn(dependencies, "team.size")).toBe(false)
    expect(dependsOn(undefined, "team.name")).toBe(false)
  })
})
