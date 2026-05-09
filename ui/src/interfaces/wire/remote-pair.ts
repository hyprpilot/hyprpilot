/**
 * Payload of `remote:pair-request` — emitted on every WS upgrade
 * the daemon receives from a phone (or any browser) hitting the
 * remote bridge. Carries BOTH codes: the desktop renders its own
 * (`desktopCode`) as QR + words and expects the captain to present
 * the device's code (`deviceCode`) — typed manually, or scanned
 * from the device's QR. Asymmetric codes are the whole point of
 * the pairing.
 */
export interface RemotePairRequestEventPayload {
  pendingId: string
  /** Code rendered on the connecting device — desktop's expected input. */
  deviceCode: string
  /** Code rendered on the desktop modal — device's expected input. */
  desktopCode: string
  remoteAddr: string
}

/**
 * Payload of `remote:pair-resolved` — emitted whenever a pending
 * pair transitions out of `pending` (confirmed by either side, or
 * rejected via timeout / captain-reject / attempt-cap / connection
 * drop). The desktop modal listens for this and clears its state.
 */
export interface RemotePairResolvedEventPayload {
  pendingId: string
  outcome: 'confirmed' | 'rejected'
}

/**
 * Snapshot row from `remote_pending_pairs`. Diagnostic surface for
 * "queue of waiting devices" UX.
 */
export interface RemotePendingPair {
  pendingId: string
  remoteAddr: string
  expiresInSeconds: number
}
