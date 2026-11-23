// Variables in a key's title (docs/variables.md, "Variables in key titles";
// lib/display-templates.ts): the picker of display-safe references, which
// inserts `{{ reference }}`, and what the key shows of the title as the
// agent resolves it, with a non-destructive note for each reference that
// shows `-`. The editor never resolves a title itself.

import { Braces } from "lucide-react"
import { useEffect, useId, useState } from "react"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover"
import { useTranslation } from "@/i18n"
import {
  type ResolvedDisplayTemplate,
  type UnresolvedReference,
  listDisplayVariables,
  resolveDisplayTemplates,
  shownOnKey,
  titleSegments,
} from "@/lib/display-templates"
import type { CatalogEntry } from "@/lib/types"
import { insertedReference } from "@/lib/variable-catalog"

/** How long typing rests before the title is resolved again. */
const RESOLVE_DELAY_MS = 250

/** The button beside the title that inserts a display-safe reference. */
export function TitleVariablePicker({
  profileId,
  onPick,
}: {
  profileId: string
  /** The reference to insert, without braces. */
  onPick: (reference: string) => void
}) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const [entries, setEntries] = useState<CatalogEntry[] | null>(null)
  const [failed, setFailed] = useState(false)
  const [query, setQuery] = useState("")
  const searchId = useId()

  useEffect(() => {
    if (!open) return
    let cancelled = false
    listDisplayVariables({ profile: profileId }).then(
      (found) => {
        if (!cancelled) {
          setEntries(found)
          setFailed(false)
        }
      },
      () => {
        if (!cancelled) setFailed(true)
      },
    )
    return () => {
      cancelled = true
    }
  }, [open, profileId])

  const needle = query.trim().toLowerCase()
  const shown = (entries ?? []).filter(
    (e) =>
      needle === "" ||
      e.label.toLowerCase().includes(needle) ||
      e.reference.toLowerCase().includes(needle),
  )
  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        setOpen(next)
        if (!next) setQuery("")
      }}
    >
      <PopoverTrigger asChild>
        <Button
          type="button"
          variant="outline"
          size="icon-sm"
          data-slot="title-variables"
          aria-label={t("editor:titleVariables.insert")}
          title={t("editor:titleVariables.insert")}
        >
          <Braces />
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" className="grid w-80 max-w-[90vw] gap-2 p-2">
        <Input
          id={searchId}
          autoFocus
          className="h-8"
          aria-label={t("editor:titleVariables.search")}
          placeholder={t("editor:titleVariables.search")}
          value={query}
          onChange={(event) => {
            setQuery(event.target.value)
          }}
        />
        {failed ? (
          <p className="text-xs text-muted-foreground" role="status">
            {t("editor:titleVariables.unavailable")}
          </p>
        ) : entries === null ? (
          <p className="text-xs text-muted-foreground" role="status">
            {t("editor:titleVariables.loading")}
          </p>
        ) : shown.length === 0 ? (
          <p className="text-xs text-muted-foreground" role="status">
            {t("editor:titleVariables.none")}
          </p>
        ) : (
          <ul
            className="relative grid max-h-64 gap-0.5 overflow-y-auto"
            aria-label={t("editor:titleVariables.list")}
          >
            {shown.map((entry) => (
              <li key={entry.reference}>
                <button
                  type="button"
                  className="grid w-full min-w-0 gap-0.5 rounded-sm px-2 py-1.5 text-left hover:bg-accent focus-visible:bg-accent focus-visible:outline-none"
                  onClick={() => {
                    setOpen(false)
                    setQuery("")
                    onPick(insertedReference(entry))
                  }}
                >
                  <span className="truncate text-sm">{entry.label}</span>
                  <code className="truncate text-xs text-muted-foreground">
                    {insertedReference(entry)}
                  </code>
                </button>
              </li>
            ))}
          </ul>
        )}
      </PopoverContent>
    </Popover>
  )
}

/** The agent's answer for `template`, or `null` while there is none. */
function useResolvedTitle(profileId: string, template: string): ResolvedDisplayTemplate | null {
  const [resolved, setResolved] = useState<ResolvedDisplayTemplate | null>(null)
  const uses = template.includes("{")
  useEffect(() => {
    if (!uses) return
    let cancelled = false
    const timer = setTimeout(() => {
      resolveDisplayTemplates({ profile: profileId }, [template]).then(
        ([answer]) => {
          if (!cancelled) setResolved(answer ?? null)
        },
        () => {
          // Without an agent the title is shown as typed.
          if (!cancelled) setResolved(null)
        },
      )
    }, RESOLVE_DELAY_MS)
    return () => {
      cancelled = true
      clearTimeout(timer)
    }
  }, [profileId, template, uses])
  // An answer for another text (typed since) is not shown.
  return uses && resolved?.template === template ? resolved : null
}

/**
 * What the key shows of a title with placeholders or references, cut as on
 * the key, each `-` that stands for a reference marked, and why it has no
 * value. Nothing for a plain title, or while no agent answers.
 */
export function TitlePreview({ profileId, template }: { profileId: string; template: string }) {
  const { t } = useTranslation()
  const resolved = useResolvedTitle(profileId, template)
  if (!resolved) return null
  const shown = shownOnKey(resolved)
  return (
    <div className="grid gap-1 text-xs" data-slot="title-preview">
      <p className="min-w-0 text-muted-foreground" role="status">
        {t("editor:titleVariables.shows")}{" "}
        <span className="font-medium text-foreground" data-slot="title-preview-text">
          {titleSegments(shown).map((segment, index) =>
            segment.marker ? (
              <mark
                key={index}
                className="rounded-sm bg-amber-500/20 px-0.5 text-foreground"
                title={segment.marker.reference}
                data-reference={segment.marker.reference}
              >
                {segment.text}
              </mark>
            ) : (
              <span key={index}>{segment.text}</span>
            ),
          )}
        </span>
      </p>
      {shown.cut && <p className="text-muted-foreground">{t("editor:titleVariables.cut")}</p>}
      {(resolved.unresolved ?? []).map((entry) => (
        <p key={entry.reference} className="text-amber-600 dark:text-amber-400" role="note">
          {t("editor:titleVariables.unresolved", {
            reference: entry.reference,
            reason: t(unresolvedReason(entry)),
          })}
        </p>
      ))}
      {(resolved.invalid ?? []).map((text) => (
        <p key={text} className="text-amber-600 dark:text-amber-400" role="note">
          {t("editor:titleVariables.invalid", { text })}
        </p>
      ))}
    </div>
  )
}

function unresolvedReason(entry: UnresolvedReference) {
  switch (entry.reason) {
    case "notDisplayable":
      return "editor:titleVariables.reason.notDisplayable" as const
    case "missing":
      return "editor:titleVariables.reason.missing" as const
    case "unavailable":
      return "editor:titleVariables.reason.unavailable" as const
    case "notText":
      return "editor:titleVariables.reason.notText" as const
    case "runtimeOnly":
      return "editor:titleVariables.reason.runtimeOnly" as const
  }
}
