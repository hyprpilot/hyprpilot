import { mount } from '@vue/test-utils'
import { defineComponent, h, ref, type Ref } from 'vue'
import { describe, expect, it, vi } from 'vitest'

import { type Binding, Modifier } from '@ipc'

import { type KeymapEntry, useKeymap } from './use-keymaps'

interface Harness {
  wrapper: ReturnType<typeof mount>
  target: EventTarget
}

/**
 * Mounts a tiny SFC that installs `useKeymap` against the given
 * target-factory. `onMounted` fires synchronously during mount, so
 * the listener is attached by the time `mount()` returns.
 */
function mountHarness(target: () => EventTarget, entries: () => KeymapEntry[]): Harness {
  const Component = defineComponent({
    setup() {
      useKeymap(target, entries)
      return () => h('div')
    }
  })
  const wrapper = mount(Component, { attachTo: document.body })

  return { wrapper, target: target() }
}

function bind(modifiers: Modifier[], key: string): Binding {
  return { modifiers, key }
}

describe('useKeymap', () => {
  it('matches on case-insensitive key', () => {
    const handler = vi.fn()
    const { wrapper } = mountHarness(
      () => document,
      () => [{ binding: bind([], 'enter'), handler }]
    )

    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    expect(handler).toHaveBeenCalledTimes(1)
    wrapper.unmount()
  })

  it('rejects when a required modifier is missing', () => {
    const handler = vi.fn()
    const { wrapper } = mountHarness(
      () => document,
      () => [{ binding: bind([Modifier.Ctrl], 'k'), handler }]
    )

    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'k', bubbles: true }))
    expect(handler).not.toHaveBeenCalled()
    wrapper.unmount()
  })

  it('rejects when an extra modifier is present', () => {
    const handler = vi.fn()
    const { wrapper } = mountHarness(
      () => document,
      () => [{ binding: bind([], 'a'), handler }]
    )

    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'a', ctrlKey: true, bubbles: true }))
    expect(handler).not.toHaveBeenCalled()
    wrapper.unmount()
  })

  it('matches on exact modifier set (ctrl+shift+enter)', () => {
    const handler = vi.fn()
    const { wrapper } = mountHarness(
      () => document,
      () => [{ binding: bind([Modifier.Ctrl, Modifier.Shift], 'enter'), handler }]
    )

    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', ctrlKey: true, shiftKey: true, bubbles: true }))
    expect(handler).toHaveBeenCalledTimes(1)
    wrapper.unmount()
  })

  it('ignores auto-repeat by default', () => {
    const handler = vi.fn()
    const { wrapper } = mountHarness(
      () => document,
      () => [{ binding: bind([], 'a'), handler }]
    )

    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'a', repeat: true, bubbles: true }))
    expect(handler).not.toHaveBeenCalled()
    wrapper.unmount()
  })

  it('fires on repeat when allowRepeat: true', () => {
    const handler = vi.fn()
    const { wrapper } = mountHarness(
      () => document,
      () => [{ binding: bind([Modifier.Ctrl], 'arrowup'), handler, allowRepeat: true }]
    )

    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowUp', ctrlKey: true, repeat: true, bubbles: true }))
    expect(handler).toHaveBeenCalledTimes(1)
    wrapper.unmount()
  })

  it('Space binding maps to event.key = " "', () => {
    const handler = vi.fn()
    const { wrapper } = mountHarness(
      () => document,
      () => [{ binding: bind([], 'space'), handler }]
    )

    document.dispatchEvent(new KeyboardEvent('keydown', { key: ' ', bubbles: true }))
    expect(handler).toHaveBeenCalledTimes(1)
    wrapper.unmount()
  })

  it('handler returning true calls preventDefault', () => {
    const handler = vi.fn(() => true)
    const { wrapper } = mountHarness(
      () => document,
      () => [{ binding: bind([], 'enter'), handler }]
    )

    const e = new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true })
    document.dispatchEvent(e)
    expect(e.defaultPrevented).toBe(true)
    wrapper.unmount()
  })

  it('handler returning false does NOT preventDefault', () => {
    const handler = vi.fn(() => false)
    const { wrapper } = mountHarness(
      () => document,
      () => [{ binding: bind([], 'enter'), handler }]
    )

    const e = new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true })
    document.dispatchEvent(e)
    expect(e.defaultPrevented).toBe(false)
    wrapper.unmount()
  })

  it('multiple entries: first match wins', () => {
    const first = vi.fn()
    const second = vi.fn()
    const { wrapper } = mountHarness(
      () => document,
      () => [
        { binding: bind([], 'a'), handler: first },
        { binding: bind([], 'a'), handler: second }
      ]
    )

    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'a', bubbles: true }))
    expect(first).toHaveBeenCalledTimes(1)
    expect(second).not.toHaveBeenCalled()
    wrapper.unmount()
  })

  it('target scoping: listener on a textarea fires only from that element', () => {
    const textarea = document.createElement('textarea')
    document.body.appendChild(textarea)
    const handler = vi.fn()

    const Component = defineComponent({
      setup() {
        const ref1: Ref<EventTarget | undefined> = ref(textarea)
        useKeymap(ref1, () => [{ binding: bind([], 'a'), handler }])
        return () => h('div')
      }
    })
    const wrapper = mount(Component, { attachTo: document.body })

    // Event on the unrelated document target — handler must NOT fire,
    // because the listener is scoped to the textarea.
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'a', bubbles: true }))
    expect(handler).not.toHaveBeenCalled()

    textarea.dispatchEvent(new KeyboardEvent('keydown', { key: 'a', bubbles: true }))
    expect(handler).toHaveBeenCalledTimes(1)

    wrapper.unmount()
    textarea.remove()
  })

  it('unmount tears down the listener', () => {
    const handler = vi.fn()
    const { wrapper } = mountHarness(
      () => document,
      () => [{ binding: bind([], 'a'), handler }]
    )

    wrapper.unmount()
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'a', bubbles: true }))
    expect(handler).not.toHaveBeenCalled()
  })

  it('ignores non-keydown event types', () => {
    const handler = vi.fn()
    const { wrapper } = mountHarness(
      () => document,
      () => [{ binding: bind([], 'a'), handler }]
    )

    document.dispatchEvent(new KeyboardEvent('keyup', { key: 'a', bubbles: true }))
    expect(handler).not.toHaveBeenCalled()
    wrapper.unmount()
  })
})
