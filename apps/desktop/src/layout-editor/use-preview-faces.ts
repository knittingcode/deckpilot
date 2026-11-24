import { useEffect, useState } from "react"

import { api } from "@/lib/api"
import type { LabelBox } from "@/lib/key-labels"
import { controlKey } from "@/lib/layout-model"
import type {
  ControlFace,
  ControlNavigation,
  DeviceLayout,
  FolderCountdown,
  KeyFace,
  LivePreview,
  TextLayoutReport,
} from "@/lib/types"

/**
 * Faces and key images exactly as the agent would render them (with
 * appearance and icons), debounced while editing. With `live`, keys show
 * the live states the agent reported (`useLiveKeyStates`), drawn by the
 * same renderer as the device's keys.
 *
 * Labels follow the profile's active project, as on the device; the editor
 * has no project of its own (projects are an optional module).
 *
 * `keyPixels` is the width the editor shows a key at in device pixels (its
 * CSS width times the display's pixel ratio): the renderer draws the
 * images at that size, so they are not scaled up from the device's own
 * 72 pixels or so and stay sharp. `null`: the device's size.
 *
 * `reports` says how each key's title comes out on the device (measured at
 * the device's own size, docs/key-text.md): whether it fits, at which size,
 * whether it scrolls.
 *
 * `folderCountdown` is the agent's Auto Exit countdown of this page while
 * the device shows it in the live preview: the folder's Back key is drawn
 * with it, as on the device.
 */
export function usePreviewFaces(
  profileId: string,
  draft: DeviceLayout,
  pageId: string,
  /** Changes when the profile's icons change, so previews pick them up. */
  iconsVersion: string,
  live: LivePreview | null = null,
  keyPixels: number | null = null,
  /**
   * Changes when what the action types can do changes (a plugin was
   * installed, enabled, disabled, removed, or got a permission): keys whose
   * plugin was missing are drawn again as their action, and the other way
   * round, without waiting for an edit.
   */
  catalogVersion = "",
  folderCountdown: FolderCountdown | null = null,
) {
  const countdownSeconds = folderCountdown?.seconds ?? null
  const countdownPaused = folderCountdown?.paused === true
  const [preview, setPreview] = useState({
    faces: new Map<string, KeyFace>(),
    images: new Map<string, string>(),
    /** Bindings kept although they cannot run from their control. */
    incompatible: new Map<string, NonNullable<ControlFace["incompatible"]>>(),
    reports: new Map<string, TextLayoutReport>(),
    /** Where the renderer put each key's labels (lib/key-labels.ts). */
    labelBoxes: new Map<string, LabelBox[]>(),
    /**
     * What each control does to the editor's view, as the agent resolves
     * it (docs/editor-navigation-and-simulation.md): its only source for
     * the automatic Back key, which is not a binding.
     */
    navigation: new Map<string, ControlNavigation>(),
  })
  useEffect(() => {
    let cancelled = false
    const timer = setTimeout(() => {
      const countdown =
        countdownSeconds === null ? null : { seconds: countdownSeconds, paused: countdownPaused }
      api.previewLayout(profileId, draft, pageId, null, true, live, keyPixels, countdown).then(
        (result) => {
          if (cancelled) return
          setPreview({
            faces: new Map(result.map((r) => [controlKey(r.control), r.face])),
            images: new Map(
              result.flatMap((r) => (r.image ? [[controlKey(r.control), r.image] as const] : [])),
            ),
            incompatible: new Map(
              result.flatMap((r) =>
                r.incompatible?.length ? [[controlKey(r.control), r.incompatible] as const] : [],
              ),
            ),
            reports: new Map(
              result.flatMap((r) => (r.text ? [[controlKey(r.control), r.text] as const] : [])),
            ),
            labelBoxes: new Map(
              result.flatMap((r) =>
                r.labelBoxes?.length ? [[controlKey(r.control), r.labelBoxes] as const] : [],
              ),
            ),
            navigation: new Map(
              result.flatMap((r) =>
                r.navigation ? [[controlKey(r.control), r.navigation] as const] : [],
              ),
            ),
          })
        },
        () => undefined,
      )
    }, 150)
    return () => {
      cancelled = true
      clearTimeout(timer)
    }
  }, [
    profileId,
    draft,
    pageId,
    iconsVersion,
    live,
    keyPixels,
    catalogVersion,
    countdownSeconds,
    countdownPaused,
  ])
  return preview
}
