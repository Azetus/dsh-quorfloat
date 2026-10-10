/**
 * The panel's language: which one it is set to, and where the first value comes from.
 *
 * Exactly two ids are supported and they are the Harness's own — `zh` and `en` — so
 * the panel's setting and the locale a Harness user already chose are spelled the
 * same way and never have to be translated between two vocabularies.
 *
 * The setting is a *panel* setting (see `config.ts`), and it starts empty on a fresh
 * install. Empty is not a language: it means "nobody has chosen one here yet", and
 * the first launch answers it from the Harness's own locale preference
 * (`packages/client/locale/src/locale-settings.ts` in the Harness checkout: namespace
 * `locale`, field `preference`, stored in the Host user-settings document). The
 * sidecar persists that answer as the panel's own setting, so every later launch
 * reads an explicit value and never consults the Harness again — there is no
 * "follow" mode and no live following.
 *
 * Reading is deliberately total and best-effort. The locale preference is another
 * plugin's business, its service may not be composed at all, and its value may name
 * a language this build does not ship. Every one of those cases resolves to `en`
 * rather than failing activation or blocking the first frame: a panel that opens in
 * English is useful, and a panel that never opens is not.
 */

/** The two languages the panel ships; the same ids the Harness uses. */
export type Language = 'zh' | 'en'

/** Every id the panel supports, in the Harness's own spelling. */
export const SUPPORTED_LANGUAGES: readonly Language[] = ['zh', 'en']

/** The language a value that is not one of ours resolves to. */
export const FALLBACK_LANGUAGE: Language = 'en'

/**
 * Settings namespace the Harness's locale plugin owns.
 *
 * Confirmed in the Harness checkout at
 * `packages/client/locale/src/locale-settings.ts`: `LOCALE_SETTINGS_NAMESPACE`.
 */
export const HARNESS_LOCALE_NAMESPACE = 'locale'

/**
 * Field carrying the explicit locale selection inside that namespace.
 *
 * Confirmed in the same file: `LOCALE_PREFERENCE_FIELD`. Its absence means "delegate
 * to the browser", which is exactly the "nobody has chosen" state this module reads.
 */
export const HARNESS_LOCALE_PREFERENCE_FIELD = 'preference'

/** Read one optional service from the plugin context, exactly as `Context.get` does. */
export type ServiceLookup = (name: string) => unknown

/**
 * Whether a value names a language this build supports.
 *
 * Case-sensitive on purpose: the ids cross a wire and a settings file, and accepting
 * `ZH` here would create a second spelling that the frontend's `'zh' | 'en'` type
 * could never receive.
 *
 * @param value - candidate id.
 * @returns `true` when it is exactly `zh` or `en`.
 */
export function isLanguage(value: unknown): value is Language {
  return typeof value === 'string' && (SUPPORTED_LANGUAGES as readonly string[]).includes(value)
}

/**
 * Resolve the language a launch should use.
 *
 * The order is the documented precedence of this plugin: an explicit panel setting is
 * the user's decision and wins; only when there is none does the Harness's own locale
 * preference seed it; everything else — unsupported, unreadable, absent — is `en`.
 *
 * Pure, and total over `unknown` on purpose: it is the one decision that has to be
 * asserted without a runtime, because its failure mode (a panel stuck in the wrong
 * language) is only visible to the person reading it.
 *
 * @param configured - the panel's own setting (`config.window.language`), possibly empty.
 * @param harnessPreference - the Harness's locale preference, when it could be read.
 * @returns the language to use.
 */
export function resolvePanelLanguage(configured: unknown, harnessPreference: unknown): Language {
  if (isLanguage(configured)) return configured
  if (isLanguage(harnessPreference)) return harnessPreference
  return FALLBACK_LANGUAGE
}

/**
 * Read the Harness's locale preference through whichever host service provides it.
 *
 * Two readers, because the composition is not guaranteed:
 *
 * 1. the `settings` service (`SettingsForms.describe()`) is the canonical projection —
 *    its `value` is the effective locale the browser half follows, with the profile's
 *    own layers already applied;
 * 2. the `configEditor` service (`configuration()`) reads the persisted user-settings
 *    document directly, which still answers when the locale plugin's fiber is not
 *    active yet.
 *
 * Both are read defensively: a service that is absent, a method that throws, or a
 * shape this build does not recognise all mean "no preference", never an exception.
 *
 * @param get - service lookup, normally `name => ctx.get(name)`.
 * @returns the raw preference string, or `undefined` when it could not be read.
 */
export function readHarnessLocalePreference(get: ServiceLookup): string | undefined {
  return preferenceFromSettings(get('settings')) ?? preferenceFromConfigEditor(get('configEditor'))
}

/**
 * Read the preference from the effective `settings` projection.
 *
 * @param service - the `settings` service, when present.
 * @returns the raw preference, or `undefined`.
 */
function preferenceFromSettings(service: unknown): string | undefined {
  try {
    const describe = (service as { describe?: unknown } | undefined)?.describe
    if (typeof describe !== 'function') return undefined
    const rows = (describe as () => unknown).call(service)
    if (!Array.isArray(rows)) return undefined
    for (const row of rows) {
      if (typeof row !== 'object' || row === null) continue
      if ((row as { ns?: unknown }).ns !== HARNESS_LOCALE_NAMESPACE) continue
      // `value` is the effective projection; `user` is the profile's own override. The
      // effective one is what the browser half follows, so it is the honest reading.
      return preferenceOf((row as { value?: unknown }).value) ?? preferenceOf((row as { user?: unknown }).user)
    }
    return undefined
  } catch {
    // Another plugin's service must not be able to fail this plugin's activation.
    return undefined
  }
}

/**
 * Read the preference from the persisted user-settings document.
 *
 * @param service - the `configEditor` service, when present.
 * @returns the raw preference, or `undefined`.
 */
function preferenceFromConfigEditor(service: unknown): string | undefined {
  try {
    const configuration = (service as { configuration?: unknown } | undefined)?.configuration
    if (typeof configuration !== 'function') return undefined
    const rows = (configuration as () => unknown).call(service)
    if (!Array.isArray(rows)) return undefined
    for (const row of rows) {
      if (typeof row !== 'object' || row === null) continue
      const entry = (row as { entry?: unknown }).entry
      const id = (entry as { options?: { id?: unknown } } | undefined)?.options?.id
      if (id !== HARNESS_LOCALE_NAMESPACE) continue
      // The profile patch's own config is the user-settings document's value; the
      // inherited layer is the bundle's shipped row. The override wins, as everywhere.
      const override = preferenceOf((row as { override?: unknown }).override)
      const inherited = preferenceOf((row as { inherited?: unknown }).inherited)
      return override ?? inherited
    }
    return undefined
  } catch {
    return undefined
  }
}

/**
 * Pull the preference field out of one settings section.
 *
 * @param section - an object that may carry `preference`.
 * @returns the raw string, or `undefined`.
 */
function preferenceOf(section: unknown): string | undefined {
  if (typeof section !== 'object' || section === null) return undefined
  const value = (section as Record<string, unknown>)[HARNESS_LOCALE_PREFERENCE_FIELD]
  if (typeof value !== 'string') return undefined
  const trimmed = value.trim()
  return trimmed === '' ? undefined : trimmed
}
