/**
 * The groups of the main window's rail: the built-in pages' groups as they
 * are, then the pages that plugins and modules contribute, in a group of
 * their own. That group is built from the contributed pages alone, so
 * removing or changing a built-in page (as the Action Library and the
 * Dashboard were) can never take contributed pages with it.
 */
export interface RailGroup<Item> {
  id: string
  label: string
  items: Item[]
}

export function railGroups<Item>(
  builtIn: RailGroup<Item>[],
  contributed: { id: string; label: string; items: Item[] },
): RailGroup<Item>[] {
  return [
    // A built-in group without pages is not shown.
    ...builtIn.filter((group) => group.items.length > 0),
    ...(contributed.items.length > 0 ? [contributed] : []),
  ]
}
