// Texts the Connections settings share: errors and test results.

import { useTranslation } from "@/i18n"

import { pluginErrorText } from "../api"
import type { ConnectionTestOutcome, PluginConnection } from "./api"

/** What went wrong, for people: the agent's reason and message. */
export function errorText(error: unknown): string {
  const text = pluginErrorText(error)
  return [text.title, text.message].filter(Boolean).join(": ")
}

/** A test's result as a message. */
export function useTestMessage() {
  const { t } = useTranslation()
  return (outcome: ConnectionTestOutcome, connection: PluginConnection) =>
    outcome.ok
      ? connection.account
        ? t("plugins:connections.test.okAs", { account: connection.account })
        : t("plugins:connections.test.ok")
      : t("plugins:connections.test.failed", {
          reason: outcome.message || outcome.code || "",
        })
}
