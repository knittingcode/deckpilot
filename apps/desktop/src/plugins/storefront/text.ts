// Text and links from the storefront, made inert before they are shown
// (storefront design §4). The agent and CI refuse most of this already;
// the UI does not rely on it.
//
// - Text is always rendered as React text nodes (never HTML), so markup
//   stays literal. On top of that, bidi overrides and isolates, zero-width
//   characters and control characters (except `\n`) are removed, so a
//   string cannot reorder or hide the text around it.
// - Every string is clipped to a length the layout can take.
// - A link is shown only when it is an https URL by the design's rules;
//   anything else is not a link.

/**
 * Characters that are invisible, reorder text or change how the text
 * around them looks:
 * - bidi marks, embeddings, overrides and isolates (U+061C, U+200E/F,
 *   U+202A-202E, U+2066-2069);
 * - zero-width and other format characters (U+200B-200D, U+2060-2064,
 *   U+206A-206F deprecated format characters, U+17B4/17B5 Khmer inherent
 *   vowels, U+1BCA0-1BCA3 shorthand format controls, U+1D173-1D17A
 *   musical formatting, U+FEFF, U+180E, U+034F combining grapheme joiner,
 *   U+00AD soft hyphen, U+FFF9-FFFB interlinear annotation);
 * - line and paragraph separators (U+2028/2029);
 * - Hangul fillers (U+115F, U+1160, U+3164, U+FFA0);
 * - variation selectors (U+FE00-FE0F, U+E0100-E01EF) and tag characters
 *   (U+E0000-E007F).
 */
const INVISIBLE: readonly (readonly [number, number])[] = [
  [0x00ad, 0x00ad],
  [0x034f, 0x034f],
  [0x061c, 0x061c],
  [0x115f, 0x1160],
  [0x17b4, 0x17b5],
  [0x180e, 0x180e],
  [0x200b, 0x200f],
  [0x2028, 0x202e],
  [0x2060, 0x206f],
  [0x3164, 0x3164],
  [0xfe00, 0xfe0f],
  [0xfeff, 0xfeff],
  [0xffa0, 0xffa0],
  [0xfff9, 0xfffb],
  [0x1bca0, 0x1bca3],
  [0x1d173, 0x1d17a],
  [0xe0000, 0xe007f],
  [0xe0100, 0xe01ef],
]

function invisible(char: string): boolean {
  const cp = char.codePointAt(0) ?? 0
  return INVISIBLE.some(([from, to]) => cp >= from && cp <= to)
}
/** C0 (except `\n`) and C1 control characters. */
// eslint-disable-next-line no-control-regex
const CONTROL = /[\u0000-\u0009\u000B-\u001F\u007F-\u009F]/g

/** Removes the characters that could reorder or hide text, and `\n` too unless `multiline`. */
export function inertText(text: unknown, multiline = false): string {
  if (typeof text !== "string") return ""
  // By code point, so astral ranges (tags, variation selectors) match whole.
  const cleaned = Array.from(text.replace(CONTROL, ""))
    .filter((char) => !invisible(char))
    .join("")
  return multiline ? cleaned : cleaned.replace(/\n+/g, " ")
}

/** `inertText`, at most `max` characters (an ellipsis when cut). */
export function clip(text: unknown, max: number, multiline = false): string {
  const clean = inertText(text, multiline)
  // By code points, so a surrogate pair is never split.
  const chars = Array.from(clean)
  return chars.length <= max
    ? clean
    : `${chars
        .slice(0, max - 1)
        .join("")
        .trimEnd()}\u2026`
}

/** Display lengths (the design's limits, with room). */
export const LIMITS = {
  title: 120,
  body: 280,
  cta: 40,
  alt: 250,
  caption: 160,
  promo: 100,
  highlightTitle: 60,
  highlightText: 200,
  version: 64,
  /** The markdown subset's input (description 6000, release notes 2000). */
  markdown: 8000,
} as const

/**
 * Whether `labels` are an IPv4 address in an `inet_aton` form: one to
 * four parts, each decimal, octal (leading 0) or hex (`0x`).
 */
function isInetAton(labels: readonly string[]): boolean {
  return (
    labels.length >= 1 &&
    labels.length <= 4 &&
    labels.every((label) => /^(0x[0-9a-f]*|0[0-7]*|[1-9][0-9]*)$/.test(label))
  )
}

/**
 * Whether `url` is a link the Marketplace may show (design §4 "URLs"):
 * https only, at most 512 characters, no userinfo, no port, no IP-literal
 * host, a lowercase ASCII host without punycode, nothing invisible.
 */
export function safeHttpsUrl(url: unknown): string | null {
  if (typeof url !== "string" || url.length > 512) return null
  if (url !== inertText(url) || /\s/.test(url)) return null
  if (!url.startsWith("https://")) return null
  let parsed: URL
  try {
    parsed = new URL(url)
  } catch {
    return null
  }
  if (parsed.protocol !== "https:" || parsed.username || parsed.password || parsed.port) return null
  // The host as written: `URL` would lowercase and punycode it for us.
  const host = /^https:\/\/([^/?#]*)/.exec(url)?.[1] ?? ""
  if (!/^[a-z0-9.-]+$/.test(host) || !host.includes(".")) return null
  const labels = host.split(".")
  if (labels.some((label) => label === "" || label.startsWith("xn--"))) return null
  // No IP literal in any form (the plugins repo's rule): a last label
  // that is all digits or hex-like (`acme.123`, `0x7f.1`: the URL parser
  // reads such a host as IPv4), or a host that is IPv4 in any `inet_aton`
  // form (`127.1`, `0177.0.0.1`). Numeric labels elsewhere are ordinary
  // (`163.com`, `www.1.com`). IPv6 needs brackets and an integer host has
  // no dot: both refused above.
  if (/^(0x[0-9a-f]*|[0-9]+)$/.test(labels.at(-1) ?? "") || isInetAton(labels)) return null
  // Nothing the parser rewrote: the host shown is the host reached.
  if (parsed.hostname !== host) return null
  // No dot segments, plain or percent-encoded, in the path.
  const path = /^https:\/\/[^/?#]*([^?#]*)/.exec(url)?.[1] ?? ""
  if (path.split("/").some((segment) => /^(\.|%2e){1,2}$/i.test(segment))) return null
  return url
}
