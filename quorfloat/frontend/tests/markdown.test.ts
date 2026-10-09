import { describe, expect, test } from 'vitest'
import { renderMarkdown } from '../src/lib/markdown'

describe('renderMarkdown', () => {
  test('renders the model text to HTML', () => {
    const html = renderMarkdown('**加粗** 与 `行内代码`\n\n- 一项\n- 两项')
    expect(html).toContain('<strong>加粗</strong>')
    expect(html).toContain('<code>行内代码</code>')
    expect(html).toContain('<li>一项</li>')
  })

  test('a table survives as a table', () => {
    const html = renderMarkdown('| a | b |\n| - | - |\n| 1 | 2 |')
    expect(html).toContain('<table>')
    expect(html).toContain('<td>1</td>')
  })

  test('script from the model never reaches the DOM', () => {
    const html = renderMarkdown('<script>alert(1)</script>\n\n[点我](javascript:alert(1))')
    expect(html).not.toContain('<script')
    // The javascript: URL must not become a link destination; appearing as
    // escaped *text* is harmless — the assertion checks href, not the words.
    expect(html).not.toMatch(/href=["']?javascript:/i)
  })
})
