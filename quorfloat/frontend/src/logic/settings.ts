// The settings page's pure decisions, kept here so they are testable without a
// DOM: which CSS color-scheme a theme preference means, and what the keep-open
// switch reports.

/**
 * The `color-scheme` value for one theme preference.
 *
 * The design's tokens are written as `light-dark(…)`, which follows the
 * element's `color-scheme` — so one property switches both palettes, and
 * "follow the system" is the two-value form.
 *
 * @param theme - the effective theme preference.
 * @returns the value for `document.documentElement.style.colorScheme`.
 */
export function themeColorScheme(theme: 'system' | 'light' | 'dark'): 'light dark' | 'light' | 'dark' {
  return theme === 'system' ? 'light dark' : theme
}

/**
 * What the keep-open switch reports.
 *
 * The design labels the switch by what it gives the user — "失焦时保持展开" —
 * while the configuration names the behaviour the process implements,
 * `hideOnBlur`. The inversion happens once, here.
 *
 * @param hideOnBlur - the effective setting.
 * @returns the switch's aria-checked value.
 */
export function keepOpenChecked(hideOnBlur: boolean): 'true' | 'false' {
  return hideOnBlur ? 'false' : 'true'
}

/**
 * Whether losing focus should hide the panel.
 *
 * The semantic, stated plainly: the panel hides when the setting that says so
 * is true.
 *
 * @param hideOnBlur - the effective setting.
 * @returns whether a blur should hide the panel.
 */
export function hidesOnBlur(hideOnBlur: boolean): boolean {
  return hideOnBlur
}
