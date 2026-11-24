// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { act, cleanup, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { installDomStubs } from "@/test/dom"
import {
  HELLO,
  ICON,
  LIGHTS,
  createFakePluginClient,
  availablePlugins,
  readyRuntime,
  type FakeOptions,
} from "@/test/plugins"
import {
  activeNavItems,
  card,
  marketplaceSidebar,
  discoverReady,
  installedItem,
  navItem,
  openDetails,
  openInstalled,
  openSection,
  searchField,
} from "@/test/marketplace"
import { renderWithApp } from "@/test/render"
import { resetToasts, toast } from "@/test/toast"

import { PluginClientContext, PluginError } from "./api"
import { takeFocusedPlugin } from "./focus"
import { PluginsSettings } from "./marketplace"

vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)
// The default client must never be created in these tests.
vi.mock("./client", () => ({
  createTauriPluginClient: () => {
    throw new Error("the Tauri client is not used in tests")
  },
}))

beforeAll(installDomStubs)
beforeEach(resetToasts)
afterEach(cleanup)

function renderMarketplace(options: FakeOptions = {}) {
  const client = createFakePluginClient(options)
  const result = renderWithApp(
    <PluginClientContext value={client}>
      <PluginsSettings />
    </PluginClientContext>,
  )
  return { ...result, client }
}

/** The catalog's category or search results. */
const results = () => within(screen.getByRole("list", { name: "Plugins" })).getAllByRole("listitem")
/** "Recently added": the newest registry plugins not installed here. */
const cards = () =>
  within(screen.getByRole("list", { name: "Recently added" })).getAllByRole("listitem")

/** Picks a package file and confirms its installation. */
async function installFile(
  user: ReturnType<typeof renderMarketplace>["user"],
  client: ReturnType<typeof renderMarketplace>["client"],
  path: string,
) {
  vi.mocked(client.pickPackage).mockResolvedValueOnce(path)
  await user.click(await screen.findByRole("button", { name: "Install from file" }))
  const confirm = await screen.findByRole("alertdialog", { name: "Install the plugin package?" })
  expect(confirm).toHaveTextContent(path)
  await user.click(within(confirm).getByRole("button", { name: "Install" }))
}

describe("runtime availability", () => {
  it.each([
    [
      "not built",
      {
        runtime: {
          ...readyRuntime(),
          available: false,
          built: false,
          enabled: false,
          reason: "built without plugin support",
        },
      },
      "Plugins are not available: the agent was built without plugin support",
    ],
    [
      "not enabled",
      { runtime: { ...readyRuntime(), available: false, enabled: false } },
      "Plugins are not available: they are turned off in the agent",
    ],
    [
      "locked",
      { runtime: { ...readyRuntime(), available: false, reason: "the configuration is locked" } },
      "Plugins are not available right now",
    ],
    [
      "no agent",
      { runtimeError: { reason: "agentNotRunning", message: "the agent is not running" } },
      "Plugins are not available: the agent is not running",
    ],
    [
      "an old agent",
      { runtimeError: { reason: "agentTooOld", message: "protocol 9" } },
      "Plugins are not available: the running agent is too old",
    ],
  ] as const)("says why with %s and asks nothing else", async (_, options, title) => {
    const { client, user } = renderMarketplace(options)
    expect(await screen.findByText(title)).toBeInTheDocument()
    expect(screen.queryByRole("tab")).not.toBeInTheDocument()
    expect(client.listPlugins).not.toHaveBeenCalled()
    expect(client.listAvailablePlugins).not.toHaveBeenCalled()
    await user.click(screen.getByRole("button", { name: "Check again" }))
    await waitFor(() => {
      expect(client.getRuntime).toHaveBeenCalledTimes(2)
    })
  })

  it("installs but does not enable or run on an unsupported platform", async () => {
    const { user } = renderMarketplace({
      runtime: readyRuntime({ executionSupported: false, platform: "windows" }),
      helloInstalled: true,
    })
    expect(await screen.findByText("Plugins cannot run on Windows")).toBeInTheDocument()
    await openInstalled()
    const item = installedItem("Hello Weather")
    await user.click(item.getByRole("switch", { name: "Disable Hello Weather" }))
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith("Hello Weather disabled")
    })
    expect(
      installedItem("Hello Weather").getByRole("switch", { name: "Enable Hello Weather" }),
    ).toBeDisabled()
  })

  it("reports other failures with the backend's hint and retries", async () => {
    const { user, client } = renderMarketplace()
    vi.mocked(client.listPlugins).mockRejectedValueOnce(
      Object.assign(new Error("database is busy"), { reason: "storeUnavailable" }),
    )
    // A plain Error without the PluginError class still shows its message.
    expect(await screen.findByText("Plugins could not be loaded")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Try again" }))
    expect(await screen.findByRole("list", { name: "Recently added" })).toBeInTheDocument()
  })
})

describe("registry", () => {
  it.each([
    [{ freshness: "stale" as const }, "Registry stale"],
    [{ freshness: "neverVerified" as const }, "Registry not verified yet"],
  ])("pauses installs while the registry is %o", async (registry, title) => {
    const { user } = renderMarketplace({
      runtime: readyRuntime({ registry: { source: "/tmp/registry", ...registry } }),
    })
    expect(await screen.findByText(title)).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Install from file" })).toBeDisabled()
    await openDetails(user, "Home Lights")
    expect(screen.getByRole("button", { name: "Install Home Lights" })).toBeDisabled()
  })

  it("shows a registry that cannot be read, with Retry reading it again", async () => {
    const { user, client } = renderMarketplace({
      registryError: {
        reason: "registryUnavailable",
        message: "the timestamp signature is invalid",
      },
    })
    expect(await screen.findByText("The registry could not be read")).toBeInTheDocument()
    expect(screen.getByText("the timestamp signature is invalid")).toBeInTheDocument()
    expect(screen.getByText("No plugins to show")).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Retry" }))
    await waitFor(() => {
      expect(client.listAvailablePlugins).toHaveBeenLastCalledWith(true)
    })
  })

  it("says when no registry is configured", async () => {
    renderMarketplace({ runtime: { ...readyRuntime(), registry: undefined }, available: [] })
    expect(await screen.findByText("No plugin registry configured")).toBeInTheDocument()
  })
})

describe("discover and details", () => {
  it("shows Discover with its heading and categories: no banner, no Refresh, no search in the content", async () => {
    renderMarketplace()
    await discoverReady()
    expect(screen.getByText(/Add integrations, timers, games and more/)).toBeInTheDocument()
    expect(screen.getByRole("group", { name: "Categories" })).toBeInTheDocument()
    // Refreshing is automatic; a registry problem offers Retry where it is shown.
    expect(screen.queryByRole("button", { name: "Refresh" })).not.toBeInTheDocument()
    expect(screen.queryByRole("toolbar")).not.toBeInTheDocument()
    // The search is the sidebar's, not the content's.
    expect(screen.getAllByRole("searchbox")).toEqual([searchField()])
    // The experimental banner is gone; nothing else changed about consent or verification.
    expect(screen.queryByRole("note", { name: "Experimental feature" })).not.toBeInTheDocument()
    expect(screen.queryByText(/experimental/i)).not.toBeInTheDocument()
  })

  it("offers Install from file at the foot of the sidebar everywhere, never as the selected item", async () => {
    const { user, client } = renderMarketplace({ helloInstalled: true })
    await discoverReady()
    const footer = () =>
      within(marketplaceSidebar()).getByRole("button", { name: "Install from file" })
    const check = () => {
      expect(footer()).toBeEnabled()
      expect(footer()).not.toHaveAttribute("aria-current")
      expect(footer()).not.toHaveAttribute("data-active", "true")
      expect(activeNavItems()).not.toContain(footer())
    }
    check()
    for (const section of ["Installed", "Updates", "Categories", "Code"]) {
      await openSection(section)
      check()
    }
    await user.type(searchField(), "lights")
    await screen.findByRole("heading", { name: "Results for “lights”" })
    check()
    await openDetails(user, "Home Lights")
    check()
    // It opens the existing flow: the native picker, then the confirmation.
    vi.mocked(client.pickPackage).mockResolvedValueOnce("/tmp/a.dpplugin")
    await user.click(footer())
    expect(
      await screen.findByRole("alertdialog", { name: "Install the plugin package?" }),
    ).toBeInTheDocument()
  })

  it("searches the whole catalog from the sidebar once typing pauses", async () => {
    const { user } = renderMarketplace({ helloInstalled: true })
    await discoverReady()
    await user.type(searchField(), "weather")
    // Not on every keystroke: the results follow a short pause.
    expect(screen.queryByRole("heading", { name: "Results for “weather”" })).toBeNull()
    expect(
      await screen.findByRole("heading", { name: "Results for “weather”", level: 2 }),
    ).toBeInTheDocument()
    // The field keeps the focus while results arrive.
    expect(searchField()).toHaveFocus()
    expect(screen.getByRole("status")).toHaveTextContent("1 plugin")
    // Installed plugins are found too, with their state: Update.
    expect(results()).toHaveLength(1)
    expect(
      within(card("Hello Weather")).getByRole("button", { name: "Update Hello Weather" }),
    ).toBeEnabled()
    // By description and category as well.
    await user.clear(searchField())
    await user.type(searchField(), "fun")
    expect(await screen.findByRole("heading", { name: "Results for “fun”" })).toBeInTheDocument()
    expect(results()).toHaveLength(1)
    await user.clear(searchField())
    await user.type(searchField(), "nothing like this")
    expect(await screen.findByText("No plugins found")).toBeInTheDocument()
  })

  it("selects no section while results show, and Escape or × returns to the section where it was", async () => {
    const { user } = renderMarketplace({ helloInstalled: true })
    await discoverReady()
    await openSection("Installed")
    expect(activeNavItems().map((i) => i.textContent)).toEqual(["Installed"])
    await user.type(searchField(), "lights")
    await screen.findByRole("heading", { name: "Results for “lights”" })
    expect(activeNavItems()).toEqual([])
    // Escape in the field clears it: Installed again.
    await user.keyboard("{Escape}")
    expect(searchField()).toHaveValue("")
    expect(await screen.findByRole("heading", { name: "Installed", level: 2 })).toBeInTheDocument()
    expect(activeNavItems().map((i) => i.textContent)).toEqual(["Installed"])
    // The × button too.
    await user.type(searchField(), "lights")
    await screen.findByRole("heading", { name: "Results for “lights”" })
    await user.click(within(marketplaceSidebar()).getByRole("button", { name: "Clear search" }))
    expect(await screen.findByRole("heading", { name: "Installed", level: 2 })).toBeInTheDocument()
    expect(within(marketplaceSidebar()).queryByRole("button", { name: "Clear search" })).toBeNull()
  })

  it("comes back from a plugin's page to the same results", async () => {
    const { user } = renderMarketplace()
    await discoverReady()
    await user.type(searchField(), "home")
    await screen.findByRole("heading", { name: "Results for “home”" })
    await user.click(
      within(results()[0] as HTMLElement).getByRole("button", { name: "Home Lights" }),
    )
    await screen.findByRole("heading", { name: "Home Lights", level: 3 })
    await user.click(screen.getByRole("button", { name: "Back to plugins" }))
    expect(await screen.findByRole("heading", { name: "Results for “home”" })).toBeInTheDocument()
    expect(searchField()).toHaveValue("home")
    expect(results()).toHaveLength(1)
  })

  it("filters Discover by category and shows icon, developer, category and summary on each tile", async () => {
    const { user } = renderMarketplace({ helloInstalled: true })
    await discoverReady()
    const categories = screen.getByRole("group", { name: "Categories" })
    expect(
      within(categories)
        .getAllByRole("button")
        .map((b) => b.textContent),
    ).toEqual(["All categories", "Fun1", "Home1", "Weather1"])
    expect(within(categories).getByRole("button", { name: "All categories" })).toHaveAttribute(
      "aria-pressed",
      "true",
    )
    await user.click(within(categories).getByRole("button", { name: /^Weather/ }))
    expect(results()).toHaveLength(1)
    expect(screen.getByRole("status")).toHaveTextContent("1 plugin in Weather")
    const hello = card("Hello Weather")
    expect(hello).toHaveTextContent("Acme Tools")
    expect(within(hello).getByRole("img", { name: "Verified publisher" })).toBeInTheDocument()
    expect(hello).toHaveTextContent("Weather")
    expect(hello).toHaveTextContent("Greets someone and shows the temperature.")
    // The installed version's icon, as the agent sent it.
    expect(hello.querySelector("img")).toHaveAttribute("src", ICON)

    await user.click(within(categories).getByRole("button", { name: /^Home/ }))
    const lights = card("Home Lights")
    expect(lights).toHaveTextContent("knit")
    expect(within(lights).queryByRole("img", { name: "Verified publisher" })).toBeNull()
    // No icon from the agent: a glyph or the initial instead, never a remote image.
    expect(lights.querySelector("img")).toBeNull()
    expect(lights.querySelector('[data-slot="plugin-avatar"]')).toHaveTextContent("H")
    await user.click(screen.getByRole("button", { name: "Clear" }))
    expect(screen.queryByRole("list", { name: "Plugins" })).not.toBeInTheDocument()
  })

  it("keeps managing installed plugins under Installed, whatever Discover is filtered by", async () => {
    const { user } = renderMarketplace({ helloInstalled: true })
    await discoverReady()
    await user.click(
      within(screen.getByRole("group", { name: "Categories" })).getByRole("button", {
        name: /^Home/,
      }),
    )
    // Discover shows no management list, at most a strip leading to Installed.
    expect(screen.queryByRole("list", { name: "Installed plugins" })).not.toBeInTheDocument()
    await openInstalled()
    const item = installedItem("Hello Weather")
    expect(item.getByRole("switch", { name: "Disable Hello Weather" })).toBeEnabled()
    expect(item.getByRole("button", { name: "Update to 1.1.0" })).toBeEnabled()
    // Back in Discover, the category is still chosen.
    await openSection("Discover")
    expect(
      within(screen.getByRole("group", { name: "Categories" })).getByRole("button", {
        name: /^Home/,
      }),
    ).toHaveAttribute("aria-pressed", "true")
  })

  it("marks plugins that cannot be installed here, without an Install button", async () => {
    renderMarketplace()
    await discoverReady()
    expect(card("Future Keys")).toHaveTextContent("Incompatible")
    expect(within(card("Future Keys")).queryByRole("button", { name: /^Install/ })).toBeNull()
    expect(card("Gone Plugin")).toHaveTextContent("Unavailable")
    expect(within(card("Gone Plugin")).queryByRole("button", { name: /^Install/ })).toBeNull()
    expect(
      within(card("Home Lights")).getByRole("button", { name: "Install Home Lights" }),
    ).toBeEnabled()
  })

  it("lists the plugins not installed newest first, and nothing ranked by popularity", async () => {
    renderMarketplace()
    await discoverReady()
    // Only Hello Weather has publication dates (its first version counts); the rest by name.
    expect(cards().map((c) => c.querySelector("button")?.textContent)).toEqual([
      "Hello Weather",
      "Future Keys",
      "Gone Plugin",
      "Home Lights",
    ])
    expect(card("Hello Weather")).toHaveTextContent(/Added Dec 1, 2025/)
    expect(screen.queryByText(/popular|downloads|rating|featured/i)).not.toBeInTheDocument()
  })

  it("leads from one sidebar list to every section, with one selected item and counts only above 0", async () => {
    renderMarketplace({ helloInstalled: true })
    await discoverReady()
    expect(screen.queryByRole("tablist")).not.toBeInTheDocument()
    // One list, in this order, every entry the same kind of item; no section label.
    const lists = within(marketplaceSidebar()).getAllByRole("list")
    const nav = lists[0] as HTMLElement
    expect(
      within(nav)
        .getAllByRole("button")
        .map((b) => b.querySelector("span")?.textContent),
    ).toEqual([
      "Discover",
      "Installed",
      "Updates",
      "Categories",
      "Code",
      "Play",
      "Work",
      "Automate",
      "Monitor",
    ])
    const classes = new Set(
      within(nav)
        .getAllByRole("button")
        .map((b) => b.className.replace(/\s+/g, " ")),
    )
    // The same component and classes for every item; only data-active differs.
    expect(classes.size).toBe(1)
    expect(within(marketplaceSidebar()).queryByText("Collections")).not.toBeInTheDocument()
    expect(marketplaceSidebar()).toHaveTextContent("Installed1")
    expect(marketplaceSidebar()).toHaveTextContent("Updates1")

    const sections: [string, string][] = [
      ["Installed", "Installed"],
      ["Updates", "Updates"],
      ["Categories", "Categories"],
      ["Code", "Code"],
      ["Play", "Play"],
      ["Work", "Work"],
      ["Automate", "Automate"],
      ["Monitor", "Monitor"],
      ["Discover", "Discover plugins"],
    ]
    for (const [item, heading] of sections) {
      await openSection(item)
      expect(await screen.findByRole("heading", { name: heading, level: 2 })).toBeInTheDocument()
      expect(activeNavItems().map((i) => i.textContent)).toEqual([item])
      expect(navItem(item)).toHaveAttribute("aria-current", "page")
    }
    // Diagnostics live in Settings › Logs & Diagnostics.
    expect(screen.queryByRole("region", { name: "Diagnostics" })).not.toBeInTheDocument()
  })

  it("says when every plugin is up to date, and counts no updates", async () => {
    renderMarketplace()
    await discoverReady()
    expect(marketplaceSidebar()).not.toHaveTextContent(/Updates\d/)
    await openSection("Updates")
    expect(await screen.findByText("All plugins are up to date")).toBeInTheDocument()
    expect(screen.queryByRole("list", { name: "Available updates" })).not.toBeInTheDocument()
  })

  it("shows a collection from the category map, and an empty one plainly", async () => {
    const available = availablePlugins().map((p) =>
      p.id === HELLO ? { ...p, categories: ["monitoring"] } : p,
    )
    const { user } = renderMarketplace({ available })
    await discoverReady()
    await openSection("Monitor")
    const monitor = await screen.findByRole("list", { name: "Monitor" })
    expect(within(monitor).getAllByRole("listitem")).toHaveLength(1)
    expect(monitor).toHaveTextContent("Hello Weather")
    // No category of the fixtures maps to Play, and no name decides it.
    await openSection("Play")
    expect(await screen.findByText("Nothing here yet")).toBeInTheDocument()
    // It says what would fill it, invents nothing, and leads to Categories.
    expect(
      screen.getByText("Plugins in Media appear here as soon as the registry has some."),
    ).toBeInTheDocument()
    expect(screen.queryByRole("list", { name: "Play" })).not.toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Browse all categories" }))
    expect(
      await screen.findByText("Every category the registry uses. Choose one to see its plugins."),
    ).toBeInTheDocument()
  })

  it("opens the catalog filtered by a category from Categories", async () => {
    const { user } = renderMarketplace()
    await discoverReady()
    await openSection("Categories")
    const directory = await screen.findByRole("list", { name: "Categories" })
    expect(
      within(directory)
        .getAllByRole("listitem")
        .map((li) => li.querySelector("span span")?.textContent),
    ).toEqual(["Fun", "Home", "Weather"])
    await user.click(within(directory).getByRole("button", { name: /^Home/ }))
    expect(
      await screen.findByRole("heading", { name: "Discover plugins", level: 2 }),
    ).toBeInTheDocument()
    expect(screen.getByRole("status")).toHaveTextContent("1 plugin in Home")
    expect(results()).toHaveLength(1)
  })

  it("comes back from a plugin's page with the category and the scroll position", async () => {
    // The workspace that scrolls: here the document's body.
    let top = 0
    document.body.style.overflowY = "auto"
    Object.defineProperty(document.body, "scrollTop", {
      configurable: true,
      get: () => top,
      set: (value: number) => {
        top = value
      },
    })
    try {
      const { user } = renderMarketplace()
      await discoverReady()
      await user.click(
        within(screen.getByRole("group", { name: "Categories" })).getByRole("button", {
          name: /^Home/,
        }),
      )
      top = 420
      await user.click(
        within(results()[0] as HTMLElement).getByRole("button", { name: "Home Lights" }),
      )
      await screen.findByRole("heading", { name: "Home Lights", level: 3 })
      top = 0
      await user.click(screen.getByRole("button", { name: "Back to plugins" }))
      expect(
        within(screen.getByRole("group", { name: "Categories" })).getByRole("button", {
          name: /^Home/,
        }),
      ).toHaveAttribute("aria-pressed", "true")
      expect(results()).toHaveLength(1)
      expect(top).toBe(420)
    } finally {
      document.body.style.overflowY = ""
      Reflect.deleteProperty(document.body, "scrollTop")
    }
  })

  it("never offers DeckPilot's bundled-only plugins for installation", async () => {
    const available = [
      ...availablePlugins(),
      {
        id: "deckpilot.docker",
        name: "Docker",
        summary: "Containers on your keys.",
        publisher: "deckpilot",
        latest: "0.1.0",
        versions: [{ version: "0.1.0", permissions: [] }],
      },
    ]
    const { user } = renderMarketplace({ available })
    await discoverReady()
    const docker = card("Docker")
    expect(docker).toHaveTextContent("Included with DeckPilot")
    expect(within(docker).queryByRole("button", { name: /^Install/ })).toBeNull()
    // It is one of DeckPilot's official plugins.
    expect(docker).toHaveTextContent("Official")
    await openDetails(user, "Docker")
    expect(screen.queryByRole("button", { name: "Install Docker" })).toBeNull()
    expect(
      screen.getByText(
        "Comes with DeckPilot and is installed with it; it is not installed from the Marketplace.",
      ),
    ).toBeInTheDocument()
    expect(screen.getByRole("list", { name: "Verification" })).toHaveTextContent(
      "Made by DeckPilot and shipped only with DeckPilot itself.",
    )
  })

  it("shows a registry plugin in full, without inventing a changelog", async () => {
    const { user } = renderMarketplace()
    await openDetails(user, "Hello Weather")
    const article = screen.getByRole("article")
    expect(article).toHaveTextContent(
      "Greets someone by name and shows the current temperature of a city.",
    )
    expect(within(article).getByRole("img", { name: "Verified publisher" })).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Install Hello Weather" })).toBeEnabled()

    const about = screen.getByLabelText("About")
    expect(about).toHaveTextContent("Version1.1.0")
    expect(about).toHaveTextContent("RequiresDeckPilot >=0.5.0 <1.0.0")
    expect(about).toHaveTextContent("PlatformsmacOS, Windows and Linux")
    expect(about).toHaveTextContent("This computerCompatiblemacOS")

    const trust = screen.getByRole("list", { name: "Verification" })
    expect(trust).toHaveTextContent("Signed registry")
    expect(trust).toHaveTextContent(
      "Acme Tools is a publisher the registry verified (GitHub: acme).",
    )
    // Not an official plugin: no claim of DeckPilot's signatures.
    expect(trust).not.toHaveTextContent("Official")

    const links = screen.getByRole("list", { name: "Links" })
    expect(links).toHaveTextContent("Homepageacme.example/plugins")
    expect(links).toHaveTextContent("Sourcegithub.com/acme/plugins")

    const permissions = screen.getByRole("group", { name: "Permissions" })
    expect(permissions).toHaveTextContent("Publisher's reason: Reads the weather.")
    expect(permissions).toHaveTextContent("Publisher's reason: Says when it is done.")

    // Not installed: the registry does not list actions.
    expect(screen.getByRole("region", { name: "Actions" })).toHaveTextContent(
      "The registry does not list a plugin's actions",
    )
    const versions = screen.getByRole("list", { name: "Versions" })
    expect(within(versions).getAllByRole("listitem")).toHaveLength(2)
    expect(versions).toHaveTextContent("published")
    expect(versions).toHaveTextContent("Latest")
    expect(screen.queryByRole("heading", { name: "What’s new" })).not.toBeInTheDocument()
    expect(
      screen.getByText("The registry publishes no release notes for this plugin."),
    ).toBeInTheDocument()

    // The technical record stays folded until asked for.
    expect(screen.queryByRole("definition", { name: /Plugin ID/ })).not.toBeInTheDocument()
    const technical = screen.getByRole("button", { name: "Details" })
    expect(technical).toHaveAttribute("aria-expanded", "false")
    await user.click(technical)
    const record = screen.getByLabelText("Details", { selector: "dl" })
    expect(record).toHaveTextContent(`Plugin ID${HELLO}`)
    expect(record).toHaveTextContent("LicenseApache-2.0")
    expect(record).toHaveTextContent("CategoriesWeather, Fun")
  })

  it("shows release notes and screenshots when the registry sends them", async () => {
    const available = availablePlugins().map((p) =>
      p.id === HELLO
        ? {
            ...p,
            screenshots: [
              { src: ICON, caption: "The greeting key" },
              // Remote images are never loaded.
              { src: "https://tracker.example/shot.png" },
            ],
            versions: p.versions.map((v) =>
              v.version === "1.1.0" ? { ...v, releaseNotes: "Notifications when done." } : v,
            ),
          }
        : p,
    )
    const { user } = renderMarketplace({ available })
    await openDetails(user, "Hello Weather")
    expect(screen.getByRole("heading", { name: "What’s new" })).toBeInTheDocument()
    expect(screen.getAllByText("Notifications when done.")).toHaveLength(1)
    const gallery = screen.getByRole("list", { name: "Screenshots" })
    expect(within(gallery).getAllByRole("img")).toHaveLength(1)
    expect(within(gallery).getByRole("img", { name: "The greeting key" })).toHaveAttribute(
      "src",
      ICON,
    )
  })

  it("lists the installed version's actions and icon in the details", async () => {
    const { user } = renderMarketplace({ helloInstalled: true, helloState: "disabled" })
    await openDetails(user, "Hello Weather")
    const header = screen.getByRole("article").querySelector("header") as HTMLElement
    expect(header.querySelector("img")).toHaveAttribute("src", ICON)
    // The installed version's actions, each with Run (unavailable while disabled).
    const actions = screen.getByRole("region", { name: "Actions of Hello Weather" })
    expect(within(actions).getAllByRole("listitem")).toHaveLength(2)
    expect(actions).toHaveTextContent("the plugin is disabled")
    expect(within(actions).getByRole("button", { name: "Run Greet" })).toBeDisabled()
  })

  it("names the platforms a version needs on this computer", async () => {
    const { user } = renderMarketplace({
      runtime: readyRuntime({ platform: "windows", executionSupported: false }),
    })
    await openDetails(user, "Home Lights")
    const about = screen.getByLabelText("About")
    expect(about).toHaveTextContent("RequiresDeckPilot >=0.6.0")
    expect(about).toHaveTextContent("PlatformsmacOS and Linux")
    expect(about).toHaveTextContent("This computerCompatibleWindows")
  })

  it("explains incompatible and withdrawn versions in words, the agent's reasons under Details", async () => {
    const { user } = renderMarketplace()
    await openDetails(user, "Future Keys")
    expect(screen.queryByRole("button", { name: /^Install (?!from file)/ })).toBeNull()
    expect(screen.getByText("Incompatible")).toBeInTheDocument()
    expect(
      screen.getByText(
        "No version can be installed: every version was withdrawn or does not run here.",
      ),
    ).toBeInTheDocument()
    // The agent's own words are technical: folded away.
    expect(screen.queryByText("requires DeckPilot >=9.0.0")).not.toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Details" }))
    expect(screen.getByLabelText("Details", { selector: "dl" })).toHaveTextContent(
      "Reported by the agentrequires DeckPilot >=9.0.0",
    )
    await user.click(screen.getByRole("button", { name: "Back to plugins" }))
    await openDetails(user, "Gone Plugin")
    expect(screen.getByText("Version 1.0.0 was withdrawn")).toBeInTheDocument()
    expect(screen.queryByRole("button", { name: /^Install (?!from file)/ })).toBeNull()
  })
})

describe("plugin pages and permissions", () => {
  it("goes back from a plugin's page with Escape", async () => {
    const { user } = renderMarketplace()
    await openDetails(user, "Home Lights")
    await user.keyboard("{Escape}")
    expect(await discoverReady()).toBeInTheDocument()
  })

  it("says on an installed plugin's page where its copy came from, with its grants and status", async () => {
    const { user } = renderMarketplace({ helloInstalled: true, helloOrigin: { type: "bundled" } })
    await openDetails(user, "Hello Weather")
    const trust = screen.getByRole("list", { name: "Verification" })
    expect(trust).toHaveTextContent("Included with DeckPilot")
    // Its grants, with Revoke, from the agent's data.
    expect(
      screen.getByRole("button", {
        name: "Revoke: Send data to and receive data from https://api.open-meteo.com",
      }),
    ).toBeEnabled()
    expect(screen.getByRole("region", { name: "Status and resources" })).toHaveTextContent(
      "No failures since the agent started",
    )
  })

  it("never features a plugin because its own record says so (placements are the storefront's)", async () => {
    // A field a registry record might carry: ignored, as is any placement on a plugin.
    const available = availablePlugins().map((p) =>
      p.id === LIGHTS ? ({ ...p, featured: true, collections: ["play"] } as typeof p) : p,
    )
    renderMarketplace({ available })
    await discoverReady()
    expect(screen.queryByRole("list", { name: "Featured" })).not.toBeInTheDocument()
    expect(screen.queryByText(/featured/i)).not.toBeInTheDocument()
  })

  it("reviews and revokes every plugin's permissions in one place (Settings › Permissions)", async () => {
    const client = createFakePluginClient({ helloInstalled: true })
    const { user, context } = renderWithApp(
      <PluginClientContext value={client}>
        <PluginsSettings variant="permissions" />
      </PluginClientContext>,
    )
    const list = await screen.findByRole("list", { name: "Permissions of installed plugins" })
    expect(list).toHaveTextContent("Hello Weather")
    await user.click(
      within(list).getByRole("button", {
        name: "Revoke: Send data to and receive data from https://api.open-meteo.com",
      }),
    )
    await waitFor(() => {
      expect(client.revokePermission).toHaveBeenCalledWith(HELLO, "network:api.open-meteo.com")
    })
    // The plugin's page in the Marketplace has the same list.
    await user.click(screen.getByRole("button", { name: "Open Hello Weather in the Marketplace" }))
    expect(context.navigate).toHaveBeenCalledWith("marketplace")
    // …opened at that plugin.
    expect(takeFocusedPlugin()).toBe(HELLO)
  })
})

describe("install", () => {
  it("sends exactly the consented grant keys", async () => {
    const { user, client } = renderMarketplace()
    const heading = await openDetails(user, "Home Lights")
    expect(heading).toHaveFocus()
    await user.click(screen.getByRole("button", { name: "Install Home Lights" }))
    const dialog = screen.getByRole("dialog", { name: "Install Home Lights 0.3.0?" })
    const install = within(dialog).getByRole("button", { name: "Install" })
    expect(within(dialog).getAllByRole("checkbox")).toHaveLength(5)
    expect(install).toBeDisabled()
    await user.click(within(dialog).getByRole("checkbox", { name: /local network/ }))
    expect(dialog).toHaveTextContent("4 of 5 permissions still need your consent.")
    await user.click(within(dialog).getByRole("button", { name: "Allow all 5" }))
    await user.click(install)
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
    })
    expect(client.installPlugin).toHaveBeenCalledWith(
      { type: "registry", plugin: LIGHTS, version: "0.3.0" },
      [
        "network:api.lifx.com",
        "network:private-network",
        "credentials:token",
        "device:state",
        "device:icons",
      ],
    )
    expect(toast.success).toHaveBeenCalledWith("Home Lights installed")
    // Installed (disabled until enabled with consent), as the agent now lists it.
    expect(await screen.findByRole("button", { name: "Enable Home Lights" })).toBeEnabled()
    expect(screen.getByLabelText("About")).toHaveTextContent("Version0.3.0Disabled")
  })

  it("installs nothing when cancelled", async () => {
    const { user, client } = renderMarketplace()
    await openDetails(user, "Home Lights")
    await user.click(screen.getByRole("button", { name: "Install Home Lights" }))
    const dialog = screen.getByRole("dialog")
    expect(dialog).toContainElement(document.activeElement as HTMLElement)
    await user.keyboard("{Escape}")
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
    expect(client.installPlugin).not.toHaveBeenCalled()
  })

  it("shows a refusal in the dialog with the backend's hint", async () => {
    const { user, client } = renderMarketplace()
    vi.mocked(client.installPlugin).mockRejectedValueOnce(
      Object.assign(new (await import("./api")).PluginError("revoked", "the version was revoked")),
    )
    await openDetails(user, "Hello Weather")
    await user.click(screen.getByRole("button", { name: "Install Hello Weather" }))
    const dialog = screen.getByRole("dialog")
    await user.click(within(dialog).getByRole("button", { name: "Allow all 2" }))
    await user.click(within(dialog).getByRole("button", { name: "Install" }))
    // Title, the agent's message and the hint, as one inline error.
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "The version was withdrawn: the version was revoked The registry withdrew this version; choose another one or uninstall it.",
    )
  })

  it("installs a local package file after confirmation, granting nothing, then shows its permissions", async () => {
    const { user, client } = renderMarketplace()
    await installFile(user, client, "/tmp/acme.hello-weather-1.0.0.dpplugin")
    await waitFor(() => {
      expect(client.installPlugin).toHaveBeenCalledWith(
        { type: "file", path: "/tmp/acme.hello-weather-1.0.0.dpplugin" },
        [],
      )
    })
    expect(toast.success).toHaveBeenCalledWith(
      "Hello Weather 1.0.0 installed from the package file",
    )
    // Its permissions, for consent before it can run; cancelling keeps it
    // installed, disabled and without grants.
    const review = await screen.findByRole("dialog", { name: "Enable Hello Weather?" })
    expect(review).toHaveTextContent("https://api.open-meteo.com")
    expect(review).toHaveTextContent("Publisher's reason: Reads the weather.")
    await user.click(within(review).getByRole("button", { name: "Cancel" }))
    expect(client.enablePlugin).not.toHaveBeenCalled()
    await openInstalled()
    const item = installedItem("Hello Weather")
    expect(item.getByText("Local package")).toBeInTheDocument()
    expect(item.getByText("Disabled")).toBeInTheDocument()
    expect(item.getByText(/1 permission not granted/)).toBeInTheDocument()
  })

  it("installs nothing when the package confirmation is cancelled", async () => {
    const { user, client } = renderMarketplace()
    vi.mocked(client.pickPackage).mockResolvedValueOnce("/tmp/acme.hello-weather-1.0.0.dpplugin")
    await user.click(await screen.findByRole("button", { name: "Install from file" }))
    const confirm = await screen.findByRole("alertdialog", { name: "Install the plugin package?" })
    await user.click(within(confirm).getByRole("button", { name: "Cancel" }))
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument()
    expect(client.installPlugin).not.toHaveBeenCalled()
  })

  it("reports a package the registry does not pin in plain words", async () => {
    const { user, client } = renderMarketplace()
    await installFile(user, client, "/tmp/other.dpplugin")
    await waitFor(() => {
      expect(toast.error).toHaveBeenCalledWith("The package was refused", {
        description:
          "the package is not a version the registry pins Only package files whose bytes the verified registry pins can be installed.",
      })
    })
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
  })

  it.each([
    ["incompatible", "Not compatible with this DeckPilot"],
    ["unsupportedRuntime", "Unsupported kind of plugin"],
    ["registryUnavailable", "The registry is not available"],
    ["alreadyInstalled", "The plugin is already installed"],
    ["storeUnavailable", "The plugin database cannot be opened"],
    ["somethingNew", "The request failed"],
  ])("words the install refusal %s for people", async (reason, title) => {
    const { user, client } = renderMarketplace()
    vi.mocked(client.installPlugin).mockRejectedValueOnce(new PluginError(reason, "details"))
    await openDetails(user, "Home Lights")
    await user.click(screen.getByRole("button", { name: "Install Home Lights" }))
    const dialog = screen.getByRole("dialog")
    await user.click(within(dialog).getByRole("button", { name: "Allow all 5" }))
    await user.click(within(dialog).getByRole("button", { name: "Install" }))
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(`${title}: details`)
    // The dialog stays open: nothing was installed.
    expect(within(dialog).getByRole("button", { name: "Install" })).toBeEnabled()
  })

  it("does nothing when the file dialog is cancelled", async () => {
    const { user, client } = renderMarketplace()
    await user.click(await screen.findByRole("button", { name: "Install from file" }))
    await waitFor(() => {
      expect(client.pickPackage).toHaveBeenCalled()
    })
    expect(client.installPlugin).not.toHaveBeenCalled()
  })

  it("shows the agent's progress and reloads when a plugin changed", async () => {
    const { client } = renderMarketplace()
    await discoverReady()
    act(() => {
      client.emit({ type: "progress", plugin: HELLO, operation: "install", stage: "verifying" })
    })
    expect(screen.getByRole("status", { name: "Plugin operations" })).toHaveTextContent(
      // By name: the page knows the plugin from the registry.
      "Installing Hello Weather: verifying signatures and pins",
    )
    act(() => {
      client.emit({ type: "progress", plugin: HELLO, operation: "install", stage: "finished" })
    })
    expect(screen.queryByRole("status", { name: "Plugin operations" })).not.toBeInTheDocument()
    const calls = vi.mocked(client.listPlugins).mock.calls.length
    act(() => {
      client.emit({ type: "changed", plugin: HELLO, state: "disabled" })
    })
    await waitFor(() => {
      expect(vi.mocked(client.listPlugins).mock.calls.length).toBe(calls + 1)
    })
  })
})

describe("install states", () => {
  /** Consents to every permission of Home Lights and confirms its install. */
  async function consentAndInstall(user: ReturnType<typeof renderMarketplace>["user"]) {
    await user.click(screen.getByRole("button", { name: "Install Home Lights" }))
    const dialog = screen.getByRole("dialog", { name: "Install Home Lights 0.3.0?" })
    await user.click(within(dialog).getByRole("button", { name: "Allow all 5" }))
    await user.click(within(dialog).getByRole("button", { name: "Install" }))
    return dialog
  }

  it("shows Installing… while the agent installs, and Installed only once it lists the plugin", async () => {
    const { user, client } = renderMarketplace()
    const real = client.installPlugin.getMockImplementation()
    if (!real) throw new Error("the fake installs plugins")
    let finish: () => void = () => undefined
    client.installPlugin.mockImplementationOnce(
      (source, consent) =>
        new Promise((resolve, reject) => {
          finish = () => {
            real(source, consent).then(resolve, reject)
          }
        }),
    )
    await openDetails(user, "Home Lights")
    await consentAndInstall(user)
    // Behind the consent dialog, which waits for the agent too.
    const installing = await screen.findByRole("button", { name: /Installing…/, hidden: true })
    expect(installing).toBeDisabled()
    expect(
      within(screen.getByRole("article", { hidden: true })).queryByText("Installed"),
    ).not.toBeInTheDocument()
    expect(
      screen.queryByRole("button", { name: "Enable Home Lights", hidden: true }),
    ).not.toBeInTheDocument()
    act(() => {
      finish()
    })
    expect(await screen.findByRole("button", { name: "Enable Home Lights" })).toBeEnabled()
    expect(screen.queryByRole("button", { name: /Installing…/ })).not.toBeInTheDocument()
  })

  it("shows a failed install with the reason in words and Retry, which asks for consent again", async () => {
    const { user, client } = renderMarketplace()
    client.installPlugin.mockRejectedValueOnce(new PluginError("registryUnavailable", "offline"))
    await openDetails(user, "Home Lights")
    const dialog = await consentAndInstall(user)
    // The consent dialog says it first, and keeps everything as it was.
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "The registry is not available: offline",
    )
    await user.keyboard("{Escape}")
    const failure = await screen.findByRole("alert")
    expect(failure).toHaveTextContent("Couldn’t install. The registry is not available: offline")
    expect(screen.queryByRole("button", { name: "Install Home Lights" })).not.toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Try installing Home Lights again" }))
    // The same consent as the first time: nothing is installed without it.
    const again = screen.getByRole("dialog", { name: "Install Home Lights 0.3.0?" })
    expect(within(again).getByRole("button", { name: "Install" })).toBeDisabled()
    expect(screen.queryByText(/Couldn’t install/)).not.toBeInTheDocument()
    await user.click(within(again).getByRole("button", { name: "Allow all 5" }))
    await user.click(within(again).getByRole("button", { name: "Install" }))
    expect(await screen.findByRole("button", { name: "Enable Home Lights" })).toBeEnabled()
    expect(client.installPlugin).toHaveBeenCalledTimes(2)
  })

  it("keeps a failed install visible on its tile in the catalog", async () => {
    const { user, client } = renderMarketplace()
    client.installPlugin.mockRejectedValueOnce(new PluginError("storeUnavailable", "locked"))
    await openDetails(user, "Home Lights")
    await consentAndInstall(user)
    await within(screen.getByRole("dialog")).findByRole("alert")
    await user.keyboard("{Escape}")
    await user.click(screen.getByRole("button", { name: "Back to plugins" }))
    const tile = card("Home Lights")
    expect(within(tile).getByRole("alert")).toHaveTextContent(
      "Couldn’t install. The plugin database cannot be opened: locked",
    )
    expect(
      within(tile).getByRole("button", { name: "Try installing Home Lights again" }),
    ).toBeEnabled()
  })

  it("shows a failed update with Retry, and updates on retry", async () => {
    const { user, client } = renderMarketplace({ helloInstalled: true })
    client.updatePlugin
      .mockRejectedValueOnce(new PluginError("registryUnavailable", "offline"))
      .mockResolvedValueOnce({
        plugin: HELLO,
        outcome: {
          type: "applied",
          version: "1.1.0",
          changes: { added: [], broadened: [], removed: [], escalation: false },
        },
      })
    await discoverReady()
    // Installed plugins are in the search results too.
    await user.type(searchField(), "weather")
    await screen.findByRole("heading", { name: "Results for “weather”" })
    await user.click(
      within(card("Hello Weather")).getByRole("button", { name: "Update Hello Weather" }),
    )
    expect(await within(card("Hello Weather")).findByRole("alert")).toHaveTextContent(
      "Couldn’t update. The registry is not available: offline",
    )
    expect(toast.error).toHaveBeenCalledWith("The registry is not available", expect.anything())
    await user.click(
      within(card("Hello Weather")).getByRole("button", {
        name: "Try updating Hello Weather again",
      }),
    )
    await waitFor(() => {
      expect(client.updatePlugin).toHaveBeenCalledTimes(2)
    })
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith("Hello Weather updated to 1.1.0")
    })
    expect(within(card("Hello Weather")).queryByRole("alert")).not.toBeInTheDocument()
  })
})

describe("your plugins", () => {
  it("separates all, disabled and incompatible plugins", async () => {
    const { user } = renderMarketplace({ helloInstalled: true, helloState: "disabled" })
    await openInstalled()
    const views = screen.getByRole("group", { name: "Installed plugins to show" })
    expect(
      within(views)
        .getAllByRole("button")
        .map((b) => b.textContent),
    ).toEqual(["All1", "Disabled1", "Incompatible0"])
    await user.click(within(views).getByRole("button", { name: /^Disabled/ }))
    expect(installedItem("Hello Weather").getByText("Disabled")).toBeInTheDocument()
    await user.click(within(views).getByRole("button", { name: /^Incompatible/ }))
    expect(screen.queryByRole("list", { name: "Installed plugins" })).not.toBeInTheDocument()
    expect(screen.getByText("Everything is compatible")).toBeInTheDocument()
  })

  it("lists a broken plugin as incompatible", async () => {
    const { user } = renderMarketplace({ helloInstalled: true, helloState: "broken" })
    await openInstalled()
    const views = screen.getByRole("group", { name: "Installed plugins to show" })
    await user.click(within(views).getByRole("button", { name: /^Disabled/ }))
    expect(screen.getByText("No disabled plugins")).toBeInTheDocument()
    await user.click(within(views).getByRole("button", { name: /^Incompatible/ }))
    expect(installedItem("Hello Weather").getByText("Hello Weather is broken")).toBeInTheDocument()
  })
})

describe("installed plugins", () => {
  it("enables with consent to every declared permission", async () => {
    const { user, client } = renderMarketplace()
    await installFile(user, client, "/tmp/acme.hello-weather-1.0.0.dpplugin")
    await user.click(
      within(await screen.findByRole("dialog", { name: "Enable Hello Weather?" })).getByRole(
        "button",
        { name: "Cancel" },
      ),
    )
    await openInstalled()
    expect(installedItem("Hello Weather").getByText(/1 permission not granted/)).toBeInTheDocument()
    await user.click(
      installedItem("Hello Weather").getByRole("switch", { name: "Enable Hello Weather" }),
    )
    const dialog = screen.getByRole("dialog", { name: "Enable Hello Weather?" })
    const enable = within(dialog).getByRole("button", { name: "Enable" })
    expect(enable).toBeDisabled()
    await user.click(within(dialog).getByRole("checkbox", { name: /api\.open-meteo\.com/ }))
    await user.click(enable)
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith("Hello Weather enabled")
    })
    expect(client.enablePlugin).toHaveBeenCalledWith(HELLO, ["network:api.open-meteo.com"])
  })

  it("runs actions, revokes, sees the refusal, grants again and disables", async () => {
    const { user, client } = renderMarketplace({ helloInstalled: true })
    // On the plugin's page: its actions with Run, and its permissions.
    await openDetails(user, "Hello Weather")
    const item = () => within(screen.getByRole("article"))

    await user.type(item().getByLabelText("Name"), "Ada")
    await user.click(item().getByRole("button", { name: "Run Greet" }))
    expect(await item().findByRole("status", { name: "Result of Greet" })).toHaveTextContent(
      "Succeeded: Hello, Ada.",
    )
    expect(client.invokeAction).toHaveBeenCalledWith(HELLO, "greet", { name: "Ada" })

    await user.click(
      item().getByRole("button", {
        name: "Revoke: Send data to and receive data from https://api.open-meteo.com",
      }),
    )
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith("Permission revoked")
    })
    expect(client.revokePermission).toHaveBeenCalledWith(HELLO, "network:api.open-meteo.com")
    await user.click(item().getByRole("button", { name: "Run Current temperature" }))
    expect(
      await item().findByRole("status", { name: "Result of Current temperature" }),
    ).toHaveTextContent("Failed (not-permitted)")

    await user.click(
      item().getByRole("button", {
        name: "Grant: Send data to and receive data from https://api.open-meteo.com",
      }),
    )
    await waitFor(() => {
      expect(client.grantPermissions).toHaveBeenCalledWith(HELLO, ["network:api.open-meteo.com"])
    })

    // Disabling is the row's, under Installed (Back returns there).
    await user.click(screen.getByRole("button", { name: "Back to plugins" }))
    await user.click(
      installedItem("Hello Weather").getByRole("switch", { name: "Disable Hello Weather" }),
    )
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith("Hello Weather disabled")
    })
    await openDetails(user, "Hello Weather")
    expect(item().getByRole("button", { name: "Run Greet" })).toBeDisabled()
  })

  it("reports a refused action with its reason", async () => {
    const { user, client } = renderMarketplace({ helloInstalled: true })
    vi.mocked(client.invokeAction).mockRejectedValueOnce(
      new (await import("./api")).PluginError("hostFailed", "the host crashed"),
    )
    await openDetails(user, "Hello Weather")
    await user.click(screen.getByRole("button", { name: "Run Greet" }))
    expect(await screen.findByRole("status", { name: "Result of Greet" })).toHaveTextContent(
      "The plugin host failed: the host crashed The plugin host failed",
    )
  })

  it("shows the host's crashes and its last error", async () => {
    renderMarketplace({
      helloInstalled: true,
      helloHealth: {
        crashes: 2,
        lastError: "the plugin host stopped during the action (Trap: unreachable)",
        lastErrorAt: 1_767_225_600,
      },
    })
    await openInstalled()
    const health = installedItem("Hello Weather").getByRole("group", {
      name: "Health of Hello Weather",
    })
    expect(health).toHaveTextContent("The plugin host stopped 2 times since the agent started")
    expect(health).toHaveTextContent(
      "the plugin host stopped during the action (Trap: unreachable)",
    )
    expect(health).toHaveTextContent("Last error (")
  })

  it("shows a sandbox refusal as the last error without a crash", async () => {
    renderMarketplace({
      helloInstalled: true,
      helloHealth: {
        crashes: 0,
        lastError: "the plugin host's sandbox is not full (windows-job-lowil: partial)",
      },
    })
    await openInstalled()
    const health = installedItem("Hello Weather").getByRole("group", {
      name: "Health of Hello Weather",
    })
    expect(health).toHaveTextContent("The last action of this plugin failed")
    expect(health).toHaveTextContent("windows-job-lowil: partial")
  })

  it("shows no health notice for a healthy plugin", async () => {
    renderMarketplace({ helloInstalled: true })
    await openInstalled()
    expect(
      installedItem("Hello Weather").queryByRole("group", { name: "Health of Hello Weather" }),
    ).not.toBeInTheDocument()
  })

  it.each([
    ["broken", "Hello Weather is broken", "assets/icon-128.png was modified"],
    ["quarantined", "Disabled: revoked by the registry", "revoked: it leaked data"],
  ] as const)(
    "shows a %s plugin with its reason and cannot enable it",
    async (state, title, why) => {
      renderMarketplace({
        helloInstalled: true,
        helloState: state,
        helloStateReason: why,
      })
      await openInstalled()
      const item = installedItem("Hello Weather")
      expect(item.getByText(title)).toBeInTheDocument()
      expect(item.getByText(why)).toBeInTheDocument()
      expect(
        item.getByText("It cannot run or be enabled. Roll back, update or uninstall it."),
      ).toBeInTheDocument()
      expect(item.getByRole("switch", { name: "Enable Hello Weather" })).toBeDisabled()
      expect(item.getByRole("button", { name: "Uninstall" })).toBeEnabled()
    },
  )

  it("says on each plugin that it cannot run on this platform", async () => {
    renderMarketplace({
      runtime: readyRuntime({
        executionSupported: false,
        platform: "windows",
        reason: "no verified sandbox on windows",
      }),
      helloInstalled: true,
    })
    await openInstalled()
    const item = installedItem("Hello Weather")
    expect(item.getByText("Cannot run on Windows")).toBeInTheDocument()
    expect(item.getByText("no verified sandbox on windows")).toBeInTheDocument()
  })

  it("shows compatibility, the icon and the registry's newest version", async () => {
    renderMarketplace({ helloInstalled: true })
    await openInstalled()
    const item = installedItem("Hello Weather")
    const li = item.getByText("v1.0.0").closest("li") as HTMLElement
    expect(li.querySelector("img")).toHaveAttribute("src", ICON)
    expect(li).toHaveTextContent("DeckPilot >=0.5.0 <1.0.0")
    expect(li).toHaveTextContent("Latest in the registry: 1.1.0")
    expect(li).toHaveTextContent("by Acme Tools")
  })

  it("reads the plugins again after an action, so its health is current", async () => {
    const { user, client } = renderMarketplace({ helloInstalled: true })
    await openDetails(user, "Hello Weather")
    const calls = vi.mocked(client.listPlugins).mock.calls.length
    await user.click(screen.getByRole("button", { name: "Run Greet" }))
    await waitFor(() => {
      expect(vi.mocked(client.listPlugins).mock.calls.length).toBe(calls + 1)
    })
  })

  it("uninstalls after confirmation", async () => {
    const { user, client } = renderMarketplace({ helloInstalled: true })
    await openInstalled()
    await user.click(installedItem("Hello Weather").getByRole("button", { name: "Uninstall" }))
    const confirm = screen.getByRole("alertdialog", { name: "Uninstall Hello Weather?" })
    await user.click(within(confirm).getByRole("button", { name: "Uninstall" }))
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith("Hello Weather uninstalled")
    })
    expect(client.uninstallPlugin).toHaveBeenCalledWith(HELLO)
    expect(await screen.findByText("No plugins installed")).toBeInTheDocument()
  })

  it("reports a failed request as a toast with the reason's hint", async () => {
    const { user, client } = renderMarketplace({ helloInstalled: true })
    vi.mocked(client.disablePlugin).mockRejectedValueOnce(
      new (await import("./api")).PluginError("locked", "the configuration is locked"),
    )
    await openInstalled()
    await user.click(
      installedItem("Hello Weather").getByRole("switch", { name: "Disable Hello Weather" }),
    )
    await waitFor(() => {
      expect(toast.error).toHaveBeenCalledWith("DeckPilot is locked", {
        description: "the configuration is locked Unlock DeckPilot, then try again.",
      })
    })
  })
})

describe("updates", () => {
  it("stages an escalation, approves it with exactly the new keys and rolls back", async () => {
    const { user, client } = renderMarketplace({ helloInstalled: true })
    await discoverReady()
    await openSection("Updates")
    const row = within(
      await screen.findByRole("listitem", { name: /Hello Weather 1.0.0 to 1.1.0/ }),
    )
    await user.click(row.getByRole("button", { name: "Update" }))
    expect(client.updatePlugin).toHaveBeenCalledWith(HELLO, "1.1.0", [])
    const dialog = await screen.findByRole("dialog", { name: "Update Hello Weather to 1.1.0?" })
    expect(within(dialog).getAllByRole("checkbox")).toHaveLength(1)
    const update = within(dialog).getByRole("button", { name: "Update" })
    expect(update).toBeDisabled()
    await user.click(within(dialog).getByRole("checkbox", { name: /Show notifications/ }))
    await user.click(update)
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith("Hello Weather updated to 1.1.0")
    })
    expect(client.approveUpdate).toHaveBeenCalledWith(HELLO, ["notifications"])
    // Updated: nothing left under Updates; rolling back is Installed's.
    expect(await screen.findByText("All plugins are up to date")).toBeInTheDocument()
    await openInstalled()
    await user.click(
      installedItem("Hello Weather").getByRole("button", { name: "Roll back to 1.0.0" }),
    )
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith("Hello Weather rolled back to 1.0.0")
    })
    expect(client.rollbackPlugin).toHaveBeenCalledWith(HELLO)
  })

  it("keeps a cancelled escalation staged and declines it later", async () => {
    const { user, client } = renderMarketplace({ helloInstalled: true })
    await discoverReady()
    await openSection("Updates")
    await user.click(await screen.findByRole("button", { name: "Update" }))
    await user.click(
      within(await screen.findByRole("dialog")).getByRole("button", { name: "Cancel" }),
    )
    const row = within(await screen.findByRole("listitem", { name: /Hello Weather 1.0.0 to/ }))
    expect(row.getByText("Waiting for consent: 1 permission")).toBeInTheDocument()
    await user.click(row.getByRole("button", { name: "Review and update" }))
    const dialog = screen.getByRole("dialog", { name: "Update Hello Weather to 1.1.0?" })
    await user.click(within(dialog).getByRole("button", { name: "Decline update" }))
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith("Update of Hello Weather declined")
    })
    expect(client.declineUpdate).toHaveBeenCalledWith(HELLO)
    expect(client.approveUpdate).not.toHaveBeenCalled()
  })

  it("applies an update that asks for nothing new without a dialog", async () => {
    const { user, client } = renderMarketplace({ helloInstalled: true })
    vi.mocked(client.updatePlugin).mockResolvedValueOnce({
      plugin: HELLO,
      outcome: {
        type: "applied",
        version: "1.1.0",
        changes: { added: [], broadened: [], removed: [], escalation: false },
      },
    })
    await discoverReady()
    await openSection("Updates")
    await user.click(await screen.findByRole("button", { name: "Update" }))
    await waitFor(() => {
      expect(toast.success).toHaveBeenCalledWith("Hello Weather updated to 1.1.0")
    })
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
  })
})
