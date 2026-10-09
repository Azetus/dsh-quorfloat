// The picker shell every top-bar and runtime menu shares: a trigger button and the
// popover it opens, with the click-outside rule.

import { useEffect, useRef, type ReactNode } from 'react'

/** Props for {@link PickerPopover}. */
export interface PickerPopoverProps {
  /** Trigger element id, e.g. `q-workspace`. */
  readonly id: string
  /** Popover element id, e.g. `q-workspace-menu`. */
  readonly menuId: string
  /** Accessible label for the trigger. */
  readonly label: string
  /** Accessible label for the popover. */
  readonly menuLabel: string
  /** Extra classes the design puts on this popover, e.g. `q-up`. */
  readonly menuClass?: string
  /** Extra classes for the wrapper, e.g. `q-session-wrap`. */
  readonly wrapClass?: string
  /** Whether this popover is the open one. */
  readonly open: boolean
  /** Whether the trigger is disabled. */
  readonly disabled?: boolean
  /** Toggle request from the trigger. */
  readonly onToggle: () => void
  /** Called when a click lands outside the wrapper. */
  readonly onClose: () => void
  /** The trigger's own content. */
  readonly trigger: ReactNode
  /** The popover's content. */
  readonly children: ReactNode
}

/**
 * Render one picker.
 *
 * The click-outside listener is registered only while the popover is open, and it watches
 * the whole wrapper: a click on the trigger is the trigger's own business (it toggles),
 * and a click inside the menu belongs to the menu.
 *
 * @param props - ids, labels, open state, handlers, and both content slots.
 * @returns the wrapper.
 */
export function PickerPopover({
  id, menuId, label, menuLabel, menuClass = '', wrapClass = '', open, disabled = false,
  onToggle, onClose, trigger, children,
}: PickerPopoverProps) {
  const wrap = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!open) return
    const onDocumentClick = (event: MouseEvent): void => {
      const target = event.target
      if (!(target instanceof Node)) return
      if (wrap.current?.contains(target) === true) return
      onClose()
    }
    document.addEventListener('click', onDocumentClick)
    return () => document.removeEventListener('click', onDocumentClick)
  }, [open, onClose])

  return (
    <div className={wrapClass === '' ? 'q-menu-wrap' : `q-menu-wrap ${wrapClass}`} ref={wrap}>
      <button
        type="button"
        id={id}
        className="q-picker"
        aria-label={label}
        aria-expanded={open}
        aria-controls={menuId}
        disabled={disabled}
        onClick={onToggle}
      >
        {trigger}
      </button>
      <div
        id={menuId}
        className={menuClass === '' ? 'q-popover' : `q-popover ${menuClass}`}
        aria-label={menuLabel}
        hidden={!open}
      >
        {open && children}
      </div>
    </div>
  )
}
