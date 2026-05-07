---
title: Remote bridge
order: 6
---

# Remote bridge

Phone-as-overlay. Browser-as-overlay. Anything on the LAN that speaks HTTPS + WebSocket loads the same Vue overlay the desktop runs and operates the daemon as a remote — full mirror, not a viewer.

## What it gives you

- The desktop is at the workstation; you're on the couch with a phone. Browse to `https://<workstation>:7423/` — same overlay, same chat, same palette, same everything.
- Tablet on the side of the desk for monitoring while the laptop is doing other work.
- Quickly fire prompts from another machine without SSH'ing in.

## How pairing works

Hyprpilot doesn't ship a trust store, doesn't manage tokens, doesn't track "known devices". Every connection re-pairs from scratch — same interaction model as Bluetooth, Chromecast, AirDrop.

1. Phone opens `https://<host>:7423/` in a browser. The SPA detects it's running outside Tauri and opens a WebSocket to `wss://<host>:7423/ws`.
2. Daemon mints a 4-word [BIP39](https://en.bitcoin.it/wiki/BIP_0039) code (one of 2048<sup>4</sup> ≈ 16 trillion phrases). Phone shows it.
3. Desktop overlay automatically opens a confirm modal showing the same 4 words. Captain reads the code off the phone and types it into the desktop.
4. On match the WebSocket upgrades to authenticated. The phone's SPA can now drive the daemon — just like the desktop overlay does over Tauri's IPC.
5. Disconnect → re-pair on reconnect. The contract is one connection.

The pair window expires after 60 seconds with a 3-attempt cap. Wrong code three times → daemon burns the pending state and closes the WS.

## Discovery

There's no mDNS / Bonjour / Zeroconf. Bookmark `https://<workstation-ip>:7423` on the phone after the first pair and reuse it. If your workstation's IP rotates, re-bookmark. Keeping discovery static keeps the trust story simple.

## TLS

The bridge is TLS-only. There is no plain-HTTP fallback. On first start the daemon auto-generates a self-signed cert covering `127.0.0.1`, `::1`, `localhost`, the OS hostname, and every detected non-loopback IPv4 — so the per-IP TLS warning stays manageable.

The phone sees the self-signed warning on first connect; once you click through, it's persistent for that `(device, daemon)` pair until the cert rotates. To eliminate the warning entirely, supply a cert signed by a CA your phone already trusts via `[remote] tls_cert` / `tls_key` — see [Configuration → Remote bridge](../configuration/remote).

## Mobile chrome

Below 540px viewport (every common phone width in portrait), the overlay hides keyboard-hint chrome since touch users have no keybinds to read. The full mobile-shell with a touch-friendly bottom toolbar is on the roadmap.

## What's not in scope today

- **Persistent device pairing.** No "trust this device" toggle. Re-pair every time. By design.
- **Camera QR scanning on the desktop modal.** Today the captain types the 4 words. QR scan slots in as a future "scan" button next to the input.
- **Public-internet exposure.** The TLS + pair-on-connect model handles untrusted networks within reason, but hyprpilot doesn't ship rate-limiting / IP allowlists / DDoS shielding. Run it on the LAN, or behind a VPN / Tailscale tailnet, not on a public IP.

## Setup

See [Configuration → Remote bridge](../configuration/remote) for the config knobs. The minimal opt-in is:

```toml
[remote]
enabled = true
```

Restart the daemon, point the phone's browser at `https://<workstation-ip>:7423/`, and pair.
