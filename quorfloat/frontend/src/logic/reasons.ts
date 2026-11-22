// Structured reasons from the sidecar or the host, rendered by the panel's dictionary.
//
// The wire carries a code and its template values, never a finished sentence. An unknown code
// is a protocol bug; the dictionary lookup falls back to the code itself, which is visible
// during development instead of being mistaken for a translated sentence.

import { msg, type MessageKey, type Text } from './i18n'
import type { Reason } from './state'

/**
 * Turn one structured reason into dictionary text.
 *
 * @param reason - the wire reason.
 * @returns the dictionary key and parameters.
 */
export function reasonText(reason: Reason): Text {
  return msg(reason.code as MessageKey, reason.params)
}
