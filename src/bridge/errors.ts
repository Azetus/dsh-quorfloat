/**
 * Typed channel failures.
 *
 * Every rejection that escapes {@link QuorfloatChannel} carries one of these
 * codes so callers can tell "the peer said no" from "the pipe died" from "we
 * gave up waiting" — the three cases the session layer must treat differently.
 */

import { ErrorCode } from '../protocol.js'

/** Stable failure taxonomy for the host↔quorfloat channel. */
export type ChannelErrorCode =
  | 'protocol-mismatch'
  | 'request-timeout'
  | 'channel-closed'
  | 'unavailable'
  | 'stale'
  | 'protocol-violation'
  | 'spawn-failed'
  | 'handshake-timeout'
  | 'helper-exited'

/** Error raised by the channel, the supervisor, or the executable resolver. */
export class ChannelError extends Error {
  readonly code: ChannelErrorCode
  readonly detail: Record<string, unknown>

  /**
   * @param code - stable machine-readable failure kind.
   * @param message - human-readable explanation.
   * @param detail - extra structured context for logs and diagnostics.
   */
  constructor(code: ChannelErrorCode, message: string, detail: Record<string, unknown> = {}) {
    super(message)
    this.name = 'ChannelError'
    this.code = code
    this.detail = detail
  }
}

/** Map this project's failure kinds onto JSON-RPC error codes for the peer. */
export function rpcCodeFor(code: ChannelErrorCode): number {
  switch (code) {
    case 'protocol-mismatch':
      return ErrorCode.ProtocolMismatch
    case 'request-timeout':
      return ErrorCode.RequestTimeout
    case 'channel-closed':
    case 'helper-exited':
    case 'spawn-failed':
      return ErrorCode.ChannelClosed
    case 'unavailable':
      return ErrorCode.Unavailable
    case 'stale':
      return ErrorCode.Stale
    case 'protocol-violation':
      return ErrorCode.ProtocolViolation
    case 'handshake-timeout':
      return ErrorCode.RequestTimeout
  }
}

/** Extract a {@link ChannelError} from an unknown rejection value.
 *
 * A host error already carries a project-level code and diagnostics. Preserve both rather
 * than flattening it to `channel-closed`: the sidecar needs the code to choose a structured
 * reason, and the detail is what makes the refusal explainable.
 */
export function asChannelError(error: unknown): ChannelError {
  if (error instanceof ChannelError) return error
  if (error instanceof Error && error.name === 'HarnessError') {
    const candidate = error as Error & { code?: unknown; detail?: unknown }
    if (typeof candidate.code === 'string') {
      const detail = typeof candidate.detail === 'object' && candidate.detail !== null
        ? candidate.detail as Record<string, unknown>
        : {}
      return new ChannelError(candidate.code as ChannelError['code'], error.message, detail)
    }
  }
  return new ChannelError('channel-closed', error instanceof Error ? error.message : String(error))
}
