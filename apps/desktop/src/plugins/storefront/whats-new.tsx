// "What's new": the release notes of the version a plugin's page shows,
// from the listing (markdown subset) or, failing that, the registry's
// plain-text notes. The version and its date are the signed catalog's.

import { useTranslation } from "@/i18n"

import { formatDate } from "../model"
import type { ReleaseNoteView } from "../storefront-types"
import type { AvailableVersion } from "../types"
import { SafeMarkdown } from "./safe-markdown"

export function WhatsNew({ version, note }: { version: AvailableVersion; note: ReleaseNoteView }) {
  const { t } = useTranslation()
  return (
    <section aria-labelledby="plugin-detail-news" className="grid gap-2" data-slot="whats-new">
      <h4 id="plugin-detail-news" className="text-base font-semibold">
        {t("marketplace:details.whatsNew")}
      </h4>
      <p className="text-xs text-muted-foreground">
        {t("marketplace:details.versionLine", { version: version.version })}
        {version.publishedAt !== undefined && ` · ${formatDate(version.publishedAt)}`}
      </p>
      <SafeMarkdown source={note.text} />
    </section>
  )
}
