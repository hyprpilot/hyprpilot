import { capitalCase } from 'change-case'

/**
 * Convert a snake_case canonical tool key into a human-readable
 * label — `bash` → "Bash", `bash_output` → "Bash output",
 * `multi_edit` → "Multi edit", `web_fetch` → "Web fetch". Delegates
 * to the `change-case` library's `capitalCase`, which handles
 * snake_case / camelCase / kebab-case input uniformly + applies the
 * "first word capitalised, rest lower-case" convention we want for
 * the chip's text identifier (sentence case, not title case — title
 * case capitalises every word, which reads stiff for two-word
 * tool names like "Web fetch").
 */
export function titleCaseFromCanonical(canonical: string): string {
  if (!canonical) {
    return ''
  }
  const cased = capitalCase(canonical)

  // `capitalCase` returns "Web Fetch"; we prefer "Web fetch"
  // (sentence case) so multi-word tool names read as one phrase
  // rather than a Caps Title.
  const [head, ...rest] = cased.split(' ')

  return rest.length === 0 ? head : `${head} ${rest.join(' ').toLowerCase()}`
}
