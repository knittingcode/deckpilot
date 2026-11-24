import { createContext, useContext } from "react"

import type { LabelBox } from "@/lib/key-labels"

import type {
  ActionType,
  ControlId,
  DefaultAppearance,
  DeviceLayout,
  KeyFace,
  MarqueePreview,
  TextLayoutReport,
  VisualLibrary,
} from "@/lib/types"

import type { KeyLookChange } from "./appearance"
import type { IconState } from "./icon-library"

/**
 * What the Inspector's key header, its title appearance popover and its
 * icon menu need from the editor (editor.tsx provides it): the draft, the
 * key previews exactly as the Canvas shows them, the profile's icons, and
 * the one way to change the look of the selected keys (an undo step of the
 * editor's history). Without it (an Inspector rendered on its own) none of
 * them is shown.
 */
export interface KeyLook {
  /** The profile whose layout is edited: titles resolve in it. */
  profileId?: string
  draft: DeviceLayout
  pageId: string
  /** The device model's colours for keys that set none. */
  defaults: DefaultAppearance
  /** Rendered key images, by control key: the Canvas's. */
  images: Map<string, string>
  animations?: Map<string, MarqueePreview>
  faces: Map<string, KeyFace>
  /** The action icon names of the drawn faces (before an image arrives). */
  symbols?: Map<string, string | null | undefined>
  /** How each key's title comes out on the device. */
  reports?: Map<string, TextLayoutReport>
  /** Where the renderer put each key's labels (lib/key-labels.ts). */
  labelBoxes?: Map<string, LabelBox[]>
  /**
   * The selected label of a key (the Inspector's Labels section and the
   * outlines on the device canvas share it); `selectLabel` changes it,
   * `edit` asks for its text field.
   */
  labelSelection?: LabelSelection | null
  selectLabel?: (key: string, id: string | null, edit?: boolean) => void
  /** The agent draws labels (capability `keyLabels`); without it they are kept, not shown. */
  keyLabels?: boolean
  icons: IconState
  /** DeckPilot's icons and gradients and the plugins' assets. */
  library: VisualLibrary
  /** The action type the key's shown binding runs, if any. */
  typeOf: (control: ControlId) => ActionType | undefined
  /** Changes the look of the selected keys; `group` merges typing. */
  restyle: (change: KeyLookChange, group?: string) => void
  /**
   * An icon of the key menu that could not be imported or pasted: set by the
   * preview's menu, shown under the key's title in the settings column.
   */
  problem?: string | null
  setProblem?: (message: string | null) => void
}

/** A key's selected label. */
export interface LabelSelection {
  /** The control key (`key:0`). */
  key: string
  id: string | null
  /** Non-zero, new for each request, when the label's text is to be edited. */
  edit: number
}

let edits = 0

/** The selection after choosing label `id` of `key` (to edit it, with `edit`). */
export function nextSelection(
  _current: LabelSelection | null,
  key: string,
  id: string | null,
  edit = false,
): LabelSelection {
  return { key, id, edit: edit && id !== null ? ++edits : 0 }
}

export const KeyLookContext = createContext<KeyLook | null>(null)

/** The editor's key look, `null` outside the editor. */
export function useKeyLook(): KeyLook | null {
  return useContext(KeyLookContext)
}
