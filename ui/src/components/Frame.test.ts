import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import Frame from './Frame.vue'
import { Phase, type GitStatus } from './types'

describe('Frame.vue', () => {
  it('renders header rows 1 + 2 + body slot', () => {
    const wrapper = mount(Frame, {
      props: {
        profile: 'captain',
        modeTag: 'ask',
        provider: 'claude-code',
        model: 'sonnet-4.5',
        title: 'utils/hyprpilot',
        cwd: '~/dev/hyprpilot',
        counts: [
          { label: 'turns', count: 3, color: '#98c379' },
          { label: 'tools', count: 12, color: '#61afef' }
        ]
      },
      slots: {
        default: '<p data-testid="body-slot">body</p>',
        composer: '<div data-testid="composer-slot" />'
      }
    })

    expect(wrapper.text()).toContain('captain')
    expect(wrapper.text()).toContain('ask')
    expect(wrapper.text()).toContain('claude-code')
    expect(wrapper.text()).toContain('sonnet-4.5')
    expect(wrapper.text()).toContain('utils/hyprpilot')
    expect(wrapper.text()).toContain('~/dev/hyprpilot')
    expect(wrapper.text()).toContain('turns')
    expect(wrapper.text()).toContain('12')
    expect(wrapper.find('[data-testid="body-slot"]').exists()).toBe(true)
    expect(wrapper.find('[data-testid="composer-slot"]').exists()).toBe(true)
  })

  it('emits close on the close button', async () => {
    const wrapper = mount(Frame, { props: { profile: 'captain' } })

    await wrapper.find('button[aria-label="close"]').trigger('click')

    expect(wrapper.emitted('close')).toHaveLength(1)
  })

  it('emits toggleCwd on the cwd row button', async () => {
    const wrapper = mount(Frame, { props: { profile: 'captain', cwd: '/tmp' } })

    await wrapper.find('button.frame-cwd').trigger('click')

    expect(wrapper.emitted('toggleCwd')).toHaveLength(1)
  })

  it('paints the profile pill with the phase state color', () => {
    const wrapper = mount(Frame, {
      props: { profile: 'captain', phase: Phase.Streaming }
    })

    const pill = wrapper.find('.frame-profile-pill').element as HTMLElement
    expect(pill.style.backgroundColor).toContain('var(--theme-state-stream)')
  })

  it('animates the dot on streaming and working phases only', () => {
    const streaming = mount(Frame, { props: { profile: 'captain', phase: Phase.Streaming } })
    const working = mount(Frame, { props: { profile: 'captain', phase: Phase.Working } })
    const idle = mount(Frame, { props: { profile: 'captain', phase: Phase.Idle } })

    expect(streaming.find('.frame-profile-dot').classes()).toContain('animate-pulse-slow')
    expect(working.find('.frame-profile-dot').classes()).toContain('animate-pulse-slow')
    expect(idle.find('.frame-profile-dot').classes()).not.toContain('animate-pulse-slow')
  })

  it('renders git branch + ahead/behind + worktree chip when gitStatus is set', () => {
    const gitStatus: GitStatus = { branch: 'feat/k-250', ahead: 2, behind: 1, worktree: 'feat-k-250' }
    const wrapper = mount(Frame, {
      props: { profile: 'captain', cwd: '~/dev/hyprpilot', gitStatus }
    })

    expect(wrapper.find('.frame-cwd-git').exists()).toBe(true)
    expect(wrapper.text()).toContain('feat/k-250')
    expect(wrapper.text()).toContain('↑2')
    expect(wrapper.text()).toContain('↓1')
    expect(wrapper.find('.frame-cwd-worktree').exists()).toBe(true)
    expect(wrapper.text()).toContain('worktree: feat-k-250')
  })

  it('omits ahead/behind chips when both are zero', () => {
    const wrapper = mount(Frame, {
      props: {
        profile: 'captain',
        cwd: '~/dev/hyprpilot',
        gitStatus: { branch: 'main' }
      }
    })

    expect(wrapper.find('.frame-cwd-git-ahead').exists()).toBe(false)
    expect(wrapper.find('.frame-cwd-git-behind').exists()).toBe(false)
    expect(wrapper.find('.frame-cwd-worktree').exists()).toBe(false)
    expect(wrapper.text()).toContain('main')
  })

  it('emits pillClick with target=mode when the mode pill is clicked', async () => {
    const wrapper = mount(Frame, { props: { profile: 'captain', modeTag: 'plan' } })

    await wrapper.find('button[aria-label="mode"]').trigger('click')

    expect(wrapper.emitted('pillClick')).toEqual([['mode']])
  })

  it('emits pillClick with target=provider when the provider pill is clicked', async () => {
    const wrapper = mount(Frame, {
      props: { profile: 'captain', provider: 'claude-code', model: 'sonnet-4.5' }
    })

    await wrapper.find('button[aria-label="provider"]').trigger('click')

    expect(wrapper.emitted('pillClick')).toEqual([['provider']])
  })

  it('emits breadcrumbClick with the pill id when a count is clicked', async () => {
    const wrapper = mount(Frame, {
      props: {
        profile: 'captain',
        counts: [
          { id: 'mcps', label: 'mcps', count: 3 },
          { id: 'skills', label: 'skills', count: 7 }
        ]
      }
    })

    await wrapper.find('button[aria-label="skills"]').trigger('click')

    expect(wrapper.emitted('breadcrumbClick')).toEqual([['skills']])
  })

  it('falls back to the label when BreadcrumbCount.id is unset', async () => {
    const wrapper = mount(Frame, {
      props: { profile: 'captain', counts: [{ label: 'turns', count: 4 }] }
    })

    await wrapper.find('button[aria-label="turns"]').trigger('click')

    expect(wrapper.emitted('breadcrumbClick')).toEqual([['turns']])
  })

  it('renders the restored tag only when restored=true', async () => {
    const wrapper = mount(Frame, { props: { profile: 'captain', restored: true } })
    expect(wrapper.find('.frame-restored-tag').exists()).toBe(true)
    expect(wrapper.text()).toContain('restored')

    await wrapper.setProps({ restored: false })
    expect(wrapper.find('.frame-restored-tag').exists()).toBe(false)
  })
})
