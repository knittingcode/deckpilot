// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { NotificationsButton } from "@/components/notifications"
import { AppStatusBar } from "@/components/status-bar"
import { CoreStatusItems } from "@/components/status-sources"
import { StatusContext, createStatusStores } from "@/hooks/use-status"
import { i18n } from "@/i18n"
import { NOTIFICATION_EVENT } from "@/lib/api"
import { AppContext } from "@/lib/app-context"
import { SecurityContext } from "@/lib/security-context"
import type { ActionNotification, SecurityView } from "@/lib/types"
import { installDomStubs } from "@/test/dom"
import { agentConnection, agentStatus } from "@/test/fixtures"
import { appContext } from "@/test/render"
import { emitBackend, resetBackend } from "@/test/tauri"

vi.mock("@tauri-apps/api/event", async () => (await import("@/test/tauri")).eventModule)

const security = {
  view: { problem: null, legacyCopies: [] } as unknown as SecurityView,
  setView: vi.fn(),
  refresh: vi.fn(),
}

beforeAll(installDomStubs)
beforeEach(resetBackend)
afterEach(cleanup)

async function renderStatusBar(agent = agentConnection()) {
  const stores = createStatusStores()
  render(
    <AppContext value={appContext({ agent })}>
      <SecurityContext value={security}>
        <StatusContext value={stores}>
          <AppStatusBar stores={stores} />
          <NotificationsButton store={stores.notifications} />
          <CoreStatusItems />
        </StatusContext>
      </SecurityContext>
    </AppContext>,
  )
  // Listeners register asynchronously.
  await waitFor(() => {
    expect(emitBackend(NOTIFICATION_EVENT, { statusBar: false })).toBeGreaterThan(0)
  })
  return stores
}

function notify(notification: Partial<ActionNotification>) {
  act(() => {
    emitBackend(NOTIFICATION_EVENT, {
      title: "Notice",
      severity: "info",
      desktop: false,
      statusBar: true,
      ...notification,
    })
  })
}

describe("toolbar notifications", () => {
  it("counts unread notifications, lists them newest first and marks them read", async () => {
    const user = userEvent.setup()
    await renderStatusBar()
    expect(screen.getByRole("button", { name: "Notifications" })).toBeInTheDocument()

    notify({ title: "Deploy finished", body: "3 nodes succeeded", severity: "success" })
    notify({ title: "Disk almost full", severity: "warning" })
    notify({ title: "Backup failed", severity: "error" })
    notify({ title: "Desktop only", severity: "error", statusBar: false })

    const bell = screen.getByRole("button", { name: "Notifications, 3 unread" })
    await user.click(bell)
    const list = within(screen.getByRole("list", { name: "Notifications" }))
    const items = list.getAllByRole("listitem")
    expect(items.map((li) => li.querySelector(".font-medium")?.textContent)).toEqual([
      "Backup failed",
      "Disk almost full",
      "Deploy finished",
    ])
    // Severity is conveyed in words, not only by color; success is shown as info.
    expect(within(items[0] as HTMLElement).getByLabelText("Error")).toBeInTheDocument()
    expect(within(items[2] as HTMLElement).getByLabelText("Information")).toBeInTheDocument()
    expect(list.getByText("3 nodes succeeded")).toBeInTheDocument()

    await user.keyboard("{Escape}")
    const read = screen.getByRole("button", { name: "Notifications" })
    expect(read).toHaveFocus()
  })

  it("says once when the agent has no tray icon, and how to stop it", async () => {
    const user = userEvent.setup()
    const stores = await renderStatusBar(
      agentConnection({ status: agentStatus({ protocol: 22, tray: "noHost" }) }),
    )
    await user.click(screen.getByRole("button", { name: "Notifications, 1 unread" }))
    const list = within(screen.getByRole("list", { name: "Notifications" }))
    expect(list.getByText("No tray icon")).toBeInTheDocument()
    expect(
      list.getByText(
        "DeckPilot keeps running in the background. Stop it in Settings › Background Agent.",
      ),
    ).toBeInTheDocument()
    expect(stores.notifications.getSnapshot()).toHaveLength(1)
  })

  it.each(["shown", "hostUnknown", "disabled", undefined] as const)(
    "says nothing about the tray when it is %s",
    async (tray) => {
      const stores = await renderStatusBar(
        agentConnection({ status: agentStatus({ protocol: 22, tray }) }),
      )
      expect(stores.notifications.getSnapshot()).toEqual([])
    },
  )

  it("clears the list", async () => {
    const user = userEvent.setup()
    const stores = await renderStatusBar()
    notify({ title: "Build failed", severity: "error" })
    await user.click(screen.getByRole("button", { name: "Notifications, 1 unread" }))
    await user.click(screen.getByRole("button", { name: "Clear" }))
    expect(screen.getByText("No notifications")).toBeInTheDocument()
    expect(stores.notifications.getSnapshot()).toEqual([])
  })

  for (const [language, pattern, listName] of [
    ["en", /^\d{2}\/\d{2}\/\d{4}, \d{2}:\d{2}\s?[AP]M$/, "Notifications"],
    ["pl", /^\d{2}\.\d{2}\.\d{4}, \d{2}:\d{2}$/, "Powiadomienia"],
  ] as const) {
    it(`shows the time at the right of the title row, once (${language})`, async () => {
      await act(async () => {
        await i18n.changeLanguage(language)
      })
      try {
        const user = userEvent.setup()
        await renderStatusBar()
        notify({
          title: "A rather long title of a notification that has to wrap beside its time",
          body: "The devices stay blank until the agent is started again.",
          severity: "warning",
        })
        notify({ title: "Short", severity: "info" })
        await user.click(screen.getByRole("button", { name: /^(Notifications|Powiadomienia)/ }))
        const list = within(screen.getByRole("list", { name: listName }))
        for (const item of list.getAllByRole("listitem")) {
          const times = item.querySelectorAll("time")
          // One time per notification, in the title row, after the title.
          expect(times).toHaveLength(1)
          const time = times[0] as HTMLTimeElement
          const heading = time.parentElement as HTMLElement
          expect(heading).toHaveAttribute("data-slot", "notification-heading")
          expect(heading).toHaveClass("flex", "flex-wrap", "items-baseline")
          expect(heading.firstElementChild).toHaveClass(
            "min-w-[min(100%,10rem)]",
            "flex-1",
            "break-words",
          )
          expect(heading.lastElementChild).toBe(time)
          // At the right, never shrunk or wrapped, small and muted, tabular.
          expect(time).toHaveClass(
            "ml-auto",
            "shrink-0",
            "whitespace-nowrap",
            "text-xs",
            "text-muted-foreground",
            "tabular-nums",
          )
          expect(time.textContent).toMatch(pattern)
          // Machine-readable and with seconds on hover; screen readers read it.
          expect(Number.isNaN(Date.parse(time.dateTime))).toBe(false)
          expect(time).not.toHaveAttribute("aria-hidden")
          expect(item.textContent).toContain(time.textContent)
        }
        // The description directly under the title row.
        const first = list.getAllByRole("listitem")[1] as HTMLElement
        const heading = first.querySelector("[data-slot=notification-heading]") as HTMLElement
        expect(heading.nextElementSibling?.textContent).toBe(
          "The devices stay blank until the agent is started again.",
        )
        expect(heading.parentElement?.children).toHaveLength(2)
      } finally {
        await act(async () => {
          await i18n.changeLanguage("en")
        })
      }
    })
  }
})
