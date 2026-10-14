// A turn's answer body: the model's Markdown, rendered and sanitised.

import { renderMarkdown } from '../../logic/markdown'
import { externalWebUrl } from '../../logic/links'
import { api } from '../../api'
import { useLanguage, useT } from '../../hooks/useI18n'
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type MouseEvent } from 'react'
import { createPortal } from 'react-dom'
import { MarkdownImage } from './MarkdownImage'

/** Props for {@link AnswerBody}. */
export interface AnswerBodyProps {
  /** The answer's raw Markdown (also what "复制回答" copies). */
  readonly markdown: string
  readonly sessionId: string | null
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
export function AnswerBody({ markdown, sessionId }: AnswerBodyProps) {
  const language = useLanguage()
  const t = useT()
  const [note, setNote] = useState<string | null>(null)
  const body = useRef<HTMLDivElement>(null)
  const html = useMemo(() => renderMarkdown(markdown, language), [markdown, language])
  const [images, setImages] = useState<readonly { target: HTMLElement; src: string; alt: string; local: boolean }[]>([])
  useEffect(() => { setNote(null) }, [markdown, language])
  useLayoutEffect(() => {
    const targets = body.current?.querySelectorAll<HTMLElement>('[data-image-src], [data-image-path]') ?? []
    setImages(Array.from(targets).flatMap(target => {
      const local = target.dataset.imagePath !== undefined
      // Local attributes already carry a decoded path, including literal %/#/?
      // in filenames; do not decode them a second time.
      const src = local ? target.dataset.imagePath! : externalWebUrl(target.dataset.imageSrc ?? '')
      return src === null ? [] : [{ target, src, alt: target.dataset.imageAlt ?? '', local }]
    }))
  }, [html])

  const openLink = (event: MouseEvent<HTMLDivElement>) => {
    if (event.type === 'auxclick' && event.button !== 1) return
    // A Markdown image may be wrapped in an authored link. Neither loading it
    // nor clicking the displayed image should activate that enclosing anchor.
    if (event.target instanceof Element && event.target.closest('[data-image-src], [data-image-path]') !== null) {
      event.preventDefault()
      return
    }
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
      ref={body}
      onClick={openLink}
      onAuxClick={openLink}
      dangerouslySetInnerHTML={{ __html: html }}
    />
    {images.map(({ target, src, alt, local }, index) => createPortal(
      <MarkdownImage src={src} alt={alt} local={local} sessionId={sessionId} />,
      target,
      `${sessionId}:${index}:${local}:${src}`,
    ))}
    {note !== null && <p className="q-markdown-note" role="status">{note}</p>}
  </>
}
