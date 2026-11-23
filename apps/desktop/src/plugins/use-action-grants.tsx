import { useEffect, useState } from "react"
import { toast } from "sonner"

import { i18n } from "@/i18n"
import { legacyAlias } from "@/lib/plugin-actions"
import type { ModuleInfo } from "@/lib/types"

import { type PluginClient, usePluginClientOverride } from "./api"
import { pluginErrorSentence } from "./dialog-error"
import { actionNamesOf, grantOfInfo } from "./permissions"
import { PermissionReviewDialog, type ReviewRequest } from "./review-dialog"

/** What an action needs granted before it can run, from the agent's grants. */
export interface ActionGrant {
  pluginId: string
  pluginName: string
  actionName: string
  /** The plugin's declared permissions this action needs and that are not granted. */
  keys: string[]
}

/**
 * The permissions plugin actions wait for, and their granting from where
 * the action is (the Device Editor's library): decided only from the
 * structured grants the agent reports (the plugin modules' `grantable`,
 * from `getPlugin`: each declared permission, its status, and for an
 * optional one the actions that need it), never from the agent's message. Granting asks first, in the
 * Marketplace's permission dialog, offering exactly those permissions;
 * the existing `grantPermissions` request does the rest (declarations,
 * publisher, scopes, runtime authorization). The plugin's change event
 * then refreshes every action that needed it.
 */
export function useActionGrants(modules: ModuleInfo[]) {
  const override = usePluginClientOverride()
  // The provider's client (tests, other windows), else the app's, loaded
  // once in builds with plugins.
  const [tauri, setTauri] = useState<PluginClient | null>(null)
  useEffect(() => {
    if (override || import.meta.env.VITE_DECKPILOT_PLUGINS !== "1") return
    let active = true
    void import("./client").then((m) => {
      if (active) setTauri(m.createTauriPluginClient())
    })
    return () => {
      active = false
    }
  }, [override])
  const client = override ?? tauri

  /**
   * What `typeId` needs granted, when that is all that stops it (the
   * plugin modules' `grantable`, read from the agent's grants); `null`
   * otherwise. A legacy id is its plugin's type.
   */
  const missingFor = (typeId: string | undefined): ActionGrant | null => {
    if (!typeId) return null
    const target = legacyAlias(modules, typeId)?.target ?? typeId
    const module = modules.find((m) => m.plugin && target.startsWith(`${m.id}.`))
    if (module?.plugin?.state !== "enabled" || !module.plugin.unavailable[target]) return null
    const keys = module.plugin.grantable?.[target] ?? []
    if (keys.length === 0) return null
    return {
      pluginId: module.id,
      pluginName: module.name,
      actionName: module.actionTypes.find((t) => t.id === target)?.name ?? target,
      keys,
    }
  }

  const [review, setReview] = useState<ReviewRequest | null>(null)
  const [pending, setPending] = useState<string | null>(null)

  /** Asks to grant what `typeId` needs; nothing is granted until the user allows it. */
  const request = (typeId: string) => {
    if (pending || !client) return
    const needed = missingFor(typeId)
    if (!needed) return
    setPending(typeId)
    // The plugin's current grants, for the dialog's words and checks.
    client.getPlugin(needed.pluginId).then(
      (plugin) => {
        const items = plugin.permissions
          .filter((p) => needed.keys.includes(p.key) && p.status !== "granted")
          .map((info) => ({
            grant: grantOfInfo(info, actionNamesOf(plugin.summary.actions)),
            state: "new" as const,
          }))
        if (items.length === 0) {
          setPending(null)
          return
        }
        setReview({
          mode: "grant",
          pluginId: needed.pluginId,
          name: plugin.summary.name || needed.pluginName,
          publisher: plugin.summary.publisher,
          version: plugin.summary.version,
          // Exactly the declared permissions this action needs and lacks.
          items,
          confirm: async (consent) => {
            // A failure rejects: the dialog shows it, and nothing changes
            // here. On success the agent's change event refreshes the
            // plugin modules, and with them every action that needed it.
            await client.grantPermissions(needed.pluginId, consent)
            toast.success(i18n.t("editor:grant.granted"))
          },
        })
      },
      (e: unknown) => {
        setPending(null)
        toast.error(pluginErrorSentence(e))
      },
    )
  }

  const close = () => {
    setReview(null)
    setPending(null)
  }
  const dialog = review ? <PermissionReviewDialog request={review} onClose={close} /> : null

  return {
    missingFor,
    request,
    isPending: (typeId: string) => pending === typeId,
    busy: pending !== null,
    dialog,
  }
}
