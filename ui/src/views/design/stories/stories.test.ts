import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import Conversation from './Conversation.vue'
import Idle from './Idle.vue'
import PaletteModes from './PaletteModes.vue'
import PalettePickers from './PalettePickers.vue'
import PaletteRoot from './PaletteRoot.vue'
import PaletteSessions from './PaletteSessions.vue'
import Permission from './Permission.vue'
import Queue from './Queue.vue'
import ToolCalls from './ToolCalls.vue'

import { liveSessions } from './idle.fixture'
import { planItems } from './conversation.fixture'
import { prompts } from './permission.fixture'
import { messages } from './queue.fixture'
import { rootRows } from './palette-root.fixture'
import { modeRows } from './palette-modes.fixture'
import { skillRows } from './palette-pickers.fixture'
import { sessionRows } from './palette-sessions.fixture'

describe('design/stories', () => {
  it('Idle renders one live-session row per fixture entry', () => {
    const wrapper = mount(Idle)

    expect(wrapper.findAll('.live-session-row')).toHaveLength(liveSessions.length)
  })

  it('Conversation renders every turn + the active plan checklist', () => {
    const wrapper = mount(Conversation)

    expect(wrapper.findAll('.turn').length).toBeGreaterThanOrEqual(2)
    expect(wrapper.findAll('.stream-card-item')).toHaveLength(planItems.length)
  })

  it('ToolCalls renders chips + terminal card', () => {
    const wrapper = mount(ToolCalls)

    // Every small tool renders as a pill; the terminal card carries
    // the running bash; a big-tool row carries the done bash.
    expect(wrapper.findAll('.tool-pill-small').length).toBeGreaterThanOrEqual(1)
    expect(wrapper.findAll('.tool-row-big').length).toBeGreaterThanOrEqual(1)
    expect(wrapper.find('[data-testid="terminal-card"]').exists()).toBe(true)
  })

  it('Permission renders every prompt and one active row', () => {
    const wrapper = mount(Permission)

    expect(wrapper.findAll('.permission-stack-row')).toHaveLength(prompts.length)
    const active = wrapper.findAll('.permission-stack-row').filter((r) => r.attributes('data-active') === 'true')
    expect(active).toHaveLength(1)
  })

  it('Queue renders every queued message', () => {
    const wrapper = mount(Queue)

    expect(wrapper.findAll('.queue-strip-row')).toHaveLength(messages.length)
  })

  it('PaletteRoot renders every root row', () => {
    const wrapper = mount(PaletteRoot)

    expect(wrapper.findAll('.palette-row')).toHaveLength(rootRows.length)
  })

  it('PaletteModes renders every mode row', () => {
    const wrapper = mount(PaletteModes)

    expect(wrapper.findAll('.palette-row')).toHaveLength(modeRows.length)
  })

  it('PalettePickers renders the multi-select skills palette', () => {
    const wrapper = mount(PalettePickers)

    // D5_Pickers is a single multi-select palette (skills w/ checkboxes)
    // per the bundle — not a 3-column set of mini palettes. Every skill
    // row renders inside the one shell.
    expect(wrapper.findAll('.palette-row')).toHaveLength(skillRows.length)
  })

  it('PaletteSessions renders rows + preview pane', () => {
    const wrapper = mount(PaletteSessions)

    expect(wrapper.findAll('.palette-row')).toHaveLength(sessionRows.length)
    expect(wrapper.find('.palette-sessions-preview').exists()).toBe(true)
  })
})
