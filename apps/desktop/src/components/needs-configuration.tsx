import { CircleDashed } from "lucide-react"

import { Badge } from "@/components/ui/badge"
import { useTranslation } from "@/i18n"
import { cn } from "@/lib/utils"

/**
 * "Needs configuration" (`actions:completeness.badge`): on a key, a
 * gesture, a Multi Action step, a workflow node or an action whose
 * required settings are not filled in yet. It is kept and saved, and does
 * not run until they are (`lib/action-completeness.ts`). `compact`: the
 * icon alone, named for assistive technology (e.g. on a key's face).
 */
export function NeedsConfiguration({
  name,
  compact = false,
  className,
}: {
  /** What needs it, for the accessible name ("Open application needs …"). */
  name?: string
  compact?: boolean
  className?: string
}) {
  const { t } = useTranslation()
  const label = name
    ? t("actions:completeness.badgeLabel", { name })
    : t("actions:completeness.badge")
  if (compact) {
    return (
      <span
        role="img"
        aria-label={label}
        title={t("actions:completeness.badge")}
        data-slot="needs-configuration"
        className={cn(
          "flex size-4 items-center justify-center rounded-full bg-amber-500 text-white shadow-sm",
          className,
        )}
      >
        <CircleDashed aria-hidden className="size-3" />
      </span>
    )
  }
  return (
    <Badge
      variant="outline"
      data-slot="needs-configuration"
      aria-label={label}
      className={cn(
        "h-5 gap-1 border-amber-500/60 px-1.5 text-[11px] text-amber-700 dark:text-amber-400",
        className,
      )}
    >
      <CircleDashed aria-hidden className="size-3" />
      {t("actions:completeness.badge")}
    </Badge>
  )
}
