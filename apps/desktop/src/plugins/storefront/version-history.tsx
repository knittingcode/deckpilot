// Every version the registry lists for a plugin, newest first, with its
// signed publication date and state (latest, withdrawn, incompatible,
// installed) from the catalog, and its release notes: the listing's
// (markdown subset) where it has them, else the registry's plain text.
// The listing can add words to a version, never a version.

import { Badge } from "@/components/ui/badge"
import { useTranslation } from "@/i18n"

import { formatDate } from "../model"
import type { ReleaseNoteView } from "../storefront-types"
import type { AvailableVersion } from "../types"
import { SafeMarkdown } from "./safe-markdown"
import { noteFor } from "./resolve"

export function VersionHistory({
  versions,
  latest,
  installed,
  notes,
  skipNotesOf,
}: {
  /** Newest first. */
  versions: AvailableVersion[]
  latest: string | undefined
  installed: string | undefined
  /** The listing's release notes. */
  notes: readonly ReleaseNoteView[] | undefined
  /** Whose notes are already shown as "What's new". */
  skipNotesOf: string | undefined
}) {
  const { t } = useTranslation()
  const any = versions.some((v) => noteFor(notes, v.version) ?? v.releaseNotes)
  return (
    <section
      aria-labelledby="plugin-detail-versions"
      className="grid gap-2"
      data-slot="version-history"
    >
      <h4 id="plugin-detail-versions" className="text-base font-semibold">
        {t("marketplace:details.versions")}
      </h4>
      <ul className="grid divide-y border-y text-sm" aria-label={t("marketplace:details.versions")}>
        {versions.map((v) => {
          const listed = noteFor(notes, v.version)
          const showNotes = v.version !== skipNotesOf
          return (
            <li key={v.version} className="grid gap-1 py-2">
              <span className="flex flex-wrap items-center gap-2">
                <span className="font-medium tabular-nums">{v.version}</span>
                {v.publishedAt !== undefined && (
                  <span className="text-xs text-muted-foreground">
                    {t("marketplace:details.published", { date: formatDate(v.publishedAt) })}
                  </span>
                )}
                {v.version === latest && (
                  <Badge variant="outline">{t("marketplace:details.latest")}</Badge>
                )}
                {v.withdrawn && (
                  <Badge variant="destructive">{t("marketplace:badge.withdrawn")}</Badge>
                )}
                {(v.incompatible ?? []).length > 0 && (
                  <Badge variant="outline">{t("marketplace:badge.incompatible")}</Badge>
                )}
                {installed === v.version && (
                  <Badge variant="secondary">{t("marketplace:details.installedBadge")}</Badge>
                )}
              </span>
              {showNotes &&
                (listed ? (
                  <SafeMarkdown
                    source={listed.text}
                    className="text-xs leading-5 text-muted-foreground"
                  />
                ) : (
                  v.releaseNotes && (
                    <p className="text-xs whitespace-pre-line text-muted-foreground">
                      {v.releaseNotes}
                    </p>
                  )
                ))}
            </li>
          )
        })}
      </ul>
      {!any && (
        <p className="text-xs text-muted-foreground">{t("marketplace:details.noChangelog")}</p>
      )}
    </section>
  )
}
