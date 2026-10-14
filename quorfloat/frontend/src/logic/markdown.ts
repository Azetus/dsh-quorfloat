// Markdown, rendered for a panel: the model's text becomes HTML, and the HTML
// is sanitised before it reaches the DOM. The model is not a trusted author —
// this is the one place in the panel that turns text into markup, and it is the
// place that must not turn markup into script.

import DOMPurify from 'dompurify'
import MarkdownIt from 'markdown-it'
import { translate, type Language } from './i18n'
import { externalWebUrl } from './links'

const markdown = new MarkdownIt({
  html: false,
  linkify: true,
  breaks: false,
})

// Images describe a resource; rendering a conversation must not fetch that resource.
markdown.renderer.rules.image = (tokens, index, options, env, renderer) => {
  const token = tokens[index]!
  const language: Language = env.language === 'en' ? 'en' : 'zh'
  const escape = markdown.utils.escapeHtml
  const description = renderer.renderInlineAsText(token.children ?? [], options, env)
    || translate(language, 'markdown.image')
  const address = externalWebUrl(token.attrGet('src') ?? '')
  // A linked image already has an enclosing anchor; never emit nested anchors.
  const insideLink = tokens.slice(0, index).reduce((depth, entry) =>
    depth + (entry.type === 'link_open' ? 1 : entry.type === 'link_close' ? -1 : 0), 0) > 0
  const action = address === null
    ? escape(translate(language, 'markdown.imageUnsupported'))
    : insideLink ? '' : `<a href="${escape(address)}" rel="noopener noreferrer">${escape(translate(language, 'markdown.viewImage'))}</a>`
  return `<span class="q-image-reference">${escape(description)}${action === '' ? '' : ` · ${action}`}</span>`
}

/**
 * Render one Markdown string to sanitised HTML.
 *
 * @param text - the model's text.
 * @returns HTML safe to assign to `innerHTML`.
 */
export function renderMarkdown(text: string, language: Language = 'zh'): string {
  const rendered = markdown.render(text, { language })
  return DOMPurify.sanitize(rendered, { USE_PROFILES: { html: true }, FORBID_TAGS: ['img'] })
}
