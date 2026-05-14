import { mount } from '@vue/test-utils'
import { describe, expect, it, vi } from 'vitest'

import XtermView from './XtermView.vue'

// xterm.js writes its output into a canvas/DOM tree it manages;
// jsdom can't render the canvas. We only assert that the host
// element mounts, the props bind, and unmount disposes cleanly.
// The rendered output is integration-tested manually against a
// live daemon.

describe('XtermView.vue', () => {
  it('mounts a host div carrying the running data attribute', () => {
    const wrapper = mount(XtermView, { props: { text: 'hello\n', running: true } })

    const host = wrapper.find('.xterm-host')

    expect(host.exists()).toBe(true)
    expect(host.attributes('data-running')).toBe('true')
    wrapper.unmount()
  })

  it('disposes the underlying terminal on unmount without throwing', () => {
    const wrapper = mount(XtermView, { props: { text: 'x' } })

    expect(() => wrapper.unmount()).not.toThrow()
  })

  it('handles a wholesale text rewrite (prop got shorter) without crashing', async() => {
    const wrapper = mount(XtermView, { props: { text: 'aaaa\nbbbb\n' } })

    // setProps's typing is loose here — vue-tsc can't infer the SFC's
    // typed prop set through the test-utils wrapper. The test asserts
    // runtime behavior (no throw, host stays mounted) rather than the
    // exact prop shape; the cast keeps the assertion sharp.
    await wrapper.setProps({ text: 'cc\n' } as never)
    expect(wrapper.find('.xterm-host').exists()).toBe(true)
    wrapper.unmount()
  })

  it('handles append-only text growth without crashing', async() => {
    const wrapper = mount(XtermView, { props: { text: 'one\n' } })

    await wrapper.setProps({ text: 'one\ntwo\n' } as never)
    await wrapper.setProps({ text: 'one\ntwo\nthree\n' } as never)
    expect(wrapper.find('.xterm-host').exists()).toBe(true)
    wrapper.unmount()
  })

  // Silence `getContext()` jsdom warnings — the test asserts wiring,
  // not pixels.
  it('does not throw when canvas context is unavailable (jsdom)', () => {
    const spy = vi.spyOn(console, 'error').mockImplementation(() => {})
    const wrapper = mount(XtermView, { props: { text: '' } })

    wrapper.unmount()
    spy.mockRestore()
    expect(true).toBe(true)
  })
})
