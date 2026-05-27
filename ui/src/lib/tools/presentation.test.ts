import { describe, expect, it } from 'vitest'

import { presentationFor } from './presentation'
import { AdapterId, PermissionUi, ToolKind } from '@constants/ui'

describe('presentationFor', () => {
  it('routes a known wire name through the adapter override table', () => {
    const p = presentationFor(ToolKind.Edit, AdapterId.ClaudeCode, 'Edit', undefined)

    expect(p.permissionUi).toBe(PermissionUi.Modal)
  })

  it('falls back to the kind default when no override matches', () => {
    const p = presentationFor(ToolKind.Read, AdapterId.ClaudeCode, 'TotallyUnknownTool', undefined)

    expect(p.permissionUi).toBe(PermissionUi.Row)
  })

  it('routes MCP tools through structured toolKind instead of wire-name prefixes', () => {
    const p = presentationFor(
      {
        type: 'mcp',
        server: 'hyprpilot',
        tool: 'read_skill'
      },
      AdapterId.ClaudeCode,
      'Read skill',
      undefined
    )

    expect(p.permissionUi).toBe(PermissionUi.Row)
  })

  /**
   * Regression: claude-agent-acp ≥0.32 emits the plan-exit permission
   * with a prose title (`Ready to code?`, `EnterPlanMode`, etc.) that
   * doesn't snake-case to any registered override. Discriminator is
   * the `rawInput.plan` string — same signal the daemon-side
   * PlanExitFormatter matcher uses. Must route to Modal even when
   * `kind` is the catch-all `other`.
   */
  it('routes plan-exit (prose title + rawInput.plan) to Modal via the rawInput fallback', () => {
    const p = presentationFor(ToolKind.Other, AdapterId.ClaudeCode, 'Ready to code?', {
      plan: '# Plan\n\n- step 1'
    })

    expect(p.permissionUi).toBe(PermissionUi.Modal)
  })

  /**
   * Regression: the agent-registry's `agents/list` lookup is
   * lazy-async, so a permission can land before `adapterFor()`
   * resolves — `adapter` arrives as `undefined`. The plan-exit
   * fallback must still fire so the user gets the modal-class UI
   * instead of an inline row.
   */
  it('routes plan-exit to Modal even when the adapter id is unknown', () => {
    const p = presentationFor(ToolKind.Other, undefined, 'EnterPlanMode', {
      plan: '# Plan'
    })

    expect(p.permissionUi).toBe(PermissionUi.Modal)
  })

  it('does not treat an empty rawInput.plan as plan-exit', () => {
    const p = presentationFor(ToolKind.Other, AdapterId.ClaudeCode, 'switch_mode', { plan: '' })

    // Override table still hits `switch_mode` directly here.
    expect(p.permissionUi).toBe(PermissionUi.Modal)
  })

  it('falls through to Other when rawInput.plan is missing and wireName does not match', () => {
    const p = presentationFor(ToolKind.Other, AdapterId.ClaudeCode, 'Mystery Tool', { other: 'data' })

    expect(p.permissionUi).toBe(PermissionUi.Row)
  })
})
