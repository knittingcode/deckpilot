// A registry catalog (`listAvailablePlugins`) whose ids match the
// storefront fixtures (`plugins/storefront-mocks.ts`), for the storefront
// tests and the dev preview. Deliberately incomplete against them:
// `deckpilot.break-reminder` (in the "Stay focused" row) is not in it, so
// its placement must be dropped; `acme.future` runs nowhere here.

import { MEDIA } from "@/plugins/storefront-mocks"
import type { AvailablePlugin, ManifestPermission } from "@/plugins/types"

const DAY = 86_400
/** 2026-11-01, in seconds. */
const NOV_1 = 1_793_491_200

const network = (host: string, why: string): ManifestPermission => ({
  kind: "network",
  hosts: [host],
  methods: ["GET"],
  justification: why,
})

export function storefrontCatalog(): AvailablePlugin[] {
  return [
    {
      id: "deckpilot.weather",
      name: "Weather",
      summary: "Current conditions and a forecast on your keys.",
      description: "Current conditions and a seven-day forecast.",
      publisher: "deckpilot",
      publisherInfo: {
        namespace: "deckpilot",
        displayName: "DeckPilot",
        login: "knittingcode",
        verified: true,
      },
      categories: ["monitoring"],
      latest: "0.2.1",
      official: true,
      icon: MEDIA.weatherIcon,
      versions: [
        {
          version: "0.2.1",
          permissions: [network("api.open-meteo.com", "Reads the forecast.")],
          publishedAt: NOV_1 + 10 * DAY,
        },
        {
          version: "0.2.0",
          permissions: [network("api.open-meteo.com", "Reads the forecast.")],
          publishedAt: NOV_1,
        },
      ],
    },
    {
      id: "deckpilot.pomodoro",
      name: "Pomodoro",
      summary: "Focus timers on a key.",
      publisher: "deckpilot",
      publisherInfo: {
        namespace: "deckpilot",
        displayName: "DeckPilot",
        login: "knittingcode",
        verified: true,
      },
      categories: ["productivity"],
      latest: "1.0.0",
      official: true,
      icon: MEDIA.pomodoroIcon,
      versions: [{ version: "1.0.0", permissions: [], publishedAt: NOV_1 - 30 * DAY }],
    },
    {
      id: "acme.clock",
      name: "Acme Clock",
      summary: "The time, in any zone.",
      publisher: "acme",
      publisherInfo: {
        namespace: "acme",
        displayName: "Acme Tools",
        login: "acme",
        verified: true,
      },
      categories: ["productivity"],
      latest: "1.2.0",
      versions: [{ version: "1.2.0", permissions: [], publishedAt: NOV_1 + 5 * DAY }],
    },
    {
      id: "acme.github-prs",
      name: "Pull Requests",
      summary: "Your open pull requests at a glance.",
      publisher: "acme",
      publisherInfo: { namespace: "acme", login: "acme", verified: false },
      categories: ["developer-tools"],
      latest: "0.4.0",
      versions: [
        {
          version: "0.4.0",
          permissions: [network("api.github.com", "Lists your pull requests.")],
          publishedAt: NOV_1 + 2 * DAY,
        },
      ],
    },
    {
      id: "acme.future",
      name: "Future Keys",
      summary: "Needs a newer DeckPilot.",
      publisher: "acme",
      categories: ["productivity"],
      latest: "2.0.0",
      versions: [
        { version: "2.0.0", permissions: [], incompatible: ["requires DeckPilot >=9.0.0"] },
      ],
    },
  ]
}
