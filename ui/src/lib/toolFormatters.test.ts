import { describe, expect, it } from 'vitest'

import { ToolKind, ToolState } from '@components'
import { baseRegistry, extendRegistry, formatToolBody, formatToolCall, registries, resolveRegistry, shortHeader } from '@lib'

import type { ToolCallView } from '../composables/useTools'

function view(overrides: Partial<ToolCallView> = {}): ToolCallView {
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

describe('formatToolCall — base registry', () => {
  it('formats Read with a single path arg and a line-range detail', () => {
    const chip = formatToolCall(view({ title: 'Read', rawInput: { file_path: '/repo/src/app.ts', offset: 10, limit: 20 }, status: 'completed' }))
    expect(chip.label).toBe('R')
    expect(chip.arg).toContain('app.ts')
    expect(chip.detail).toBe('lines 10..30')
    expect(chip.state).toBe(ToolState.Done)
    expect(chip.kind).toBe(ToolKind.Read)
  })

  it('formats Write with a char-count stat', () => {
    const chip = formatToolCall(view({ title: 'Write', rawInput: { file_path: '/tmp/out.md', content: 'hello world' }, status: 'in_progress' }))
    expect(chip.label).toBe('\u21F2')
    expect(chip.arg).toBe('/tmp/out.md')
    expect(chip.stat).toBe('11 chars')
    expect(chip.state).toBe(ToolState.Running)
    expect(chip.kind).toBe(ToolKind.Write)
  })

  it('formats Edit with a replace-all detail', () => {
    const chip = formatToolCall(view({ title: 'Edit', rawInput: { file_path: '/a/b/c/d.ts', old_string: 'x', new_string: 'y', replace_all: true } }))
    expect(chip.label).toBe('\u270E')
    expect(chip.arg).toContain('d.ts')
    expect(chip.detail).toBe('replace all')
    expect(chip.kind).toBe(ToolKind.Write)
  })

  it('formats MultiEdit with an edit-count stat', () => {
    const chip = formatToolCall(view({ title: 'MultiEdit', rawInput: { file_path: '/a/b.ts', edits: [{ old_string: 'a', new_string: 'b' }, { old_string: 'c', new_string: 'd' }] } }))
    expect(chip.label).toBe('\u270E')
    expect(chip.stat).toBe('2 edits')
    expect(chip.kind).toBe(ToolKind.Write)
  })

  it('formats Bash with the command as the arg', () => {
    const chip = formatToolCall(view({ title: 'Bash', rawInput: { command: 'pnpm test', description: 'run the suite', run_in_background: true } }))
    expect(chip.label).toBe('$')
    expect(chip.arg).toBe('pnpm test')
    expect(chip.detail).toBe('run the suite (background)')
    expect(chip.kind).toBe(ToolKind.Bash)
  })

  it('formats BashOutput with shell id + filter (distinct from formatBash)', () => {
    const chip = formatToolCall(view({ title: 'BashOutput', rawInput: { bash_id: 'sh-9', filter: 'ERROR' } }))
    expect(chip.label).toBe('$\u00B7')
    expect(chip.arg).toBe('sh-9')
    expect(chip.detail).toBe('filter ERROR')
    expect(chip.kind).toBe(ToolKind.Bash)
  })

  it('formats KillShell with shell id only', () => {
    const chip = formatToolCall(view({ title: 'KillShell', rawInput: { shell_id: 'sh-9' } }))
    expect(chip.label).toBe('$\u2715')
    expect(chip.arg).toBe('sh-9')
    expect(chip.detail).toBeUndefined()
    expect(chip.kind).toBe(ToolKind.Bash)
  })

  it('formats Grep with default path `.` when omitted and surfaces -i / -n flags', () => {
    const chip = formatToolCall(view({ title: 'Grep', rawInput: { pattern: 'TODO', '-i': true, '-n': true } }))
    expect(chip.label).toBe('/')
    expect(chip.arg).toBe('TODO')
    expect(chip.detail).toContain('in .')
    expect(chip.detail).toContain('-i')
    expect(chip.detail).toContain('-n')
    expect(chip.kind).toBe(ToolKind.Search)
  })

  it('formats Grep with pattern as arg and path + glob in detail', () => {
    const chip = formatToolCall(view({ title: 'Grep', rawInput: { pattern: 'TODO', path: 'src/', glob: '*.ts', output_mode: 'content' } }))
    expect(chip.label).toBe('/')
    expect(chip.arg).toBe('TODO')
    expect(chip.detail).toContain('in src/')
    expect(chip.detail).toContain('glob=*.ts')
    expect(chip.detail).toContain('mode=content')
    expect(chip.kind).toBe(ToolKind.Search)
  })

  it('formats Glob with pattern + path', () => {
    const chip = formatToolCall(view({ title: 'Glob', rawInput: { pattern: '**/*.vue', path: 'ui/src' } }))
    expect(chip.label).toBe('\u25B3')
    expect(chip.arg).toBe('**/*.vue')
    expect(chip.detail).toBe('in ui/src')
    expect(chip.kind).toBe(ToolKind.Search)
  })

  it('formats Task with subagent + description + prompt (all in detail)', () => {
    const chip = formatToolCall(view({ title: 'Task', rawInput: { subagent_type: 'general-purpose', description: 'investigate bug', prompt: 'find the flaky test' } }))
    expect(chip.label).toBe('\u203A_')
    expect(chip.arg).toBe('general-purpose')
    expect(chip.detail).toContain('investigate bug')
    expect(chip.detail).toContain('find the flaky test')
    expect(chip.kind).toBe(ToolKind.Agent)
  })

  it('formats WebFetch with url as arg and prompt as detail', () => {
    const chip = formatToolCall(view({ title: 'WebFetch', rawInput: { url: 'https://example.com', prompt: 'summarise' } }))
    expect(chip.label).toBe('\u21E9')
    expect(chip.arg).toBe('https://example.com')
    expect(chip.detail).toBe('summarise')
  })

  it('formats WebSearch with allowed + blocked domain lists', () => {
    const chip = formatToolCall(view({ title: 'WebSearch', rawInput: { query: 'vite config', allowed_domains: ['vitejs.dev'], blocked_domains: ['bad.example'] } }))
    expect(chip.label).toBe('?')
    expect(chip.arg).toBe('vite config')
    expect(chip.detail).toContain('allowed: vitejs.dev')
    expect(chip.detail).toContain('blocked: bad.example')
    expect(chip.kind).toBe(ToolKind.Search)
  })

  it('formats WebSearch with query only when no domain lists are set', () => {
    const chip = formatToolCall(view({ title: 'WebSearch', rawInput: { query: 'vite config' } }))
    expect(chip.arg).toBe('vite config')
    expect(chip.detail).toBeUndefined()
  })

  it('formats Terminal with id as arg + command detail', () => {
    const chip = formatToolCall(view({ title: 'Terminal', rawInput: { terminal_id: 't-9', command: 'npm run dev' } }))
    expect(chip.label).toBe('\u203A_')
    expect(chip.arg).toBe('t-9')
    expect(chip.detail).toBe('npm run dev')
    expect(chip.kind).toBe(ToolKind.Terminal)
  })

  it('formats NotebookEdit with cell id in detail', () => {
    const chip = formatToolCall(view({ title: 'NotebookEdit', rawInput: { notebook_path: '/n.ipynb', cell_id: 'c1', edit_mode: 'replace' } }))
    expect(chip.label).toBe('\u270Enb')
    expect(chip.detail).toContain('cell=c1')
    expect(chip.detail).toContain('mode=replace')
    expect(chip.kind).toBe(ToolKind.Write)
  })

  it('formats TodoWrite with item count and status breakdown', () => {
    const chip = formatToolCall(view({
      title: 'TodoWrite',
      rawInput: {
        todos: [
          { content: 'a', status: 'pending' },
          { content: 'b', status: 'completed' },
          { content: 'c', status: 'completed' }
        ]
      }
    }))
    expect(chip.label).toBe('\u2630')
    expect(chip.arg).toBe('3 items')
    expect(chip.detail).toContain('pending:1')
    expect(chip.detail).toContain('completed:2')
    expect(chip.kind).toBe(ToolKind.Think)
  })

  it('resolves the `todo` alias to TodoWrite', () => {
    const chip = formatToolCall(view({ title: 'todo', rawInput: { todos: [{ content: 'x', status: 'pending' }] } }))
    expect(chip.label).toBe('\u2630')
    expect(chip.arg).toBe('1 item')
  })

  it('formats PlanEnter (ExitPlanMode alias routes to PlanExit)', () => {
    const planEnter = formatToolCall(view({ title: 'EnterPlanMode', rawInput: { plan: 'step one — investigate' } }))
    expect(planEnter.label).toBe('\u25CE')
    expect(planEnter.arg).toBe('step one — investigate')
    expect(planEnter.kind).toBe(ToolKind.Think)

    const planExit = formatToolCall(view({ title: 'ExitPlanMode', rawInput: { plan: 'plan finalised' } }))
    expect(planExit.label).toBe('\u25CE\u00B7')
    expect(planExit.arg).toBe('plan finalised')
    expect(planExit.kind).toBe(ToolKind.Think)
  })

  it('normalises snake_case vs camelCase argument keys for Read', () => {
    const snake = formatToolCall(view({ title: 'read', rawInput: { file_path: '/a/b.ts' } }))
    const camel = formatToolCall(view({ title: 'Read', rawInput: { filePath: '/a/b.ts' } }))
    expect(camel.arg).toBe(snake.arg)
    expect(camel.label).toBe(snake.label)
  })

  it('falls back to a server + tool-name chip for unknown mcp__ tools', () => {
    const chip = formatToolCall(view({ title: 'mcp__playwright__browser_navigate', rawInput: { url: 'https://example.com' } }))
    expect(chip.label).toBe('playwright')
    expect(chip.arg).toContain('browser navigate')
    expect(chip.kind).toBe(ToolKind.Acp)
  })

  it('falls back through shortHeader + summariseArgs for unknown non-MCP tools', () => {
    const chip = formatToolCall(view({ title: 'somecommand', rawInput: { query: 'hello' }, status: 'completed' }))
    expect(chip.label).toBe('Somecommand')
    expect(chip.arg).toBe('hello')
    expect(chip.kind).toBe(ToolKind.Acp)
    expect(chip.state).toBe(ToolState.Done)
  })
})

describe('shortHeader', () => {
  it('returns the registered glyph for known tools', () => {
    expect(shortHeader('Read')).toBe('R')
    expect(shortHeader('edit')).toBe('\u270E')
    expect(shortHeader('BASH')).toBe('$')
  })

  it('title-cases the leaf name for unknown mcp tools', () => {
    expect(shortHeader('mcp__foo__bar_baz')).toBe('Bar baz')
  })

  it('title-cases the full canonical leaf for unknown built-ins (matches Python)', () => {
    expect(shortHeader('somecommand')).toBe('Somecommand')
    expect(shortHeader('bash_output_foo')).toBe('Bash output foo')
  })

  it('returns a bullet for empty input', () => {
    expect(shortHeader('')).toBe('\u00B7')
  })
})

describe('resolveRegistry', () => {
  it('returns the base registry when no provider is given', () => {
    expect(resolveRegistry()).toBe(baseRegistry)
  })

  it('returns an adapter-specific registry when provider is known', () => {
    for (const provider of ['acp-claude-code', 'acp-codex', 'acp-opencode']) {
      const r = resolveRegistry(provider)
      expect(r).toBe(registries[provider])
      expect(r.formatters.bash).toBeDefined()
    }
  })

  it('falls through to base when provider is unknown', () => {
    expect(resolveRegistry('acp-does-not-exist')).toBe(baseRegistry)
  })

  it('round-trips formatToolCall through a named provider registry', () => {
    const chip = formatToolCall(view({ title: 'Bash', rawInput: { command: 'ls' } }), 'acp-claude-code')
    expect(chip.label).toBe('$')
    expect(chip.arg).toBe('ls')
    expect(chip.kind).toBe(ToolKind.Bash)
  })
})

describe('extendRegistry', () => {
  it('overrides individual formatters while inheriting the rest from base', () => {
    const custom = extendRegistry(baseRegistry, {
      formatters: {
        bash: ({ args, state }) => ({
          label: 'CUSTOM-BASH',
          arg: typeof args.command === 'string' ? args.command : undefined,
          state,
          kind: ToolKind.Bash
        })
      }
    })
    const bashChip = custom.formatters.bash!({ name: 'bash', rawName: 'Bash', args: { command: 'x' }, state: ToolState.Done })
    expect(bashChip.label).toBe('CUSTOM-BASH')

    // The base `read` formatter still resolves through the extended registry.
    expect(custom.formatters.read).toBe(baseRegistry.formatters.read)
    expect(custom.fallback).toBe(baseRegistry.fallback)
  })

  it('merges short-headers and aliases additively', () => {
    const custom = extendRegistry(baseRegistry, {
      shortHeaders: { custom_tool: '★' },
      aliases: { customtool: 'custom_tool' }
    })
    expect(custom.shortHeaders.read).toBe('R')
    expect(custom.shortHeaders.custom_tool).toBe('★')
    expect(custom.aliases.customtool).toBe('custom_tool')
  })
})

describe('formatToolBody', () => {
  it('throws in dev builds to surface premature integration', () => {
    expect(() => formatToolBody(view({ title: 'Read', rawInput: { file_path: '/x' } }))).toThrow(/not implemented yet/)
  })
})
