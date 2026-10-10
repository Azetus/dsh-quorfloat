// The picker shell every top-bar and runtime menu shares: a trigger button and the
// popover it opens, with the click-outside and Escape rules.

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
 * Both ways out are registered only while the popover is open: a click outside the wrapper
 * closes it (a click on the trigger is the trigger's own business — it toggles — and a
 * click inside the menu belongs to the menu), and Escape closes it and hands focus back to
 * the trigger, so the keyboard leaves a popover where the mouse would.
 *
 * @param props - ids, labels, open state, handlers, and both content slots.
 * @returns the wrapper.
 */
export function PickerPopover({
  id, menuId, label, menuLabel, menuClass = '', wrapClass = '', open, disabled = false,
  onToggle, onClose, trigger, children,
}: PickerPopoverProps) {
  const wrap = useRef<HTMLDivElement>(null)
  const triggerRef = useRef<HTMLButtonElement>(null)

  useEffect(() => {
    if (!open) return
    const onDocumentClick = (event: MouseEvent): void => {
      const target = event.target
      if (!(target instanceof Node)) return
      if (wrap.current?.contains(target) === true) return
      onClose()
    }
    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.key !== 'Escape') return
      triggerRef.current?.focus()
      onClose()
    }
    document.addEventListener('click', onDocumentClick)
    document.addEventListener('keydown', onKeyDown)
    return () => {
      document.removeEventListener('click', onDocumentClick)
      document.removeEventListener('keydown', onKeyDown)
    }
  }, [open, onClose])

  return (
    <div className={wrapClass === '' ? 'q-menu-wrap' : `q-menu-wrap ${wrapClass}`} ref={wrap}>
      <button
        type="button"
        id={id}
        ref={triggerRef}
        className="q-picker"
        aria-label={label}
        aria-expanded={open}
        aria-haspopup="menu"
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
