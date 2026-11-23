// Driving the Marketplace in tests as a user does: its sidebar (sections,
// collections, search, Install from file), its catalog lists and the rows
// of Installed.

import { fireEvent, screen, within } from "@testing-library/react"
import type { UserEvent } from "@testing-library/user-event"

/** The Marketplace's own sidebar. */
export function marketplaceSidebar(): HTMLElement {
  const sidebar = document.querySelector<HTMLElement>('[data-slot="marketplace-sidebar"]')
  if (!sidebar) throw new Error("the Marketplace's sidebar is not shown")
  return sidebar
}

/** A destination of the sidebar by its label (Discover, Installed, Code, …). */
export function navItem(label: string): HTMLElement {
  return within(marketplaceSidebar()).getByRole("button", { name: label })
}

/** The sidebar items that look selected. */
export function activeNavItems(): HTMLElement[] {
  return [
    ...marketplaceSidebar().querySelectorAll<HTMLElement>(
      '[data-sidebar="menu-button"][data-active="true"]',
    ),
  ]
}

/** The Marketplace has loaded (Discover's heading). */
export function discoverReady(name = "Discover plugins") {
  return screen.findByRole("heading", { name, level: 2 })
}

/** Opens a section of the sidebar. */
export async function openSection(label: string) {
  fireEvent.click(await within(marketplaceSidebar()).findByRole("button", { name: label }))
}

/** Installed, with its list of plugins. */
export async function openInstalled(label = "Installed", list = "Installed plugins") {
  await screen.findByRole("heading", { level: 2 })
  await openSection(label)
  return screen.findByRole("list", { name: list })
}

/** A row of Installed by its plugin's name. */
export function installedItem(name: string, list = "Installed plugins") {
  return within(
    within(screen.getByRole("list", { name: list }))
      .getByText(name, { selector: "span" })
      .closest("li") as HTMLElement,
  )
}

/** The sidebar's search field. */
export function searchField(label = "Search plugins"): HTMLElement {
  return within(marketplaceSidebar()).getByRole("searchbox", { name: label })
}

/** The catalog lists on screen: results, then Recently added and others. */
export function catalogLists(names = ["Plugins", "Recently added", "Featured"]): HTMLElement[] {
  return names.flatMap((name) => screen.queryAllByRole("list", { name }))
}

/** A tile of the catalog by its plugin's name. */
export function card(name: string, names?: string[]): HTMLElement {
  return catalogLists(names)
    .flatMap((list) => within(list).getAllByRole("listitem"))
    .find((c) => c.textContent.includes(name)) as HTMLElement
}

/**
 * Opens a plugin's page as a user would: from its tile on screen, else
 * (installed) from its row in Installed, else by searching for it.
 */
export async function openDetails(
  user: UserEvent,
  name: string,
  labels: { details: string; installed: string; list: string; search: string } = {
    details: `Details of ${name}`,
    installed: "Installed",
    list: "Installed plugins",
    search: "Search plugins",
  },
) {
  await screen.findByRole("heading", { level: 2 })
  const tile = () =>
    catalogLists()
      .map((list) => within(list).queryByRole("button", { name }))
      .find(Boolean)
  const shown = tile()
  if (shown) {
    await user.click(shown)
    return screen.findByRole("heading", { name, level: 3 })
  }
  await openSection(labels.installed)
  const installed = screen.queryByRole("list", { name: labels.list })
  const row = installed
    ? within(installed).queryByText(name, { selector: "span" })?.closest("li")
    : null
  if (row) {
    await user.click(within(row as HTMLElement).getByRole("button", { name: labels.details }))
    return screen.findByRole("heading", { name, level: 3 })
  }
  await user.type(searchField(labels.search), name)
  const results = await screen.findByRole("list", { name: "Plugins" })
  await user.click(within(results).getByRole("button", { name }))
  return screen.findByRole("heading", { name, level: 3 })
}
