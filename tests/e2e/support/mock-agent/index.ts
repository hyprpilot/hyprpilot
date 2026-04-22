#!/usr/bin/env bun
// Scripted ACP-speaking child process for tests/e2e/.
//
// Why a scripted stub over a real vendor runtime: `bunx
// @zed-industries/claude-code-acp` (etc.) hits the network, needs
// credentials, and isn't reproducible. This stub replays a fixed
// transcript over stdio JSON-RPC so the daemon's live-session bridge
// (K-240) has a deterministic counterpart in CI.
//
// Protocol: minimal subset of agent-client-protocol needed for the
// initialize / session.new / session.prompt / session.cancel
// loop. Extend per-test via `HYPRPILOT_MOCK_SCRIPT=<path>`; default
// replies with a single assistant message.
import { readFileSync } from 'node:fs'
import { createInterface } from 'node:readline'

interface ScriptStep {
  updates?: Record<string, unknown>[]
  stop_reason?: string
  request_permission?: {
    request_id: string
    tool_name: string
    options: Record<string, unknown>[]
  }
}

interface Script {
  prompts: ScriptStep[]
}

interface RpcMessage {
  jsonrpc?: '2.0'
  id?: number | string | null
  method?: string
  params?: Record<string, unknown>
}

const log = (msg: string, extra: Record<string, unknown> = {}): void => {
  process.stderr.write(
    `${JSON.stringify({
      ts: Date.now(),
      msg,
      ...extra
    })}\n`
  )
}

const scriptPath = process.env.HYPRPILOT_MOCK_SCRIPT
const script: Script = scriptPath
  ? (JSON.parse(readFileSync(scriptPath, 'utf8')) as Script)
  : {
    prompts: [
      {
        updates: [{ kind: 'message.assistant', text: 'mock-agent: scripted reply' }],
        stop_reason: 'end_turn'
      }
    ]
  }

let promptCursor = 0
const sessions = new Set<string>()

function reply(id: number | string | null | undefined, result: unknown): void {
  process.stdout.write(
    `${JSON.stringify({
      jsonrpc: '2.0',
      id,
      result
    })}\n`
  )
}

function notify(method: string, params: Record<string, unknown>): void {
  process.stdout.write(
    `${JSON.stringify({
      jsonrpc: '2.0',
      method,
      params
    })}\n`
  )
}

function errorReply(id: number | string | null, code: number, message: string): void {
  process.stdout.write(
    `${JSON.stringify({
      jsonrpc: '2.0',
      id,
      error: { code, message }
    })}\n`
  )
}

const rl = createInterface({ input: process.stdin, crlfDelay: Infinity })

rl.on('line', (line: string) => {
  if (!line.trim()) {
    return
  }

  let msg: RpcMessage

  try {
    msg = JSON.parse(line) as RpcMessage
  } catch(err) {
    log('parse_error', { line, err: String(err) })

    return
  }

  log('rx', { method: msg.method, id: msg.id })

  switch (msg.method) {
    case 'initialize':
      reply(msg.id, {
        protocol_version: 'v1',
        server_info: { name: 'hyprpilot-mock-agent', version: '0.0.0' },
        capabilities: { permissions: true, terminal: false }
      })

      break

    case 'session/new': {
      const sessionId = `mock-session-${sessions.size + 1}`

      sessions.add(sessionId)
      reply(msg.id, { session_id: sessionId })

      break
    }

    case 'session/prompt': {
      const step = script.prompts[Math.min(promptCursor, script.prompts.length - 1)]

      promptCursor += 1
      const sid = msg.params?.session_id

      for (const update of step.updates ?? []) {
        notify('session/update', { session_id: sid, update })
      }

      if (step.request_permission) {
        notify('session/request_permission', {
          session_id: sid,
          request_id: step.request_permission.request_id,
          tool_name: step.request_permission.tool_name,
          options: step.request_permission.options
        })
      }
      reply(msg.id, { stop_reason: step.stop_reason ?? 'end_turn' })

      break
    }

    case 'session/cancel':
      reply(msg.id, { cancelled: true })

      break

    default:
      errorReply(msg.id ?? null, -32601, `method not found: ${msg.method}`)
  }
})

rl.on('close', () => {
  log('stdin closed, exiting')
  process.exit(0)
})

log('mock-agent ready')
