// Markdown, rendered for a panel: the model's text becomes HTML, and the HTML
// is sanitised before it reaches the DOM. The model is not a trusted author —
// this is the one place in the panel that turns text into markup, and it is the
// place that must not turn markup into script.

import DOMPurify from 'dompurify'
import MarkdownIt from 'markdown-it'
import { translate, type Language } from './i18n'
import { externalWebUrl, localImagePath } from './links'

const markdown = new MarkdownIt({
  html: false,
  linkify: true,
  breaks: false,
})

// Only emit inert placeholders. React owns image loading and failure handling;
// URLs in data attributes do not fetch resources while Markdown is being parsed.
markdown.renderer.rules.image = (tokens, index, options, env, renderer) => {
  const token = tokens[index]!
  const language: Language = env.language === 'en' ? 'en' : 'zh'
  const escape = markdown.utils.escapeHtml
  const description = renderer.renderInlineAsText(token.children ?? [], options, env)
    || translate(language, 'markdown.image')
  const address = externalWebUrl(token.attrGet('src') ?? '')
  if (address === null) {
    const path = localImagePath(token.attrGet('src') ?? '')
    if (path !== null) return `<span class="q-image-preview" data-image-path="${escape(path)}" data-image-alt="${escape(description)}"></span>`
    return `<span class="q-image-reference">${escape(description)} · ${escape(translate(language, 'markdown.imageUnsupported'))}</span>`
  }
  return `<span class="q-image-preview" data-image-src="${escape(address)}" data-image-alt="${escape(description)}"></span>`
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
