// The browsing scope of the conversation list, remembered across the frames in which the
// workspace field cannot answer.
//
// The scope is not a second selection: it is the workspace field's own value, derived by
// `logic/scope.ts`'s `fieldScope` — the same call the field's label is drawn from. But the field
// can be momentarily unresolved: at startup, on the frame a summon lands, or while a
// just-created conversation is not yet in the visible list. A list that fell back to "every
// conversation" for those frames would flicker between two answers, so this hook keeps the last
// *resolved* scope and hands it out until the field answers again.
//
// A freshly resolved scope always wins, so the list re-derives on the frame the field does —
// including right after a summon, whose snapshot clears the visit's workspace choice (the choice
// clearing is the re-derivation, not a reset).

import { useState } from 'react'
import { fieldScope, sameScope, type Scope } from '../logic/scope'
import type { Snapshot } from '../logic/state'

/**
 * The scope the workspace field is showing, sticky while the field is unresolved.
 *
 * @param state - the snapshot.
 * @param target - the resolved workspace id for a new conversation (the user's pick, or the shell's).
 * @returns the scope to list conversations under, or null when the field has never resolved
 *   (the list then shows every conversation, which is what "no scope chosen yet" means).
 */
export function useSessionScope(state: Snapshot, target: string | null): Scope | null {
  const [remembered, setRemembered] = useState<Scope | null>(null)
  const derived = fieldScope(state, target)
  // Adjusting state during render — React's own pattern for a value derived from props: the
  // re-render happens before any child sees the stale scope, so once the field answers, the
  // list never paints a frame of the previous scope. `sameScope` is the loop guard.
  if (derived !== null && !sameScope(derived, remembered)) setRemembered(derived)
  return derived ?? remembered
}
