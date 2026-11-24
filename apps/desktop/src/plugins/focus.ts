// Which plugin the Marketplace opens at: set right before another screen
// opens it (the Inspector's "Install plugin" for a key whose plugin is
// missing), read once when the Marketplace mounts. Only a plugin id; the
// Marketplace shows that plugin's details when the registry lists it and
// its catalog otherwise. Nothing is installed from here.

let pending: string | null = null

/** The Marketplace opens next at `plugin`'s details. */
export function focusPlugin(plugin: string): void {
  pending = plugin
}

/** The plugin to open at, once; `null` when none was asked for. */
export function takeFocusedPlugin(): string | null {
  const plugin = pending
  pending = null
  return plugin
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
