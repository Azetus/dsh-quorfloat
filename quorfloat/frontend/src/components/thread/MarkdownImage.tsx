import { useState } from 'react'
import { useT } from '../../hooks/useI18n'

/** An inert Markdown placeholder becomes an image only after the user requests it. */
export function MarkdownImage({ src, alt, requested }: {
  readonly src: string
  readonly alt: string
  /** Consent lives with the answer so streaming updates do not discard it. */
  readonly requested: Set<string>
}) {
  const t = useT()
  const [status, setStatus] = useState<'idle' | 'loading' | 'loaded' | 'failed'>(() =>
    requested.has(src) ? 'loading' : 'idle')
  const load = () => {
    requested.add(src)
    setStatus('loading')
  }

  return <>
    {(status === 'idle' || status === 'failed') && <span className="q-image-reference">
      {alt}{' · '}
      {status === 'failed' && <>{t('markdown.imageFailed')}{' · '}</>}
      <button type="button" className="q-image-load" onClick={load}>
        {t(status === 'failed' ? 'markdown.retryImage' : 'markdown.loadImage')}
      </button>
    </span>}
    {status === 'loading' && <span className="q-image-reference" role="status">
      {t('markdown.imageLoading')}
    </span>}
    {(status === 'loading' || status === 'loaded') && <img
      src={src}
      alt={alt}
      decoding="async"
      referrerPolicy="no-referrer"
      onLoad={() => { setStatus('loaded') }}
      onError={() => { setStatus('failed') }}
    />}
  </>
}
