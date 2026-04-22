import type { FullConfig } from '@playwright/test'
import { spawn, type ChildProcess } from 'node:child_process'
import { access, constants, mkdir, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'

declare global {
  var __HYPRPILOT_E2E__: { child: ChildProcess; socket: string; runtimeDir: string } | undefined
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

async function waitForSocket(socketPath: string, timeoutMs: number): Promise<void> {
  const deadline = Date.now() + timeoutMs

  while (Date.now() < deadline) {
    try {
      await access(socketPath, constants.F_OK)

      return
    } catch {
      await sleep(100)
    }
  }
  throw new Error(`Timed out waiting for bridge socket ${socketPath}`)
}

export default async function globalSetup(config: FullConfig): Promise<void> {
  const meta = config.metadata as { binary: string; socket: string; configOverride: string; repoRoot: string }

  const runtimeDir = await mkdir(path.join(tmpdir(), `hyprpilot-e2e-${process.pid}`), { recursive: true }).then(
    (created) => created ?? path.join(tmpdir(), `hyprpilot-e2e-${process.pid}`)
  )
  const stateDir = path.join(runtimeDir, 'state')

  await mkdir(stateDir, { recursive: true })

  const env: NodeJS.ProcessEnv = {
    ...process.env,
    HYPRPILOT_CONFIG: meta.configOverride,
    HYPRPILOT_SOCKET: path.join(runtimeDir, 'hyprpilot.sock'),
    XDG_RUNTIME_DIR: runtimeDir,
    XDG_STATE_HOME: stateDir,
    HYPRPILOT_LOG_LEVEL: 'debug'
  }

  const child = spawn(meta.binary, ['daemon'], {
    cwd: meta.repoRoot,
    env,
    stdio: ['ignore', 'pipe', 'pipe'],
    detached: false
  })

  const logPath = path.join(runtimeDir, 'daemon.log')
  const logLines: string[] = []

  child.stdout?.on('data', (b) => logLines.push(b.toString()))
  child.stderr?.on('data', (b) => logLines.push(b.toString()))
  child.on('exit', (code, signal) => {
    logLines.push(`[exit] code=${code} signal=${signal}\n`)
    writeFile(logPath, logLines.join('')).catch(() => undefined)
  })

  try {
    await waitForSocket(meta.socket, 15_000)
  } catch(err) {
    child.kill('SIGKILL')
    await writeFile(logPath, logLines.join(''))
    throw new Error(`daemon failed to expose ${meta.socket}; see ${logPath}\n---\n${logLines.join('')}`, { cause: err })
  }

  globalThis.__HYPRPILOT_E2E__ = {
    child,
    socket: meta.socket,
    runtimeDir
  }
}
