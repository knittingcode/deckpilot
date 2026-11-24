// How the Marketplace shows its editorial collections: icon, sidebar label,
// title and description. Which plugins belong to each is `COLLECTIONS`
// (catalog.ts), the one place membership is defined.

import { Activity, Briefcase, Code, Gamepad2, type LucideIcon, Workflow } from "lucide-react"

import type { TranslationKey } from "@/i18n"

import type { CollectionId } from "./catalog"

/** The collections' labels and icons; their membership is `COLLECTIONS` (catalog.ts). */
export const COLLECTION_LOOK = {
  code: {
    icon: Code,
    label: "marketplace:nav.code",
    title: "marketplace:collection.code.title",
    text: "marketplace:collection.code.text",
  },
  play: {
    icon: Gamepad2,
    label: "marketplace:nav.play",
    title: "marketplace:collection.play.title",
    text: "marketplace:collection.play.text",
  },
  work: {
    icon: Briefcase,
    label: "marketplace:nav.work",
    title: "marketplace:collection.work.title",
    text: "marketplace:collection.work.text",
  },
  automate: {
    icon: Workflow,
    label: "marketplace:nav.automate",
    title: "marketplace:collection.automate.title",
    text: "marketplace:collection.automate.text",
  },
  monitor: {
    icon: Activity,
    label: "marketplace:nav.monitor",
    title: "marketplace:collection.monitor.title",
    text: "marketplace:collection.monitor.text",
  },
} as const satisfies Record<
  CollectionId,
  { icon: LucideIcon; label: TranslationKey; title: TranslationKey; text: TranslationKey }
>
