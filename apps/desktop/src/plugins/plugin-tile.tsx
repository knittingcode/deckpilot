// The pieces the Marketplace home, the search results and a plugin's page
// share: the plugin's icon (with a category glyph or a letter when there is
// none), its state control (Install, Installing…, Open, Update, …) and the
// catalog tile.

import {
  ArchiveRestore,
  BadgeCheck,
  Bell,
  CircleAlert,
  CircleCheck,
  Code,
  Download,
  Gamepad2,
  Gauge,
  House,
  Loader2,
  type LucideIcon,
  Music,
  Power,
  RefreshCw,
  RotateCcw,
  Settings2,
  SquareArrowOutUpRight,
  Timer,
} from "lucide-react"
import { type ReactNode, useId } from "react"

import { Button } from "@/components/ui/button"
import { useTranslation } from "@/i18n"
import { cn } from "@/lib/utils"

import {
  categoryName,
  developerName,
  type EntryContext,
  type EntryState,
  primaryCategory,
  useEntry,
} from "./catalog"
import { errorSentence } from "./dialog-error"
import type { MarketplaceActions } from "./marketplace"
import type { UpdateCandidate } from "./model"
import type { MediaRef } from "./storefront-types"
import { VerifiedImage } from "./storefront/verified-image"
import type { AvailablePlugin, MarketplaceSnapshot } from "./types"

const CATEGORY_GLYPHS: Record<string, LucideIcon> = {
  "developer-tools": Code,
  productivity: Timer,
  communication: Bell,
  monitoring: Gauge,
  media: Music,
  "smart-home": House,
  other: Gamepad2,
}

const SIZES = {
  sm: { box: "size-8 rounded-md", glyph: "size-4", letter: "text-sm" },
  md: { box: "size-11 rounded-xl", glyph: "size-5", letter: "text-lg" },
  lg: { box: "size-16 rounded-2xl", glyph: "size-7", letter: "text-2xl" },
} as const

/**
 * A plugin's icon: an installed version's verified PNG (`src`, the agent's
 * `PluginDetails.icon`), else the storefront listing's verified image
 * (`media`, `AvailablePlugin.icon`, fetched by sha256). Without either, or
 * while it loads or when it cannot be shown: the glyph of its category, or
 * its initial. Never a remote image.
 */
export function PluginAvatar({
  name,
  category,
  src,
  media,
  size = "md",
  muted = false,
}: {
  name: string
  category?: string | undefined
  src?: string | undefined
  media?: MediaRef | undefined
  size?: keyof typeof SIZES
  muted?: boolean
}) {
  const s = SIZES[size]
  if (!src?.startsWith("data:image/png;base64,") && media) {
    return (
      <VerifiedImage
        media={media}
        alt=""
        decorative
        aspect={1}
        className={cn("shrink-0 border", s.box, muted && "opacity-60")}
        fallback={<Monogram name={name} category={category} size={size} muted={muted} bare />}
      />
    )
  }
  if (src?.startsWith("data:image/png;base64,")) {
    return (
      <img
        src={src}
        alt=""
        aria-hidden
        className={cn("shrink-0 border object-cover", s.box, muted && "opacity-60")}
      />
    )
  }
  return <Monogram name={name} category={category} size={size} muted={muted} />
}

/** The category glyph or the initial, in the icon's box (`bare`: filling a box it is in). */
function Monogram({
  name,
  category,
  size,
  muted,
  bare = false,
}: {
  name: string
  category: string | undefined
  size: keyof typeof SIZES
  muted: boolean
  bare?: boolean
}) {
  const s = SIZES[size]
  const Glyph = category ? CATEGORY_GLYPHS[category] : undefined
  return (
    <div
      aria-hidden
      data-slot="plugin-avatar"
      className={cn(
        "flex shrink-0 items-center justify-center bg-gradient-to-br from-muted to-secondary font-semibold",
        bare ? "size-full" : cn("border", s.box),
        muted ? "text-muted-foreground" : "text-foreground",
      )}
    >
      {Glyph ? (
        <Glyph className={s.glyph} />
      ) : (
        <span className={s.letter}>{name.trim().charAt(0).toUpperCase() || "?"}</span>
      )}
    </div>
  )
}

/** A short status next to a control (Installed, Incompatible, …). */
function Status({
  icon: Icon,
  children,
  tone = "muted",
}: {
  icon?: LucideIcon
  children: string
  tone?: "muted" | "destructive"
}) {
  return (
    <span
      className={cn(
        "inline-flex h-8 shrink-0 items-center gap-1.5 text-xs font-medium whitespace-nowrap",
        tone === "destructive" ? "text-destructive" : "text-muted-foreground",
      )}
    >
      {Icon && <Icon className="size-3.5" aria-hidden />}
      {children}
    </span>
  )
}

/**
 * The one thing to do with a plugin now: Install, Update, Open, Configure,
 * Enable, Retry, or why there is nothing to do. Every change goes through
 * the Marketplace's own flows (consent, plans, verification); "Installed"
 * shows only once the agent lists the plugin as installed.
 */
export function StateControl({
  plugin,
  state,
  context,
  actions,
  size = "sm",
}: {
  plugin: AvailablePlugin
  state: EntryState
  context: EntryContext
  actions: MarketplaceActions
  size?: "sm" | "default"
}) {
  const { t } = useTranslation()
  const name = plugin.name
  const busy = actions.busy !== null
  const pill = size === "sm" ? "rounded-full px-3.5" : "px-4"
  switch (state.kind) {
    case "install":
      return (
        <Button
          size={size}
          className={pill}
          disabled={busy || actions.blocked !== null}
          title={actions.blocked ?? undefined}
          aria-label={t("marketplace:state.installNamed", { name })}
          onClick={(e) => {
            e.stopPropagation()
            actions.install(plugin, state.version)
          }}
        >
          <Download /> {t("marketplace:state.install")}
        </Button>
      )
    case "installing":
      return (
        <Button size={size} className={pill} variant="secondary" disabled aria-live="polite">
          <Loader2 className="animate-spin" /> {t("marketplace:state.installing")}
        </Button>
      )
    case "update":
      return (
        <Button
          size={size}
          className={pill}
          disabled={busy || actions.blocked !== null}
          title={actions.blocked ?? undefined}
          aria-label={t("marketplace:state.updateNamed", { name })}
          onClick={(e) => {
            e.stopPropagation()
            actions.update(state.candidate)
          }}
        >
          <RefreshCw /> {t("marketplace:state.update")}
        </Button>
      )
    case "updating":
      return (
        <Button size={size} className={pill} variant="secondary" disabled aria-live="polite">
          <Loader2 className="animate-spin" /> {t("marketplace:state.updating")}
        </Button>
      )
    case "open":
      return (
        <Button
          size={size}
          className={pill}
          variant="secondary"
          aria-label={t("marketplace:state.openNamed", { name })}
          onClick={(e) => {
            e.stopPropagation()
            actions.openView(plugin.id, state.entry)
          }}
        >
          <SquareArrowOutUpRight /> {t("marketplace:state.open")}
        </Button>
      )
    case "configure":
      return (
        <Button
          size={size}
          className={pill}
          variant="secondary"
          aria-label={t("marketplace:state.configureNamed", { name })}
          onClick={(e) => {
            e.stopPropagation()
            if (context.installed) actions.configure(context.installed)
          }}
        >
          <Settings2 /> {t("marketplace:state.configure")}
        </Button>
      )
    case "enable":
      return (
        <Button
          size={size}
          className={pill}
          variant="outline"
          disabled={busy}
          aria-label={t("marketplace:state.enableNamed", { name })}
          onClick={(e) => {
            e.stopPropagation()
            if (context.installed) actions.setEnabled(context.installed, true)
          }}
        >
          <Power /> {t("marketplace:state.enable")}
        </Button>
      )
    case "installed":
      return <Status icon={CircleCheck}>{t("marketplace:state.installed")}</Status>
    case "attention":
      return (
        <Status icon={CircleAlert} tone="destructive">
          {t("marketplace:state.attention")}
        </Status>
      )
    case "incompatible":
      return <Status>{t("marketplace:state.incompatible")}</Status>
    case "withdrawn":
      return <Status>{t("marketplace:state.withdrawn")}</Status>
    case "bundled":
      return state.restorable ? (
        <Button
          size={size}
          className={pill}
          variant="outline"
          disabled={busy}
          aria-label={t("marketplace:installed.removed.restoreNamed", { name })}
          onClick={(e) => {
            e.stopPropagation()
            actions.restore(plugin.id, name, false)
          }}
        >
          <ArchiveRestore /> {t("marketplace:installed.removed.restore")}
        </Button>
      ) : (
        <Status>{t("marketplace:state.bundled")}</Status>
      )
    case "failed":
      return (
        <Button
          size={size}
          className={pill}
          variant="outline"
          disabled={busy}
          aria-label={
            state.failure.operation === "update"
              ? t("marketplace:state.retryUpdateNamed", { name })
              : t("marketplace:state.retryNamed", { name })
          }
          onClick={(e) => {
            e.stopPropagation()
            actions.retry(plugin)
          }}
        >
          <RotateCcw /> {t("marketplace:state.retry")}
        </Button>
      )
  }
}

/** Why the last install or update of a plugin failed, in the window's language. */
export function FailureText({ state, className }: { state: EntryState; className?: string }) {
  const { t } = useTranslation()
  if (state.kind !== "failed") return null
  const { failure } = state
  return (
    <p role="alert" className={cn("text-xs text-destructive", className)}>
      <span className="font-medium">
        {failure.operation === "install"
          ? t("marketplace:state.installFailed")
          : t("marketplace:state.updateFailed")}
      </span>{" "}
      {errorSentence(failure.error, false)}
      {failure.error.hint && <span className="text-muted-foreground"> {failure.error.hint}</span>}
    </p>
  )
}

/** The developer, and whether the registry verified who they are. */
export function DeveloperLine({
  plugin,
  official,
  extra,
  className,
}: {
  plugin: AvailablePlugin
  official?: boolean
  /** Shown after the developer, e.g. the category. */
  extra?: string | undefined
  className?: string
}) {
  const { t } = useTranslation()
  return (
    <span
      className={cn(
        "flex min-w-0 flex-wrap items-center gap-x-1 text-xs text-muted-foreground",
        className,
      )}
    >
      <span>{developerName(plugin)}</span>
      {plugin.publisherInfo?.verified && (
        <BadgeCheck
          className="size-3.5 shrink-0"
          role="img"
          aria-label={t("marketplace:publisher.verified")}
        />
      )}
      {official && (
        <span>
          <span aria-hidden>· </span>
          {t("marketplace:badge.official")}
        </span>
      )}
      {extra && (
        <span>
          <span aria-hidden>· </span>
          {extra}
        </span>
      )}
    </span>
  )
}

/**
 * One catalog entry: icon, name, developer, category, summary and its
 * state control. The tile opens the plugin's page; the control acts.
 */
export function PluginTile({
  plugin,
  snapshot,
  updates,
  actions,
  official,
  note,
}: {
  plugin: AvailablePlugin
  snapshot: MarketplaceSnapshot
  updates: UpdateCandidate[]
  actions: MarketplaceActions
  official: boolean
  /** A line above the summary, e.g. when it was added. */
  note?: string | undefined
}) {
  const { t } = useTranslation()
  const { state, context } = useEntry(plugin, snapshot, updates, actions)
  const category = primaryCategory(plugin)
  // Unique per tile: a plugin can be listed in several sections at once.
  const id = `plugin-tile-${useId().replace(/:/g, "")}`
  const muted = state.kind === "incompatible" || state.kind === "withdrawn"
  return (
    <div
      className={cn(
        "group/tile relative flex h-full items-start gap-3 rounded-xl border bg-card p-3 transition-colors hover:bg-accent/40",
        state.kind === "failed" && "border-destructive/50",
      )}
    >
      <PluginAvatar
        name={plugin.name}
        category={category}
        src={context.installed?.icon}
        media={plugin.icon}
        muted={muted}
      />
      <div className="grid min-w-0 flex-1 gap-0.5">
        <button
          type="button"
          id={`${id}-name`}
          aria-describedby={`${id}-summary`}
          className="w-fit max-w-full truncate text-left text-sm font-semibold outline-none after:absolute after:inset-0 after:rounded-xl focus-visible:after:ring-[3px] focus-visible:after:ring-ring/50"
          onClick={() => {
            actions.openDetails(plugin.id)
          }}
        >
          {plugin.name}
        </button>
        <DeveloperLine
          plugin={plugin}
          official={official}
          extra={category ? categoryName(category) : undefined}
        />
        <span id={`${id}-summary`} className="mt-1 line-clamp-2 text-xs text-foreground/80">
          {plugin.summary}
        </span>
        {note && <span className="mt-0.5 text-[11px] text-muted-foreground">{note}</span>}
        {plugin.deprecated && (
          <span className="text-xs text-muted-foreground">{t("marketplace:badge.deprecated")}</span>
        )}
        <FailureText state={state} className="mt-1" />
      </div>
      {/* Above the tile's stretched button, so it stays clickable. */}
      <div className="relative z-10 shrink-0 self-center">
        <StateControl plugin={plugin} state={state} context={context} actions={actions} />
      </div>
    </div>
  )
}

/** A grid of tiles: one column when narrow, two when there is room. */
export function TileGrid<T>({
  label,
  items,
  keyOf,
  render,
}: {
  label: string
  items: T[]
  keyOf: (item: T) => string
  render: (item: T) => ReactNode
}) {
  return (
    <ul aria-label={label} className="grid gap-3 @2xl:grid-cols-2">
      {items.map((item) => (
        <li key={keyOf(item)} className="min-w-0">
          {render(item)}
        </li>
      ))}
    </ul>
  )
}
