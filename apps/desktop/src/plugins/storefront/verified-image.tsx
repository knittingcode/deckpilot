// A storefront image, by the sha256 of its verified bytes (storefront
// design §6 and §7). The agent checked the bytes against the signed
// storefront, the backend re-checked them and built a `data:` URL; this
// checks once more that it is a base64 PNG, WebP or JPEG `data:` URL of
// the type the reference names, and renders nothing else. Never a remote
// image, never SVG.
//
// The box has the image's aspect ratio from the reference before the image
// arrives (no layout shift): a quiet placeholder while loading, the
// `fallback` (a monogram) when the image cannot be shown.

import { type ReactNode, useEffect, useState } from "react"

import { cn } from "@/lib/utils"

import type { MediaRef } from "../storefront-types"
import { acceptableMediaUrl, mediaCache, loadMedia } from "./media-cache"
import { validMediaRef } from "./resolve"
import { useStorefront } from "./use-storefront"

/** The box's aspect ratio, kept sane whatever dimensions a reference claims. */
function aspectOf(ref: MediaRef | undefined, fallback: number): number {
  if (!ref) return fallback
  const ratio = ref.width / ref.height
  return Number.isFinite(ratio) ? Math.min(6, Math.max(1 / 4, ratio)) : fallback
}

export function VerifiedImage({
  media,
  alt,
  decorative = false,
  fallback,
  className,
  imageClassName,
  aspect,
  fit = "cover",
}: {
  media: MediaRef | undefined
  /** The text alternative; ignored when `decorative`. */
  alt: string
  /** The image carries no information: `alt=""`. */
  decorative?: boolean
  /** Shown in the box when the image cannot be shown (a monogram). */
  fallback?: ReactNode
  className?: string
  imageClassName?: string
  /** The box's aspect ratio; by default the reference's. */
  aspect?: number
  fit?: "cover" | "contain"
}) {
  const { client, view } = useStorefront()
  const seq = view?.seq
  const ref = validMediaRef(media) ? media : undefined
  const [loaded, setLoaded] = useState<{ sha256: string; url: string | null } | null>(() => {
    const cached = ref ? mediaCache.get(ref.sha256) : undefined
    return ref && cached !== undefined ? { sha256: ref.sha256, url: cached } : null
  })
  const sha256 = ref?.sha256
  const mediaType = ref?.mediaType

  useEffect(() => {
    if (!sha256 || !mediaType) return
    let live = true
    // Width and height do not change what is fetched.
    void loadMedia(client, { sha256, mediaType, width: 1, height: 1 }, seq).then((url) => {
      if (live) setLoaded({ sha256, url })
    })
    return () => {
      live = false
    }
  }, [client, sha256, mediaType, seq])

  const url = ref && loaded?.sha256 === ref.sha256 ? loaded.url : undefined
  const failed = !ref || url === null
  const shown = Boolean(ref && url && acceptableMediaUrl(url, ref))
  // Until (or unless) the image shows, the box itself carries the text alternative.
  const named = !shown && !decorative && alt !== ""
  return (
    <div
      data-slot="verified-image"
      data-state={failed ? "fallback" : url ? "loaded" : "loading"}
      className={cn("relative overflow-hidden bg-muted", className)}
      style={{ aspectRatio: String(aspect ?? aspectOf(ref, 16 / 10)) }}
      {...(named ? { role: "img", "aria-label": alt } : {})}
    >
      {ref && url && shown ? (
        <img
          src={url}
          alt={decorative ? "" : alt}
          draggable={false}
          decoding="async"
          className={cn(
            "absolute inset-0 size-full",
            fit === "cover" ? "object-cover" : "object-contain",
            imageClassName,
          )}
        />
      ) : failed ? (
        fallback && <div className="absolute inset-0">{fallback}</div>
      ) : null}
    </div>
  )
}
