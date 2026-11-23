// The compact inline number field of a settings row ("Automatic exit after
// [10] seconds", "Maximum lines [Auto]"): a text field (no spinner arrows)
// only as wide as its content, whose value is stored with Enter or when it
// is left, and only when it is valid. Escape cancels an unfinished edit
// (brings back the stored value); so do Enter and leaving the field when
// what is typed is not valid.

import { useRef, type ComponentProps } from "react"

import { Input } from "@/components/ui/input"
import { cn } from "@/lib/utils"

import { INLINE_NUMBER_INPUT, type InlineNumberState } from "./inline-number"

/** The field itself, bound to `useInlineNumber`'s state. */
export function InlineNumberInput({
  field,
  auto,
  mixed,
  className,
  style,
  ...props
}: Omit<
  ComponentProps<"input">,
  "value" | "onChange" | "onBlur" | "onKeyDown" | "onFocus" | "onMouseUp" | "type" | "placeholder"
> & {
  field: InlineNumberState
  /** The word shown while the value is automatic. */
  auto?: string
  /** The word shown while several keys differ. */
  mixed?: string
}) {
  const focused = useRef(false)
  const placeholder = field.text === "" ? (mixed ?? auto) : undefined
  // As wide as the word for automatic, whatever is shown: switching between
  // it and a number does not move the row.
  const room = Math.max(auto?.length ?? 0, mixed?.length ?? 0)
  return (
    <Input
      type="text"
      inputMode="numeric"
      autoComplete="off"
      className={cn(INLINE_NUMBER_INPUT, room > 2 && "max-w-none", className)}
      style={room > 2 ? { minWidth: `calc(${String(room)}ch + 1.25rem)`, ...style } : style}
      data-slot="inline-number"
      data-editing={field.editing ? "" : undefined}
      placeholder={placeholder}
      value={field.text}
      aria-invalid={field.error ? true : undefined}
      onFocus={(event) => {
        // The whole value selected, to type over it; a click's mouseup
        // would put the caret back (WebKit), so that one is skipped.
        event.currentTarget.select()
        focused.current = true
      }}
      onMouseUp={(event) => {
        if (focused.current) event.preventDefault()
        focused.current = false
      }}
      onBlur={field.commit}
      onKeyDown={(event) => {
        if (event.key === "Enter") {
          event.preventDefault()
          field.commit()
        } else if (event.key === "Escape" && field.editing) {
          // An unfinished edit is cancelled, not the popover or panel around
          // it (Escape without one is theirs).
          event.preventDefault()
          event.stopPropagation()
          field.cancel()
        }
      }}
      onChange={(event) => {
        field.setText(event.target.value)
      }}
      {...props}
    />
  )
}
