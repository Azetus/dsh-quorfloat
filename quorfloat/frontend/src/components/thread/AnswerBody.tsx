// A turn's answer body: the model's Markdown, rendered and sanitised.

import { renderMarkdown } from '../../logic/markdown'
import { externalWebUrl } from '../../logic/links'
import { api } from '../../api'
import { useLanguage, useT } from '../../hooks/useI18n'
import { useEffect, useState, type MouseEvent } from 'react'

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
  const language = useLanguage()
  const t = useT()
  const [note, setNote] = useState<string | null>(null)
  useEffect(() => { setNote(null) }, [markdown, language])

  const openLink = (event: MouseEvent<HTMLDivElement>) => {
    if (event.type === 'auxclick' && event.button !== 1) return
    const anchor = event.target instanceof Element ? event.target.closest('a') : null
    if (anchor === null || !event.currentTarget.contains(anchor)) return
    const address = anchor.getAttribute('href')
    if (address?.startsWith('#') === true && event.button === 0) return
    event.preventDefault()
    const url = address === null ? null : externalWebUrl(address)
    if (url === null) {
      setNote(t('markdown.linkUnsupported'))
      return
    }
    setNote(null)
    void api.openExternalUrl(url).catch(() => { setNote(t('markdown.openFailed')) })
  }

  return <>
    <div
      className="q-answer"
      onClick={openLink}
      onAuxClick={openLink}
      dangerouslySetInnerHTML={{ __html: renderMarkdown(markdown, language) }}
    />
    {note !== null && <p className="q-markdown-note" role="status">{note}</p>}
  </>
}
