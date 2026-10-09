// A turn's answer body: the model's Markdown, rendered and sanitised.

import { renderMarkdown } from '../lib/markdown'

/** Props for {@link AnswerBody}. */
export interface AnswerBodyProps {
  /** The answer's raw Markdown (also what "复制回答" copies). */
  readonly markdown: string
}

/**
 * Render the answer.
 *
 * `renderMarkdown` is the only place in the panel that turns text into markup, and it
 * sanitises before returning — the model is not a trusted author.
 *
 * @param props - the raw Markdown.
 * @returns the answer block.
 */
export function AnswerBody({ markdown }: AnswerBodyProps) {
  return <div className="q-answer" dangerouslySetInnerHTML={{ __html: renderMarkdown(markdown) }} />
}
