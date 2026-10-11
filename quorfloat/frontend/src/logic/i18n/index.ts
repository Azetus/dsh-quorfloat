// The panel's dictionary lookup — pure, and deliberately tiny.
//
// Shape (the same one the Harness's own locale service uses): a flat key per string, one
// `{ zh, en }` pair, and `t(key, params)` resolving `{placeholder}`s. The key type is
// derived from the dictionaries themselves, not written by hand, so a component that
// invents a key fails `tsc` and neither language can silently miss one (see `en.ts` /
// `zh.ts`).
//
// What lives here is only the *lookup*; the React binding is `hooks/useI18n.ts`, so this
// module stays testable without a DOM.

import { en, type MessageKey } from './en'
import { zh } from './zh'

export type { MessageKey }

/** The two languages the wire field can carry, the same ids the Harness uses. */
export type Language = 'zh' | 'en'

/** The values a `{placeholder}` may be filled with. */
export type Params = Readonly<Record<string, string | number>>

/**
 * A piece of UI text: host data to show verbatim, or a dictionary key plus its data.
 *
 * The pure helpers (`logic/view-text.ts`, `logic/stats.ts`, `logic/hotkey.ts`) return this
 * instead of finished sentences, so the wording lives in the dictionaries and no Chinese
 * string is hard-coded outside `zh.ts`.
 */
export type Text =
  | { readonly kind: 'raw'; readonly text: string }
  | { readonly kind: 'key'; readonly key: MessageKey; readonly params?: Params }

/** Host data, shown as it came. */
export function raw(text: string): Text {
  return { kind: 'raw', text }
}

/** A dictionary key, with the data its placeholders need. */
export function msg(key: MessageKey, params?: Params): Text {
  return { kind: 'key', key, params }
}

/** Both dictionaries, keyed by language. */
export const dictionaries: Readonly<Record<Language, Readonly<Record<MessageKey, string>>>> = {
  en,
  zh,
}

/**
 * The language a `settings.language` value means.
 *
 * Read defensively on purpose: the field is newer than this build, so an absent value (a
 * shell that predates it), a `null`, or a value this build does not know all mean `'zh'` —
 * the panel's default, and the language its copy is written in. No "follow the
 * system" logic: the snapshot's own answer is the answer.
 *
 * @param value - the raw wire value.
 * @returns `'en'` only for exactly `'en'`, otherwise `'zh'`.
 */
export function languageOf(value: unknown): Language {
  return value === 'en' ? 'en' : 'zh'
}

/**
 * Resolve one key for one language.
 *
 * **The fallback rule.** A key that is not in the active language's table falls back to
 * the Chinese string, and one that is in neither renders as the key itself. Nothing ever
 * renders blank: `translate` returns a non-empty string for every input, including keys
 * that only exist at runtime (a dictionary from a newer build, or a lookup through a
 * cast). Chinese is the fallback because it is this panel's source language and the one
 * every shipped string was written in.
 *
 * A `{placeholder}` with no matching param is left visible, for the same reason: a
 * literal `{speed}` tells a reader something is missing, an empty string does not.
 *
 * @param language - the active language.
 * @param key - a dictionary key.
 * @param params - values for the key's `{placeholders}`.
 * @returns the string to show.
 */
export function translate(language: Language, key: MessageKey, params?: Params): string {
  // `'en'` is the only value that selects English; every other value — including one a
  // cast smuggled past the type — reads Chinese, matching `languageOf`.
  const table = language === 'en' ? en : zh
  // The middle term is the documented fallback; types say it cannot be needed, runtime is
  // where "a newer dictionary than this build" lives.
  const template = table[key] ?? zh[key] ?? key
  return interpolate(template, params)
}

/**
 * Resolve a {@link Text}: raw data passes through, a key is looked up.
 *
 * @param language - the active language.
 * @param text - the text token.
 * @returns the string to show.
 */
export function resolveText(language: Language, text: Text): string {
  return text.kind === 'raw' ? text.text : translate(language, text.key, text.params)
}

/** Fill `{name}` placeholders; unknown ones are left as written. */
function interpolate(template: string, params?: Params): string {
  if (params === undefined) return template
  return template.replace(/\{(\w+)\}/g, (whole, name: string) => {
    const value = params[name]
    return value === undefined ? whole : String(value)
  })
}
