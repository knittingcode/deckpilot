// The state of a compact inline number field (components/inline-number-input.tsx):
// what is typed, whether it is valid, and storing or cancelling it.

import { useState } from "react"

/** The one look of a compact inline field (Automatic exit, Maximum lines). */
export const INLINE_NUMBER_INPUT =
  "[field-sizing:content] h-8 w-auto max-w-[8ch] min-w-[calc(2ch+1.25rem)] px-2 text-center tabular-nums"

/** A stored value: a number, automatic (`null`, only with `auto`) or mixed (several keys). */
export type InlineNumber = number | null | "mixed"

const textOf = (value: InlineNumber) => (typeof value === "number" ? String(value) : "")

/**
 * What is typed in an inline number field, for the row around it (the
 * plural of its unit, why it is not valid). `check` says why a text is not a
 * value (an i18n key or message), `null` when it is; with `auto`, an empty
 * field (or the word for automatic) is the automatic value.
 */
export function useInlineNumber({
  stored,
  check,
  auto,
  onCommit,
}: {
  stored: InlineNumber
  check: (text: string) => string | null
  /** The word shown for the automatic value; without it, a value is required. */
  auto?: string
  onCommit: (value: number | null) => void
}) {
  const [text, setText] = useState(textOf(stored))
  const [synced, setSynced] = useState(stored)
  if (stored !== synced) {
    // Undo, redo, revert or another key changed it.
    setSynced(stored)
    setText(textOf(stored))
  }
  const trimmed = text.trim()
  const automatic =
    auto !== undefined &&
    (trimmed === "" || trimmed.toLocaleLowerCase() === auto.toLocaleLowerCase())
  const error = automatic ? null : check(trimmed)
  /** The value typed, when valid: a number, or `null` for automatic. */
  const typed: number | null | undefined = error ? undefined : automatic ? null : Number(trimmed)
  const commit = () => {
    if (typed === undefined) {
      setText(textOf(stored))
      return
    }
    // Nothing typed over a mixed value keeps it mixed.
    if (stored === "mixed" && trimmed === "") return
    setText(textOf(typed))
    if (typed !== stored) {
      setSynced(typed)
      onCommit(typed)
    }
  }
  const cancel = () => {
    setText(textOf(stored))
  }
  /** Something typed that is not stored yet. */
  const editing = text !== textOf(stored)
  return { text, setText, error, typed, editing, commit, cancel }
}

export type InlineNumberState = ReturnType<typeof useInlineNumber>

/**
 * Whether an Escape is an inline number field's, to cancel its unfinished
 * edit: a popover around it (whose Escape listener runs first, on the
 * document) then stays open (`onEscapeKeyDown`).
 */
export function cancelsInlineEdit(event: KeyboardEvent): boolean {
  const target = event.target
  return target instanceof Element && target.matches("[data-slot=inline-number][data-editing]")
}
