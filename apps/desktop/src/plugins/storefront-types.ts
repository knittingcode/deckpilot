// The storefront contract between the agent and the Marketplace UI
// (storefront design §6, frozen for WP6). Mirrors the IPC shapes of
// `crates/deckpilot-ipc/src/plugins.rs` (feature `storefront`): every
// field is camelCase as the agent sends it.
//
// Trust: everything here comes from the signed storefront, verified by the
// agent (signature by the registry's index role, bound to the verified
// index, every document and image pinned by sha256). The agent has already
// localized the text (exact locale, then language, then `en`), dropped
// sections outside their window or platforms, dropped placements of
// plugins that are unknown, revoked, without an installable version or
// incompatible here, and filtered screenshots to this platform.
//
// What the storefront can NOT do: change a plugin's name, publisher,
// verification, permissions, versions or badges. Cards are always built
// from `AvailablePlugin` (`listAvailablePlugins`); editorial supplies only
// ids and copy. `official` and `bundledOnly` are derived by the agent,
// never read from a document.
//
// Text is plain text. Only `ListingView.descriptionMd` and
// `ReleaseNoteView.text` are the safe markdown subset, rendered by our own
// tokenizer to React elements, never as HTML.

/** The agent runtime feature (`PluginRuntimeStatus.features`). */
export const STOREFRONT_FEATURE = "storefront"

/** The only media types a storefront image can have. No SVG. */
export type StorefrontMediaType = "image/png" | "image/webp" | "image/jpeg"

export const STOREFRONT_MEDIA_TYPES: readonly StorefrontMediaType[] = [
  "image/png",
  "image/webp",
  "image/jpeg",
]

/**
 * A verified image, by the sha256 of its bytes. Fetch it with
 * `StorefrontClient.getRegistryMedia(sha256)`; `width` and `height` are the
 * image's real dimensions (checked by the agent), so a placeholder of the
 * right aspect can be shown before it arrives.
 */
export interface MediaRef {
  /** 64 lowercase hex digits. */
  sha256: string
  mediaType: StorefrontMediaType
  width: number
  height: number
}

/** Where a banner leads; never an external link. */
export interface TargetView {
  type: "plugin" | "collection" | "category"
  id: string
}

export interface BannerView {
  id: string
  title: string
  body?: string
  /** The call to action's label. */
  cta?: string
  artwork: MediaRef
  /** The artwork's text alternative; empty when `decorative`. */
  alt: string
  /** The artwork carries no information (render with `alt=""`). */
  decorative?: boolean
  target: TargetView
}

export interface CategoryView {
  /** An index category id (`monitoring`, `developer-tools`, …). */
  id: string
  blurb?: string
}

/**
 * Section kinds this contract defines. A later agent may send others: the
 * UI skips a section whose kind it does not know.
 */
export type SectionKind =
  | "bannerCarousel"
  | "collectionRow"
  | "spotlight"
  | "recentlyAdded"
  | "categoryHighlights"
  | "catalog"

export const SECTION_KINDS: readonly SectionKind[] = [
  "bannerCarousel",
  "collectionRow",
  "spotlight",
  "recentlyAdded",
  "categoryHighlights",
  "catalog",
]

/**
 * One section of Discover, in display order (the agent sorted by `order`,
 * then `id`). Which fields a kind uses:
 *
 * - `bannerCarousel`: `items` (1 to 6).
 * - `collectionRow`: `title`, `description?`, `layout`, `plugins` (1 to 24).
 * - `spotlight`: `plugins` (exactly one), `title` (the headline),
 *   `description?` (the body), `version?` (whose release notes to show;
 *   present only if that version is published and not withdrawn).
 * - `recentlyAdded`: `title?`, `limit` (4 to 24); the UI picks the plugins
 *   from `AvailablePlugin.versions[].publishedAt` (signed data).
 * - `categoryHighlights`: `title?`, `categories` (1 to 12).
 * - `catalog`: `title?`; the full catalog.
 */
export interface SectionView {
  id: string
  kind: SectionKind | (string & {})
  layout?: "cards" | "compact"
  title?: string
  description?: string
  items: BannerView[]
  /** Plugin ids, each present in `listAvailablePlugins`. */
  plugins: string[]
  categories: CategoryView[]
  limit?: number
  version?: string
}

/** An editorial collection (the sidebar), plugins in display order. */
export interface CollectionView {
  id: string
  title: string
  description?: string
  plugins: string[]
}

/** `getStorefront`: Discover's editorial content. */
export interface StorefrontView {
  /** The registry index seq this storefront belongs to. */
  seq: number
  sections: SectionView[]
  collections: CollectionView[]
}

export interface HighlightView {
  title: string
  text: string
}

export interface ScreenshotView {
  media: MediaRef
  alt: string
  caption?: string
}

export interface ArtView {
  media: MediaRef
  alt: string
}

/**
 * https links (checked by CI and by the agent: no userinfo, IP address,
 * port or punycode host). Open them only through the app's external-open
 * path, never in the webview; show the full URL for a non-DeckPilot plugin.
 */
export interface ListingLinks {
  documentation?: string
  support?: string
  changelog?: string
}

export interface ReleaseNoteView {
  version: string
  /** Safe markdown subset. */
  text: string
}

/** `getRegistryListing`: a plugin's listing content (author-written). */
export interface ListingView {
  /** One line, at most 80 characters. */
  promo?: string
  /** Safe markdown subset, at most 6000 characters. */
  descriptionMd?: string
  highlights: HighlightView[]
  icon?: MediaRef
  /** Already filtered to this platform. */
  screenshots: ScreenshotView[]
  promoArt?: ArtView
  links: ListingLinks
  /** Newest version first; only versions the registry publishes. */
  releaseNotes: ReleaseNoteView[]
}

/**
 * Optional fields an agent with the `storefront` feature adds to each
 * `AvailablePlugin` of `listAvailablePlugins`. Older agents send none of
 * them; the UI then shows the monogram and no badge.
 */
export interface AvailablePluginStorefrontFields {
  /** The listing's icon (a verified image). */
  icon?: MediaRef
  /** One of DeckPilot's official plugins (derived by the agent). */
  official?: boolean
  /** Only ever installed as the copy bundled with DeckPilot (derived by the agent). */
  bundledOnly?: boolean
}
