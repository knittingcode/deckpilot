import { useState } from "react"
import { toast } from "sonner"

import { i18n } from "@/i18n"
import { api } from "@/lib/api"
import { toastFailure } from "@/lib/failure"

/**
 * Makes a layout the profile's default for its device model
 * (`activate_layout`: the profile's `activeLayouts[model]`; there is no
 * "no default"). The one way to do it, for the layout selector's star and
 * the Layout Manager. `pending` is the layout being set, so a second click
 * waits; `onDone` runs after the backend saved it, and a failure leaves
 * everything as it was, with the app's error toast.
 */
export function useSetDefaultLayout(onDone: () => void) {
  const [pending, setPending] = useState<string | null>(null)
  const setDefault = async (
    profileId: string,
    layout: { id: string; name: string },
    modelName: string,
  ) => {
    if (pending) return
    setPending(layout.id)
    try {
      await api.activateLayout(profileId, layout.id)
      toast.success(
        i18n.t("layouts:menu.toast.madeDefault", { layout: layout.name, model: modelName }),
      )
      onDone()
    } catch (e) {
      toastFailure(i18n.t("layouts:menu.toast.defaultFailed", { layout: layout.name }), e)
    } finally {
      setPending(null)
    }
  }
  return { pending, setDefault }
}
