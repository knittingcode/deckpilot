// Action type categories (`ActionType.category`): a stable identifier from
// the module that declares the type ("Navigation", "System", a plugin's
// "GitLab"), shown in the window's language where the app knows it
// (`actions:categories.*`), else as the module gives it (plugin categories,
// usually product names).

import { i18n, type TranslationKey } from "@/i18n"

/** The category's identifier: its name, trimmed and in lower case. */
export function actionCategoryId(category: string): string {
  return category.trim().toLowerCase()
}

/** The app's own categories (declared by its core modules), by identifier. */
const CATEGORY_KEYS = {
  navigation: "actions:categories.navigation",
  device: "actions:categories.device",
  general: "actions:categories.general",
  system: "actions:categories.system",
  deck: "actions:categories.deck",
  http: "actions:categories.http",
  notifications: "actions:categories.notifications",
} as const satisfies Record<string, TranslationKey>

/** The category's name in the window's language. */
export function actionCategoryLabel(category: string): string {
  const key = CATEGORY_KEYS[actionCategoryId(category) as keyof typeof CATEGORY_KEYS] as
    TranslationKey | undefined
  return key ? i18n.t(key) : category
}
