// One promise the panel makes to the reader: while you are at the bottom, you stay there.
//
// The thread owns the decision (only it sees the scroll position), but the thread is not the
// only thing that moves the reader: the composer grows as you type, and while the panel is at
// its height cap that comes out of the thread's box. That happens in an input handler, outside
// React's commit cycle, so the thread's own layout effect never sees it.
//
// A resize observer would be the frame-shaped way to notice, but the promise is "do not move
// the reader", and a frame of movement is exactly what the reader sees. So the mover asks for
// the pin directly, synchronously, after it has resized.
//
// This is a module-level slot rather than a prop or a context because there is exactly one
// thread per panel, and the alternative is threading a ref through four components to say one
// sentence. The setter is the only writer and it clears on unmount, so a stale thread can
// never be pinned.

/** How the thread keeps the reader at the bottom, or null when no thread is mounted. */
let pin: (() => void) | null = null

/**
 * Offer the panel a way to put the reader back at the bottom.
 *
 * @param follower - called to re-pin; it must itself decide whether the reader is following
 *   (a reader who scrolled up is not to be yanked), or null on unmount.
 */
export function setBottomPin(follower: (() => void) | null): void {
  pin = follower
}

/**
 * Ask the mounted thread to keep the reader at the bottom, if they were there.
 *
 * Safe to call at any time, including when no thread is mounted (the settings page): doing
 * nothing is the correct answer then.
 */
export function pinToBottom(): void {
  pin?.()
}
