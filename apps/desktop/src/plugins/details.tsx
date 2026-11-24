import {
  ArrowLeft,
  BadgeCheck,
  ChevronDown,
  Copy,
  Library,
  Settings2,
  ShieldCheck,
  Stethoscope,
} from "lucide-react"
import { type ReactNode, useEffect, useRef, useState } from "react"
import { toast } from "sonner"

import { Button } from "@/components/ui/button"
import { type TranslationKey, useTranslation } from "@/i18n"
import { cn } from "@/lib/utils"

import {
  categoryName,
  developerName,
  isBundledOnly,
  officialIds,
  primaryCategory,
  shownVersion,
  useEntry,
} from "./catalog"
import { Notice, PermissionList } from "./components"
import { DependencySection, DependentsSection } from "./dependencies"
import { ActionList, HealthNotice, InstalledPermissions } from "./installed"
import type { MarketplaceActions } from "./marketplace"
import {
  compareSemver,
  declaredConfiguration,
  formatDate,
  knownUninstalled,
  platformLabel,
  platformsText,
  stateLabel,
  storageText,
  type UpdateCandidate,
} from "./model"
import { grantsOf } from "./permissions"
import { DeveloperLine, FailureText, PluginAvatar, StateControl } from "./plugin-tile"
import type { ListingLinks } from "./storefront-types"
import { Highlights } from "./storefront/highlights"
import { SafeMarkdown } from "./storefront/safe-markdown"
import { ScreenshotCarousel } from "./storefront/screenshot-carousel"
import { clip, LIMITS, safeHttpsUrl } from "./storefront/text"
import { useListing } from "./storefront/use-storefront"
import { VerifiedImage } from "./storefront/verified-image"
import { VersionHistory } from "./storefront/version-history"
import { noteFor } from "./storefront/resolve"
import { WhatsNew } from "./storefront/whats-new"
import type {
  AvailablePlugin,
  MarketplaceSnapshot,
  ModuleMigration,
  PluginDetails,
  PluginOrigin,
  PluginScreenshot,
} from "./types"

const ORIGIN_LABELS = {
  registry: "marketplace:details.originOf.registry",
  sideload: "marketplace:details.originOf.sideload",
  dev: "marketplace:details.originOf.dev",
  bundled: "marketplace:details.originOf.bundled",
} as const satisfies Record<PluginOrigin["type"], TranslationKey>

/** Only images the agent sent as data, never a remote one. */
function isLocalImage(src: string): boolean {
  return /^data:image\/(png|jpeg);base64,/.test(src)
}

function SectionTitle({ id, children }: { id: string; children: ReactNode }) {
  return (
    <h4 id={id} className="text-base font-semibold">
      {children}
    </h4>
  )
}

/** One fact of the information strip (version, DeckPilot, platforms, …). */
function Fact({ label, value, hint }: { label: string; value: string; hint?: string }) {
  return (
    <div className="grid min-w-0 content-start gap-0.5 px-4 py-2 first:pl-0">
      <dt className="text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
        {label}
      </dt>
      <dd className="truncate text-sm font-semibold" title={value}>
        {value}
      </dd>
      {hint && <dd className="text-xs text-muted-foreground">{hint}</dd>}
    </div>
  )
}

/**
 * One registry plugin, like a store's product page: who makes it and the
 * one thing to do with it first; then what it needs, what it is, what is
 * new, what it may do, who vouches for it and where to learn more. The
 * technical record (ids, sources, raw reasons) is folded into Details.
 * Nothing is invented: sections without data are left out.
 */
export function PluginDetailsView({
  plugin,
  snapshot,
  updates,
  migrations,
  actions,
  onBack,
}: {
  plugin: AvailablePlugin
  snapshot: MarketplaceSnapshot
  updates: UpdateCandidate[]
  migrations: ModuleMigration[]
  actions: MarketplaceActions
  onBack: () => void
}) {
  const { t } = useTranslation()
  const heading = useRef<HTMLHeadingElement>(null)

  // Move focus to the page that replaced the list.
  useEffect(() => {
    heading.current?.focus()
  }, [])

  const { state, context } = useEntry(plugin, snapshot, updates, actions)
  // The storefront listing (author copy and media); `null` without one.
  const listing = useListing(plugin.id)
  const installed = context.installed
  const known = installed ? undefined : knownUninstalled(snapshot, plugin.id)
  const versions = [...plugin.versions].sort((a, b) => compareSemver(b.version, a.version))
  const shown = shownVersion(plugin)
  const category = primaryCategory(plugin)
  const official = officialIds(snapshot, migrations).has(plugin.id)
  const configurable =
    installed !== undefined &&
    (() => {
      const d = declaredConfiguration(installed.manifest)
      return d.settings.length > 0 || d.credentials.length > 0
    })()
  const description = plugin.description ?? installed?.description ?? plugin.summary
  const screenshots = (plugin.screenshots ?? []).filter((s) => isLocalImage(s.src))
  const notes = versions.filter((v) => v.releaseNotes)
  // The listing's notes of the version shown, else the registry's newest.
  const listedNote = noteFor(listing?.releaseNotes, shown?.version)
  const incompatible = shown && (shown.incompatible ?? []).length > 0

  /** Why the primary action is not offered, in words. */
  const reason =
    state.kind === "install" || state.kind === "update"
      ? actions.blocked
      : state.kind === "incompatible"
        ? shown?.deckpilot
          ? t("marketplace:details.needsDeckPilot", { range: shown.deckpilot })
          : t("marketplace:details.noVersion")
        : state.kind === "bundled"
          ? t("marketplace:details.bundledOnly")
          : state.kind === "withdrawn"
            ? t("marketplace:details.withdrawnShort")
            : null

  return (
    <div
      className="@container grid gap-6"
      // Escape goes back, unless something inside (a menu, a field) used it.
      onKeyDown={(e) => {
        if (e.key === "Escape" && !e.defaultPrevented) {
          e.preventDefault()
          onBack()
        }
      }}
    >
      <div>
        <Button variant="ghost" size="sm" className="-ml-2" onClick={onBack}>
          <ArrowLeft /> {t("marketplace:details.back")}
        </Button>
      </div>
      <article className="grid gap-8" aria-labelledby="plugin-detail-name">
        {listing?.promoArt && (
          <VerifiedImage
            media={listing.promoArt.media}
            alt={clip(listing.promoArt.alt, LIMITS.alt)}
            aspect={4}
            className="w-full rounded-2xl ring-1 ring-border"
          />
        )}
        <header className="flex flex-col gap-4 @xl:flex-row @xl:items-start">
          <PluginAvatar
            name={plugin.name}
            category={category}
            src={installed?.icon}
            media={plugin.icon ?? listing?.icon}
            size="lg"
            muted={state.kind === "incompatible" || state.kind === "withdrawn"}
          />
          <div className="grid min-w-0 flex-1 gap-1">
            <h3
              id="plugin-detail-name"
              ref={heading}
              tabIndex={-1}
              className="text-2xl leading-8 font-semibold tracking-tight outline-none"
            >
              {plugin.name}
            </h3>
            <DeveloperLine plugin={plugin} official={official} className="text-sm" />
            {listing?.promo && (
              <p dir="auto" className="text-sm [overflow-wrap:anywhere] text-foreground/80">
                {clip(listing.promo, LIMITS.promo)}
              </p>
            )}
            {category && <p className="text-sm text-muted-foreground">{categoryName(category)}</p>}
            <div className="mt-3 flex flex-wrap items-center gap-2">
              <StateControl
                plugin={plugin}
                state={state}
                context={context}
                actions={actions}
                size="default"
              />
              {installed && configurable && state.kind !== "configure" && (
                <Button
                  variant="outline"
                  onClick={() => {
                    actions.configure(installed)
                  }}
                >
                  <Settings2 /> {t("marketplace:state.configure")}
                </Button>
              )}
              {installed && (
                <Button variant="ghost" onClick={actions.showInstalled}>
                  <Library /> {t("marketplace:details.manage")}
                </Button>
              )}
            </div>
            {reason && (
              <p id="plugin-install-reason" className="text-sm text-muted-foreground">
                {reason}
              </p>
            )}
            <FailureText state={state} className="text-sm" />
            {installed?.summary.installedFor && (
              <p className="text-sm text-muted-foreground">
                {t("marketplace:details.installedFor", { plugin: installed.summary.installedFor })}
              </p>
            )}
            {known && (
              <p className="text-sm text-muted-foreground">
                {t("marketplace:details.knownBefore", {
                  name: known.name,
                  version: known.lastVersion,
                  date: formatDate(known.lastSeenAt),
                })}
              </p>
            )}
          </div>
        </header>

        <dl
          className="grid grid-cols-2 divide-x border-y py-2 @xl:grid-cols-4"
          aria-label={t("marketplace:details.about")}
        >
          <Fact
            label={t("marketplace:details.version")}
            value={installed?.summary.version ?? shown?.version ?? "–"}
            hint={
              installed && shown && shown.version !== installed.summary.version
                ? t("marketplace:details.latestIs", { version: shown.version })
                : installed
                  ? stateLabel(installed.summary.state)
                  : undefined
            }
          />
          <Fact
            label={t("marketplace:details.requires")}
            value={
              shown?.deckpilot
                ? t("marketplace:details.deckpilotRange", { range: shown.deckpilot })
                : t("marketplace:details.notStated")
            }
          />
          <Fact
            label={t("marketplace:details.platforms")}
            value={platformsText(shown?.platforms)}
          />
          <Fact
            label={t("marketplace:details.thisComputer")}
            value={
              shown && !incompatible && !shown.withdrawn
                ? t("marketplace:details.compatible")
                : t("marketplace:details.notCompatible")
            }
            hint={platformLabel(snapshot.runtime.platform)}
          />
        </dl>

        {plugin.deprecated && (
          <Notice title={t("marketplace:details.deprecated")}>
            <p>{plugin.deprecated}</p>
          </Notice>
        )}
        {shown?.withdrawn && (
          <Notice
            tone="destructive"
            title={t("marketplace:details.withdrawn", { version: shown.version })}
          >
            <p>{shown.withdrawn}</p>
          </Notice>
        )}

        {listing && listing.screenshots.length > 0 ? (
          <ScreenshotCarousel screenshots={listing.screenshots} name={plugin.name} />
        ) : (
          screenshots.length > 0 && <Gallery screenshots={screenshots} name={plugin.name} />
        )}

        <section aria-labelledby="plugin-detail-description" className="grid gap-2">
          <SectionTitle id="plugin-detail-description">
            {t("marketplace:details.description")}
          </SectionTitle>
          {listing?.descriptionMd ? (
            <SafeMarkdown source={listing.descriptionMd} />
          ) : (
            <p className="max-w-prose text-sm leading-6 whitespace-pre-line select-text">
              {description}
            </p>
          )}
        </section>

        {listing && <Highlights highlights={listing.highlights} />}

        {shown && listedNote ? (
          <WhatsNew version={shown} note={listedNote} />
        ) : (
          notes.length > 0 &&
          notes[0] && (
            <section aria-labelledby="plugin-detail-news" className="grid gap-2">
              <SectionTitle id="plugin-detail-news">
                {t("marketplace:details.whatsNew")}
              </SectionTitle>
              <p className="text-xs text-muted-foreground">
                {t("marketplace:details.versionLine", { version: notes[0].version })}
                {notes[0].publishedAt !== undefined && ` · ${formatDate(notes[0].publishedAt)}`}
              </p>
              <p className="max-w-prose text-sm whitespace-pre-line select-text">
                {notes[0].releaseNotes}
              </p>
            </section>
          )
        )}

        {installed && installed.summary.version === shown?.version ? (
          <DependencySection
            title={t("plugins:dependencies.title")}
            dependencies={installed.dependencies ?? shown.dependencies ?? []}
          />
        ) : (
          shown && (
            <DependencySection
              title={t("plugins:dependencies.titleOfVersion", { version: shown.version })}
              dependencies={shown.dependencies ?? []}
            />
          )
        )}
        {installed && <DependentsSection dependents={installed.dependents ?? []} />}

        {installed ? (
          <section aria-labelledby="plugin-detail-permissions" className="grid gap-2">
            <SectionTitle id="plugin-detail-permissions">
              {t("marketplace:details.grantedPermissions")}
            </SectionTitle>
            <p className="text-xs text-muted-foreground">
              {t("marketplace:details.grantedPermissionsHint")}
            </p>
            <InstalledPermissions
              item={installed}
              actions={actions}
              label={t("marketplace:details.permissionsLabel")}
            />
          </section>
        ) : (
          shown && (
            <section aria-labelledby="plugin-detail-permissions" className="grid gap-2">
              <SectionTitle id="plugin-detail-permissions">
                {t("marketplace:details.permissions", { version: shown.version })}
              </SectionTitle>
              <p className="text-xs text-muted-foreground">
                {t("marketplace:details.permissionsHint")}
              </p>
              <PermissionList
                label={t("marketplace:details.permissionsLabel")}
                grants={grantsOf(shown.permissions)}
              />
            </section>
          )
        )}

        {installed ? (
          <section aria-labelledby="plugin-detail-actions" className="grid gap-2">
            <SectionTitle id="plugin-detail-actions">
              {t("marketplace:details.actions")}
            </SectionTitle>
            {installed.summary.actions.length === 0 ? (
              <p className="text-xs text-muted-foreground">{t("marketplace:details.noActions")}</p>
            ) : (
              <ActionList item={installed} actions={actions} />
            )}
          </section>
        ) : (
          <section aria-labelledby="plugin-detail-actions" className="grid gap-2">
            <SectionTitle id="plugin-detail-actions">
              {t("marketplace:details.actions")}
            </SectionTitle>
            <p className="text-xs text-muted-foreground">
              {t("marketplace:details.actionsNotListed")}
            </p>
          </section>
        )}

        {installed && (
          <UsageSection
            item={installed}
            migrating={migrations.some((m) => m.plugin === plugin.id)}
            onDiagnostics={actions.openDiagnostics}
          />
        )}

        <TrustSection
          plugin={plugin}
          snapshot={snapshot}
          official={official}
          origin={installed?.summary.origin.type}
          listed={snapshot.available.some((p) => p.id === plugin.id)}
        />

        <LinksSection plugin={plugin} listed={listing?.links} />

        <VersionHistory
          versions={versions}
          latest={plugin.latest}
          installed={installed?.summary.version}
          notes={listing?.releaseNotes}
          skipNotesOf={listedNote ? shown?.version : notes[0]?.version}
        />

        <TechnicalDetails
          plugin={plugin}
          snapshot={snapshot}
          installed={installed}
          incompatibleReasons={shown?.incompatible ?? []}
        />
      </article>
    </div>
  )
}

/** Screenshots the agent sent, side by side, scrolling sideways. */
function Gallery({ screenshots, name }: { screenshots: PluginScreenshot[]; name: string }) {
  const { t } = useTranslation()
  return (
    <section aria-labelledby="plugin-detail-screenshots" className="grid gap-2">
      <SectionTitle id="plugin-detail-screenshots">
        {t("marketplace:details.screenshots")}
      </SectionTitle>
      <ul
        className="relative flex snap-x snap-mandatory gap-3 overflow-x-auto pb-2"
        aria-label={t("marketplace:details.screenshots")}
      >
        {screenshots.map((shot, i) => (
          <li key={shot.src.slice(-32) + String(i)} className="shrink-0 snap-start">
            <figure className="grid gap-1">
              <img
                src={shot.src}
                alt={shot.caption ?? t("marketplace:details.screenshotOf", { name, number: i + 1 })}
                className="h-56 w-auto rounded-lg border object-contain"
              />
              {shot.caption && (
                <figcaption className="text-xs text-muted-foreground">{shot.caption}</figcaption>
              )}
            </figure>
          </li>
        ))}
      </ul>
    </section>
  )
}

/** Who vouches for the plugin: the signed registry, its publisher record, DeckPilot's keys. */
function TrustSection({
  plugin,
  snapshot,
  official,
  origin,
  listed,
}: {
  plugin: AvailablePlugin
  snapshot: MarketplaceSnapshot
  official: boolean
  /** Where the installed copy came from; absent when not installed. */
  origin: PluginOrigin["type"] | undefined
  /** The registry lists the plugin. */
  listed: boolean
}) {
  const { t } = useTranslation()
  const registry = snapshot.registry
  const rows: { title: string; text: string; ok: boolean }[] = []
  // The installed copy: what it was checked against when installed.
  if (origin === "bundled") {
    rows.push({
      title: t("marketplace:trust.bundled.title"),
      text: t("marketplace:trust.bundled.text"),
      ok: true,
    })
  } else if (origin === "sideload") {
    rows.push({
      title: t("marketplace:trust.sideload.title"),
      text: t("marketplace:trust.sideload.text"),
      ok: true,
    })
  } else if (origin === "dev") {
    rows.push({
      title: t("marketplace:trust.dev.title"),
      text: t("marketplace:trust.dev.text"),
      ok: false,
    })
  }
  if (listed) {
    rows.push({
      title: t("marketplace:trust.registry.title"),
      text:
        registry?.freshness === "fresh"
          ? t("marketplace:trust.registry.fresh")
          : registry?.freshness === "stale"
            ? t("marketplace:trust.registry.stale")
            : t("marketplace:trust.registry.unverified"),
      ok: registry?.freshness === "fresh",
    })
  }
  if (plugin.publisherInfo) {
    rows.push({
      title: t("marketplace:trust.publisher.title"),
      text: plugin.publisherInfo.verified
        ? t("marketplace:trust.publisher.verified", {
            name: developerName(plugin),
            login: plugin.publisherInfo.login,
          })
        : t("marketplace:trust.publisher.notVerified", { name: developerName(plugin) }),
      ok: plugin.publisherInfo.verified,
    })
  }
  if (plugin.officialSignature) {
    rows.push({
      title: t("marketplace:trust.official.title"),
      text: t("marketplace:trust.official.signed", {
        threshold: plugin.officialSignature.threshold,
        count: plugin.officialSignature.keys,
      }),
      ok: true,
    })
  } else if (official) {
    rows.push({
      title: t("marketplace:trust.official.title"),
      text: isBundledOnly(plugin, snapshot.runtime)
        ? t("marketplace:trust.official.bundled")
        : t("marketplace:trust.official.firstParty"),
      ok: true,
    })
  }
  return (
    <section aria-labelledby="plugin-detail-trust" className="grid gap-2">
      <SectionTitle id="plugin-detail-trust">{t("marketplace:trust.title")}</SectionTitle>
      <ul
        className="grid gap-3 rounded-xl border bg-card p-4"
        aria-label={t("marketplace:trust.title")}
      >
        {rows.map((row) => (
          <li key={row.title} className="flex items-start gap-3 text-sm">
            {row.ok ? (
              <ShieldCheck className="mt-0.5 size-4 shrink-0 text-success" aria-hidden />
            ) : (
              <BadgeCheck className="mt-0.5 size-4 shrink-0 text-muted-foreground" aria-hidden />
            )}
            <span className="grid gap-0.5">
              <span className="font-medium">{row.title}</span>
              <span className="text-xs text-muted-foreground">{row.text}</span>
            </span>
          </li>
        ))}
      </ul>
    </section>
  )
}

/**
 * The plugin's links. The app does not open web pages itself, so each can
 * be copied. The listing's links (storefront) count only when they are
 * https by the storefront's rules; a plugin not of DeckPilot's own
 * namespace shows every link in full.
 */
function LinksSection({ plugin, listed }: { plugin: AvailablePlugin; listed?: ListingLinks }) {
  const { t } = useTranslation()
  const firstParty = plugin.id.startsWith("deckpilot.")
  const fromListing = (label: string, url: unknown) => {
    const safe = safeHttpsUrl(url)
    return safe ? { label, url: safe, full: !firstParty } : null
  }
  const links = [
    plugin.homepage && { label: t("marketplace:details.homepage"), url: plugin.homepage },
    plugin.sourceRepository && {
      label: t("marketplace:details.source"),
      url: `https://github.com/${plugin.sourceRepository}`,
    },
    plugin.documentationUrl && {
      label: t("marketplace:details.documentation"),
      url: plugin.documentationUrl,
    },
    plugin.supportUrl && { label: t("marketplace:details.support"), url: plugin.supportUrl },
    fromListing(t("marketplace:details.documentation"), listed?.documentation),
    fromListing(t("marketplace:details.support"), listed?.support),
    fromListing(t("marketplace:storefront.changelog"), listed?.changelog),
  ]
    .filter((l): l is { label: string; url: string; full?: boolean } => Boolean(l))
    // The listing's link of a kind replaces the proposed field's (same label).
    .filter((l, i, all) => all.findLastIndex((m) => m.label === l.label) === i)
  if (links.length === 0) return null
  return (
    <section aria-labelledby="plugin-detail-links" className="grid gap-2">
      <SectionTitle id="plugin-detail-links">{t("marketplace:details.links")}</SectionTitle>
      {!firstParty && listed && (
        <p className="text-xs text-muted-foreground">{t("marketplace:storefront.fullLink")}</p>
      )}
      <ul className="grid divide-y border-y" aria-label={t("marketplace:details.links")}>
        {links.map((link) => (
          <li key={link.label} className="flex items-center gap-3 py-2 text-sm">
            <span className="w-32 shrink-0 text-muted-foreground">{link.label}</span>
            <span className="min-w-0 flex-1 truncate select-text" title={link.url}>
              {link.full ? link.url : link.url.replace(/^https:\/\//, "")}
            </span>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label={t("marketplace:details.copyLink", { label: link.label })}
              onClick={() => {
                navigator.clipboard.writeText(link.url).then(
                  () => toast.success(t("marketplace:details.linkCopied")),
                  () => undefined,
                )
              }}
            >
              <Copy />
            </Button>
          </li>
        ))}
      </ul>
    </section>
  )
}

/** The technical record, folded: ids, license, registry, raw reasons, storage. */
function TechnicalDetails({
  plugin,
  snapshot,
  installed,
  incompatibleReasons,
}: {
  plugin: AvailablePlugin
  snapshot: MarketplaceSnapshot
  installed: PluginDetails | undefined
  incompatibleReasons: string[]
}) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const facts: [string, string][] = [
    [t("marketplace:details.pluginId"), plugin.id],
    ...(plugin.license
      ? [[t("marketplace:details.license"), plugin.license] as [string, string]]
      : []),
    ...(plugin.categories && plugin.categories.length > 0
      ? [
          [t("marketplace:details.categories"), plugin.categories.map(categoryName).join(", ")] as [
            string,
            string,
          ],
        ]
      : []),
    ...(snapshot.registry
      ? [
          [
            t("marketplace:details.registry"),
            snapshot.registry.seq !== undefined
              ? t("marketplace:details.registrySeq", {
                  source: snapshot.registry.source,
                  seq: snapshot.registry.seq,
                })
              : snapshot.registry.source,
          ] as [string, string],
        ]
      : []),
    ...(installed
      ? [
          [t("marketplace:details.origin"), t(ORIGIN_LABELS[installed.summary.origin.type])] as [
            string,
            string,
          ],
        ]
      : []),
    ...(installed?.storage
      ? [[t("marketplace:details.storage"), storageText(installed.storage)] as [string, string]]
      : []),
    ...incompatibleReasons.map(
      (reason) => [t("marketplace:details.incompatibleReason"), reason] as [string, string],
    ),
  ]
  return (
    <section className="grid gap-2 border-t pt-4">
      <Button
        variant="ghost"
        size="sm"
        className="-ml-2 w-fit"
        aria-expanded={open}
        aria-controls="plugin-detail-technical"
        onClick={() => {
          setOpen(!open)
        }}
      >
        <ChevronDown className={cn("transition-transform", open && "rotate-180")} />
        {t("marketplace:details.technical")}
      </Button>
      {open && (
        <dl
          id="plugin-detail-technical"
          className="grid grid-cols-[max-content_1fr] gap-x-6 gap-y-1.5 text-sm"
          aria-label={t("marketplace:details.technical")}
        >
          {facts.map(([term, value], i) => (
            <div key={`${term}-${String(i)}`} className="contents">
              <dt className="text-muted-foreground">{term}</dt>
              <dd className="font-mono text-xs break-all select-text">{value}</dd>
            </div>
          ))}
        </dl>
      )}
    </section>
  )
}

/**
 * What an installed plugin uses and how its host fared since the agent
 * started: its storage against the quota, crashes and its last error.
 * The diagnostics in Settings have the technical record.
 */
function UsageSection({
  item,
  migrating,
  onDiagnostics,
}: {
  item: PluginDetails
  /** A built-in module moves to this plugin: its migration is in the diagnostics. */
  migrating: boolean
  onDiagnostics: () => void
}) {
  const { t } = useTranslation()
  const health = item.summary.health
  const troubled = (health?.crashes ?? 0) > 0 || Boolean(health?.lastError)
  return (
    <section aria-labelledby="plugin-detail-usage" className="grid gap-2">
      <SectionTitle id="plugin-detail-usage">{t("marketplace:details.usage")}</SectionTitle>
      <dl className="grid grid-cols-[max-content_1fr] gap-x-6 gap-y-1.5 text-sm">
        <dt className="text-muted-foreground">{t("marketplace:details.storage")}</dt>
        <dd>{item.storage ? storageText(item.storage) : t("marketplace:details.notReported")}</dd>
        <dt className="text-muted-foreground">{t("marketplace:details.health")}</dt>
        <dd>
          {troubled
            ? t("marketplace:details.healthCrashes", { count: health?.crashes ?? 0 })
            : t("marketplace:details.healthy")}
        </dd>
      </dl>
      <HealthNotice item={item} />
      {(troubled || migrating) && (
        <div>
          <Button variant="outline" size="sm" onClick={onDiagnostics}>
            <Stethoscope /> {t("marketplace:details.openDiagnostics")}
          </Button>
        </div>
      )}
    </section>
  )
}
