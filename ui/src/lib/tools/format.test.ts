import { describe, expect, it } from 'vitest'

import { PermissionUi, ToolState, ToolType } from '@components'
import type { WireToolCall } from '@interfaces/ui'
import { format } from '@lib'

function call(overrides: Partial<WireToolCall> = {}): WireToolCall {
  return {
    id: 'tc-1',
    sessionId: 's-1',
    toolCallId: 'tc-1',
    content: [],
    createdAt: 1,
    updatedAt: 1,
    ...overrides
  }
}

describe('format()', () => {
  it('routes Bash to the bash formatter and composes the title', () => {
    const view = format(
      call({
        title: 'Bash',
        rawInput: { command: 'pnpm test', description: 'run the suite' },
        status: 'completed'
      })
    )

    expect(view.type).toBe(ToolType.Bash)
    expect(view.title).toBe('bash · pnpm')
    expect(view.description).toBe('run the suite')
    expect(view.fields).toEqual([{ label: 'command', value: 'pnpm test' }])
    expect(view.state).toBe(ToolState.Done)
  })

  it('flags background bash with `(background)`', () => {
    const view = format(call({ title: 'Bash', rawInput: { command: 'sleep 5', isbackground: true } }))

    expect(view.title).toContain('(background)')
  })

  it('routes BashOutput through the bash formatter (paired family)', () => {
    const view = format(call({ title: 'BashOutput', rawInput: { bash_id: 'sh-9', filter: 'ERROR' } }))

    expect(view.type).toBe(ToolType.Bash)
    expect(view.title).toContain('tail')
    expect(view.title).toContain('sh-9')
    expect(view.title).toContain('ERROR')
  })

  it('routes KillShell to the kill-shell formatter', () => {
    const view = format(call({ title: 'KillShell', rawInput: { shell_id: 'sh-9' } }))

    expect(view.type).toBe(ToolType.KillShell)
    expect(view.title).toContain('sh-9')
  })

  it('routes Read to the read formatter and composes path + line range', () => {
    const view = format(
      call({
        title: 'Read',
        rawInput: {
          file_path: '/a/b/c/d.ts',
          offset: 10,
          limit: 20
        },
        status: 'in_progress'
      })
    )

    expect(view.type).toBe(ToolType.Read)
    expect(view.title).toContain('d.ts')
    expect(view.title).toContain('10..30')
    expect(view.state).toBe(ToolState.Running)
  })

  it('routes Write with a char-count stat', () => {
    const view = format(call({ title: 'Write', rawInput: { file_path: '/tmp/out.md', content: 'hello world' } }))

    expect(view.type).toBe(ToolType.Write)
    expect(view.title).toContain('out.md')
    expect(view.stat).toBe('11 chars')
  })

  it('routes Edit and surfaces (replace all)', () => {
    const view = format(call({ title: 'Edit', rawInput: { file_path: '/a/b/c/d.ts', replace_all: true } }))

    expect(view.type).toBe(ToolType.Edit)
    expect(view.title).toContain('d.ts')
    expect(view.title).toContain('(replace all)')
  })

  it('routes MultiEdit with an edit-count stat', () => {
    const view = format(
      call({
        title: 'MultiEdit',
        rawInput: {
          file_path: '/a/b.ts',
          edits: [
            { old_string: 'a', new_string: 'b' },
            { old_string: 'c', new_string: 'd' }
          ]
        }
      })
    )

    expect(view.type).toBe(ToolType.MultiEdit)
    expect(view.stat).toBe('2 edits')
  })

  it('routes NotebookEdit with cell + mode in the title', () => {
    const view = format(
      call({
        title: 'NotebookEdit',
        rawInput: {
          notebook_path: '/n.ipynb',
          cell_id: 'c1',
          edit_mode: 'replace'
        }
      })
    )

    expect(view.type).toBe(ToolType.NotebookEdit)
    expect(view.title).toContain('cell c1')
    expect(view.title).toContain('replace')
  })

  it('routes Grep with the pattern + path bits', () => {
    const view = format(
      call({
        title: 'Grep',
        rawInput: {
          pattern: 'TODO',
          path: 'src/',
          glob: '*.ts'
        }
      })
    )

    expect(view.type).toBe(ToolType.Grep)
    expect(view.title).toContain('TODO')
    expect(view.title).toContain('in src/')
    expect(view.title).toContain('glob=*.ts')
  })

  it('routes Glob with pattern + path', () => {
    const view = format(call({ title: 'Glob', rawInput: { pattern: '**/*.vue', path: 'ui/src' } }))

    expect(view.type).toBe(ToolType.Glob)
    expect(view.title).toContain('**/*.vue')
    expect(view.title).toContain('ui/src')
  })

  it('routes ToolSearch with query', () => {
    const view = format(call({ title: 'ToolSearch', rawInput: { query: 'browser_navigate', maxresults: 10 } }))

    expect(view.type).toBe(ToolType.ToolSearch)
    expect(view.title).toContain('browser_navigate')
    expect(view.title).toContain('max 10')
  })

  it('routes WebFetch surfacing host + prompt', () => {
    const view = format(call({ title: 'WebFetch', rawInput: { url: 'https://example.com/path', prompt: 'summarise' } }))

    expect(view.type).toBe(ToolType.WebFetch)
    expect(view.title).toContain('example.com')
    expect(view.title).toContain('summarise')
  })

  it('routes WebSearch with allowed/blocked domain lists', () => {
    const view = format(
      call({
        title: 'WebSearch',
        rawInput: {
          query: 'vite config',
          allowed_domains: ['vitejs.dev'],
          blocked_domains: ['bad.example']
        }
      })
    )

    expect(view.type).toBe(ToolType.WebSearch)
    expect(view.title).toContain('vite config')
    expect(view.title).toContain('allowed: vitejs.dev')
    expect(view.title).toContain('blocked: bad.example')
  })

  it('routes ExitPlanMode and declares permissionUi: Modal with the plan as description', () => {
    const view = format(call({ title: 'ExitPlanMode', rawInput: { plan: '# Plan\n\n- step one\n- step two' } }))

    expect(view.type).toBe(ToolType.PlanExit)
    expect(view.permissionUi).toBe(PermissionUi.Modal)
    expect(view.description).toContain('step one')
  })

  it('routes TodoWrite with item count + status breakdown stat', () => {
    const view = format(
      call({
        title: 'TodoWrite',
        rawInput: {
          todos: [
            { content: 'a', status: 'pending' },
            { content: 'b', status: 'completed' },
            { content: 'c', status: 'completed' }
          ]
        }
      })
    )

    expect(view.type).toBe(ToolType.Todo)
    expect(view.title).toContain('3 items')
    expect(view.stat).toContain('pending:1')
    expect(view.stat).toContain('completed:2')
  })

  it('routes Skill with slug as title suffix', () => {
    const view = format(call({ title: 'Skill', rawInput: { skill: 'superpowers:using-superpowers' } }))

    expect(view.type).toBe(ToolType.Skill)
    expect(view.title).toContain('superpowers:using-superpowers')
  })

  it('routes Task with subagent + description in the title', () => {
    const view = format(call({ title: 'Task', rawInput: { subagent_type: 'general-purpose', description: 'investigate bug' } }))

    expect(view.type).toBe(ToolType.Task)
    expect(view.title).toContain('general-purpose')
    expect(view.title).toContain('investigate bug')
  })

  it('routes Terminal with id + command', () => {
    const view = format(call({ title: 'Terminal', rawInput: { terminal_id: 't-9', command: 'npm run dev' } }))

    expect(view.type).toBe(ToolType.Terminal)
    expect(view.title).toContain('#t-9')
    expect(view.title).toContain('npm run dev')
  })

  it('routes mcp__ tools to the MCP formatter with structured fields', () => {
    const view = format(call({ title: 'mcp__playwright__browser_navigate', rawInput: { url: 'https://example.com' } }))

    expect(view.type).toBe(ToolType.Mcp)
    expect(view.title).toContain('playwright')
    expect(view.title).toContain('browser navigate')
    expect(view.fields).toEqual([{ label: 'url', value: 'https://example.com' }])
  })

  it('falls back to the wire name with no synthetic Other label', () => {
    const view = format(
      call({
        title: 'curl-something',
        kind: 'execute',
        rawInput: { command: 'curl example.com' },
        status: 'completed'
      })
    )

    expect(view.type).toBe(ToolType.Other)
    expect(view.title).toBe('curl-something')
  })

  it('canonicalises camelCase / snake_case wire names through the same formatter', () => {
    const a = format(call({ title: 'BashOutput', rawInput: { bash_id: 'sh-1' } }))
    const b = format(call({ title: 'bash_output', rawInput: { bash_id: 'sh-1' } }))

    expect(a.type).toBe(b.type)
    expect(a.title).toBe(b.title)
  })
})
