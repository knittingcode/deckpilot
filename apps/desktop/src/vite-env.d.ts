/// <reference types="vite/client" />

interface ImportMetaEnv {
  /**
   * "1" builds the experimental plugin marketplace into the app
   * (Settings › Extensions › Plugins). Unset in normal and release builds.
   */
  readonly VITE_DECKPILOT_PLUGINS?: string
  /**
   * Development only: the Marketplace's storefront from a fixture instead of
   * the agent: `default`, `empty`, `hostile` or the path of a JSON fixture
   * (src/plugins/storefront/dev-fixture.ts).
   */
  readonly VITE_STOREFRONT_FIXTURE?: string
  /** Development only: delays every fixture answer by this many milliseconds. */
  readonly VITE_STOREFRONT_FIXTURE_DELAY_MS?: string
}
