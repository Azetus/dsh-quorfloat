// Category 1 (interface): what `logic/turns.ts` hands the panel as a turn's identity.
//
// The key is not a display detail: it is the map key for the open/closed fold memory
// (`App.tsx`'s `folds`) and the virtualizer's row key (`Thread.tsx`'s `getItemKey`). A key that
// moves when an older page is prepended forgets the whole thread's fold state and makes every
// row re-measure — which is exactly what a position-based key did.

import { describe, expect, test } from 'vitest'
import type { Entry } from '../src/logic/state'
import { foldTurns } from '../src/logic/turns'

/** A user line, which is what starts a turn. */
const question = (text: string): Entry => ({ kind: 'user', text })

/** The keys of the turns one set of entries folds into, oldest first. */
const keysOf = (entries: Entry[]): string[] => foldTurns(entries, null, false).turns.map(turn => turn.id)

describe('the turn fold key', () => {
  test('does not move when an older page is prepended above the thread', () => {
    const before = keysOf([question('第二个问题')])
    // The prepend: an older page arrives above everything already on screen.
    expect(keysOf([question('第一个问题'), question('第二个问题')]).slice(1)).toEqual(before)
  })

  test('keeps two turns that ask the same question apart', () => {
    const keys = keysOf([question('同一个问题'), question('同一个问题')])
    expect(keys).toHaveLength(2)
    expect(new Set(keys).size).toBe(2)
  })

  test('a prepended duplicate of a question does not renumber the newer turn', () => {
    const before = keysOf([question('同一个问题')])
    expect(keysOf([question('同一个问题'), question('同一个问题')]).slice(1)).toEqual(before)
  })
})
