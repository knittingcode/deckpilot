import { useSyncExternalStore } from "react"

const QUERY = "(prefers-reduced-motion: reduce)"

function subscribe(onChange: () => void): () => void {
  if (typeof window.matchMedia !== "function") return () => undefined
  const media = window.matchMedia(QUERY)
  // Older WebKit has only addListener.
  if (typeof media.addEventListener === "function") {
    media.addEventListener("change", onChange)
    return () => {
      media.removeEventListener("change", onChange)
    }
  }
  return () => undefined
}

function reduced(): boolean {
  // A stubbed `matchMedia` (tests) may answer without `matches`.
  const media = typeof window.matchMedia === "function" ? window.matchMedia(QUERY) : undefined
  return Boolean(media?.matches)
}

/** Whether the system asks for reduced motion (no slide transitions, no smooth scrolling). */
export function useReducedMotion(): boolean {
  return useSyncExternalStore(subscribe, reduced, () => false)
}
