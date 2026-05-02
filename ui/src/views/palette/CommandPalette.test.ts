import { mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { defineComponent, h, nextTick } from 'vue'

import CommandPalette from './CommandPalette.vue'
import { __resetPaletteStackForTests, PaletteMode, type PaletteSpec, usePalette } from '@composables'

// focus-trap-vue relies on real browser focus / tabbable-node detection
// to guard activation; jsdom never reports any node as tabbable because
// elements have no computed layout. Replace the wrapper with a dumb
// passthrough so the palette's behaviour can still be exercised.
vi.mock('focus-trap-vue', () => ({
  FocusTrap: defineComponent({
    name: 'FocusTrapStub',
    setup(_, { slots }) {
      return () => h('div', { 'data-testid': 'focus-trap-stub' }, slots.default?.())
    }
  })
}))

function dispatchKey(init: KeyboardEventInit): void {
  document.dispatchEvent(
    new KeyboardEvent('keydown', {
      bubbles: true,
      cancelable: true,
      ...init
    })
  )
}

function makeSelectSpec(overrides: Partial<PaletteSpec> = {}): PaletteSpec {
  return {
    mode: PaletteMode.Select,
    entries: [
      { id: 'alpha', name: 'alpha' },
      { id: 'beta', name: 'beta' },
      { id: 'gamma', name: 'gamma' }
    ],
    onCommit: vi.fn(),
    ...overrides
  }
}

function makeMultiSpec(overrides: Partial<PaletteSpec> = {}): PaletteSpec {
  return {
    mode: PaletteMode.MultiSelect,
    entries: [
      { id: 'a', name: 'apple' },
      { id: 'b', name: 'banana' },
      { id: 'c', name: 'cherry' }
    ],
    onCommit: vi.fn(),
    ...overrides
  }
}

beforeEach(() => {
  __resetPaletteStackForTests()
})

afterEach(() => {
  __resetPaletteStackForTests()
})

describe('CommandPalette.vue', () => {
  it('renders nothing when the stack is empty', () => {
    const wrapper = mount(CommandPalette)

    expect(wrapper.find('[data-testid="palette-overlay"]').exists()).toBe(false)
  })

  it('renders the top of the stack', async() => {
    const wrapper = mount(CommandPalette)

    usePalette().open(makeSelectSpec({ title: 'root' }))
    await nextTick()

    expect(wrapper.find('[data-testid="palette-overlay"]').exists()).toBe(true)
    expect(wrapper.text()).toContain('root')
    expect(wrapper.find('[data-testid="palette-row-alpha"]').exists()).toBe(true)

    wrapper.unmount()
  })

  it('ArrowDown advances highlight; wraps at end', async() => {
    const wrapper = mount(CommandPalette)

    usePalette().open(makeSelectSpec())
    await nextTick()

    dispatchKey({ key: 'ArrowDown' })
    await nextTick()
    expect(wrapper.find('[data-testid="palette-row-beta"]').attributes('data-selected')).toBe('true')

    dispatchKey({ key: 'ArrowDown' })
    await nextTick()
    expect(wrapper.find('[data-testid="palette-row-gamma"]').attributes('data-selected')).toBe('true')

    dispatchKey({ key: 'ArrowDown' })
    await nextTick()
    expect(wrapper.find('[data-testid="palette-row-alpha"]').attributes('data-selected')).toBe('true')

    wrapper.unmount()
  })

  it('ArrowUp wraps from start to end', async() => {
    const wrapper = mount(CommandPalette)

    usePalette().open(makeSelectSpec())
    await nextTick()

    dispatchKey({ key: 'ArrowUp' })
    await nextTick()
    expect(wrapper.find('[data-testid="palette-row-gamma"]').attributes('data-selected')).toBe('true')

    wrapper.unmount()
  })

  it('Tab toggles ticked state on the highlighted row in multi-select; pins ticked rows to top', async() => {
    const wrapper = mount(CommandPalette)

    usePalette().open(makeMultiSpec())
    await nextTick()

    dispatchKey({ key: 'ArrowDown' })
    await nextTick()
    dispatchKey({ key: 'Tab' })
    await nextTick()

    expect(wrapper.find('[data-testid="palette-row-b"]').attributes('data-ticked')).toBe('true')

    const rows = wrapper.findAll('.palette-row')

    expect(rows[0]?.attributes('data-testid')).toBe('palette-row-b')

    wrapper.unmount()
  })

  it('Tab is a no-op in select mode', async() => {
    const wrapper = mount(CommandPalette)

    usePalette().open(makeSelectSpec())
    await nextTick()

    dispatchKey({ key: 'Tab' })
    await nextTick()

    expect(wrapper.find('[data-testid="palette-row-alpha"]').attributes('data-ticked')).toBe('false')

    wrapper.unmount()
  })

  it('filter query pins ticked rows at the top even when the query would exclude them', async() => {
    const wrapper = mount(CommandPalette)

    usePalette().open(
      makeMultiSpec({
        preseedActive: [{ id: 'a', name: 'apple' }]
      })
    )
    await nextTick()

    const input = wrapper.get('[data-testid="palette-input"]').element as HTMLInputElement

    input.value = 'cher'
    input.dispatchEvent(new Event('input'))
    await nextTick()

    const rows = wrapper.findAll('.palette-row')

    expect(rows[0]?.attributes('data-testid')).toBe('palette-row-a')
    expect(rows[1]?.attributes('data-testid')).toBe('palette-row-c')

    wrapper.unmount()
  })

  it('Ctrl+D fires onDelete on the highlighted row and keeps the palette mounted', async() => {
    const onDelete = vi.fn()
    const wrapper = mount(CommandPalette)

    usePalette().open(makeSelectSpec({ onDelete }))
    await nextTick()

    dispatchKey({ key: 'd', ctrlKey: true })
    await nextTick()

    expect(onDelete).toHaveBeenCalledWith({ id: 'alpha', name: 'alpha' })
    expect(wrapper.find('[data-testid="palette-overlay"]').exists()).toBe(true)

    wrapper.unmount()
  })

  it('Enter commits highlighted row in select mode and closes', async() => {
    const onCommit = vi.fn()
    const wrapper = mount(CommandPalette)

    usePalette().open(makeSelectSpec({ onCommit }))
    await nextTick()

    dispatchKey({ key: 'ArrowDown' })
    await nextTick()
    dispatchKey({ key: 'Enter' })
    await nextTick()

    expect(onCommit).toHaveBeenCalledWith([{ id: 'beta', name: 'beta' }], '')
    expect(wrapper.find('[data-testid="palette-overlay"]').exists()).toBe(false)

    wrapper.unmount()
  })

  it('Enter in multi-select commits every ticked entry', async() => {
    const onCommit = vi.fn()
    const wrapper = mount(CommandPalette)

    usePalette().open(makeMultiSpec({ onCommit, preseedActive: [{ id: 'a', name: 'apple' }] }))
    await nextTick()

    // Tick the currently highlighted row (first visible — which is the ticked 'a'
    // since ticked rows pin to top — so we navigate to 'c' and tick it).
    dispatchKey({ key: 'ArrowDown' })
    await nextTick()
    dispatchKey({ key: 'ArrowDown' })
    await nextTick()
    dispatchKey({ key: 'Tab' })
    await nextTick()
    dispatchKey({ key: 'Enter' })
    await nextTick()

    expect(onCommit).toHaveBeenCalledTimes(1)
    const arg = onCommit.mock.calls[0]?.[0] as { id: string }[]
    const ids = arg.map((e) => e.id).sort()

    expect(ids).toEqual(['a', 'c'])

    wrapper.unmount()
  })

  it('Esc pops one stack level, leaving the underlying spec rendered', async() => {
    const wrapper = mount(CommandPalette)
    const { open } = usePalette()

    open(makeSelectSpec({ title: 'root' }))
    open(
      makeSelectSpec({
        title: 'child',
        entries: [{ id: 'child-1', name: 'child-1' }]
      })
    )
    await nextTick()

    expect(wrapper.text()).toContain('child')

    dispatchKey({ key: 'Escape' })
    await nextTick()

    expect(wrapper.text()).toContain('root')
    expect(wrapper.find('[data-testid="palette-row-alpha"]').exists()).toBe(true)

    wrapper.unmount()
  })

  it('recursive open: onCommit that pushes a child renders the child after commit', async() => {
    const wrapper = mount(CommandPalette)
    const { open } = usePalette()

    open(
      makeSelectSpec({
        title: 'root',
        onCommit() {
          open(
            makeSelectSpec({
              title: 'child',
              entries: [{ id: 'child-only', name: 'child-only' }]
            })
          )
        }
      })
    )
    await nextTick()

    dispatchKey({ key: 'Enter' })
    await nextTick()

    expect(wrapper.find('[data-testid="palette-overlay"]').exists()).toBe(true)
    expect(wrapper.text()).toContain('child')
    expect(wrapper.find('[data-testid="palette-row-child-only"]').exists()).toBe(true)

    wrapper.unmount()
  })

  it('ticked glyphs do not render in select mode', async() => {
    const wrapper = mount(CommandPalette)

    usePalette().open(makeSelectSpec())
    await nextTick()

    expect(wrapper.find('.palette-tick').exists()).toBe(false)

    wrapper.unmount()
  })

  it('ticked glyphs render in multi-select mode', async() => {
    const wrapper = mount(CommandPalette)

    usePalette().open(makeMultiSpec())
    await nextTick()

    expect(wrapper.find('.palette-tick').exists()).toBe(true)

    wrapper.unmount()
  })
})
