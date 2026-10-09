// The workspace the panel names: the user's own pick, or the shell's ladder. Verified by
// value here; the field itself is verified by eye (docs/ui).

import { describe, expect, it } from 'vitest'
import { targetWorkspace } from '../src/lib/view-text'
import type { Snapshot } from '../src/lib/state'

/** A snapshot whose only interesting field is where a new conversation would go. */
const snapshot = (createWorkspace: string | null): Snapshot =>
  ({ session: { createWorkspace } }) as unknown as Snapshot

describe('targetWorkspace', () => {
  it('uses the shell’s answer when the user has picked nothing', () => {
    expect(targetWorkspace(snapshot('ws-project'), null)).toBe('ws-project')
  })

  it('prefers the workspace the user picked for this visit', () => {
    expect(targetWorkspace(snapshot('ws-project'), 'ws-notes')).toBe('ws-notes')
  })

  it('is null when neither knows, which is what asks the user', () => {
    expect(targetWorkspace(snapshot(null), null)).toBeNull()
  })
})
