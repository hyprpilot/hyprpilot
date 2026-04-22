import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import PermissionPrompt from '@components/PermissionPrompt.vue'
import { EventKind, type PermissionRequestEvent } from '@composables/useAcpAgent'

const request: PermissionRequestEvent = {
  kind: EventKind.PermissionRequest,
  agent_id: 'claude-code',
  session_id: 'sess-42',
  options: [
    { option_id: 'allow-once', name: 'Allow once', kind: 'allow_once' },
    { option_id: 'reject', name: 'Reject', kind: 'reject_once' }
  ]
}

describe('PermissionPrompt.vue', () => {
  it('does not render when no request is bound', () => {
    const wrapper = mount(PermissionPrompt)

    expect(wrapper.find('[data-testid="permission-prompt"]').exists()).toBe(false)
  })

  it('renders the session id + every option when a request is bound', () => {
    const wrapper = mount(PermissionPrompt, { props: { request } })

    const aside = wrapper.get('[data-testid="permission-prompt"]')
    expect(aside.text()).toContain('sess-42')
    expect(aside.text()).toContain('allow-once')
    expect(aside.text()).toContain('Allow once')
    expect(aside.text()).toContain('reject')
  })
})
