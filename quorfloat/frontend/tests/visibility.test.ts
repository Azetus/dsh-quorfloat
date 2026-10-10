import { describe, expect, test } from 'vitest'
import { hotkeyAction, stepVisibility, type VisibilityState } from '../src/lib/visibility'

const shown: VisibilityState = { visible: true, hiding: false, hideSent: false }
const hidden: VisibilityState = { visible: false, hiding: false, hideSent: false }

describe('stepVisibility', () => {
  test('a hide request starts one hide', () => {
    expect(stepVisibility(shown, 'request-hide')).toEqual({ visible: true, hiding: true, hideSent: false })
  })

  test('a hide request while one is in flight is ignored', () => {
    const hiding = stepVisibility(shown, 'request-hide')
    expect(stepVisibility(hiding, 'request-hide')).toEqual(hiding)
  })

  test('a hide request while hidden is ignored', () => {
    // A programmatic hide blurs the webview too, and the blur handler must not
    // start a second fade-out.
    expect(stepVisibility(hidden, 'request-hide')).toEqual(hidden)
  })

  test('hiding holds until the shell acknowledges the hide', () => {
    const hiding = stepVisibility(shown, 'request-hide')
    const sent = stepVisibility(hiding, 'hide-sent')
    expect(sent.hiding).toBe(true)
    expect(sent.hideSent).toBe(true)
    const acknowledged = stepVisibility(sent, 'hidden-ack')
    expect(acknowledged).toEqual({ visible: false, hiding: false, hideSent: false })
  })

  test('hide-sent before a hide is in flight changes nothing', () => {
    expect(stepVisibility(shown, 'hide-sent')).toEqual(shown)
  })

  test('a show clears an in-flight hide, sent or not', () => {
    const fading = stepVisibility(shown, 'request-hide')
    expect(stepVisibility(fading, 'show')).toEqual({ visible: true, hiding: false, hideSent: false })
    const sent = stepVisibility(fading, 'hide-sent')
    expect(stepVisibility(sent, 'show')).toEqual({ visible: true, hiding: false, hideSent: false })
  })
})

describe('hotkeyAction', () => {
  test('idle means hide', () => {
    expect(hotkeyAction({ hiding: false, hideSent: false })).toBe('hide')
  })

  test('during the fade the hotkey cancels it', () => {
    expect(hotkeyAction({ hiding: true, hideSent: false })).toBe('cancel-fade')
  })

  test('after the request was sent the hotkey recovers with a show', () => {
    expect(hotkeyAction({ hiding: true, hideSent: true })).toBe('recover-show')
  })
})
