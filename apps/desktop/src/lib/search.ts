// One rule for every search field of the app (the Device Editor's library,
// the workflow catalog, the Marketplace, Settings, pickers): a text is
// found when it holds every word of the query, in any order, whatever the
// case. An empty query finds everything. Lists are sorted the same way
// everywhere too: by name, in the window's language (`compareText`,
// i18n/format.ts).

/** The words of a search query, in lower case. */
export function searchWords(query: string): string[] {
  return query.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean)
}

/** Whether `texts` together hold every word of `query`; an empty query matches. */
export function matchesWords(query: string, ...texts: (string | null | undefined)[]): boolean {
  const words = searchWords(query)
  if (words.length === 0) return true
  const haystack = texts
    .filter((text): text is string => typeof text === "string" && text !== "")
    .join(" ")
    .toLocaleLowerCase()
  return words.every((word) => haystack.includes(word))
}
