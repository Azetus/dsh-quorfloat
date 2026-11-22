// Category 1 (interface): the display bound never changes the stored payload.

import { describe, expect, test } from 'vitest'
import { previewText } from '../src/logic/text-preview'

describe('structured text preview', () => {
  test('text under the limit is returned whole', () => {
    expect(previewText('{"a":1}', 100)).toEqual({ text: '{"a":1}', total: 7, truncated: false })
  })

  test('text over the limit keeps a prefix and reports the full size', () => {
    const preview = previewText('abcdef', 3)
    expect(preview.text).toBe('abc')
    expect(preview.total).toBe(6)
    expect(preview.truncated).toBe(true)
  })
})
