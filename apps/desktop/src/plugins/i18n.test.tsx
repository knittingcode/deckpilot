// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { act, cleanup, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { i18n } from "@/i18n"
import { installDomStubs } from "@/test/dom"
import { LIGHTS, createFakePluginClient } from "@/test/plugins"
import { marketplaceSidebar } from "@/test/marketplace"
import { renderWithApp } from "@/test/render"
import { resetToasts, toast } from "@/test/toast"

import { PluginClientContext } from "./api"
import { PluginsSettings } from "./marketplace"
import { MigrationConsentDialog } from "./migration-dialog"
import type { ModuleMigration } from "./types"

vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)
vi.mock("./client", () => ({
  createTauriPluginClient: () => {
    throw new Error("the Tauri client is not used in tests")
  },
}))

beforeAll(installDomStubs)
beforeEach(async () => {
  resetToasts()
  await act(async () => {
    await i18n.changeLanguage("pl")
  })
})
afterEach(async () => {
  cleanup()
  await act(async () => {
    await i18n.changeLanguage("en")
  })
})

function renderMarketplace(variant: "marketplace" | "diagnostics" = "marketplace") {
  const client = createFakePluginClient({ helloInstalled: true })
  const result = renderWithApp(
    <PluginClientContext value={client}>
      <PluginsSettings variant={variant} />
    </PluginClientContext>,
  )
  return { ...result, client }
}

describe("the marketplace in Polish", () => {
  it("shows the sidebar, the catalog, its states and plural forms in Polish; plugin data stays as provided", async () => {
    const { user } = renderMarketplace()
    expect(await screen.findByRole("heading", { name: "Odkrywaj wtyczki" })).toBeInTheDocument()
    const nav = within(marketplaceSidebar())
    expect(
      within(nav.getAllByRole("list")[0] as HTMLElement)
        .getAllByRole("button")
        .map((b) => b.querySelector("span")?.textContent),
    ).toEqual([
      "Odkrywaj",
      "Zainstalowane",
      "Uaktualnienia",
      "Kategorie",
      "Programuj",
      "Graj",
      "Pracuj",
      "Automatyzuj",
      "Monitoruj",
    ])
    expect(nav.getByRole("searchbox", { name: "Szukaj wtyczek" })).toBeInTheDocument()
    expect(nav.getByRole("button", { name: "Zainstaluj z pliku" })).toBeInTheDocument()
    expect(screen.queryByRole("note", { name: "Funkcja eksperymentalna" })).not.toBeInTheDocument()
    expect(screen.getByRole("heading", { name: "Twoje wtyczki" })).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Zarządzaj 1 wtyczką" })).toBeInTheDocument()
    const recent = screen.getByRole("list", { name: "Ostatnio dodane" })
    expect(within(recent).getAllByRole("listitem")).toHaveLength(3)
    expect(within(recent).getByText("Niezgodna")).toBeInTheDocument()
    expect(within(recent).getByText("Niedostępna")).toBeInTheDocument()

    // Searching finds installed plugins too, with Polish quotes in the heading.
    await user.type(nav.getByRole("searchbox", { name: "Szukaj wtyczek" }), "weather")
    expect(
      await screen.findByRole("heading", { name: "Wyniki dla: „weather”" }),
    ).toBeInTheDocument()
    expect(screen.getByRole("status")).toHaveTextContent("1 wtyczka")
    const list = screen.getByRole("list", { name: "Wtyczki" })
    const hello = within(list).getByText("Hello Weather").closest("li") as HTMLElement
    // The plugin's own texts are not translated.
    expect(hello).toHaveTextContent("Greets someone and shows the temperature.")
    expect(hello).toHaveTextContent("Acme Tools")
    expect(within(hello).getByRole("img", { name: "Zweryfikowany wydawca" })).toBeInTheDocument()
    expect(within(hello).getByRole("button", { name: "Aktualizuj Hello Weather" })).toBeEnabled()
    expect(document.body).not.toHaveTextContent(
      /Install from|Refresh|Discover|Incompatible|Recently added|Results for/,
    )
  })

  it("shows what DeckPilot is made of in Polish in the diagnostics", async () => {
    renderMarketplace("diagnostics")
    const overview = await screen.findByRole("region", { name: "Co zawiera DeckPilot" })
    expect(overview).toHaveTextContent("1 wtyczka zainstalowana przez Ciebie")
    expect(overview).toHaveTextContent("3 wtyczki w rejestrze, niezainstalowane.")
    expect(screen.getByRole("region", { name: "Rejestr" })).toHaveTextContent("Zweryfikowany")
  })

  it("asks for consent in Polish, with the risk tiers and every permission spelled out", async () => {
    const { user, client } = renderMarketplace()
    const catalog = await screen.findByRole("list", { name: "Ostatnio dodane" })
    await user.click(within(catalog).getByRole("button", { name: "Home Lights" }))
    await screen.findByRole("heading", { name: "Home Lights", level: 3 })
    await user.click(screen.getByRole("button", { name: "Zainstaluj Home Lights" }))

    const dialog = within(screen.getByRole("dialog", { name: "Zainstalować Home Lights 0.3.0?" }))
    expect(dialog.getByRole("region", { name: "Wysokie ryzyko" })).toBeInTheDocument()
    expect(dialog.getByRole("region", { name: "Średnie ryzyko" })).toBeInTheDocument()
    expect(dialog.getByRole("region", { name: "Niskie ryzyko" })).toBeInTheDocument()
    expect(
      dialog.getByText("Łączenie z dodanymi przez Ciebie urządzeniami w Twojej sieci lokalnej"),
    ).toBeInTheDocument()
    expect(dialog.getByText("Używanie sekretu „token”, który dla niej przechowujesz")).toBeVisible()
    expect(
      dialog.getByText("Wysyłanie danych do https://api.lifx.com i odbieranie danych z tego hosta"),
    ).toBeInTheDocument()
    // Publisher-provided reasons stay as they are.
    expect(dialog.getAllByText(/Uzasadnienie wydawcy:/).length).toBeGreaterThan(0)
    expect(dialog.getAllByText("Talks to bulbs.").length).toBeGreaterThan(0)

    const install = dialog.getByRole("button", { name: "Zainstaluj" })
    expect(install).toBeDisabled()
    await user.click(
      dialog.getByRole("checkbox", {
        name: "Zezwól: Łączenie z dodanymi przez Ciebie urządzeniami w Twojej sieci lokalnej",
      }),
    )
    expect(dialog.getByText("Uprawnienia oczekujące na Twoją zgodę: 4 z 5.")).toBeInTheDocument()
    await user.click(dialog.getByRole("button", { name: "Zezwól na wszystkie (5)" }))
    expect(dialog.getByText("Wyrażono zgodę na wszystkie uprawnienia.")).toBeInTheDocument()
    await user.click(install)
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
    })
    expect(client.installPlugin).toHaveBeenCalledWith(
      { type: "registry", plugin: LIGHTS, version: "0.3.0" },
      expect.arrayContaining(["network:private-network", "credentials:token"]),
    )
    expect(toast.success).toHaveBeenCalledWith("Zainstalowano: Home Lights")
  })

  it("asks for a module migration's consent in Polish, item by item", async () => {
    const migration: ModuleMigration = {
      module: "github",
      moduleName: "GitHub",
      plugin: "deckpilot.github",
      pluginName: "GitHub",
      phase: "awaitingConsent",
      attempts: 0,
      maxAttempts: 3,
      canRetry: false,
      route: "builtin",
      aliases: [],
      updatedAt: 0,
      consent: [
        {
          key: "network:private-network",
          decision: "pending",
          why: "Your GitHub Enterprise server is on your network.",
          permission: {
            key: "network:private-network",
            kind: "network",
            tier: "high",
            status: "notGranted",
            scope: { type: "network" },
          },
          origin: {
            origin: "https://ghe.local",
            setting: "apiUrl",
            privateNetwork: true,
          },
        },
      ],
    }
    const { user } = renderWithApp(
      <MigrationConsentDialog
        migration={migration}
        onDecide={() => Promise.resolve()}
        onClose={() => undefined}
      />,
    )
    const dialog = within(
      screen.getByRole("dialog", { name: "Akcje modułu GitHub działają teraz jako wtyczka" }),
    )
    expect(dialog.getByText("Łączenie z https://ghe.local w Twojej sieci prywatnej")).toBeVisible()
    expect(dialog.getByText("Wysokie ryzyko")).toBeInTheDocument()
    expect(
      dialog.getByText(/Poświadczenia i dostęp do Twojej sieci lokalnej nigdy nie są przyznawane/),
    ).toBeInTheDocument()
    expect(dialog.getByText("Elementy oczekujące na Twoją decyzję: 1 z 1.")).toBeInTheDocument()
    const decision = within(
      dialog.getByRole("radiogroup", { name: "Decyzja: network:private-network" }),
    )
    await user.click(decision.getByRole("radio", { name: "Nie zezwalaj" }))
    expect(
      dialog.getByText("GitHub pozostaje modułem wbudowanym; możesz zapytać ponownie później."),
    ).toBeInTheDocument()
    expect(dialog.getByRole("button", { name: "Nie teraz" })).toBeInTheDocument()
    expect(dialog.getByRole("button", { name: "Potwierdź" })).toBeEnabled()
  })
})
