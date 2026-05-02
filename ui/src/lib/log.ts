import { debug as pluginDebug, error as pluginError, info as pluginInfo, trace as pluginTrace, warn as pluginWarn } from '@tauri-apps/plugin-log'

/**
 * UI-side bridge into the daemon's tracing subscriber. Every call fans
 * through `@tauri-apps/plugin-log` → backend `log::Record` →
 * `tracing_log::LogTracer` (installed in `logging::init`) → the same
 * daily-rolled file at `$XDG_STATE_HOME/hyprpilot/logs/hyprpilot.log.*`
 * as the Rust side. One file, both tiers — K-283.
 *
 * Plain-browser soft-fail (vitest / `vite dev` without Tauri host): the
 * plugin's `invoke` rejects with "tauri host missing"; we swallow it so
 * UI code can call `log.*` unconditionally.
 *
 * Call-site is parsed from `new Error().stack` and appended to the
 * message — the backend's fmt layer already stamps the module target
 * via the `log` bridge, but the wrapper frame itself is what the
 * plugin sees. The embedded `at <file>:<line>` in the body points at
 * the real caller.
 */

type Fields = Record<string, unknown>

function callSite(): string {
  const stack = new Error().stack

  if (!stack) {
    return 'unknown'
  }
  const lines = stack.split('\n')

  for (const raw of lines) {
    const line = raw.trim()

    if (!line) {
      continue
    }

    if (line.includes('lib/log.ts') || line.includes('lib/log.js')) {
      continue
    }
    // Match "...(path:line:col)" and "... path:line:col" forms.
    const paren = /\(([^)]+)\)\s*$/.exec(line)

    if (paren) {
      return paren[1]
    }
    const bare = /\s([^\s]+:\d+:\d+)\s*$/.exec(line)

    if (bare) {
      return bare[1]
    }
  }

  return 'unknown'
}

function fmt(msg: string, fields?: Fields, err?: unknown): string {
  const parts = [msg]

  if (fields) {
    parts.push(JSON.stringify(fields))
  }

  if (err !== undefined) {
    if (err instanceof Error) {
      parts.push(`${err.message}\n${err.stack ?? ''}`)
    } else {
      parts.push(String(err))
    }
  }
  parts.push(`at ${callSite()}`)

  return parts.join(' ')
}

function mirrorToConsole(level: 'trace' | 'debug' | 'info' | 'warn' | 'error', msg: string, fields?: Fields, err?: unknown): void {
  if (!import.meta.env.DEV) {
    return
  }
  const args: unknown[] = [msg]

  if (fields) {
    args.push(fields)
  }

  if (err !== undefined) {
    args.push(err)
  }
  // eslint-disable-next-line no-console
  const sink = level === 'trace' ? console.debug : console[level]

  sink(...args)
}

function swallow(p: Promise<void>): void {
  p.catch(() => {
    // Host missing (plain browser / vitest jsdom) — intentional soft-fail.
  })
}

export const log = {
  trace: (msg: string, fields?: Fields): void => {
    mirrorToConsole('trace', msg, fields)
    swallow(pluginTrace(fmt(msg, fields)))
  },
  debug: (msg: string, fields?: Fields): void => {
    mirrorToConsole('debug', msg, fields)
    swallow(pluginDebug(fmt(msg, fields)))
  },
  info: (msg: string, fields?: Fields): void => {
    mirrorToConsole('info', msg, fields)
    swallow(pluginInfo(fmt(msg, fields)))
  },
  warn: (msg: string, fields?: Fields, err?: unknown): void => {
    mirrorToConsole('warn', msg, fields, err)
    swallow(pluginWarn(fmt(msg, fields, err)))
  },
  error: (msg: string, fields?: Fields, err?: unknown): void => {
    mirrorToConsole('error', msg, fields, err)
    swallow(pluginError(fmt(msg, fields, err)))
  }
}
