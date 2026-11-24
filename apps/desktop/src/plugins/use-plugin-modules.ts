// The installed plugins as modules of the action editors (experimental).
//
// Only builds with `VITE_DECKPILOT_PLUGINS=1` ask the agent anything: in
// every other build the hook returns the same empty list and loads no
// plugin client, so no request is ever sent. The answer of `listPlugins`
// is also what the backend validates plugin actions against
// (`src-tauri/src/plugin_actions.rs`), so the editors and the backend see
// the same plugins. With an agent that pages plugin answers (runtime
// feature `pagedAnswers`) the backend reads every plugin's actions page by
// page (`deckpilot_ipc::plugin_answers`), so the list is complete however
// many actions and outputs a plugin has; an older agent answers in one
// message, without the definitions of plugins that do not fit it.

import { useEffect, useState } from "react"

import { pluginModules } from "@/lib/plugin-actions"
import type { ModuleInfo } from "@/lib/types"

import { type PluginClient, usePluginClientOverride } from "./api"
import type { PluginDetails } from "./types"

const NONE: ModuleInfo[] = []

/**
 * One module per installed plugin, read again whenever `agentKey` changes
 * (the agent started, stopped or was replaced) and whenever the agent
 * reports that a plugin changed. While the agent cannot answer plugin
 * requests there are none: instances in profiles then show as unavailable.
 */
export function usePluginModules(agentKey: string): ModuleInfo[] {
  const override = usePluginClientOverride()
  const [modules, setModules] = useState<ModuleInfo[]>(NONE)

  useEffect(() => {
    // Inline, so that Vite folds it to a constant and builds without
    // plugins contain neither the client nor its commands.
    if (import.meta.env.VITE_DECKPILOT_PLUGINS !== "1") return
    let active = true
    let stop: (() => void) | undefined
    let generation = 0
    const load = (client: PluginClient) => {
      const current = ++generation
      // The names of uninstalled plugins only improve messages.
      const known = client.listKnownPlugins().catch(() => [])
      // The legacy action types migrated modules keep; none from agents
      // before module migrations.
      const migrations = client.listModuleMigrations().catch(() => [])
      client.listPlugins().then(
        async (plugins) => {
          // The grants of enabled plugins with blocked actions: what each
          // of those actions waits for, in structured form.
          const blocked = plugins.filter(
            (p) => p.state === "enabled" && p.actions.some((a) => !a.available),
          )
          const details = Promise.all(
            blocked.map((p) => client.getPlugin(p.id).catch(() => null)),
          ).then((all) => all.filter((d): d is PluginDetails => d !== null))
          const [names, moved, grants] = await Promise.all([known, migrations, details])
          if (active && current === generation) {
            setModules(pluginModules(plugins, names, moved, grants))
          }
        },
        () => {
          if (active && current === generation) setModules(NONE)
        },
      )
    }
    const start = async () => {
      const client = override ?? (await import("./client")).createTauriPluginClient()
      if (!active) return
      load(client)
      stop = client.subscribe((event) => {
        if (event.type === "changed" || event.type === "migrationChanged") load(client)
      })
    }
    void start()
    return () => {
      active = false
      stop?.()
    }
  }, [override, agentKey])

  return modules
}
