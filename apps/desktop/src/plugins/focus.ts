// Which plugin the Marketplace opens at: set right before another screen
// opens it (the Inspector's "Install plugin" for a key whose plugin is
// missing, Settings › Permissions' "Open in Marketplace"), taken once by
// the Marketplace when it comes on screen: when it mounts, when it shows
// again after the settings (the workspace keeps it mounted under them), or
// at once when it is already on screen. A navigation that is refused
// (unsaved edits kept) forgets it, so it never fires later. Only a plugin
// id; the Marketplace shows that plugin's details when the registry lists
// it and its catalog otherwise. Nothing is installed from here.

let pending: string | null = null
const listeners = new Set<() => void>()

/** The Marketplace opens next at `plugin`'s details. */
export function focusPlugin(plugin: string): void {
  pending = plugin
  for (const listener of [...listeners]) listener()
}

/** The plugin to open at, once; `null` when none was asked for. */
export function takeFocusedPlugin(): string | null {
  const plugin = pending
  pending = null
  return plugin
}

/** Forgets a plugin to open at that was not taken (its navigation was refused). */
export function forgetFocusedPlugin(): void {
  pending = null
}

/** Calls `listener` whenever a screen asks to open the Marketplace at a plugin. */
export function onFocusPlugin(listener: () => void): () => void {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}

let migrations = false

/** Settings › Logs & Diagnostics opens next at the module migrations. */
export function focusMigrations(): void {
  migrations = true
}

/** Whether to open at the module migrations, once. */
export function takeFocusedMigrations(): boolean {
  const focus = migrations
  migrations = false
  return focus
}
