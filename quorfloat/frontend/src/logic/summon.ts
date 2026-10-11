// Telling a summon from a panel reload.
//
// The workspace the user chose belongs to **this visit**: a summon clears it, so the next visit
// starts from the ladder (the workspace pin, then the most recent workspace, then the host's
// default) rather than from a pick the user made half an hour ago.
//
// A bare counter cannot carry that. A webview reload or a sidecar restart starts the snapshot
// stream over with `count: 0`, and a viewer that watched only the number would read the reset as
// a summon and clear a choice the user is still making. So the count travels beside a **token**
// that identifies the process lifetime that produced it:
//
// - a *different* token is a different life: record it and clear nothing;
// - the same token with a *higher* count is one summon: clear the choice;
// - the same token with the same (or a lower) count is a re-render, not a visit.
//
// Pure, so the three cases can be exercised without a DOM or a running shell.

/** What the frontend remembers about the summon counter between snapshots. */
export interface SummonMark {
  /** The lifetime the count belongs to, once one has been seen. */
  readonly token: string
  /** The highest count seen within that lifetime. */
  readonly count: number
}

/** The snapshot field this rule reads. Structural, so it can be tested without a whole state. */
export interface SummonField {
  readonly token: string
  readonly count: number
}

/** One observation: what to remember, and whether it was a summon. */
export interface SummonObservation {
  /** The mark to keep for the next snapshot. */
  readonly mark: SummonMark
  /** Whether the user summoned the panel — the one case that clears the visit's choice. */
  readonly summoned: boolean
}

/**
 * Record one snapshot's summon field and decide whether it was a summon.
 *
 * @param mark - what the previous snapshot left behind, or `null` before the first one.
 * @param field - the snapshot's `window.summon`.
 * @returns the new mark and whether this observation is a summon.
 */
export function observeSummon(mark: SummonMark | null, field: SummonField): SummonObservation {
  const count = Number.isSafeInteger(field.count) && field.count >= 0 ? field.count : 0
  // The first snapshot of a life records the baseline without clearing anything: the count that
  // is already there belongs to a summon that happened before this viewer existed, and the user's
  // choice (if any) was made after it.
  if (mark === null) return { mark: { token: field.token, count }, summoned: false }
  if (mark.token !== field.token) {
    // A new lifetime — a reloaded panel, or a restarted process. Its count says nothing about
    // the choice made in the old one.
    return { mark: { token: field.token, count }, summoned: false }
  }
  if (count > mark.count) return { mark: { token: field.token, count }, summoned: true }
  // Same life, no rise: a re-render. The mark keeps the highest count seen, so a snapshot that
  // arrives out of order cannot make the next one look like a summon.
  return { mark, summoned: false }
}
