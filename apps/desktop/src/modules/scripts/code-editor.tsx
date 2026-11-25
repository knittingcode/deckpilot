// The script editor: CodeMirror 6, loaded with the Scripts page only.
// Diagnostics are the parser's (no code runs while typing); completion
// offers the `deckpilot` API; invisible and direction-changing characters
// are drawn as marks.

import {
  autocompletion,
  closeBrackets,
  closeBracketsKeymap,
  completionKeymap,
  type CompletionContext,
  type CompletionResult,
} from "@codemirror/autocomplete"
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands"
import { javascript } from "@codemirror/lang-javascript"
import {
  bracketMatching,
  defaultHighlightStyle,
  indentOnInput,
  syntaxHighlighting,
  syntaxTree,
} from "@codemirror/language"
import { type Diagnostic, linter, lintGutter } from "@codemirror/lint"
import { highlightSelectionMatches, searchKeymap } from "@codemirror/search"
import { Compartment, EditorState } from "@codemirror/state"
import {
  drawSelection,
  EditorView,
  highlightActiveLine,
  highlightSpecialChars,
  keymap,
  lineNumbers,
} from "@codemirror/view"
import { useEffect, useRef } from "react"

import { DECKPILOT_MEMBERS, HIDDEN_CHARS } from "./completions"

function deckpilotCompletions(context: CompletionContext): CompletionResult | null {
  const word = context.matchBefore(/deckpilot(\.[A-Za-z]*)*$/)
  if (!word || (word.from === word.to && !context.explicit)) return null
  const parts = word.text.split(".")
  if (parts.length < 2) return null
  const last = parts.pop() ?? ""
  const owner = parts.slice(1).join(".")
  const members = DECKPILOT_MEMBERS[owner]
  if (!members) return null
  return {
    from: word.to - last.length,
    options: members.map((label) => ({ label, type: "property" })),
  }
}

function syntaxErrors(message: string) {
  return (view: EditorView): Diagnostic[] => {
    const found: Diagnostic[] = []
    syntaxTree(view.state).iterate({
      enter: (node) => {
        if (node.type.isError) {
          found.push({ from: node.from, to: node.to, severity: "error", message })
        }
      },
    })
    return found
  }
}

export function CodeEditor({
  value,
  onChange,
  readOnly = false,
  label,
  syntaxErrorMessage,
}: {
  value: string
  onChange?: (value: string) => void
  readOnly?: boolean
  /** The accessible name of the editor. */
  label: string
  syntaxErrorMessage: string
}) {
  const host = useRef<HTMLDivElement>(null)
  const view = useRef<EditorView | null>(null)
  const editable = useRef(new Compartment())
  const changed = useRef(onChange)
  useEffect(() => {
    changed.current = onChange
  }, [onChange])

  useEffect(() => {
    if (!host.current) return
    const state = EditorState.create({
      doc: value,
      extensions: [
        lineNumbers(),
        highlightSpecialChars({ addSpecialChars: HIDDEN_CHARS }),
        history(),
        drawSelection(),
        indentOnInput(),
        syntaxHighlighting(defaultHighlightStyle, { fallback: true }),
        bracketMatching(),
        closeBrackets(),
        autocompletion({ override: [deckpilotCompletions] }),
        highlightActiveLine(),
        highlightSelectionMatches(),
        javascript(),
        lintGutter(),
        linter(syntaxErrors(syntaxErrorMessage), { delay: 800 }),
        keymap.of([
          ...closeBracketsKeymap,
          ...defaultKeymap,
          ...searchKeymap,
          ...historyKeymap,
          ...completionKeymap,
          indentWithTab,
        ]),
        editable.current.of([EditorState.readOnly.of(readOnly), EditorView.editable.of(!readOnly)]),
        EditorView.contentAttributes.of({ "aria-label": label }),
        EditorView.updateListener.of((update) => {
          if (update.docChanged) changed.current?.(update.state.doc.toString())
        }),
        EditorView.theme({
          "&": { fontSize: "13px", height: "100%" },
          ".cm-scroller": { fontFamily: "var(--font-mono, ui-monospace, monospace)" },
          ".cm-specialChar": { color: "var(--destructive)", fontWeight: "bold" },
        }),
      ],
    })
    view.current = new EditorView({ state, parent: host.current })
    return () => {
      view.current?.destroy()
      view.current = null
    }
    // The editor is made once; `value` and `readOnly` follow below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  useEffect(() => {
    const current = view.current
    if (current && current.state.doc.toString() !== value) {
      current.dispatch({ changes: { from: 0, to: current.state.doc.length, insert: value } })
    }
  }, [value])

  useEffect(() => {
    view.current?.dispatch({
      effects: editable.current.reconfigure([
        EditorState.readOnly.of(readOnly),
        EditorView.editable.of(!readOnly),
      ]),
    })
  }, [readOnly])

  return (
    <div
      ref={host}
      className="min-h-64 overflow-hidden rounded-md border bg-background text-foreground"
      data-testid="script-editor"
    />
  )
}
