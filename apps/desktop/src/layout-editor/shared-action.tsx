import type { ReactNode } from "react"
import { Ellipsis, ListTree, Pencil, TriangleAlert, Users } from "lucide-react"

import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { NeedsConfiguration } from "@/components/needs-configuration"
import { useTranslation } from "@/i18n"
import { missingParameters } from "@/lib/action-completeness"
import type { ActionInstance, ActionType } from "@/lib/types"

/**
 * The saved action of a binding: "Used in N places" when shared, required
 * values it lacks, and a small menu to edit its definition (for every
 * place) or see where it is used.
 */
export function SharedAction({
  action,
  type,
  users,
  onEdit,
  onUsages,
  setup,
}: {
  action: ActionInstance
  type: ActionType | undefined
  /** The places that use it: keys, gestures, steps, nodes. */
  users: number
  onEdit: () => void
  onUsages: () => void
  /** The draft's own required settings (`ActionSetup`), when it is one. */
  setup?: ReactNode
}) {
  const { t } = useTranslation()
  const shared = users > 1
  // The same rule as the agent's (`lib/action-completeness.ts`).
  const keys = type ? missingParameters(type, action.parameters) : []
  const missing = (type?.parameters ?? []).filter((spec) => keys.includes(spec.key))
  return (
    <div className="grid min-w-0 gap-1" data-slot="shared-action">
      <div className="flex min-w-0 items-center gap-2 text-xs text-muted-foreground">
        {shared && (
          <span className="flex min-w-0 items-center gap-1" data-slot="shared-count">
            <Users className="size-3.5 shrink-0" aria-hidden />
            {t("editor:savedAction.shared", { count: users })}
          </span>
        )}
        <span className="flex-1" />
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              variant="ghost"
              size="icon-xs"
              aria-label={t("editor:savedAction.menu", { name: action.name })}
            >
              <Ellipsis />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem onSelect={onEdit}>
              <Pencil />
              {shared ? t("editor:savedAction.editShared") : t("editor:savedAction.edit")}
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={onUsages}>
              <ListTree /> {t("editor:savedAction.usages")}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
      {setup ??
        (missing.length > 0 && (
          // Shared: filled in through its definition, for every place.
          <div className="grid gap-1">
            <NeedsConfiguration name={action.name} className="w-fit" />
            <p className="flex items-center gap-1.5 text-xs text-destructive">
              <TriangleAlert className="size-3.5 shrink-0" aria-hidden />
              {t("actions:completeness.missing", {
                parameters: missing.map((s) => s.label).join(", "),
              })}
            </p>
          </div>
        ))}
    </div>
  )
}
