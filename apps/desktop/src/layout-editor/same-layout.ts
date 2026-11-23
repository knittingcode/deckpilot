// Whether the editor's draft differs from the stored layout. The agent
// leaves an empty binding label out of what it stores, while the editor
// writes `label: null` (or `""` once a label is cleared): the same layout,
// so a key that was just dropped and saved is not "unsaved". Nothing here
// changes what is saved.

import type { DeviceLayout } from "@/lib/types"

/** `layout` with every empty binding label (`null`, `""`) left out. */
function withoutEmptyLabels(layout: DeviceLayout): DeviceLayout {
  return {
    ...layout,
    pages: layout.pages.map((page) => ({
      ...page,
      bindings: page.bindings.map((binding) => {
        if (binding.label !== null && binding.label !== undefined && binding.label !== "") {
          return binding
        }
        const rest: Partial<typeof binding> = { ...binding }
        delete rest.label
        return rest as typeof binding
      }),
    })),
  }
}

/** Whether `a` and `b` are the same layout, empty labels however written. */
export function sameLayout(a: DeviceLayout, b: DeviceLayout): boolean {
  return JSON.stringify(withoutEmptyLabels(a)) === JSON.stringify(withoutEmptyLabels(b))
}
