/**
 * Wire-contract shape for the `acp:permission-resolved` Tauri event.
 * Mirrors Rust `InstanceEvent::PermissionResolved` (see
 * `src-tauri/src/adapters/instance.rs`). Emitted whenever the
 * permission controller resolves a waiter — UI / remote answer, or
 * the 10-min `WAITER_TIMEOUT` expiry. Both desktop and remote
 * subscribers drop their `pendingPermissions` row keyed on
 * `requestId` so the prompt clears on every screen the moment it's
 * answered (or expires).
 *
 * The Rust enum carries an `event` discriminator field
 * (`#[serde(tag = "event")]`); it rides on the wire but isn't
 * required UI-side because the listener registration already
 * disambiguates by event name. Consumers ignore unknown fields.
 */
export interface AcpPermissionResolvedPayload {
  instanceId: string
  requestId: string
  /**
   * Real ACP option id the controller resolved with, or
   * `PERMISSION_EXPIRED_OPTION_ID` (`'__expired__'`) on timeout.
   * Captains shouldn't ever see this value reach a UI surface — by
   * the time it lands the row has already been removed from the
   * mirror's pending list.
   */
  optionId: string
}

/**
 * Sentinel `optionId` the daemon sets when a permission request
 * expires past `WAITER_TIMEOUT` (10 min) without a captain answer.
 * Mirrors Rust `PERMISSION_EXPIRED_OPTION_ID`; keep them in lockstep.
 */
export const PERMISSION_EXPIRED_OPTION_ID = '__expired__'
