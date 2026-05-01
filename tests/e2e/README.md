# End-to-end tests (`tests/e2e/`)

Playwright-driven end-to-end suite for the hyprpilot webview, running in two modes behind a shared fixture:

1. **`browser` (default)** — Vite dev server + Chromium + mocked Tauri IPC via [`@srsholmes/tauri-playwright`](https://github.com/srsholmes/tauri-playwright). Covers the UI-layer contract (render, IPC call-shape, event fan-out) without touching the real daemon. Fast, reproducible, runs in CI.
2. **`tauri`** — same bridge library, but the bridge's unix-socket plugin proxies every command to the real WebKitGTK webview via `webview.eval()`. Currently **not run by default** — see "Known limits" below.

The Rust bridge plugin, `playwright:default` capability, mock agent, and layered TOML override are all wired in either mode: flipping between them is `HYPRPILOT_E2E_MODE=tauri` once the WebKitGTK stall (documented below) clears.

## Install

```sh
task install
pnpm --filter hyprpilot-ui exec playwright install chromium
```

## Run (browser mode — default)

```sh
task test:e2e
```

Launches Vite on `http://127.0.0.1:1420`, runs every spec under `specs/` against the page with Tauri IPC mocks from `fixtures/tauri.ts`.

## Run (tauri-bridge mode)

```sh
task build:e2e              # `cargo build --features e2e-testing` + UI build
HYPRPILOT_E2E_MODE=tauri \
  pnpm --filter hyprpilot-e2e test
```

`mode: tauri` replaces the `webServer` lifecycle with `globalSetup` / `globalTeardown` hooks that spawn the daemon with `HYPRPILOT_CONFIG` pointing at `fixtures/e2e-config.toml` and tear it down on exit. Override the binary / socket / config path with env vars:

- `HYPRPILOT_E2E_BINARY` — defaults to `target/debug/hyprpilot`.
- `HYPRPILOT_E2E_SOCKET` — defaults to `/tmp/tauri-playwright.sock`.
- `HYPRPILOT_E2E_CONFIG` — defaults to `fixtures/e2e-config.toml` (mock-agent). Swap to `fixtures/live-config.toml` to drive the real `claude-code-acp` adapter against the haiku model — `task test:e2e:live` is the wired shortcut.

## Run (tauri-bridge mode against the live agent)

```sh
task test:e2e:live          # cargo build + ui build + Playwright in tauri mode
                            # with HYPRPILOT_E2E_CONFIG=fixtures/live-config.toml
```

`live-config.toml` keys the `claude-code` agent against the haiku model and routes through `bunx --bun @agentclientprotocol/claude-agent-acp`. Network-bound (the bunx fetch hits the registry on first run); not suitable for CI without a pre-warmed cache.

## Mock agent

`support/mock-agent/` ships a scripted ACP-speaking Node process so the Tauri mode never depends on `bunx @zed-industries/claude-code-acp` (or any network-bound vendor runtime). `e2e-config.toml` points the daemon at this binary via `command = "node"` + `args = ["tests/e2e/support/mock-agent/index.mjs"]` so once the live ACP bridge (K-240 follow-up) wires real sessions every spec sees deterministic replies.

`HYPRPILOT_MOCK_SCRIPT=<file.json>` swaps the scripted transcript per spec; the default bundled script emits a single assistant message and ignores `session/cancel`.

## Known limits

- **WebKitGTK eval stall.** `tauri-plugin-playwright 0.2.2` against `webkit2gtk-4.1` (the GTK3 binding Tauri 2.10 still links on Linux) never resolves the `webview.eval()` oneshot — `title` / `content` / any `eval` command hits the 30s timeout regardless of window visibility. The bridge itself is alive (`ping` round-trips), the webview maps, the page renders; only the eval result channel is inert. Root cause has not been isolated upstream. The Rust plugin, capability, feature flag, and daemon spawner are all in place for when the stall clears — flip `HYPRPILOT_E2E_MODE=tauri`.
- **Browser mode** uses `page.evaluate` which talks to the actual Chromium CDP — no native network interception limits apply, but screenshots come from Chromium, not from the OS compositor, so layer-shell geometry isn't captured here either. Tauri-mode screenshots (when it works) go through the webview, not the compositor.
- **macOS / Windows** are out of scope; hyprpilot is Linux-first.

## Fallback

If the WebKitGTK eval stall never resolves upstream, the official Tauri testing path is [`tauri-driver`](https://v2.tauri.app/develop/tests/webdriver/)

- WebdriverIO over the WebKitGTK WebDriver shim. Specs would migrate from Playwright syntax to WDIO — lift-and-shift level work, not a rewrite — and every fixture in `fixtures/` stays (mock agent, config override, daemon spawner). Browser mode would keep running regardless as the fast pre-CI lane.

## Layout

```
tests/e2e/
├── README.md
├── playwright.config.ts
├── package.json
├── tsconfig.json
├── fixtures/
│   ├── e2e-config.toml      # layered TOML override pointing at mock-agent
│   ├── global-setup.ts      # tauri mode: spawns the daemon, waits for socket
│   ├── global-teardown.ts   # tauri mode: SIGTERM / SIGKILL on exit
│   └── tauri.ts             # createTauriTest wrapper + IPC mock map
├── specs/
│   ├── smoke.spec.ts        # title + placeholder DOM assertions
│   ├── submit.spec.ts       # captured-invoke assertion on the boot IPC pair
│   └── permission.spec.ts   # acp:permission-request event fan-out
└── support/
    └── mock-agent/
        ├── index.mjs
        └── package.json
```
