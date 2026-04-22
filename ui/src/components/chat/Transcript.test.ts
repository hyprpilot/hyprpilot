import { mount } from '@vue/test-utils'
import { describe, expect, it, vi } from 'vitest'

import Transcript from '@components/chat/Transcript.vue'
import { EventKind, MessageKind, type ChatMessage, useAcpTranscript, type TranscriptEvent } from '@composables'
import { nextTick, ref } from 'vue'

function evt(session_id: string, update: Record<string, unknown>): TranscriptEvent {
  return {
    kind: EventKind.Transcript,
    agent_id: 'claude-code',
    session_id,
    update
  }
}

function deriveMessages(events: TranscriptEvent[]): ChatMessage[] {
  const source = ref(events)
  const { messages } = useAcpTranscript(source)

  return messages.value
}

describe('Transcript.vue', () => {
  it('renders the empty state when there are no messages', () => {
    const wrapper = mount(Transcript, { props: { messages: [] } })
    expect(wrapper.find('[data-testid="transcript-empty"]').exists()).toBe(true)
  })

  it('renders user + agent message variants in arrival order', () => {
    const messages = deriveMessages([
      evt('s-1', { sessionUpdate: 'user_message_chunk', content: { type: 'text', text: 'hello' }, messageId: 'u-1' }),
      evt('s-1', {
        sessionUpdate: 'agent_message_chunk',
        content: { type: 'text', text: 'hi back' },
        messageId: 'a-1'
      })
    ])
    expect(messages).toHaveLength(2)
    expect(messages[0].kind).toBe(MessageKind.User)
    expect(messages[1].kind).toBe(MessageKind.AgentMessage)

    const wrapper = mount(Transcript, { props: { messages } })
    const items = wrapper.element.querySelectorAll('article')
    expect(items).toHaveLength(2)
    expect(items[0].getAttribute('data-testid')).toBe('user-message')
    expect(items[1].getAttribute('data-testid')).toBe('agent-message')
  })

  it('accumulates chunks with the same messageId into one bubble', () => {
    const messages = deriveMessages([
      evt('s-1', {
        sessionUpdate: 'agent_message_chunk',
        content: { type: 'text', text: 'hello ' },
        messageId: 'a-1'
      }),
      evt('s-1', {
        sessionUpdate: 'agent_message_chunk',
        content: { type: 'text', text: 'world' },
        messageId: 'a-1'
      })
    ])
    expect(messages).toHaveLength(1)
    expect(messages[0].kind).toBe(MessageKind.AgentMessage)
    if (messages[0].kind === MessageKind.AgentMessage) {
      expect(messages[0].text).toBe('hello world')
    }
  })

  it('merges consecutive anon chunks of the same kind+session into one bubble', () => {
    const messages = deriveMessages([
      evt('s-1', { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'foo ' } }),
      evt('s-1', { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'bar' } })
    ])
    expect(messages).toHaveLength(1)
    expect(messages[0].kind).toBe(MessageKind.AgentMessage)
    if (messages[0].kind === MessageKind.AgentMessage) {
      expect(messages[0].text).toBe('foo bar')
    }
  })

  it('renders agent thought and plan variants when present', () => {
    const messages = deriveMessages([
      evt('s-1', {
        sessionUpdate: 'agent_thought_chunk',
        content: { type: 'text', text: 'thinking...' },
        messageId: 't-1'
      }),
      evt('s-1', {
        sessionUpdate: 'plan',
        entries: [{ content: 'do the thing', status: 'pending' }]
      })
    ])
    const wrapper = mount(Transcript, { props: { messages } })
    expect(wrapper.find('[data-testid="agent-thought"]').exists()).toBe(true)
    expect(wrapper.find('[data-testid="agent-plan"]').exists()).toBe(true)
  })

  it('merges tool-call updates on the same toolCallId', () => {
    const messages = deriveMessages([
      evt('s-1', { sessionUpdate: 'tool_call', toolCallId: 'tc-1', title: 'read file', status: 'pending' }),
      evt('s-1', { sessionUpdate: 'tool_call_update', toolCallId: 'tc-1', status: 'completed' })
    ])
    expect(messages).toHaveLength(1)
    expect(messages[0].kind).toBe(MessageKind.AgentToolCall)
    if (messages[0].kind === MessageKind.AgentToolCall) {
      expect(messages[0].call.title).toBe('read file')
      expect(messages[0].call.status).toBe('completed')
    }
  })

  it('scrolls to the bottom when the tail bubble extends in place (no length change)', async () => {
    // jsdom has no layout engine — scrollHeight / clientHeight / scrollTop
    // are all 0 by default; fake the geometry and the setter so the watcher
    // has something observable to chase. `scrollHeight` grows after the
    // in-place chunk so the test can prove the watcher re-fires.
    const proto = HTMLElement.prototype
    let fakeScrollHeight = 500
    const heightGetter = vi.spyOn(proto, 'scrollHeight', 'get').mockImplementation(() => fakeScrollHeight)
    const clientGetter = vi.spyOn(proto, 'clientHeight', 'get').mockReturnValue(100)
    const scrollTopStore = new WeakMap<HTMLElement, number>()
    const topGetter = vi.spyOn(proto, 'scrollTop', 'get').mockImplementation(function (this: HTMLElement) {
      return scrollTopStore.get(this) ?? 0
    })
    const topSetter = vi.spyOn(proto, 'scrollTop', 'set').mockImplementation(function (this: HTMLElement, v: number) {
      scrollTopStore.set(this, v)
    })

    try {
      const source = ref<TranscriptEvent[]>([evt('s-1', { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'hello ' }, messageId: 'a-1' })])
      const { messages } = useAcpTranscript(source)
      const wrapper = mount(Transcript, { props: { messages: messages.value } })
      await nextTick()
      const scroller = wrapper.get('[data-testid="transcript"]').element as HTMLElement

      // Reset the store to prove the in-place merge fires the watcher again.
      scrollTopStore.set(scroller, 0)

      fakeScrollHeight = 900
      source.value = [...source.value, evt('s-1', { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'world' }, messageId: 'a-1' })]
      expect(messages.value).toHaveLength(1)
      await wrapper.setProps({ messages: messages.value } as Record<string, unknown>)
      await nextTick()
      await nextTick()

      expect(scroller.scrollTop).toBe(900)
    } finally {
      heightGetter.mockRestore()
      clientGetter.mockRestore()
      topGetter.mockRestore()
      topSetter.mockRestore()
    }
  })
})
