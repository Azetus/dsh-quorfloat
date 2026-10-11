// The statistics footer, built from the documented rules. Every rule here
// exists because a number formatted wrong lies: "0 tok/s" reads like a stall,
// "100%" cache hit can be a lie, and a dash reads like a zero.

import { msg, type Text } from './i18n'
import type { Stats } from './state'

/** Output tokens per second: ≥10 rounds to an integer, <10 keeps one decimal. */
export function speedLabel(tokensPerSecond: number | null): string {
  if (tokensPerSecond === null) return '—'
  return tokensPerSecond >= 10 ? `${Math.round(tokensPerSecond)}` : tokensPerSecond.toFixed(1)
}

/**
 * Cumulative tokens: `<1K` as-is, `12.2K`, `1.2M`.
 *
 * @param value - the total, or null when the host has not reported it.
 * @returns the label, or `'—'` — a dash is "no data", never "zero".
 */
export function tokenLabel(value: number | null): string {
  if (value === null) return '—'
  if (value < 1_000) return `${value}`
  if (value < 1_000_000) return `${(value / 1_000).toFixed(1)}K`
  return `${(value / 1_000_000).toFixed(1)}M`
}

/**
 * Cache-hit share of the prompt, as a percentage.
 *
 * **A partial hit must never round up to a full 100** — telling somebody their
 * whole prompt was cached when a sliver was not is the one error this figure
 * must not make. An exact 100 (or more) is fine: it really was fully cached.
 *
 * @returns `'—'` when the host reported nothing, else `'99.9%'`-style.
 */
export function cacheLabel(cacheHitPercent: number | null): string {
  if (cacheHitPercent === null) return '—'
  if (cacheHitPercent >= 100) return '100%'
  const rounded = Math.round(cacheHitPercent * 10) / 10
  return `${Math.min(rounded, 99.9).toFixed(1)}%`
}

/**
 * Context usage, clamped to 100 — over the window is a real state, but "103% of
 * a ring" is not a picture.
 */
export function contextPercent(contextTokens: number | null, contextLimit: number | null): number | null {
  if (contextTokens === null || contextLimit === null || contextLimit <= 0) return null
  const percent = (contextTokens / contextLimit) * 100
  return Math.min(100, Math.round(percent * 10) / 10)
}

/**
 * The rounds group as data: the counts, and the speed already formatted.
 *
 * The phrasing (`3 轮 9 步 · 229 tok/s`) is the dictionary's; this returns the key and the
 * three figures, so the numbers keep their own tested rules (`speedLabel`) and the words
 * do not live here.
 */
export function roundsText(stats: Stats): Text {
  return msg('stats.rounds', {
    turns: stats.turns,
    steps: stats.steps,
    speed: speedLabel(stats.tokensPerSecond),
  })
}

/** The tokens group: `85.5K tok · 缓存命中 90%`. */
export function tokensLabel(stats: Stats): { count: string; cache: string } {
  return { count: tokenLabel(stats.totalTokens), cache: cacheLabel(stats.cacheHitPercent) }
}

/** The context group's percentage, or null when it cannot be computed. */
export function contextLabel(stats: Stats): string | null {
  const percent = contextPercent(stats.contextTokens, stats.contextLimit)
  return percent === null ? null : `${percent}%`
}
