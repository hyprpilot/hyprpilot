import { flushPromises, mount } from '@vue/test-utils'
import { describe, expect, it, vi } from 'vitest'

import ChatBody from './Body.vue'
import { Role } from '@components'

const writeClipboardText = vi.fn().mockResolvedValue(undefined)
vi.mock('@tauri-apps/plugin-clipboard-manager', () => ({
  writeText: (...args: unknown[]) => writeClipboardText(...args)
}))

async function flush(): Promise<void> {
  // The watcher awaits renderMarkdown which itself awaits Shiki's
  // `loadLanguage` (dynamic import on first hit per language) before
  // emitting tokens; flush several ticks until the rendered HTML
  // settles. `vue-test-utils` `flushPromises` resolves microtasks
  // only — dynamic imports add macrotask hops, hence the loop.
  for (let i = 0; i < 10; i += 1) {
    await flushPromises()
    await new Promise((r) => setTimeout(r, 0))
  }
}

describe('ChatBody.vue', () => {
  it('renders the slot for user role with no markdown prop', () => {
    const wrapper = mount(ChatBody, {
      props: { role: Role.User },
      slots: { default: 'hello world' }
    })

    expect(wrapper.text()).toContain('hello world')
    expect(wrapper.find('.chat-body-md').exists()).toBe(false)
  })

  it('renders the slot for assistant role when markdown is not enabled', () => {
    const wrapper = mount(ChatBody, {
      props: { role: Role.Assistant },
      slots: { default: 'hi from assistant' }
    })

    expect(wrapper.text()).toContain('hi from assistant')
    expect(wrapper.find('.chat-body-md').exists()).toBe(false)
  })

  it('renders sanitised markdown HTML for assistant role with :markdown + :text', async () => {
    const wrapper = mount(ChatBody, {
      props: { role: Role.Assistant, text: '**bold** and *italic*', markdown: true }
    })
    await flush()

    const md = wrapper.find('.chat-body-md')
    expect(md.exists()).toBe(true)
    expect(md.html()).toContain('<strong>bold</strong>')
    expect(md.html()).toContain('<em>italic</em>')
  })

  it('keeps user-role text raw even when markdown=true is set on the prop', async () => {
    const wrapper = mount(ChatBody, {
      props: { role: Role.User, text: '**bold**', markdown: true }
    })
    await flush()

    expect(wrapper.find('.chat-body-md').exists()).toBe(false)
    expect(wrapper.text()).toContain('**bold**')
  })

  it('mounts a copy button per fenced code block', async () => {
    const src = '```ts\nconst x = 1\n```\n\n```sh\necho hi\n```'
    const wrapper = mount(ChatBody, { props: { role: Role.Assistant, text: src, markdown: true } })
    await flush()

    expect(wrapper.findAll('button[data-md-copy]')).toHaveLength(2)
    expect(wrapper.findAll('.md-codeblock')).toHaveLength(2)
  })

  it('copy button writes the code text to the clipboard via the Tauri plugin', async () => {
    writeClipboardText.mockClear()
    const wrapper = mount(ChatBody, {
      props: { role: Role.Assistant, text: '```ts\nconst y = 2\n```', markdown: true }
    })
    await flush()

    const button = wrapper.find('button[data-md-copy]')
    expect(button.exists()).toBe(true)
    await button.trigger('click')
    await flushPromises()

    expect(writeClipboardText).toHaveBeenCalledTimes(1)
    expect(writeClipboardText.mock.calls[0]?.[0]).toContain('const y = 2')
  })
})
