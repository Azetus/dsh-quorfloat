import { useEffect, useState } from 'react'
import { useT } from '../../hooks/useI18n'
import { api } from '../../api'

/** Display a supported Markdown image automatically; failed loads remain retryable. */
export function MarkdownImage({ src, alt, local, sessionId }: {
  readonly src: string
  readonly alt: string
  readonly local: boolean
  readonly sessionId: string | null
}) {
  const t = useT()
  const [status, setStatus] = useState<'loading' | 'loaded' | 'failed'>('loading')
  const [attempt, setAttempt] = useState(0)
  const [imageUrl, setImageUrl] = useState<string | null>(null)
  const [failure, setFailure] = useState('readFailed')
  useEffect(() => {
    if (!local) return
    if (sessionId === null) {
      setFailure('session')
      setStatus('failed')
      return
    }
    const controller = new AbortController()
    let objectUrl: string | null = null
    void api.loadLocalImage(sessionId, src, controller.signal).then(url => {
      if (controller.signal.aborted) { URL.revokeObjectURL(url); return }
      objectUrl = url
      setImageUrl(url)
    }).catch((error: unknown) => {
      if (controller.signal.aborted) return
      const reason = (error as { reason?: unknown } | null)?.reason
      setFailure(typeof reason === 'string' ? reason : 'readFailed')
      setStatus('failed')
    })
    return () => {
      controller.abort()
      if (objectUrl !== null) URL.revokeObjectURL(objectUrl)
    }
  }, [local, sessionId, src, attempt])
  const retry = () => {
    setImageUrl(null)
    setStatus('loading')
    setAttempt(value => value + 1)
  }
  const failureKey = failure === 'missing' ? 'markdown.imageMissing'
    : failure === 'tooLarge' ? 'markdown.imageTooLarge'
    : failure === 'format' ? 'markdown.imageFormat'
    : failure === 'cwd' ? 'markdown.imageNoCwd'
    : failure === 'session' ? 'markdown.imageSessionGone'
    : failure === 'busy' ? 'markdown.imageBusy'
    : failure === 'unavailable' ? 'markdown.imageUnavailable'
    : 'markdown.imageFailed'
  const displayUrl = local ? imageUrl : src

  return <>
    {status === 'failed' && <span className="q-image-reference" title={local ? src : undefined}>
      {alt}{' · '}
      {t(failureKey)}{' · '}
      <button type="button" className="q-image-load" onClick={retry}>
        {t('markdown.retryImage')}
      </button>
    </span>}
    {status === 'loading' && <span className="q-image-reference" role="status">
      {t('markdown.imageLoading')}
    </span>}
    {(status === 'loading' || status === 'loaded') && displayUrl !== null && <img
      src={displayUrl}
      alt={alt}
      loading="lazy"
      decoding="async"
      referrerPolicy="no-referrer"
      onLoad={() => { setStatus('loaded') }}
      onError={() => { setFailure('readFailed'); setStatus('failed') }}
    />}
  </>
}
