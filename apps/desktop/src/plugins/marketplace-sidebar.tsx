// The Marketplace's own navigation, left of its content: Discover,
// Installed, Updates and Categories, then the collections: a signed
// storefront's (`StorefrontView.collections`, plugins the catalog cannot
// place left out, empty ones hidden), else the built-in ones. Built
// from the design system's sidebar primitives (like Settings' sidebar), it
// is part of the Marketplace's layout and never repeats the main window's
// rail or the settings. Narrow, it shows its icons only, each named by a
// tooltip and its accessible name.

import {
  Compass,
  Download,
  FileUp,
  LayoutGrid,
  type LucideIcon,
  PackageCheck,
  Search,
  Sparkles,
  X,
} from "lucide-react"
import { useRef } from "react"

import { Button } from "@/components/ui/button"
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover"
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarHeader,
  SidebarInput,
  SidebarGroup,
  SidebarGroupContent,
  SidebarMenu,
  SidebarMenuBadge,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/components/ui/sidebar"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { type TranslationKey, useTranslation } from "@/i18n"
import { cn } from "@/lib/utils"

import type { CollectionId, MarketplaceSection } from "./catalog"
import { COLLECTION_LOOK } from "./collection-look"
import { editorialSection } from "./storefront/resolve"
import { clip, LIMITS } from "./storefront/text"
import { useStorefront } from "./storefront/use-storefront"

const PRIMARY = [
  { id: "discover", icon: Compass, label: "marketplace:nav.discover" },
  { id: "installed", icon: PackageCheck, label: "marketplace:nav.installed" },
  { id: "updates", icon: Download, label: "marketplace:nav.updates" },
  { id: "categories", icon: LayoutGrid, label: "marketplace:nav.categories" },
] as const satisfies readonly { id: MarketplaceSection; icon: LucideIcon; label: TranslationKey }[]

/**
 * The Marketplace's search: the catalog as you type, in every section.
 * Escape clears it (when there is text), as does the × button.
 */
function SearchField({
  query,
  onQuery,
  autoFocus = false,
}: {
  query: string
  onQuery: (query: string) => void
  autoFocus?: boolean
}) {
  const { t } = useTranslation()
  const input = useRef<HTMLInputElement>(null)
  return (
    <div className="relative">
      <Search
        aria-hidden
        className="pointer-events-none absolute top-1/2 left-2 size-3.5 -translate-y-1/2 text-muted-foreground"
      />
      <SidebarInput
        ref={input}
        type="search"
        // A plain field: the clear button is ours, not the browser's.
        className="h-8 pr-7 pl-7 text-caption [&::-webkit-search-cancel-button]:hidden"
        placeholder={t("marketplace:discover.search")}
        aria-label={t("marketplace:discover.search")}
        autoFocus={autoFocus}
        value={query}
        onChange={(e) => {
          onQuery(e.target.value)
        }}
        onKeyDown={(e) => {
          if (e.key === "Escape" && query !== "") {
            e.preventDefault()
            e.stopPropagation()
            onQuery("")
          }
        }}
      />
      {query !== "" && (
        <Button
          variant="ghost"
          size="icon-xs"
          className="absolute top-1/2 right-1 -translate-y-1/2 text-muted-foreground"
          aria-label={t("marketplace:search.clear")}
          onClick={() => {
            onQuery("")
            input.current?.focus()
          }}
        >
          <X />
        </Button>
      )}
    </div>
  )
}

/** "Install from file": the nav items' look, without their selected state. */
function InstallFileButton({
  compact,
  disabled,
  onClick,
}: {
  compact: boolean
  disabled: boolean
  onClick: () => void
}) {
  const { t } = useTranslation()
  const label = t("marketplace:sidebar.installFile")
  const button = (
    <SidebarMenuButton
      data-slot="marketplace-install-file"
      aria-label={compact ? label : undefined}
      title={compact ? undefined : t("marketplace:discover.installFileHint")}
      disabled={disabled}
      className={cn(
        "h-8 gap-2 px-2 text-caption hover:bg-sidebar-accent/50 focus-visible:ring-2 focus-visible:ring-sidebar-ring [&>svg]:size-4 [&>svg]:text-muted-foreground",
        compact && "justify-center px-0",
      )}
      onClick={onClick}
    >
      <FileUp aria-hidden />
      {!compact && <span>{label}</span>}
    </SidebarMenuButton>
  )
  if (!compact) return button
  return (
    <Tooltip>
      <TooltipTrigger asChild>{button}</TooltipTrigger>
      <TooltipContent side="right">{label}</TooltipContent>
    </Tooltip>
  )
}

function Entry({
  id,
  icon: Icon,
  label,
  count,
  current,
  compact,
  onSelect,
}: {
  id: MarketplaceSection
  icon: LucideIcon
  label: string
  count?: number
  current: MarketplaceSection | null
  compact: boolean
  onSelect: (section: MarketplaceSection) => void
}) {
  const active = current === id
  const button = (
    <SidebarMenuButton
      isActive={active}
      aria-current={active ? "page" : undefined}
      aria-label={compact && count ? `${label} (${String(count)})` : compact ? label : undefined}
      className={cn(
        // One height for every item; only the selected one gets the accent
        // background, hover a lighter one, keyboard focus a ring.
        "h-8 gap-2 px-2 text-caption hover:bg-sidebar-accent/50 focus-visible:ring-2 focus-visible:ring-sidebar-ring data-[active=true]:bg-sidebar-accent data-[active=true]:font-medium [&>svg]:size-4 [&>svg]:text-muted-foreground data-[active=true]:[&>svg]:text-sidebar-accent-foreground",
        compact && "justify-center px-0",
      )}
      onClick={() => {
        onSelect(id)
      }}
    >
      <Icon aria-hidden />
      {!compact && <span>{label}</span>}
    </SidebarMenuButton>
  )
  return (
    <SidebarMenuItem>
      {compact ? (
        <Tooltip>
          <TooltipTrigger asChild>{button}</TooltipTrigger>
          <TooltipContent side="right">
            {count ? `${label} (${String(count)})` : label}
          </TooltipContent>
        </Tooltip>
      ) : (
        button
      )}
      {count !== undefined && count > 0 && !compact && (
        <SidebarMenuBadge className="top-1.5 right-2 min-w-5 justify-end px-0 text-xs text-muted-foreground tabular-nums">
          {count}
        </SidebarMenuBadge>
      )}
      {count !== undefined && count > 0 && compact && (
        <span
          aria-hidden
          className="pointer-events-none absolute top-1.5 right-1.5 size-1.5 rounded-full bg-foreground"
        />
      )}
    </SidebarMenuItem>
  )
}

export function MarketplaceSidebar({
  current,
  installed,
  updates,
  compact,
  onSelect,
  query,
  onQuery,
  onInstallFile,
  installFileDisabled,
}: {
  /** The section shown; `null` while search results are (no item looks selected). */
  current: MarketplaceSection | null
  installed: number
  updates: number
  compact: boolean
  onSelect: (section: MarketplaceSection) => void
  query: string
  onQuery: (query: string) => void
  /** Installs a package file (the existing picker, confirmation, verification and consent). */
  onInstallFile: () => void
  installFileDisabled: boolean
}) {
  const { t } = useTranslation()
  const { collections: editorial } = useStorefront()
  const counts: Partial<Record<MarketplaceSection, number>> = { installed, updates }
  return (
    <Sidebar
      collapsible="none"
      aria-label={t("marketplace:nav.label")}
      data-slot="marketplace-sidebar"
      className={cn(
        // Its right border is the divider, the full height of the row.
        "min-h-0 shrink-0 border-r border-border bg-transparent",
        compact ? "w-12" : "w-52",
      )}
    >
      {/* At the top, in the items' padding: the search (narrow: a button opening it). */}
      <SidebarHeader className="p-2 pb-0">
        {compact ? (
          <Popover>
            <Tooltip>
              <TooltipTrigger asChild>
                <PopoverTrigger asChild>
                  <Button
                    variant={query !== "" ? "secondary" : "ghost"}
                    size="icon-sm"
                    className="w-full"
                    aria-label={t("marketplace:discover.search")}
                  >
                    <Search />
                  </Button>
                </PopoverTrigger>
              </TooltipTrigger>
              <TooltipContent side="right">{t("marketplace:discover.search")}</TooltipContent>
            </Tooltip>
            <PopoverContent side="right" align="start" className="w-64 p-2">
              <SearchField query={query} onQuery={onQuery} autoFocus />
            </PopoverContent>
          </Popover>
        ) : (
          <SearchField query={query} onQuery={onQuery} />
        )}
      </SidebarHeader>
      {/* One list of all nine destinations: padding 8 px all round, 2 px between items. */}
      <SidebarContent className="p-2">
        <SidebarGroup className="p-0">
          <SidebarGroupContent>
            <SidebarMenu className="gap-0.5">
              {PRIMARY.map((entry) => (
                <Entry
                  key={entry.id}
                  id={entry.id}
                  icon={entry.icon}
                  label={t(entry.label)}
                  {...(counts[entry.id] !== undefined ? { count: counts[entry.id] } : {})}
                  current={current}
                  compact={compact}
                  onSelect={onSelect}
                />
              ))}
              {editorial.length > 0
                ? editorial.map((collection) => (
                    <Entry
                      key={collection.id}
                      id={editorialSection(collection.id)}
                      icon={Sparkles}
                      label={clip(collection.title, LIMITS.cta)}
                      current={current}
                      compact={compact}
                      onSelect={onSelect}
                    />
                  ))
                : (Object.keys(COLLECTION_LOOK) as CollectionId[]).map((id) => (
                    <Entry
                      key={id}
                      id={id}
                      icon={COLLECTION_LOOK[id].icon}
                      label={t(COLLECTION_LOOK[id].label)}
                      current={current}
                      compact={compact}
                      onSelect={onSelect}
                    />
                  ))}
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>
      </SidebarContent>
      {/* An action, never a destination: no selected state. Anchored at the bottom. */}
      <SidebarFooter className="border-t border-border p-2">
        <SidebarMenu>
          <SidebarMenuItem>
            <InstallFileButton
              compact={compact}
              disabled={installFileDisabled}
              onClick={onInstallFile}
            />
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarFooter>
    </Sidebar>
  )
}
