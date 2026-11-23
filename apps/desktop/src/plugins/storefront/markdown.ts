// The storefront's markdown subset: the tokenizer (storefront design §4), for a listing's
// description and release notes only. Our own tokenizer, straight to React
// elements: there is no HTML anywhere on the way, so `<script>` and every
// other tag are just the characters they are.
//
// Allowed: paragraphs; a single line break inside a paragraph; `-` and `*`
// lists one level deep; `**bold**`, `_em_`, `` `code` ``; `[text](https://…)`
// under the URL rules (`safeHttpsUrl`). Everything else, headings, images,
// tables, raw HTML, a link to any other scheme, an unclosed marker, is
// literal text.
//
// Links: the app does not open web pages, so a link shows its text and its
// full URL (to read or copy); nothing navigates the webview.
//
// Bounded work: the input is clipped (LIMITS.markdown) and the node count
// is capped; every marker search is linear and remembers when a closer no
// longer exists, so emphasis bombs (10 000 `*`) stay linear.

import { clip, LIMITS, safeHttpsUrl } from "./text"

export type Inline =
  | { type: "text"; text: string }
  | { type: "strong"; children: Inline[] }
  | { type: "em"; children: Inline[] }
  | { type: "code"; text: string }
  | { type: "link"; text: string; url: string }
  | { type: "break" }

export type Block = { type: "paragraph"; children: Inline[] } | { type: "list"; items: Inline[][] }

/** At most this many inline nodes per document; the rest is literal text. */
const MAX_NODES = 2000
/** Emphasis nests at most this deep (bold inside em inside bold). */
const MAX_DEPTH = 3

const LIST_ITEM = /^[-*] +(.*)$/

/** The blocks of `source`: paragraphs (blank-line separated) and one-level lists. */
export function parseMarkdown(source: string): Block[] {
  const text = clip(source, LIMITS.markdown, true).replace(/\r\n?/g, "\n")
  const budget = { nodes: MAX_NODES }
  const blocks: Block[] = []
  for (const chunk of text.split(/\n[ \t]*\n/)) {
    const lines = chunk.split("\n").filter((l) => l.trim() !== "")
    if (lines.length === 0) continue
    if (lines.every((l) => LIST_ITEM.test(l))) {
      blocks.push({
        type: "list",
        items: lines.map((l) => parseInline(LIST_ITEM.exec(l)?.[1] ?? "", budget)),
      })
      continue
    }
    const children: Inline[] = []
    lines.forEach((line, i) => {
      if (i > 0) children.push({ type: "break" })
      children.push(...parseInline(line, budget))
    })
    blocks.push({ type: "paragraph", children })
  }
  return blocks
}

const LINK = /^\[([^[\]\n]{1,300})\]\(([^()\s]{1,512})\)/
const WORD = /[\p{L}\p{N}]/u

/** Inline nodes of one line. */
export function parseInline(line: string, budget = { nodes: MAX_NODES }, depth = 0): Inline[] {
  const out: Inline[] = []
  let text = ""
  // Once a closer is missing from position p on, it is missing for every later p.
  const none = { bold: false, em: false, code: false }
  const flush = () => {
    if (text) out.push({ type: "text", text })
    text = ""
  }
  const push = (node: Inline) => {
    flush()
    out.push(node)
    budget.nodes--
  }
  let i = 0
  while (i < line.length) {
    if (budget.nodes <= 0) {
      text += line.slice(i)
      break
    }
    const c = line.charAt(i)
    if (c === "`" && !none.code) {
      const end = line.indexOf("`", i + 1)
      if (end === -1) none.code = true
      else if (end > i + 1) {
        push({ type: "code", text: line.slice(i + 1, end) })
        i = end + 1
        continue
      }
    } else if (c === "*" && line[i + 1] === "*" && !none.bold && depth < MAX_DEPTH) {
      const end = line.indexOf("**", i + 2)
      if (end === -1) none.bold = true
      else if (end > i + 2 && line[i + 2] !== " " && line[end - 1] !== " ") {
        push({ type: "strong", children: parseInline(line.slice(i + 2, end), budget, depth + 1) })
        i = end + 2
        continue
      } else {
        // `****` or `** x **`: literal, both markers.
        text += "**"
        i += 2
        continue
      }
    } else if (c === "_" && !none.em && depth < MAX_DEPTH && !WORD.test(line[i - 1] ?? "")) {
      const end = closingUnderscore(line, i + 1)
      if (end === -1) none.em = true
      else if (end > i + 1 && line[i + 1] !== " " && line[end - 1] !== " ") {
        push({ type: "em", children: parseInline(line.slice(i + 1, end), budget, depth + 1) })
        i = end + 1
        continue
      }
    } else if (c === "!" && line[i + 1] === "[") {
      // An image: not in the subset, so neither an image nor a link.
      text += "!["
      i += 2
      continue
    } else if (c === "[") {
      const match = LINK.exec(line.slice(i, i + 820))
      const url = match ? safeHttpsUrl(match[2]) : null
      if (match?.[1] && url) {
        push({ type: "link", text: match[1], url })
        i += match[0].length
        continue
      }
    }
    text += c
    i++
  }
  flush()
  return out
}

/** The next `_` from `from` that closes emphasis (not inside a word), or -1. */
function closingUnderscore(line: string, from: number): number {
  for (let at = line.indexOf("_", from); at !== -1; at = line.indexOf("_", at + 1)) {
    if (!WORD.test(line[at + 1] ?? "")) return at
  }
  return -1
}
