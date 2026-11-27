import type { TranslationKey } from "@/i18n"
import type { AgentConnection, TrayState } from "@/lib/types"

/** How the tray's state is named to the user. */
export const TRAY_STATE_LABEL: Record<TrayState, TranslationKey> = {
  shown: "devices:agent.tray.state.shown",
  noHost: "devices:agent.tray.state.noHost",
  hostUnknown: "devices:agent.tray.state.hostUnknown",
  unavailable: "devices:agent.tray.state.unavailable",
  disabled: "devices:agent.tray.state.disabled",
  unknown: "devices:agent.tray.state.unknown",
}

/**
 * The running agent's tray icon is known to be missing (IPC protocol 22):
 * no StatusNotifier host, or the icon could not be created. Not for an
 * agent started `--headless` on purpose, nor while it is not known.
 */
export function trayMissing(agent: AgentConnection | null | undefined): boolean {
  const tray = agent?.running ? agent.status?.tray : undefined
  return tray === "noHost" || tray === "unavailable"
}
