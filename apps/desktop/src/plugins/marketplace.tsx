import { type ReactNode, useCallback, useEffect, useLayoutEffect, useRef, useState } from "react"
import { ShieldQuestion } from "lucide-react"
import { toast } from "sonner"

import { AlertDialog, ConfirmDialog } from "@/components/dialogs"
import { Loading } from "@/components/page"
import { Button } from "@/components/ui/button"
import { i18n, useTranslation } from "@/i18n"
import { useAppContext } from "@/lib/app-context"

import {
  pluginErrorText,
  usePluginClientOverride,
  type PluginClient,
  type PluginErrorText,
} from "./api"
import { createTauriPluginClient } from "./client"
import { errorSentence } from "./dialog-error"
import { Notice, ProgressList, RegistryNotice, RuntimeNotice, type Progress } from "./components"
import { PluginDetailsView } from "./details"
import { MigrationConsentDialog } from "./migration-dialog"
import { CategoriesSection, CollectionSection, DiscoverSection, SearchResults } from "./home"
import { DiscoverEditorial } from "./storefront/discover-editorial"
import { StorefrontProvider } from "./storefront/storefront-provider"
import { MarketplaceSidebar } from "./marketplace-sidebar"
import { PluginPermissionsReview } from "./permissions-review"
import { InstalledView } from "./installed"
import { DiagnosticsView } from "./diagnostics"
import { ConfigureDialog } from "./configure"
import {
  listingOf,
  type MarketplaceSection,
  pluginDisplayName,
  type PluginFailure,
} from "./catalog"
import {
  availabilityOf,
  availabilityOfError,
  installBlock,
  loadSnapshot,
  offeredVersion,
  updateCandidates,
  type UpdateCandidate,
} from "./model"
import { actionNamesOf, grantOfInfo, grantsOf } from "./permissions"
import { PermissionReviewDialog, type ReviewRequest } from "./review-dialog"
import {
  InstallPlanDialog,
  RemovalDialog,
  type InstallPlanRequest,
  type RemovalRequest,
} from "./dependencies"
import { removalNeedsReview } from "./dependency-text"
import type {
  ActionParamValue,
  AvailablePlugin,
  AvailableVersion,
  MarketplaceSnapshot,
  ModuleMigration,
  PermissionChanges,
  PluginActionOutcome,
  PluginDetails,
  PluginNavigationEntry,
  PluginRetention,
  RuntimeAvailability,
} from "./types"
import { DEPENDENCIES_FEATURE, RESTORE_BUNDLED_FEATURE } from "./types"
import { focusMigrations, takeFocusedMigrations, takeFocusedPlugin } from "./focus"
import { UpdatesView } from "./updates"
import { pluginViewPage } from "@/shell/plugin-navigation"
import type { PageId } from "@/lib/app-context"

/**
 * Where the plugin data is shown: the Marketplace (the only place plugins
 * are discovered, installed and managed), the plugin section of Settings ›
 * Logs & Diagnostics, or the plugin section of Settings › Permissions. All
 * three use the same client, state and requests.
 */
export type PluginsVariant = "marketplace" | "diagnostics" | "permissions"

/** Below this width the Marketplace's sidebar shows its icons only. */
const COMPACT_WIDTH = 760

/** How long the search waits for the user to pause typing. */
export const SEARCH_DELAY_MS = 180

/** The nearest ancestor that scrolls (the workspace), whose position a plugin's page keeps. */
function scrollParent(element: Element | null): HTMLElement | null {
  for (let e = element?.parentElement ?? null; e; e = e.parentElement) {
    const overflow = getComputedStyle(e).overflowY
    if (overflow === "auto" || overflow === "scroll") return e
  }
  return null
}

type Load =
  | { status: "loading" }
  | { status: "unavailable"; availability: RuntimeAvailability }
  | { status: "error"; error: unknown }
  | {
      status: "ready"
      availability: RuntimeAvailability
      snapshot: MarketplaceSnapshot
      /** Built-in modules moving to plugins; empty from agents without them. */
      migrations: ModuleMigration[]
    }

/** What the screens can ask the marketplace to do. */
export interface MarketplaceActions {
  /** Id of the plugin with a request in flight, or "registry". */
  busy: string | null
  /** Installs and updates this page started that have not finished, by plugin id. */
  pending: ReadonlyMap<string, "install" | "update">
  /** The last failed install or update of a plugin, until retried or done. */
  failures: ReadonlyMap<string, PluginFailure>
  /** Tries a failed install or update again (with its consent, as before). */
  retry: (plugin: AvailablePlugin) => void
  /** Opens a toolbar view of an installed plugin. */
  openView: (plugin: string, entry: PluginNavigationEntry) => void
  /** Opens an installed plugin's settings and credentials. */
  configure: (item: PluginDetails) => void
  /** Shows the installed plugins. */
  showInstalled: () => void
  /** Opens the plugin diagnostics in Settings (module migrations, the registry). */
  openDiagnostics: () => void
  /** Why installs and updates are not possible right now. */
  blocked: string | null
  /** Plugins can be enabled and run here (not on Windows). */
  canRun: boolean
  /** The client, for the requests a dialog makes itself (settings, credentials). */
  client: PluginClient
  openDetails: (id: string) => void
  install: (plugin: AvailablePlugin, version: AvailableVersion) => void
  installFile: () => void
  update: (candidate: UpdateCandidate) => void
  setEnabled: (item: PluginDetails, enabled: boolean) => void
  setGrant: (item: PluginDetails, key: string, granted: boolean) => void
  /** Uninstalls; `keep` keeps settings and/or credentials for a reinstall. */
  uninstall: (item: PluginDetails, keep?: PluginRetention) => void
  /** The agent can restore DeckPilot's own plugins (feature `restoreBundled`). */
  canRestore: boolean
  /**
   * Installs a removed bundled plugin again from DeckPilot's own copy, or
   * (`reinstall`) replaces a broken bundled copy with it.
   */
  restore: (id: string, name: string, reinstall: boolean) => void
  rollback: (item: PluginDetails) => void
  invoke: (
    item: PluginDetails,
    action: string,
    params: Record<string, ActionParamValue>,
  ) => Promise<PluginActionOutcome>
  refresh: () => void
  /** Reads everything again (without re-reading the registry). */
  reload: () => void
}

async function loadAll(client: PluginClient, refresh = false): Promise<Load> {
  let runtime
  try {
    runtime = await client.getRuntime()
  } catch (error) {
    const availability = availabilityOfError(error)
    return availability ? { status: "unavailable", availability } : { status: "error", error }
  }
  const availability = availabilityOf(runtime)
  if (!runtime.available) return { status: "unavailable", availability }
  try {
    // An agent before module migrations refuses the request: none then.
    const migrations = client.listModuleMigrations().catch(() => [])
    const snapshot = await loadSnapshot(client, runtime, refresh)
    return { status: "ready", availability, snapshot, migrations: await migrations }
  } catch (error) {
    return { status: "error", error }
  }
}

/** A refusal as a toast: what it means, the agent's words and what to do. */
function toastError({ title, message, hint }: PluginErrorText) {
  const description = [message === title ? null : message, hint].filter(Boolean).join(" ")
  toast.error(title, description ? { description } : undefined)
}

function updateItems(changes: PermissionChanges): ReviewRequest["items"] {
  return [
    ...changes.added.map((info) => ({ grant: grantOfInfo(info), state: "new" as const })),
    ...changes.broadened.map((info) => ({
      grant: grantOfInfo(info),
      state: "broadened" as const,
    })),
  ]
}

/**
 * The experimental plugin marketplace: the catalog (Discover, search,
 * details, install with permission review) and the installed plugins
 * (enable, permissions, configure, updates, uninstall). Shown as the main
 * window's Marketplace and, starting at the installed plugins, as Settings
 * › Extensions › Plugins. Only reachable in builds with
 * VITE_DECKPILOT_PLUGINS=1. Everything comes from the agent; the page says
 * why when the agent cannot answer plugin requests.
 */
export function PluginsSettings({ variant = "marketplace" }: { variant?: PluginsVariant } = {}) {
  const { t } = useTranslation()
  const override = usePluginClientOverride()
  const [client] = useState<PluginClient>(() => override ?? createTauriPluginClient())
  const { agent, navigate, openSettings } = useAppContext()
  const [load, setLoad] = useState<Load>({ status: "loading" })
  // A screen may have asked to open at one plugin (`focusPlugin`).
  const [selected, setSelected] = useState<string | null>(() =>
    variant === "marketplace" ? takeFocusedPlugin() : null,
  )
  /** Discover's category; kept while a plugin's page is open. */
  const [category, setCategory] = useState<string | null>(null)
  /** The sidebar's search as typed, and as searched (after a short pause). */
  const [query, setQuery] = useState("")
  const [searched, setSearched] = useState("")
  /** Where the section was scrolled when search results replaced it. */
  const beforeSearch = useRef<number | null>(null)
  /** The Marketplace's sidebar destination; kept while a plugin's page is open. */
  const [section, setSection] = useState<MarketplaceSection>("discover")
  const page = useRef<HTMLDivElement>(null)
  const root = useRef<HTMLDivElement>(null)
  const [compact, setCompact] = useState(false)
  /** Where the page was scrolled when a plugin's page replaced it. */
  const scrolledTo = useRef<number | null>(null)
  const [review, setReview] = useState<ReviewRequest | null>(null)
  const [planReview, setPlanReview] = useState<InstallPlanRequest | null>(null)
  /**
   * An install the agent refused while planning it (before any consent
   * dialog could open): shown as a dialog of its own, so the click on
   * Install never ends in nothing.
   */
  const [installRefused, setInstallRefused] = useState<{
    name: string
    version: string
    error: PluginErrorText
  } | null>(null)
  const [removal, setRemoval] = useState<RemovalRequest | null>(null)
  const [busy, setBusy] = useState<string | null>(null)
  const [progress, setProgress] = useState<Map<string, Progress>>(new Map())
  /** A package file waiting for the user to confirm its installation. */
  const [packageFile, setPackageFile] = useState<string | null>(null)
  /** A module migration whose consent dialog is open. */
  const [consentFor, setConsentFor] = useState<ModuleMigration | null>(null)
  /** Installs and updates this page started, by plugin id. */
  const [pending, setPending] = useState<Map<string, "install" | "update">>(new Map())
  /** Installs and updates that failed, by plugin id, until retried. */
  const [failures, setFailures] = useState<Map<string, PluginFailure>>(new Map())
  /** The installed plugin whose settings dialog is open. */
  const [configuring, setConfiguring] = useState<string | null>(null)
  const returnFocus = useRef<HTMLElement | null>(null)
  const generation = useRef(0)

  /** Reads everything again; the page keeps showing the old data meanwhile. */
  const reload = useCallback(
    (refresh = false) => {
      const current = ++generation.current
      return loadAll(client, refresh).then((next) => {
        if (current === generation.current) setLoad(next)
      })
    },
    [client],
  )

  // The agent started, stopped or was replaced: ask again.
  const agentKey = agent
    ? `${String(agent.running)}:${agent.status?.version ?? ""}:${String(agent.protocol)}`
    : "unknown"
  useEffect(() => {
    void reload()
  }, [reload, agentKey])

  useEffect(
    () =>
      client.subscribe((event) => {
        if (event.type === "progress") {
          setProgress((previous) => {
            const next = new Map(previous)
            if (event.stage === "finished" || event.stage === "failed") next.delete(event.plugin)
            else next.set(event.plugin, event)
            return next
          })
        } else {
          void reload()
        }
      }),
    [client, reload],
  )

  // Back from a plugin's page: where the page was.
  useLayoutEffect(() => {
    if (selected !== null) return
    const top = scrolledTo.current
    scrolledTo.current = null
    const scroller = scrollParent(page.current)
    if (top !== null && scroller) scroller.scrollTop = top
  }, [selected])

  // Search as the user types, once they pause (no Enter needed).
  useEffect(() => {
    // Clearing is immediate (the query handler); only typing waits.
    if (query.trim() === "") return
    const timer = setTimeout(() => {
      setSearched(query)
    }, SEARCH_DELAY_MS)
    return () => {
      clearTimeout(timer)
    }
  }, [query])

  // Results replace the section: remember where it was scrolled, and go
  // back there once the search is cleared.
  const searching = searched.trim() !== ""
  useLayoutEffect(() => {
    const scroller = scrollParent(page.current)
    if (searching) {
      beforeSearch.current ??= scroller?.scrollTop ?? 0
      if (scroller) scroller.scrollTop = 0
    } else if (beforeSearch.current !== null) {
      if (scroller) scroller.scrollTop = beforeSearch.current
      beforeSearch.current = null
    }
  }, [searching])

  // Narrow, the sidebar shows its icons only (the content needs the room).
  const ready = load.status === "ready"
  useEffect(() => {
    const element = root.current
    if (!element || typeof ResizeObserver === "undefined") return
    const observer = new ResizeObserver(([entry]) => {
      if (entry) setCompact(entry.contentRect.width < COMPACT_WIDTH)
    })
    observer.observe(element)
    return () => {
      observer.disconnect()
    }
  }, [ready])

  // Opened from the Marketplace to review a migration: bring it into view.
  const [revealMigrations] = useState(() => variant === "diagnostics" && takeFocusedMigrations())
  useEffect(() => {
    if (revealMigrations && ready) {
      document.getElementById("module-migrations-title")?.scrollIntoView({ block: "start" })
    }
  }, [revealMigrations, ready])

  const retry = () => {
    setLoad({ status: "loading" })
    void reload()
  }

  const names = (id: string) =>
    load.status === "ready" ? pluginDisplayName(id, undefined, load.snapshot) : id
  const header = (
    <>
      {progress.size > 0 && (
        <ProgressList
          progress={[...progress.values()].map((p) => ({ ...p, name: names(p.plugin) }))}
        />
      )}
    </>
  )

  // Before the plugins are known, the Marketplace has no sidebar: the
  // state alone, padded like its content.
  const frame = (node: ReactNode) =>
    variant === "marketplace" ? (
      <div className="grid content-start gap-4 px-6 py-5">{node}</div>
    ) : (
      node
    )

  if (load.status === "loading") {
    return frame(
      <>
        {header}
        <Loading />
      </>,
    )
  }
  if (load.status === "unavailable") {
    return frame(
      <>
        {header}
        <RuntimeNotice availability={load.availability} busy={false} onRetry={retry} />
      </>,
    )
  }
  if (load.status === "error") {
    // Read at render time, so the words follow the language on screen.
    const failure = pluginErrorText(load.error)
    return frame(
      <>
        {header}
        <Notice tone="destructive" title={t("marketplace:loadFailed")}>
          <p>{errorSentence(failure, false)}</p>
          {failure.hint && <p>{failure.hint}</p>}
          <div>
            <Button variant="outline" size="sm" onClick={retry}>
              {t("common:actions.tryAgain")}
            </Button>
          </div>
        </Notice>
      </>,
    )
  }

  const { snapshot, availability } = load
  // A manifest may name its plugin with a key of its locale files
  // (`t:plugin.name`); the agent resolves it only for installed plugins.
  const migrations = load.migrations.map((m) => ({
    ...m,
    pluginName: pluginDisplayName(m.plugin, m.pluginName, snapshot),
  }))
  const blocked = installBlock(snapshot)
  // Dependency plans (plugin API 1.4): agents without them install one
  // plugin at a time, as before.
  const plans = (snapshot.runtime.features ?? []).includes(DEPENDENCIES_FEATURE)
  const canRun = snapshot.runtime.executionSupported
  const canRestore = (snapshot.runtime.features ?? []).includes(RESTORE_BUNDLED_FEATURE)
  const updates = updateCandidates(snapshot)

  /** Runs one request, then reads everything again. Rejections propagate. */
  const run = async (id: string, request: () => Promise<unknown>, success: string) => {
    setBusy(id)
    try {
      await request()
      toast.success(success)
    } finally {
      await reload()
      setBusy(null)
    }
  }
  const setPendingOf = (id: string, operation: "install" | "update" | null) => {
    setPending((previous) => {
      const next = new Map(previous)
      if (operation) next.set(id, operation)
      else next.delete(id)
      return next
    })
  }
  const setFailureOf = (id: string, failure: PluginFailure | null) => {
    setFailures((previous) => {
      if (!failure && !previous.has(id)) return previous
      const next = new Map(previous)
      if (failure) next.set(id, failure)
      else next.delete(id)
      return next
    })
  }
  /**
   * An install or update of `id`: shown as in progress until the agent
   * answers, and as failed (with Retry) when it refuses. "Installed" comes
   * only from the agent's list read afterwards.
   */
  const tracked = async <T,>(
    id: string,
    operation: "install" | "update",
    request: () => Promise<T>,
  ): Promise<T> => {
    setPendingOf(id, operation)
    setFailureOf(id, null)
    try {
      return await request()
    } catch (error) {
      setFailureOf(id, { operation, error: pluginErrorText(error) })
      throw error
    } finally {
      setPendingOf(id, null)
    }
  }

  /** Like `run`, but reports failures as a toast instead of to a dialog. */
  const runReported = (id: string, request: () => Promise<unknown>, success: string) => {
    run(id, request, success).catch((error: unknown) => {
      toastError(pluginErrorText(error))
    })
  }

  const openReview = (request: ReviewRequest) => {
    returnFocus.current =
      document.activeElement instanceof HTMLElement ? document.activeElement : null
    setReview(request)
  }

  /** The consent dialog for enabling `item` with every declared permission. */
  const reviewEnable = (item: PluginDetails) => {
    const id = item.summary.id
    openReview({
      mode: "enable",
      pluginId: id,
      name: item.summary.name,
      publisher: item.summary.publisher,
      version: item.summary.version,
      items: item.permissions.map((info) => ({
        grant: grantOfInfo(info, actionNamesOf(item.summary.actions)),
        state: info.status === "granted" ? "granted" : "new",
      })),
      confirm: (consent) =>
        run(
          id,
          () => client.enablePlugin(id, consent),
          i18n.t("marketplace:toast.enabled", { name: item.summary.name }),
        ),
    })
  }

  /** Installs the confirmed package file, then shows its permissions. */
  const installPackage = (path: string) => {
    setBusy("package")
    client
      .installPlugin({ type: "file", path }, [])
      .then(async (details) => {
        toast.success(
          i18n.t("marketplace:toast.installedFromFile", {
            name: details.summary.name,
            version: details.summary.version,
          }),
        )
        await reload()
        // Its permissions are known now that the agent verified it:
        // nothing is granted until the user consents.
        if (canRun) reviewEnable(details)
      })
      .catch((error: unknown) => {
        toastError(pluginErrorText(error))
      })
      .finally(() => {
        setBusy(null)
      })
  }

  const reviewUpdate = (item: PluginDetails, version: string, changes: PermissionChanges) => {
    const id = item.summary.id
    openReview({
      mode: "update",
      pluginId: id,
      name: item.summary.name,
      publisher: item.summary.publisher,
      version,
      items: updateItems(changes),
      removed: changes.removed,
      confirm: (consent) =>
        run(
          id,
          () => tracked(id, "update", () => client.approveUpdate(id, consent)),
          i18n.t("marketplace:toast.updated", { name: item.summary.name, version }),
        ),
      decline: () =>
        run(
          id,
          () => client.declineUpdate(id),
          i18n.t("marketplace:toast.updateDeclined", { name: item.summary.name }),
        ),
    })
  }

  /**
   * The full install plan (dependencies, built-in modules, every plugin's
   * permissions), resolved and verified by the agent before the dialog
   * opens; nothing is installed until the user confirms it.
   */
  const reviewPlan = (plugin: AvailablePlugin, version: AvailableVersion) => {
    const replan = (optional: string[]) => client.planInstall(plugin.id, version.version, optional)
    setBusy(plugin.id)
    setFailureOf(plugin.id, null)
    replan([])
      .then((plan) => {
        returnFocus.current =
          document.activeElement instanceof HTMLElement ? document.activeElement : null
        setPlanReview({
          name: plugin.name,
          plan,
          optional: [],
          activate: canRun,
          replan,
          confirm: (confirmed, optional, consent) =>
            run(
              plugin.id,
              () =>
                tracked(plugin.id, "install", () =>
                  client.installPlan(confirmed, optional, consent, canRun),
                ),
              confirmed.steps.length > 1
                ? i18n.t("marketplace:toast.installedWithMore", {
                    name: plugin.name,
                    count: confirmed.steps.length - 1,
                  })
                : i18n.t("marketplace:toast.installed", { name: plugin.name }),
            ),
        })
      })
      .catch((error: unknown) => {
        // The plan could not be made: nothing was installed. The card
        // keeps the failure; the dialog says why.
        const text = pluginErrorText(error)
        setFailureOf(plugin.id, { operation: "install", error: text })
        returnFocus.current =
          document.activeElement instanceof HTMLElement ? document.activeElement : null
        setInstallRefused({
          name: plugin.name,
          version: version.version,
          error: text,
        })
      })
      .finally(() => {
        setBusy(null)
      })
  }

  /**
   * Disabling or uninstalling with what it affects: plugins that need it
   * are turned off with it (after the user saw them), orphaned
   * dependencies are removed only if checked. Nothing to show: done at once.
   */
  const reviewRemoval = (item: PluginDetails, uninstall: boolean, keep?: PluginRetention) => {
    const id = item.summary.id
    const name = item.summary.name
    const remove = (withDependents: boolean, orphans: string[]) =>
      run(
        id,
        () => client.removePlugin(id, uninstall, keep, withDependents, orphans),
        uninstall
          ? i18n.t("marketplace:toast.uninstalled", { name })
          : i18n.t("marketplace:toast.disabled", { name }),
      )
    client
      .planRemoval(id, uninstall)
      .then((plan) => {
        if (!removalNeedsReview(plan)) return remove(false, [])
        returnFocus.current =
          document.activeElement instanceof HTMLElement ? document.activeElement : null
        setRemoval({ name, plan, confirm: remove })
        return undefined
      })
      .catch((error: unknown) => {
        toastError(pluginErrorText(error))
      })
  }

  // Progress the agent reports counts too (e.g. an install another window started).
  const pendingAll = new Map(pending)
  for (const p of progress.values()) {
    if ((p.operation === "install" || p.operation === "update") && !pendingAll.has(p.plugin)) {
      pendingAll.set(p.plugin, p.operation)
    }
  }

  const actions: MarketplaceActions = {
    busy,
    blocked,
    canRun,
    client,
    pending: pendingAll,
    failures,
    retry: (plugin) => {
      const failure = failures.get(plugin.id)
      setFailureOf(plugin.id, null)
      if (failure?.operation === "update") {
        const candidate = updates.find((u) => u.plugin.summary.id === plugin.id)
        if (candidate) actions.update(candidate)
        return
      }
      const version = offeredVersion(plugin)
      if (version) actions.install(plugin, version)
    },
    openView: (plugin, entry) => {
      navigate(pluginViewPage({ plugin, id: entry.id }) as PageId)
    },
    configure: (item) => {
      setConfiguring(item.summary.id)
    },
    showInstalled: () => {
      scrolledTo.current = null
      setSelected(null)
      setSection("installed")
    },
    openDiagnostics: () => {
      focusMigrations()
      openSettings("diagnostics")
    },
    openDetails: (id) => {
      scrolledTo.current = scrollParent(page.current)?.scrollTop ?? null
      setSelected(id)
    },
    install: (plugin, version) => {
      if (plans) {
        reviewPlan(plugin, version)
        return
      }
      openReview({
        mode: "install",
        pluginId: plugin.id,
        name: plugin.name,
        publisher: plugin.publisher,
        publisherInfo: plugin.publisherInfo,
        version: version.version,
        items: grantsOf(version.permissions).map((grant) => ({ grant, state: "new" })),
        confirm: (consent) =>
          run(
            plugin.id,
            () =>
              tracked(plugin.id, "install", () =>
                client.installPlugin(
                  { type: "registry", plugin: plugin.id, version: version.version },
                  consent,
                ),
              ),
            i18n.t("marketplace:toast.installed", { name: plugin.name }),
          ),
      })
    },
    installFile: () => {
      client.pickPackage().then(
        (path) => {
          if (path) setPackageFile(path)
        },
        (error: unknown) => {
          toastError(pluginErrorText(error))
        },
      )
    },
    update: (candidate) => {
      const item = candidate.plugin
      const id = item.summary.id
      if (candidate.kind === "staged") {
        reviewUpdate(item, candidate.pending.version, candidate.pending.changes)
        return
      }
      setBusy(id)
      tracked(id, "update", () => client.updatePlugin(id, candidate.version, []))
        .then(async (result) => {
          if (result.outcome.type === "applied") {
            toast.success(
              i18n.t("marketplace:toast.updated", {
                name: item.summary.name,
                version: result.outcome.version,
              }),
            )
          } else {
            // Staged until the user decides (approve or decline).
            reviewUpdate(item, result.outcome.version, result.outcome.changes)
          }
          await reload()
        })
        .catch((error: unknown) => {
          toastError(pluginErrorText(error))
        })
        .finally(() => {
          setBusy(null)
        })
    },
    setEnabled: (item, enabled) => {
      const id = item.summary.id
      if (!enabled) {
        if (plans) {
          reviewRemoval(item, false, undefined)
          return
        }
        runReported(
          id,
          () => client.disablePlugin(id),
          i18n.t("marketplace:toast.disabled", { name: item.summary.name }),
        )
        return
      }
      reviewEnable(item)
    },
    setGrant: (item, key, granted) => {
      const id = item.summary.id
      runReported(
        id,
        () => (granted ? client.grantPermissions(id, [key]) : client.revokePermission(id, key)),
        granted
          ? i18n.t("marketplace:toast.permissionGranted")
          : i18n.t("marketplace:toast.permissionRevoked"),
      )
    },
    uninstall: (item, keep) => {
      const id = item.summary.id
      if (plans) {
        reviewRemoval(item, true, keep)
        return
      }
      const name = item.summary.name
      const kept =
        keep?.settings && keep.credentials
          ? i18n.t("marketplace:toast.uninstalledKeptBoth", { name })
          : keep?.settings
            ? i18n.t("marketplace:toast.uninstalledKeptSettings", { name })
            : keep?.credentials
              ? i18n.t("marketplace:toast.uninstalledKeptCredentials", { name })
              : null
      runReported(
        id,
        () =>
          keep?.settings || keep?.credentials
            ? client.uninstallPlugin(id, keep)
            : client.uninstallPlugin(id),
        kept ?? i18n.t("marketplace:toast.uninstalled", { name }),
      )
    },
    canRestore,
    restore: (id, name, reinstall) => {
      setBusy(id)
      client
        .restoreBundledPlugin(id)
        .then(async (details) => {
          toast.success(
            reinstall
              ? i18n.t("marketplace:toast.reinstalled", { name: details.summary.name || name })
              : i18n.t("marketplace:toast.restored", { name: details.summary.name || name }),
          )
          await reload()
          // Permissions beyond what DeckPilot grants by itself wait for the
          // user, as after any install.
          if (
            canRun &&
            details.summary.state === "disabled" &&
            details.permissions.some((p) => p.status !== "granted")
          ) {
            reviewEnable(details)
          }
        })
        .catch((error: unknown) => {
          toastError(pluginErrorText(error))
        })
        .finally(() => {
          setBusy(null)
        })
    },
    rollback: (item) => {
      const id = item.summary.id
      runReported(
        id,
        () => client.rollbackPlugin(id),
        item.summary.previousVersion
          ? i18n.t("marketplace:toast.rolledBack", {
              name: item.summary.name,
              version: item.summary.previousVersion,
            })
          : i18n.t("marketplace:toast.rolledBackPrevious", { name: item.summary.name }),
      )
    },
    // Read again afterwards: a failure changes the plugin's health.
    invoke: (item, action, params) =>
      client.invokeAction(item.summary.id, action, params).finally(() => {
        void reload()
      }),
    refresh: () => {
      setBusy("registry")
      void reload(true).finally(() => {
        setBusy(null)
      })
    },
    reload: () => {
      void reload()
    },
  }

  const closeReview = () => {
    setReview(null)
    returnFocus.current?.focus()
  }

  // Every installed plugin has a page, listed by the registry or not.
  const selectedInstalled = selected
    ? snapshot.installed.find((p) => p.summary.id === selected)
    : undefined
  const selectedPlugin = selected
    ? (snapshot.available.find((p) => p.id === selected) ??
      (selectedInstalled ? listingOf(selectedInstalled) : undefined))
    : undefined
  const configuringItem = configuring
    ? snapshot.installed.find((p) => p.summary.id === configuring)
    : undefined
  const waiting = migrations.filter((m) => m.phase === "awaitingConsent")

  /** A sidebar destination: its content from the top, any plugin page and search closed. */
  const go = (next: MarketplaceSection) => {
    scrolledTo.current = null
    beforeSearch.current = null
    setSelected(null)
    setQuery("")
    setSearched("")
    setSection(next)
    const scroller = scrollParent(page.current)
    if (scroller) scroller.scrollTop = 0
  }

  const migrationNotice = waiting.length > 0 && (
    <div className="flex flex-wrap items-center gap-3 rounded-xl bg-card p-3 text-sm ring-1 ring-border">
      <ShieldQuestion className="size-4 shrink-0" aria-hidden />
      <p className="min-w-0 flex-1">
        {t("marketplace:home.migrationWaiting", {
          count: waiting.length,
          modules: waiting.map((m) => m.moduleName).join(", "),
        })}
      </p>
      <Button
        size="sm"
        variant="outline"
        onClick={() => {
          if (variant === "marketplace") actions.openDiagnostics()
          else {
            document.getElementById("module-migrations-title")?.scrollIntoView({ block: "start" })
          }
        }}
      >
        {variant === "marketplace"
          ? t("marketplace:home.migrationReviewInSettings")
          : t("marketplace:home.migrationReview")}
      </Button>
    </div>
  )

  const notices = (
    <>
      {header}
      <RuntimeNotice availability={availability} busy={busy !== null} onRetry={retry} />
      <RegistryNotice
        registry={snapshot.registry}
        error={snapshot.registryError}
        busy={busy !== null}
        onRetry={actions.refresh}
      />
    </>
  )

  const details = selectedPlugin && (
    <PluginDetailsView
      key={selectedPlugin.id}
      plugin={selectedPlugin}
      snapshot={snapshot}
      updates={updates}
      migrations={migrations}
      actions={actions}
      onBack={() => {
        setSelected(null)
      }}
    />
  )

  const sectionContent = (() => {
    switch (section) {
      case "discover":
        return (
          <>
            <DiscoverSection
              snapshot={snapshot}
              updates={updates}
              migrations={migrations}
              actions={actions}
              category={category}
              onCategory={setCategory}
            />
            {migrationNotice}
            {/* With a category chosen, its plugins under the chips are the catalog. */}
            {/* The storefront's sections, else (catalog-only) the built-in home. */}
            {category === null && (
              <DiscoverEditorial
                snapshot={snapshot}
                updates={updates}
                migrations={migrations}
                actions={actions}
                onSection={go}
                onCategory={setCategory}
              />
            )}
          </>
        )
      case "installed":
        return (
          <section aria-labelledby="marketplace-installed-title" className="grid gap-4">
            <div className="grid gap-1">
              <h2 id="marketplace-installed-title" className="text-xl font-semibold tracking-tight">
                {t("marketplace:nav.installed")}
              </h2>
              <p className="text-sm text-muted-foreground">
                {t("marketplace:sections.yours.text")}
              </p>
            </div>
            {migrationNotice}
            <InstalledView
              snapshot={snapshot}
              updates={updates}
              actions={actions}
              onBrowse={() => {
                go("discover")
              }}
            />
          </section>
        )
      case "updates":
        return (
          <section aria-labelledby="marketplace-updates-title" className="grid gap-4">
            <h2 id="marketplace-updates-title" className="text-xl font-semibold tracking-tight">
              {t("marketplace:nav.updates")}
            </h2>
            <UpdatesView updates={updates} actions={actions} snapshot={snapshot} />
          </section>
        )
      case "categories":
        return (
          <CategoriesSection
            snapshot={snapshot}
            onCategory={(id) => {
              go("discover")
              setCategory(id)
            }}
          />
        )
      default:
        return (
          <CollectionSection
            id={section}
            snapshot={snapshot}
            updates={updates}
            migrations={migrations}
            actions={actions}
            onCategories={() => {
              go("categories")
            }}
          />
        )
    }
  })()

  const dialogs = (
    <>
      {configuringItem && (
        <ConfigureDialog
          item={configuringItem}
          client={client}
          onChanged={actions.reload}
          onClose={() => {
            setConfiguring(null)
          }}
        />
      )}
      {review && <PermissionReviewDialog request={review} onClose={closeReview} />}
      {planReview && (
        <InstallPlanDialog
          request={planReview}
          onClose={() => {
            setPlanReview(null)
            returnFocus.current?.focus()
          }}
        />
      )}
      {installRefused && (
        <AlertDialog
          open
          tone="error"
          title={t("marketplace:installRefused.title", {
            name: installRefused.name,
            version: installRefused.version,
          })}
          description={installRefused.error.title}
          onClose={() => {
            setInstallRefused(null)
            returnFocus.current?.focus()
          }}
        >
          {installRefused.error.message !== installRefused.error.title && (
            <p role="alert" className="text-sm break-words select-text">
              {installRefused.error.message}
            </p>
          )}
          {installRefused.error.hint && (
            <p className="text-sm text-muted-foreground">{installRefused.error.hint}</p>
          )}
        </AlertDialog>
      )}
      {removal && (
        <RemovalDialog
          request={removal}
          onClose={() => {
            setRemoval(null)
            returnFocus.current?.focus()
          }}
        />
      )}
      {consentFor && (
        <MigrationConsentDialog
          migration={consentFor}
          onDecide={(approve, decline) =>
            run(
              consentFor.module,
              () => client.decideModuleMigration(consentFor.module, approve, decline),
              decline.length > 0
                ? i18n.t("marketplace:toast.migrationDeclined", { module: consentFor.moduleName })
                : i18n.t("marketplace:toast.migrationConsent", { module: consentFor.moduleName }),
            )
          }
          onClose={() => {
            setConsentFor(null)
            returnFocus.current?.focus()
          }}
        />
      )}
      <ConfirmDialog
        open={packageFile !== null}
        onOpenChange={(open) => {
          if (!open) setPackageFile(null)
        }}
        destructive={false}
        title={t("marketplace:package.title")}
        description={t("marketplace:package.description")}
        confirmLabel={t("marketplace:package.confirm")}
        onConfirm={() => {
          if (packageFile) installPackage(packageFile)
          setPackageFile(null)
        }}
      >
        <p className="font-mono text-xs break-all select-text">{packageFile}</p>
      </ConfirmDialog>
    </>
  )

  if (variant === "diagnostics") {
    return (
      <>
        {notices}
        {migrationNotice}
        <DiagnosticsView
          snapshot={snapshot}
          migrations={migrations}
          actions={actions}
          onReviewMigration={(migration) => {
            returnFocus.current =
              document.activeElement instanceof HTMLElement ? document.activeElement : null
            setConsentFor(migration)
          }}
          onRetryMigration={(migration) => {
            runReported(
              migration.module,
              () => client.retryModuleMigration(migration.module),
              i18n.t("marketplace:toast.migrationRetry", { module: migration.moduleName }),
            )
          }}
        />
        {dialogs}
      </>
    )
  }

  if (variant === "permissions") {
    return (
      <>
        {header}
        <PluginPermissionsReview snapshot={snapshot} actions={actions} />
        {dialogs}
      </>
    )
  }

  // One full-height row between the title bar and the status bar: the
  // sidebar (its right border is the divider) and the content, each
  // scrolling on its own. Spacing scale: sections 32 px apart (gap-8),
  // blocks within a section 16 px (gap-4), rows and cards 12 px (gap-3),
  // a heading and its description 4 px (gap-1); content padding 24 px
  // sideways and 20 px at the top and bottom (px-6 py-5).
  const marketplace = (
    <div ref={root} data-slot="marketplace" className="flex min-h-0 flex-1">
      <MarketplaceSidebar
        // Results are not a section: no item looks selected meanwhile.
        current={searching ? null : section}
        onInstallFile={actions.installFile}
        installFileDisabled={busy !== null || blocked !== null}
        query={query}
        onQuery={(next) => {
          // A plugin's page closes for the results; Back no longer applies.
          if (next.trim() !== "" && selected !== null) {
            scrolledTo.current = null
            setSelected(null)
          }
          setQuery(next)
          if (next.trim() === "") setSearched("")
        }}
        installed={snapshot.installed.length}
        updates={updates.length}
        compact={compact}
        onSelect={go}
      />
      <div
        data-slot="marketplace-content"
        className="@container relative min-h-0 min-w-0 flex-1 [scrollbar-gutter:stable] overflow-x-hidden overflow-y-auto"
      >
        <div ref={page} className="mx-auto grid w-full max-w-5xl content-start gap-8 px-6 py-5">
          {notices}
          {details ??
            (searching ? (
              <SearchResults
                query={searched}
                snapshot={snapshot}
                updates={updates}
                migrations={migrations}
                actions={actions}
              />
            ) : (
              sectionContent
            ))}
        </div>
      </div>
      {dialogs}
    </div>
  )
  // The signed storefront (editorial Discover, collections, listings), when the agent has one.
  return (
    <StorefrontProvider
      runtime={snapshot.runtime}
      available={snapshot.available}
      seq={snapshot.registry?.seq}
    >
      {marketplace}
    </StorefrontProvider>
  )
}

/** Settings › Logs & Diagnostics: the plugin diagnostics (module migrations, registry). */
export function PluginDiagnostics() {
  return <PluginsSettings variant="diagnostics" />
}

/** Settings › Permissions: every installed plugin's permissions, to review and revoke. */
export function PluginPermissions() {
  return <PluginsSettings variant="permissions" />
}
