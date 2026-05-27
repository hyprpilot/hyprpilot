import { faTerminal } from '@fortawesome/free-solid-svg-icons'
import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import ToolBody from './ToolBody.vue'
import { PermissionUi, ToolKind, ToolState, type ToolCallView } from '@components'

function makeView(overrides: Partial<ToolCallView> = {}): ToolCallView {
  return {
    id: 'tc-1',
    kind: ToolKind.Read,
    name: 'Read',
    state: ToolState.Done,
    icon: faTerminal,
    permissionUi: PermissionUi.Row,
    title: 'read · src/App.vue',
    stats: [],
    fields: [],
    ...overrides
  }
}

describe('ToolBody.vue', () => {
  /**
   * Output expansion mirrors the parent pill's "live show, finalized
   * hide" policy: while the tool is running / awaiting permission
   * the captain wants to see the streaming stdout in real time;
   * once the call finalizes (Done / Failed) output collapses so a
   * long log doesn't dominate the chat — captain re-expands manually.
   */
  it('expands output on Running state to show streaming stdout', () => {
    const wrapper = mount(ToolBody, {
      props: {
        view: makeView({
          state: ToolState.Running,
          description: '```text\nfile body\n```',
          output: 'lots\nof\nlines\n...'
        })
      }
    })

    const section = wrapper.find('.tool-body-output')

    expect(section.exists()).toBe(true)
    expect(section.attributes('data-expanded')).toBe('true')
    expect(wrapper.find('.tool-body-output-body').exists()).toBe(true)
  })

  it('collapses output on Done state to free chat real estate', () => {
    const wrapper = mount(ToolBody, {
      props: {
        view: makeView({
          state: ToolState.Done,
          description: '```text\nbody\n```',
          output: 'lots\nof\nlines\n...'
        })
      }
    })

    const section = wrapper.find('.tool-body-output')

    expect(section.attributes('data-expanded')).toBe('false')
    expect(wrapper.find('.tool-body-output-body').exists()).toBe(false)
  })

  it('collapses output on Failed state', () => {
    const wrapper = mount(ToolBody, {
      props: {
        view: makeView({
          state: ToolState.Failed,
          output: 'error: out of memory'
        })
      }
    })

    expect(wrapper.find('.tool-body-output').attributes('data-expanded')).toBe('false')
  })

  /**
   * Output renders through MarkdownBody — same component the
   * description path uses. The agent's tool responses are typically
   * narrative prose plus fenced code blocks (```bash, ```console,
   * ```diff, ```json), and rendering as markdown means the captain
   * sees a clean Shiki-highlighted code box instead of the literal
   * triple-backtick markers + open empty space that an xterm or
   * raw `<pre>` would produce.
   *
   * Real streaming terminal output (the Terminal tool with a
   * terminal_id) bypasses ToolBody entirely — it goes through
   * `<TerminalCard>` which hosts xterm.js fed by the live
   * `useTerminals` store.
   */
  it('renders the output via MarkdownBody so fenced code blocks render Shiki-highlighted', () => {
    // MarkdownBody emits a wrapper div even on a single fenced
    // block. We assert the section uses it (not a plain pre) and
    // that the rendered DOM contains the fenced block's content.
    const wrapper = mount(ToolBody, {
      props: {
        view: makeView({
          kind: ToolKind.Execute,
          name: 'Bash',
          state: ToolState.Running,
          output: '```console\nHTTP/2 301\n```'
        })
      }
    })

    const body = wrapper.find('.tool-body-output-body')

    expect(body.exists()).toBe(true)
    // The literal triple-backtick markers must NOT survive into the
    // rendered text — markdown rendering consumes them. Asserting
    // their absence in `text()` (visible content, no HTML comments)
    // catches regressions to the old `<pre>{{ output }}</pre>` path
    // which would echo the markers verbatim.
    expect(body.text()).not.toContain('```console')
    expect(body.text()).not.toContain('```')
    // Code-block content survives, proving markdown rendering
    // produced the inner block (Shiki tokenizes asynchronously, so
    // the immediate DOM may still show plain text inside a code
    // element on the first frame).
    expect(body.text()).toContain('HTTP/2 301')
  })

  it('renders description before fields before output', () => {
    const wrapper = mount(ToolBody, {
      props: {
        view: makeView({
          description: 'desc-text',
          fields: [{ label: 'path', value: '/x' }],
          output: 'out'
        })
      }
    })

    const order = wrapper.element.querySelectorAll('.tool-body-description, .tool-body-fields, .tool-body-output')

    expect(order.length).toBe(3)
    expect((order[0] as HTMLElement).className).toContain('tool-body-description')
    expect((order[1] as HTMLElement).className).toContain('tool-body-fields')
    expect((order[2] as HTMLElement).className).toContain('tool-body-output')
  })

  it('hides output when it duplicates the description payload', () => {
    const wrapper = mount(ToolBody, {
      props: {
        view: makeView({
          description: '```diff\n-old\n+new\n```',
          output: '-old\n+new'
        })
      }
    })

    expect(wrapper.find('.tool-body-description').exists()).toBe(true)
    expect(wrapper.find('.tool-body-output').exists()).toBe(false)
  })

  it('renders fields with each label and value', () => {
    const wrapper = mount(ToolBody, {
      props: {
        view: makeView({
          fields: [
            { label: 'path', value: '/etc/hosts' },
            { label: 'pattern', value: '127.*' }
          ]
        })
      }
    })

    const labels = wrapper.findAll('.tool-body-label').map((n) => n.text())
    const codes = wrapper.findAll('.tool-body-code').map((n) => n.text())

    expect(labels).toEqual(['path', 'pattern'])
    expect(codes).toEqual(['/etc/hosts', '127.*'])
  })

  it('renders nothing visible when description / fields / output are all empty', () => {
    const wrapper = mount(ToolBody, { props: { view: makeView() } })

    expect(wrapper.find('.tool-body').exists()).toBe(false)
  })
})
