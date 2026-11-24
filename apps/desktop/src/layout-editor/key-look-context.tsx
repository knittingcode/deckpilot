import { createContext, useContext } from "react"

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

export const KeyLookContext = createContext<KeyLook | null>(null)

/** The editor's key look, `null` outside the editor. */
export function useKeyLook(): KeyLook | null {
  return useContext(KeyLookContext)
}
