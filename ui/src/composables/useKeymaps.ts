/**
 * Boot-time fetch of the `[keymaps]` config tree from the daemon.
 * One-shot: user edits the config file + restarts for changes to take
 * effect. Soft-fails when no Tauri host is available (browser dev,
 * vitest); consumers guard on `undefined`.
 */

import { onMounted, onUnmounted, type Ref, ref, unref, type MaybeRefOrGetter } from 'vue'

import { type Binding, invoke, type KeymapsConfig, Modifier, TauriCommand } from '@ipc'
import { log } from '@lib'

const cache = ref<KeymapsConfig>()

export async function loadKeymaps(): Promise<void> {
  try {
    cache.value = await invoke(TauriCommand.GetKeymaps)
  } catch (err) {
    log.warn('get_keymaps invoke failed; keybindings will not register', undefined, err)
  }
}

export function useKeymaps(): { keymaps: Ref<KeymapsConfig | undefined> } {
  return { keymaps: cache }
}

/**
 * Shared editable-target guard for call sites that need to skip
 * bindings while an `<input>` / `<textarea>` / `contenteditable`
 * element owns focus (e.g. the global `a` / `d` approval keys).
 */
export function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) {
    return false
  }
  const tag = target.tagName
  if (tag === 'INPUT' || tag === 'TEXTAREA') {
    return true
  }
  // `isContentEditable` isn't reliably implemented in jsdom; fall back
  // to the attribute for test coverage parity with real browsers.
  if (target.isContentEditable) {
    return true
  }
  const attr = target.getAttribute('contenteditable')

  return attr === '' || attr === 'true' || attr === 'plaintext-only'
}

export interface KeymapEntry {
  binding: Binding
  /** Returning truthy calls `preventDefault()`. */
  handler: (e: KeyboardEvent) => boolean | void
  /** Default false — auto-repeat events are ignored. */
  allowRepeat?: boolean
}

/**
 * Native keydown dispatcher. Registers a single listener on `target`
 * (defaults to `document`), compares each `KeyboardEvent` against the
 * binding list via `event.key.toLowerCase()` + exact modifier flag
 * match, and invokes the first handler whose binding matches.
 */
export function useKeymap(target: MaybeRefOrGetter<EventTarget | null | undefined>, entries: () => KeymapEntry[]): void {
  const resolve = (): EventTarget => {
    const t = typeof target === 'function' ? (target as () => EventTarget | null | undefined)() : unref(target)

    return t ?? document
  }

  const listener = (e: Event): void => {
    if (!(e instanceof KeyboardEvent) || e.type !== 'keydown') {
      return
    }
    for (const entry of entries()) {
      if (matchesBinding(e, entry.binding, entry.allowRepeat ?? false)) {
        const preventDefault = entry.handler(e)
        if (preventDefault) {
          e.preventDefault()
        }
        break
      }
    }
  }

  let attached: EventTarget | undefined
  onMounted(() => {
    attached = resolve()
    attached.addEventListener('keydown', listener)
  })
  onUnmounted(() => {
    attached?.removeEventListener('keydown', listener)
    attached = undefined
  })
}

function matchesBinding(e: KeyboardEvent, binding: Binding, allowRepeat: boolean): boolean {
  if (e.repeat && !allowRepeat) {
    return false
  }
  if (e.ctrlKey !== binding.modifiers.includes(Modifier.Ctrl)) {
    return false
  }
  if (e.shiftKey !== binding.modifiers.includes(Modifier.Shift)) {
    return false
  }
  if (e.altKey !== binding.modifiers.includes(Modifier.Alt)) {
    return false
  }
  if (e.metaKey !== binding.modifiers.includes(Modifier.Meta)) {
    return false
  }

  return e.key.toLowerCase() === bindingKeyToEventKey(binding.key)
}

/**
 * Bridge the one `KeyboardEvent.key` value that isn't the lowercased
 * identifier: Space emits a literal `' '` on the event, while our wire
 * form spells it `space` so TOML can name it. Everything else (letters,
 * `arrowup`, `enter`, `tab`, `?`, …) matches `event.key.toLowerCase()`
 * directly.
 */
function bindingKeyToEventKey(key: string): string {
  if (key === 'space') {
    return ' '
  }

  return key
}

export function __resetKeymapsForTests(): void {
  cache.value = undefined
}
