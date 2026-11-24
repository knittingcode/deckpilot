// What every Discover section needs to build a plugin's card: the
// catalog's own data and the Marketplace's actions, never editorial data.

import type { ReactNode } from "react"

import { cn } from "@/lib/utils"

import type { MarketplaceActions } from "../marketplace"
import type { UpdateCandidate } from "../model"
import type { MarketplaceSnapshot, ModuleMigration } from "../types"

export interface CatalogProps {
  snapshot: MarketplaceSnapshot
  updates: UpdateCandidate[]
  migrations: ModuleMigration[]
  actions: MarketplaceActions
}

/** A section's heading and its editorial description, as plain text. */
export function SectionHeading({
  id,
  title,
  description,
  children,
  className,
}: {
  id: string
  title: string
  description?: string | undefined
  /** At the right (e.g. scroll buttons). */
  children?: ReactNode
  className?: string
}) {
  return (
    <div className={cn("flex items-end justify-between gap-3", className)}>
      <div className="grid min-w-0 gap-0.5">
        <h3
          id={id}
          dir="auto"
          className="line-clamp-2 scroll-mt-4 text-base font-semibold [overflow-wrap:anywhere]"
        >
          {title}
        </h3>
        {description && (
          <p
            dir="auto"
            className="line-clamp-2 text-xs [overflow-wrap:anywhere] text-muted-foreground"
          >
            {description}
          </p>
        )}
      </div>
      {children}
    </div>
  )
}
