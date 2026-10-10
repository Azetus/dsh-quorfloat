// The panel's shell: the stage, the window, and the children the app composes into it.
//
// The markup is the design's, element for element — including the ids the stylesheet
// selects on (`#q-main`, `#q-thread-scroll`, `#q-settings`, …): `tokens.css` came from the
// design verbatim, and this migration is about the code, not the design.

import type { ReactNode, RefObject } from 'react'
import { useT } from '../hooks/useI18n'

/** Props for {@link Panel}. */
export interface PanelProps {
  /** The panel element, owned by the app (height and flight both work on it). */
  readonly panelRef: RefObject<HTMLElement | null>
  /** Whether the panel is fading away (the design's `.q-away`). */
  readonly hiding: boolean
  /** The tallest the panel may be right now (product cap and window room). */
  readonly maxHeight: number
  /** The panel's children: top bar, pages, footer. */
  readonly children: ReactNode
}

/**
 * Render the panel shell.
 *
 * @param props - the element ref, the hiding flag, and children.
 * @returns the stage.
 */
export function Panel({ panelRef, hiding, maxHeight, children }: PanelProps) {
  const t = useT()
  return (
    <div className="q-stage">
      <section
        id="q-window"
        ref={panelRef}
        className={hiding ? 'q-window q-away' : 'q-window'}
        style={{ maxHeight }}
        aria-label={t('panel.windowLabel')}
      >
        {children}
      </section>
    </div>
  )
}
