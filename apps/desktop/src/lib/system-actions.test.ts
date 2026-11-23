import { afterEach, describe, expect, it } from "vitest"

import { i18n } from "@/i18n"

import {
  CLOSE_APPLICATION,
  OPEN,
  OPEN_APPLICATION,
  WEBSITE,
  actionTypeDescription,
  actionTypeName,
  baseName,
  bindingKeyFor,
  checkWebsite,
  executionError,
  machinePick,
  parameterTexts,
  slug,
} from "./system-actions"

afterEach(async () => {
  await i18n.changeLanguage("en")
})

describe("Website addresses", () => {
  // The same cases as the core's `websites_are_validated_and_normalised`.
  it("normalises what opens", () => {
    const cases: [string, string][] = [
      [" https://Example.COM/a?b=c ", "https://example.com/a?b=c"],
      ["example.com", "https://example.com/"],
      ["example.com/docs#intro", "https://example.com/docs#intro"],
      ["localhost:3000/x", "https://localhost:3000/x"],
      ["HTTP://example.com", "http://example.com/"],
      ["https://example.com/a b", "https://example.com/a%20b"],
      ["https://zażółć.pl", "https://xn--za-6ja4f8n1l.pl/"],
      ["http://[::1]:8080/", "http://[::1]:8080/"],
    ]
    for (const [text, url] of cases) {
      expect(checkWebsite(text), text).toEqual({ url, problem: null })
    }
  })

  it("refuses other schemes, credentials and tricks", () => {
    const cases: [string, string][] = [
      ["", "empty"],
      ["   ", "empty"],
      ["file:///etc/passwd", "scheme"],
      ["javascript:alert(1)", "scheme"],
      ["data:text/html,x", "scheme"],
      ["vscode://open", "scheme"],
      ["ftp://example.com", "scheme"],
      ["https://", "invalid"],
      ["https://user:pass@example.com", "credentials"],
      ["https://bank.example@evil.example", "credentials"],
      ["https://example.com\\@evil.example", "backslash"],
      ["https://example.com/\u0007", "control"],
      ["exa mple.com", "invalid"],
      [`https://example.com/${"a".repeat(2048)}`, "tooLong"],
    ]
    for (const [text, problem] of cases) {
      expect(checkWebsite(text).problem, text).toBe(problem)
    }
  })

  it("keeps saved mail addresses, without a browser, and templates", () => {
    expect(checkWebsite("mailto:team@example.com").problem).toBeNull()
    expect(checkWebsite("mailto:team@example.com", true).problem).toBe("mailBrowser")
    expect(checkWebsite("https://{{ shared.host }}/").problem).toBeNull()
  })
})

describe("machine-local bindings", () => {
  const bindings = [
    { key: "app.safari", kind: "application" as const, value: "com.apple.Safari" },
    { key: "app.editor", kind: "application" as const, value: "/opt/editor" },
  ]

  it("reuses a binding of the same value and never changes another", () => {
    expect(bindingKeyFor("app", "Safari", "application", "com.apple.Safari", bindings)).toBe(
      "app.safari",
    )
    expect(bindingKeyFor("app", "Editor", "application", "/usr/bin/editor", bindings)).toBe(
      "app.editor-2",
    )
    expect(bindingKeyFor("file", "Report Q1.pdf", "other", "/tmp/r.pdf", bindings)).toBe(
      "file.report-q1-pdf",
    )
  })

  it("makes valid resource keys of any name", () => {
    expect(slug("Visual Studio Code")).toBe("visual-studio-code")
    expect(slug("Zażółć Gęślą")).toBe("zazo-c-gesla")
    expect(slug("!!!")).toBe("item")
    expect(slug("x".repeat(80))).toHaveLength(40)
    expect(baseName("/Applications/Safari.app")).toBe("Safari")
    expect(baseName("C:\\Tools\\tool.exe")).toBe("tool")
    expect(baseName("/home/u/Downloads/")).toBe("Downloads")
  })

  it("gives the pickers to the four actions' machine-local parameters only", () => {
    expect(machinePick(WEBSITE, "browser")).toBe("browser")
    expect(machinePick(WEBSITE, "url")).toBeNull()
    expect(machinePick(OPEN, "location")).toBe("location")
    expect(machinePick(OPEN, "application")).toBe("application")
    expect(machinePick(OPEN_APPLICATION, "application")).toBe("application")
    expect(machinePick(CLOSE_APPLICATION, "application")).toBe("application")
    expect(machinePick("system.run-command", "program")).toBeNull()
    expect(machinePick(undefined, "application")).toBeNull()
  })
})

describe("names in the window's language", () => {
  it("translates the System actions and leaves others alone", async () => {
    const website = { id: WEBSITE, name: "Website", description: "x" }
    expect(actionTypeName(website)).toBe("Website")
    await i18n.changeLanguage("pl")
    expect(actionTypeName(website)).toBe("Strona internetowa")
    expect(actionTypeName({ id: CLOSE_APPLICATION, name: "Close Application" })).toBe(
      "Zamknij aplikację",
    )
    expect(actionTypeDescription({ id: "general.log", description: "Logs" })).toBe("Logs")
    expect(parameterTexts(CLOSE_APPLICATION, { key: "force", label: "Force" }).label).toBe(
      "Wymuś zamknięcie, jeśli się nie zamknie",
    )
    expect(parameterTexts("general.log", { key: "message", label: "Message" }).label).toBe(
      "Message",
    )
  })

  it("says what an execution error means", () => {
    expect(executionError("not-user-initiated")).toMatch(/only when you start it/)
    expect(executionError("protected-process")).toMatch(/never closed/)
    expect(executionError("something-else")).toBeNull()
    expect(executionError(undefined)).toBeNull()
  })
})
