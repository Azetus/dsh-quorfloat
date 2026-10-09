import { describe, expect, test } from 'vitest'
import { workspaceLabel } from '../src/lib/labels'

describe('workspaceLabel', () => {
  test('a real title is shown as-is', () => {
    expect(workspaceLabel('Quorvox', '/work/quorvox')).toBe('Quorvox')
  })

  test('a blank title falls back to the path tail, not to nothing', () => {
    expect(workspaceLabel('', '/work/quorvox')).toBe('quorvox')
    expect(workspaceLabel('   ', '/work/notes')).toBe('notes')
  })

  test('separators and trailing slashes are handled', () => {
    expect(workspaceLabel('', '/work/quorvox/')).toBe('quorvox')
    expect(workspaceLabel('', 'C:\\Users\\dev\\quorvox')).toBe('quorvox')
  })

  test('a path that is only separators yields the path itself, never empty', () => {
    expect(workspaceLabel('', '/')).toBe('/')
  })
})
