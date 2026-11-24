// Development only: the Marketplace against storefront fixtures instead of
// the agent (storefront design §8 "true preview"), so the editorial UI can
// be reviewed before any agent or registry serves a storefront:
//
//   VITE_STOREFRONT_FIXTURE=default pnpm --filter @deckpilot/desktop dev
//
// `default`, `empty` or `hostile` are the fixtures of `storefront-mocks.ts`;
// anything else is the path of a JSON fixture (a Marketplace Studio export),
// served to the dev build by `dev/storefront-fixture-plugin.ts`.
// `VITE_STOREFRONT_FIXTURE_DELAY_MS` delays every answer, to see the
// placeholders. Production builds never contain this: `import.meta.env.DEV`
// is false there and the mocks are only imported dynamically.

import type { StorefrontClient } from "../storefront-api"
import type { StorefrontFixture } from "../storefront-mocks"

/** Where the dev server serves a fixture file (see `dev/storefront-fixture-plugin.ts`). */
export const DEV_FIXTURE_URL = "/__deckpilot/storefront-fixture.json"

let devClient: StorefrontClient | null | undefined

/** The fixture client of `VITE_STOREFRONT_FIXTURE`, or `null` (always in production). */
export function devStorefrontClient(): StorefrontClient | null {
  if (!import.meta.env.DEV) return null
  if (devClient !== undefined) return devClient
  const name = import.meta.env.VITE_STOREFRONT_FIXTURE
  devClient = name
    ? fixtureClient(name, Number(import.meta.env.VITE_STOREFRONT_FIXTURE_DELAY_MS))
    : null
  return devClient
}

/** A client answering from the named fixture (or a fixture file), loaded on first use. */
export function fixtureClient(name: string, delayMs = 0): StorefrontClient {
  let loaded: Promise<StorefrontClient> | undefined
  const client = () =>
    (loaded ??= import("../storefront-mocks").then(async (mocks) => {
      const builtIn: Record<string, StorefrontFixture> = {
        default: mocks.defaultFixture,
        empty: mocks.emptyFixture,
        hostile: mocks.hostileFixture,
      }
      const fixture = Object.hasOwn(builtIn, name)
        ? builtIn[name]
        : mocks.fixtureFromJson(await (await fetch(DEV_FIXTURE_URL)).json())
      return mocks.createFixtureStorefrontClient(
        fixture,
        Number.isFinite(delayMs) && delayMs > 0 ? { delayMs } : {},
      )
    }))
  return {
    getStorefront: async () => (await client()).getStorefront(),
    getRegistryListing: async (plugin) => (await client()).getRegistryListing(plugin),
    getRegistryMedia: async (sha256) => (await client()).getRegistryMedia(sha256),
  }
}
