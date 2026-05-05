/**
 * UI tone / variant enums shared across feedback + button surfaces.
 */

export enum ToastTone {
  Ok = 'ok',
  Warn = 'warn',
  Err = 'err',
  Info = 'info'
}

export enum ButtonTone {
  Ok = 'ok',
  Err = 'err',
  Warn = 'warn',
  Neutral = 'neutral'
}

export enum ButtonVariant {
  Solid = 'solid',
  Ghost = 'ghost'
}

/**
 * Map a `ToastTone` onto its theme CSS variable. Shared between
 * `Modal.vue` (header pill bg + top border) and `ToolHeader.vue`
 * (tag bg) so a tone palette change lands in one place.
 */
export function toneBg(tone: ToastTone): string {
  switch (tone) {
    case ToastTone.Ok:
      return 'var(--theme-status-ok)'
    case ToastTone.Err:
      return 'var(--theme-status-err)'
    case ToastTone.Info:
      return 'var(--theme-accent)'
    case ToastTone.Warn:
    default:
      return 'var(--theme-status-warn)'
  }
}
