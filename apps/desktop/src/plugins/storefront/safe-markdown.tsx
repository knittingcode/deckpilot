// Renders the storefront's markdown subset (markdown.ts) as React elements:
// no HTML on the way, no `dangerouslySetInnerHTML`, links never navigate.

import { Fragment, type ReactNode } from "react"

import { cn } from "@/lib/utils"

import { type Inline, parseMarkdown } from "./markdown"

function renderInline(nodes: Inline[]): ReactNode[] {
  return nodes.map((node, i) => {
    switch (node.type) {
      case "text":
        return <Fragment key={i}>{node.text}</Fragment>
      case "break":
        return <br key={i} />
      case "strong":
        return (
          <strong key={i} className="font-semibold">
            {renderInline(node.children)}
          </strong>
        )
      case "em":
        return <em key={i}>{renderInline(node.children)}</em>
      case "code":
        return (
          <code key={i} className="rounded bg-muted px-1 py-0.5 font-mono text-[0.85em]">
            {node.text}
          </code>
        )
      case "link":
        return (
          <span key={i} data-slot="markdown-link">
            <span className="underline decoration-muted-foreground/60 underline-offset-2">
              {node.text}
            </span>{" "}
            <span className="text-muted-foreground select-text">({node.url})</span>
          </span>
        )
    }
  })
}

/** `source` in the markdown subset, as React elements. */
export function SafeMarkdown({ source, className }: { source: string; className?: string }) {
  const blocks = parseMarkdown(source)
  return (
    <div
      data-slot="safe-markdown"
      dir="auto"
      className={cn(
        "grid max-w-prose gap-3 text-sm leading-6 [overflow-wrap:anywhere] select-text",
        className,
      )}
    >
      {blocks.map((block, i) =>
        block.type === "list" ? (
          <ul key={i} className="grid list-disc gap-1 pl-5">
            {block.items.map((item, j) => (
              <li key={j}>{renderInline(item)}</li>
            ))}
          </ul>
        ) : (
          <p key={i}>{renderInline(block.children)}</p>
        ),
      )}
    </div>
  )
}
