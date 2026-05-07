---
title: Remote bridge
order: 5
---

# Remote bridge

The daemon can host a TLS HTTPS+WebSocket server alongside the unix socket and Tauri overlay. A phone (or any browser on the LAN) loads the same Vue overlay over `https://<host>:7423/` and operates the daemon as a remote — not a viewer, a full mirror.

Authentication is **per-connection** with a 4-word [BIP39](https://en.bitcoin.it/wiki/BIP_0039) pair code shown on the connecting device and confirmed in a desktop modal. No tokens persist; reconnect re-pairs. Mirrors how Bluetooth / Chromecast / AirDrop pair.

Off by default. Captains opt in.

## Minimal config

```toml
[remote]
enabled = true
```

That's the whole opt-in. Restart the daemon and the bridge is up on `https://0.0.0.0:7423`.

## All knobs

```toml
[remote]
enabled  = false                                          # default; flip to true to bring up the listener
bind     = "0.0.0.0:7423"                                 # default; all interfaces
tls_cert = "~/.config/hyprpilot/remote-cert.pem"          # optional override
tls_key  = "~/.config/hyprpilot/remote-key.pem"           # paired with tls_cert
```

| Field | Default | Meaning |
| --- | --- | --- |
| `enabled` | `false` | Start the TLS axum listener. |
| `bind` | `0.0.0.0:7423` | `host:port`. Default exposes on every interface (LAN + loopback). Set to `127.0.0.1:7423` for loopback-only; IPv6 forms work too. |
| `tls_cert` / `tls_key` | unset | Bring your own PEM-encoded cert + private key. When unset the daemon auto-generates a self-signed cert on first start. |

## TLS material

When `tls_cert` / `tls_key` are unset, the daemon generates a self-signed cert at first start and persists it under `$XDG_STATE_HOME/hyprpilot/remote-{cert,key}.pem`. Subsequent starts reuse the same files.

The auto-generated cert's SANs cover `127.0.0.1`, `::1`, `localhost`, the OS hostname, and every detected non-loopback IPv4 — keeps the per-IP TLS warning manageable on top of the unavoidable self-signed nag. The connecting device trusts on first use and the warning never reappears for that pair `(device, daemon)` until the cert rotates.

To rotate the cert, delete `remote-cert.pem` + `remote-key.pem` under the state dir and restart the daemon.

To bring your own cert (e.g. one signed by a private CA your devices already trust), point `tls_cert` and `tls_key` at the PEM files. The daemon doesn't watch the files; restart to pick up changes.

## Pair-on-connect flow

1. Connecting device opens `https://<host>:7423/`. The SPA detects it's running outside Tauri and opens `wss://<host>:7423/ws`.
2. Daemon mints a 4-word BIP39 code, holds the connection in `pending` state, and emits a `remote:pair-request` Tauri event to the desktop overlay.
3. Both ends render the same code as readable text **and** as a QR.
4. Confirm in any of three ways:
   - **Type** the four words into the desktop modal.
   - **Scan from desktop** — click *scan* on the desktop modal → desktop's webcam reads the QR off the connecting device → input auto-fills → click *confirm*.
   - **Scan from device** — tap *scan desktop QR* on the connecting device's pair screen → device's camera reads the desktop's QR → device pushes the decoded code back over the WS → daemon matches and authenticates.
5. On match the WS upgrades to `authenticated` and the SPA finishes booting + can issue RPCs.
6. Mismatch → the appropriate side flags the error and the WS stays pending.
7. No confirm within 60 seconds, or 3 wrong attempts → daemon expires the pending pair and closes the WS.

### Scanning

Both scan paths use [`qr-scanner`](https://github.com/nimiq/qr-scanner) — runs entirely in the browser, no external service. Camera permission is requested on first click via the Web `MediaDevices.getUserMedia()` API; deny it once and the captain falls back to typing the four words. The from-device path is the easier ergonomically — modern phones have higher-resolution rear cameras and are easier to point at the desktop screen than the reverse.

State is connection-scoped per pair. Daemon restart wipes everything; mid-run reconnects skip the captain-confirm step via an in-memory session token (see below).

### Session tokens (in-memory, daemon-lifetime)

After a successful pair, the daemon mints a UUID session token and ships it back inside the `authenticated` frame. The connecting device stores it in `localStorage` and presents it on every subsequent reconnect via a `{type:"hello"}` frame:

- Token recognized → daemon authenticates immediately, no captain-confirm modal opens.
- Token unrecognized (daemon restarted, captain rejected, browser cleared storage) → daemon silently ignores the hello and the normal pair flow runs.

The token table is **in-memory only**. There is no disk persistence and no `revoke` UX today; the captain wipes by restarting the daemon. This survives page reloads and browser restarts within the same daemon run, which is what the captain typically wants — pair once when the daemon starts, the phone can reconnect freely until the daemon goes down.

## Troubleshooting

**Bridge isn't running.** Check `~/.local/state/hyprpilot/logs/hyprpilot.log.*` for `remote: TLS listener up` after enabling. A misconfigured bind / cert warns and the bridge stays down without aborting the daemon's main path.

**`curl -k https://<lan-ip>:7423/healthz`** should return `ok`. If `curl` can't connect, the bind is wrong (or another process holds the port).

**Browser keeps showing the cert warning.** Self-signed certs trip every browser's TLS UI on first connect. Trust on first use; the warning goes away for that `(device, daemon)` pair until the cert rotates. To eliminate it, supply a cert signed by a CA your devices trust via `tls_cert` / `tls_key`.

**Pair modal doesn't open.** The desktop overlay needs to be running and visible (or visible-on-event) when a connection lands. Check the daemon log for `remote: pair request received`; absence means the WS upgrade isn't reaching the daemon.
