export default async function globalTeardown(): Promise<void> {
  const handle = globalThis.__HYPRPILOT_E2E__
  if (!handle?.child || handle.child.exitCode != null) return

  handle.child.kill('SIGTERM')

  await new Promise<void>((resolve) => {
    const timeout = setTimeout(() => {
      handle.child.kill('SIGKILL')
      resolve()
    }, 3000)
    handle.child.once('exit', () => {
      clearTimeout(timeout)
      resolve()
    })
  })
}
