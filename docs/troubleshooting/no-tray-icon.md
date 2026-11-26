# No tray icon

DeckPilot's background agent normally shows an icon in the system tray (on
macOS, in the menu bar). Its menu has Open DeckPilot, the active profile,
the connected devices, Refresh Devices, About and **Quit DeckPilot**. This
page is about the case where that icon is missing, which mostly happens on
Linux.

## What DeckPilot does

The agent keeps running without the icon. Your devices, profiles and
actions work as usual. Only the tray menu is missing, and with it the
tray's Quit DeckPilot.

The agent reports its tray to the app (`AgentStatus.tray`, IPC protocol 22):

| State | Meaning |
| ----- | ------- |
| `shown` | The icon was created and is shown. On Linux: a StatusNotifier host lists the agent's icon. |
| `noHost` | Linux: the icon was created, but no StatusNotifier host is on the session bus. It is most likely not visible. |
| `hostUnknown` | Linux: the icon was created; the agent cannot tell whether it is shown (no session bus, or the host has not listed it yet). |
| `unavailable` | The icon could not be created: on Linux, the AppIndicator library (`libayatana-appindicator3`) is missing or cannot be loaded. |
| `disabled` | The agent was started with `--headless`, without a tray. |

For `noHost` and `unavailable`, the app shows a notice in Settings ›
Background Agent and Settings › Devices, and one status bar notification
per agent run. The notice has **Stop background agent**, which stops the
agent as Settings › Background Agent › Stop agent does. Settings ›
Background Agent also lists the tray's state.

DeckPilot never installs, enables or asks to install a desktop extension.

## How the agent checks (Linux)

On Linux the tray icon is a StatusNotifierItem, published through
libayatana-appindicator3. Creating it succeeds even when nothing shows it.
The agent therefore asks the session bus, read-only, whether
`org.kde.StatusNotifierWatcher` has an owner, whether the watcher's
`IsStatusNotifierHostRegistered` is true, and whether one of its
`RegisteredStatusNotifierItems` belongs to the agent's process. It checks
every second for 10 seconds, every 3 seconds until a minute, then every 15
seconds, so a desktop shell that starts later, or an extension switched on
or off, is noticed.

A desktop with only a legacy XEmbed system tray and no StatusNotifier host
is reported as `noHost`, although libayatana-appindicator may still show
the icon there through its XEmbed fallback.

## What you can do

- Open DeckPilot from your applications menu. Closing its window does not
  stop the agent.
- Stop the agent with **Stop background agent** in the notice, or with
  Settings › Background Agent › Stop agent.
- To get the tray icon on GNOME, install and enable an AppIndicator
  extension with your distribution's tools. Desktops that come with a
  StatusNotifier host of their own need nothing. Which desktops do is not
  checked by DeckPilot's tests; Settings › Background Agent shows what the
  agent found on yours.
- If the state is `unavailable`, install `libayatana-appindicator3` (the
  .deb and .rpm packages declare it; the AppImage bundles it).

## What is tested

- Automatically, on Linux CI (`apps/agent/tests/tray_linux.rs`): the agent
  executable with its real tray library, under Xvfb on a private session
  bus. With a minimal StatusNotifier watcher and host played by the test,
  the agent's icon registers with it and the agent reports `shown`. Without
  one, it reports `noHost`. When the watcher goes away, it reports `noHost`
  again. In each case the agent keeps running and quits on a `shutdown`
  request. The test's watcher draws nothing, so this checks registration
  and the reported state, not that a panel shows the icon.
- The decision logic is unit tested against a fake bus
  (`apps/agent/src/tray/presence.rs`), and the app's notice and
  notification with vitest.
- Not tested: a real GNOME session with and without the extension, KDE
  Plasma, and an XEmbed-only tray. These need checking by hand.
