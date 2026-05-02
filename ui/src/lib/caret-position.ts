/**
 * Compute the rendered (x, y) coordinate of the caret inside a
 * `<textarea>`. Standard mirror-div trick — copy the textarea's
 * font / padding / line-height into a hidden absolutely-positioned
 * `<div>`, lay out the substring `text.slice(0, position)` into it,
 * and read the bounding rect of a marker span at the end. Returns
 * coordinates in **viewport space** (relative to the page, like
 * `getBoundingClientRect`).
 *
 * No external dependency; the popular `textarea-caret-position`
 * package does the same job in ~250 LoC, but for our caret-anchored
 * popover the minimal subset below is enough.
 */

const COPIED_PROPERTIES: readonly string[] = [
  'direction',
  'boxSizing',
  'width',
  'height',
  'overflowX',
  'overflowY',
  'borderTopWidth',
  'borderRightWidth',
  'borderBottomWidth',
  'borderLeftWidth',
  'borderStyle',
  'paddingTop',
  'paddingRight',
  'paddingBottom',
  'paddingLeft',
  'fontStyle',
  'fontVariant',
  'fontWeight',
  'fontStretch',
  'fontSize',
  'fontSizeAdjust',
  'lineHeight',
  'fontFamily',
  'textAlign',
  'textTransform',
  'textIndent',
  'textDecoration',
  'letterSpacing',
  'wordSpacing',
  'tabSize'
]

export interface CaretCoordinates {
  /** Viewport y. */
  top: number
  /** Viewport x. */
  left: number
  /** Line height — popover uses this to flip above/below the caret. */
  height: number
}

export function getCaretCoordinates(textarea: HTMLTextAreaElement, position: number): CaretCoordinates {
  const document = textarea.ownerDocument
  const win = document.defaultView

  if (!win) {
    return {
      top: 0,
      left: 0,
      height: 16
    }
  }
  const mirror = document.createElement('div')

  mirror.id = 'hyprpilot-caret-mirror'
  document.body.appendChild(mirror)

  const style = mirror.style
  const computed = win.getComputedStyle(textarea)

  style.whiteSpace = 'pre-wrap'
  style.wordWrap = 'break-word'
  style.position = 'absolute'
  style.visibility = 'hidden'
  style.top = '0'
  style.left = '0'

  for (const prop of COPIED_PROPERTIES) {
    // Each property name is a known CSSStyleDeclaration key; copy as-is.

    ;(style as any)[prop] = (computed as any)[prop]
  }

  const text = textarea.value.substring(0, position)

  mirror.textContent = text

  const span = document.createElement('span')

  // The trailing space ensures the span has a non-zero width — empty
  // spans collapse and read offsetLeft as the line's start.
  span.textContent = textarea.value.substring(position) || '.'
  mirror.appendChild(span)

  const taRect = textarea.getBoundingClientRect()
  const lineHeight = parseInt(computed.lineHeight, 10) || parseInt(computed.fontSize, 10) * 1.4

  const result: CaretCoordinates = {
    top: taRect.top + span.offsetTop - textarea.scrollTop,
    left: taRect.left + span.offsetLeft - textarea.scrollLeft,
    height: lineHeight
  }

  mirror.parentNode?.removeChild(mirror)

  return result
}
