// A picker menu's heading row (`.q-pophead`).

/** Props for {@link MenuHeading}. */
export interface MenuHeadingProps {
  /** The heading's own text. */
  readonly title: string
  /** Optional right-aligned detail, e.g. the current workspace. */
  readonly detail?: string
}

/**
 * Render a menu's `.q-pophead` heading.
 *
 * @param props - title and optional detail.
 * @returns the heading row.
 */
export function MenuHeading({ title, detail = '' }: MenuHeadingProps) {
  return (
    <div className="q-pophead">
      <strong>{title}</strong>
      {detail !== '' && <span>{detail}</span>}
    </div>
  )
}
