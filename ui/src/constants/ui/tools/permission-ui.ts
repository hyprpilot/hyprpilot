/**
 * Permission-UI surface a tool's presentation maps to. Resolved by
 * the frontend's `presentationFor()` dispatcher — daemon emits raw
 * `kind` + adapter; this enum drives whether the permission prompt
 * lands on the inline strip (`Row`) or the modal queue (`Modal`).
 */
export enum PermissionUi {
  Row = 'row',
  Modal = 'modal'
}
