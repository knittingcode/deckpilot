import { useSyncExternalStore } from "react"

/**
 * The profile the Layout Manager is filtered to, e.g. opened from a
 * profile's details in Settings › Profiles; `null` for every layout. It
 * stays until it is cleared (the chip on the page), so returning to the
 * page shows the same view.
 */
let profileId: string | null = null
const listeners = new Set<() => void>()

export function setLayoutProfileFilter(id: string | null) {
  profileId = id
  for (const listener of listeners) listener()
}

export function useLayoutProfileFilter(): string | null {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener)
      return () => {
        listeners.delete(listener)
      }
    },
    () => profileId,
  )
}
