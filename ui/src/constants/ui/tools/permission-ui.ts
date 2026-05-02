/**
 * Permission-UI surface a formatter declares for its tool. The
 * permission flow inspects this on the formatted view and routes
 * the request to the inline strip (`Row`) or the modal queue
 * (`Modal`). Plan-exit is the canonical `Modal` consumer today.
 */
export enum PermissionUi {
  Row = 'row',
  Modal = 'modal'
}
