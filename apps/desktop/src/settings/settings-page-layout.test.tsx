// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { act, cleanup, screen } from "@testing-library/react"
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest"

import { DialogProvider } from "@/components/dialogs/dialog-service"
import { SecurityContext } from "@/lib/security-context"
import { type Sensitive, SensitiveContext } from "@/lib/sensitive"
import { moduleInfo } from "@/lib/test-fixtures"
import type { SecurityView } from "@/lib/types"
import { SettingsView } from "@/settings/settings-view"
import { settingsPages } from "@/settings/registry"
import { installDomStubs } from "@/test/dom"
import { renderWithApp } from "@/test/render"

// Every call waits forever: the pages render their layout, not their data.
vi.mock("@/lib/api", () => ({
  api: new Proxy({}, { get: () => () => new Promise(() => undefined) }),
  errorMessage: (error: unknown) => String(error),
}))
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn(), warning: vi.fn() } }))

const http = moduleInfo({
  id: "http",
  name: "HTTP",
  description: "Requests to web services",
  config: [
    {
      key: "timeout",
      label: "Timeout",
      kind: { type: "text", multiline: false },
      required: false,
    },
  ],
})

const security = {
  view: { problem: null, legacyCopies: [] } as unknown as SecurityView,
  setView: vi.fn(),
  refresh: vi.fn(),
}

const sensitive: Sensitive = (task) => task()

async function renderPage(id: string) {
  renderWithApp(
    <SecurityContext value={security}>
      <SensitiveContext value={sensitive}>
        <DialogProvider>
          <SettingsView id={id} />
        </DialogProvider>
      </SensitiveContext>
    </SecurityContext>,
    { modules: [http] },
  )
  // The page's view is loaded lazily; its data never arrives.
  await act(async () => {
    await new Promise((done) => setTimeout(done, 300))
  })
}

beforeAll(installDomStubs)
afterEach(cleanup)

describe("the shared settings page layout", { timeout: 30_000 }, () => {
  // The dialog showcase is a development page of its own kind.
  const pages = settingsPages([http]).filter((page) => page.id !== "dev-dialogs")

  it.each(pages.map((page) => [page.id, page] as const))(
    "%s has one title, its own, and no section repeating it",
    async (_, page) => {
      await renderPage(page.id)
      const titles = screen.getAllByRole("heading", { level: 1 })
      expect(titles).toHaveLength(1)
      expect(titles[0]).toHaveTextContent(page.title)
      for (const heading of screen.queryAllByRole("heading", { level: 2 })) {
        expect(heading.textContent.trim()).not.toBe(page.title)
      }
    },
  )

  it("shows a module's settings under its own name and description, without a Settings heading", async () => {
    await renderPage(pages.find((p) => p.module)?.id ?? "")
    expect(await screen.findByRole("heading", { level: 1, name: "HTTP" })).toBeInTheDocument()
    expect(screen.getByText("Requests to web services")).toBeInTheDocument()
    expect(screen.queryByRole("heading", { name: "Settings" })).not.toBeInTheDocument()
    expect(screen.getByLabelText(/Timeout/)).toBeInTheDocument()
  })
})
