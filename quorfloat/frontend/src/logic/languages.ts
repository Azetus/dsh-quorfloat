// The languages this webview can see, read once and tolerantly.
//
// The Harness's own rule for "no stored preference" is "the browser decides", and this
// panel *is* a webview: `navigator.languages` is that answer, in the user's own order.
// The shell cannot read it — a native process has locale variables, not a browser's
// language list, and on macOS a GUI launch usually has no `LANG` at all — so the page
// reports it once, early, and the shell listens only while nobody has chosen anything.

/** What a report reads from: the live `navigator`, or a stand-in in a test. */
export interface LanguageSource {
  /** The user's ordered language preference list, as the browser reports it. */
  readonly languages?: readonly string[] | undefined
  /** The single preferred language; the documented fallback when there is no list. */
  readonly language?: string | undefined
}

/**
 * The language tags this webview reports, in its own order.
 *
 * Deliberately total. Reading `navigator.languages` is a property access that can be
 * absent (a non-browser stand-in, a webview with the API gated) or refuse to run (a
 * getter that throws), and a report that failed would leave the shell with no answer at
 * all. Every one of those becomes the documented `[navigator.language]` fallback, and
 * only when neither exists is the report empty — which the shell resolves to `en`.
 *
 * Entries are carried verbatim and in order, because the shell records the raw list as
 * the evidence of what this machine actually says. Only non-strings and blank entries
 * are dropped: neither is a language tag, so carrying one would put noise in that record
 * without adding an answer.
 *
 * @param source - where to read from; defaults to the live `navigator`.
 * @returns the reported tags, most preferred first; `[]` when there are none.
 */
export function reportedLanguages(source: LanguageSource | undefined = globalThis.navigator): string[] {
  const list = readUnknown(() => source?.languages)
  if (Array.isArray(list)) {
    const tags = (list as readonly unknown[]).filter(
      (entry): entry is string => typeof entry === 'string' && entry.trim() !== '',
    )
    if (tags.length > 0) return tags
  }
  const single = readUnknown(() => source?.language)
  return typeof single === 'string' && single.trim() !== '' ? [single] : []
}

/**
 * Read one property that is allowed to be absent or to throw.
 *
 * A property access can run arbitrary code — a getter, a proxy, an embedded webview that
 * gates a platform API — and none of that may fail the report, which exists precisely to
 * be available before anything else is.
 *
 * @param get - the access to attempt.
 * @returns what it produced, or `undefined` when it was absent or refused.
 */
function readUnknown(get: () => unknown): unknown {
  try {
    return get()
  } catch {
    return undefined
  }
}
