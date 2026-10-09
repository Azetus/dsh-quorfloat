// One embedded design asset as a real `<svg>` element.
//
// The panel's CSS positions the svg itself (`.q-option > svg`, `.q-picker > svg:first-child`,
// `.q-check`), so this renders the element rather than wrapping the markup in a span.

import { iconParts } from '../lib/icons'

/** Props for {@link Icon}. */
export interface IconProps {
  /** Asset key from the embedded map (`lib/icons.ts`). */
  readonly name: string
  /** Extra class, e.g. `q-check` or `q-chevron`. */
  readonly className?: string
}

/**
 * Render one icon, or nothing when this build does not carry it.
 *
 * @param props - asset key and optional class.
 * @returns the svg element, or null for an unknown key.
 */
export function Icon({ name, className = '' }: IconProps) {
  const parts = iconParts(name)
  if (parts === null) return null
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      viewBox={parts.viewBox}
      width={parts.size.width}
      height={parts.size.height}
      fill="currentColor"
      aria-hidden="true"
      focusable="false"
      className={className}
      data-phosphor="true"
      dangerouslySetInnerHTML={{ __html: parts.inner }}
    />
  )
}
