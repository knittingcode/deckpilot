/**
 * Development only: serves the storefront fixture file named by
 * `VITE_STOREFRONT_FIXTURE=<path>` (a Marketplace Studio export) to the dev
 * build at a fixed URL, read by `src/plugins/storefront/dev-fixture.ts`.
 *
 * - Exists only in `vite dev` (`apply: "serve"`), never in builds.
 * - Serves exactly the one file named when the server started; nothing in
 *   a request chooses a path. `default`, `empty` and `hostile` are built-in
 *   fixtures and need no file.
 * - At most 32 MiB, and it must parse as JSON.
 * - Reachable by whatever can reach the dev server: only this computer by
 *   default, but with `TAURI_DEV_HOST` set (vite.config.ts) the server
 *   listens on that address and so the LAN can fetch the fixture too.
 *   Use only fixtures you would share on that network.
 */
import { readFileSync, statSync } from "node:fs"
import path from "node:path"

import type { Plugin } from "vite"

const URL_PATH = "/__deckpilot/storefront-fixture.json"
const BUILT_IN = new Set(["default", "empty", "hostile"])
const MAX_BYTES = 32 * 1024 * 1024

export function storefrontFixture(): Plugin {
  return {
    name: "deckpilot-storefront-fixture",
    apply: "serve",
    configureServer(server) {
      const name = process.env.VITE_STOREFRONT_FIXTURE
      if (!name || BUILT_IN.has(name)) return
      const file = path.resolve(name)
      server.middlewares.use((req, res, next) => {
        if (req.url !== URL_PATH) {
          next()
          return
        }
        try {
          if (req.method !== "GET") throw new Error("GET only")
          if (statSync(file).size > MAX_BYTES) throw new Error("the fixture is over 32 MiB")
          const text = readFileSync(file, "utf8")
          JSON.parse(text)
          res.setHeader("content-type", "application/json")
          res.setHeader("cache-control", "no-store")
          res.end(text)
        } catch (error) {
          res.statusCode = 500
          res.end(JSON.stringify({ error: String(error) }))
        }
      })
    },
  }
}
