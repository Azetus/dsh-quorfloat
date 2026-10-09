// A picker menu's hint line (`.q-pophint`).

/** Props for {@link MenuHint}. */
export interface MenuHintProps {
  /** The sentence to show. */
  readonly text: string
}

/**
 * Render a menu's `.q-pophint` line.
 *
 * @param props - the sentence.
 * @returns the hint line.
 */
export function MenuHint({ text }: MenuHintProps) {
  return <div className="q-pophint">{text}</div>
}
