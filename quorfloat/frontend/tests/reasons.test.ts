// Category 1 (interface): structured panel reasons become dictionary text.
//
// The sidecar and host send codes plus values; the panel owns the sentence. An unknown code
// must remain visible (the code itself), not be mistaken for translated copy.

import { describe, expect, test } from 'vitest'
import { resolveText } from '../src/logic/i18n'
import { reasonText } from '../src/logic/reasons'

describe('structured reasons', () => {
  test('a known code uses the panel dictionary and interpolates its values', () => {
    const reason = {
      code: 'delivery.writeFailed',
      params: { detail: 'broken pipe' },
    }
    expect(resolveText('en', reasonText(reason))).toBe('Could not send the message: broken pipe')
    expect(resolveText('zh', reasonText(reason))).toBe('未发送：broken pipe')
  })

  test('a reason without parameters still resolves', () => {
    expect(resolveText('en', reasonText({ code: 'delivery.sending', params: {} })))
      .toBe('Submitted; waiting for Harness confirmation…')
  })

  test('an unknown code stays visible instead of becoming a sentence', () => {
    expect(resolveText('en', reasonText({ code: 'delivery.futureCode', params: {} })))
      .toBe('delivery.futureCode')
    expect(resolveText('zh', reasonText({ code: 'delivery.futureCode', params: {} })))
      .toBe('delivery.futureCode')
  })
})
