// Markdown, rendered for a panel: the model's text becomes HTML, and the HTML
// is sanitised before it reaches the DOM. The model is not a trusted author —
// this is the one place in the panel that turns text into markup, and it is the
// place that must not turn markup into script.

import DOMPurify from 'dompurify'
import MarkdownIt from 'markdown-it'

const markdown = new MarkdownIt({
  html: false,
  linkify: true,
  breaks: false,
})

/**
 * Render one Markdown string to sanitised HTML.
 *
 * @param text - the model's text.
 * @returns HTML safe to assign to `innerHTML`.
 */
export function renderMarkdown(text: string): string {
  const rendered = markdown.render(text)
  return DOMPurify.sanitize(rendered, { USE_PROFILES: { html: true } })
}
