---
title: Remote bridge
order: 5
---

# Remote bridge

Open the same overlay on your phone, tablet, or any other browser on the LAN. Off by default — turn it on with one line.

## Turn it on

```toml
[remote]
enabled = true
```

Restart the daemon. The overlay is now reachable at `https://<your-host>:6262/`.

## Open it from another device

Point a browser at `https://<your-host>:6262/`. You'll see a self-signed certificate warning the first time — trust it through your browser. The overlay loads, you pair (next section), and you're driving the same chat your desktop is.

## All knobs

```toml
[remote]
enabled = false                                            # default
bind    = "0.0.0.0:6262"                                   # default — every interface

[remote.tls]
# certificate = "~/.config/hyprpilot/remote-cert.pem"      # bring your own cert
# key         = "~/.config/hyprpilot/remote-key.pem"
# sans        = ["myhost.example.com"]                     # custom SAN list
```

| Field | Default | What it does |
| --- | --- | --- |
| `enabled` | `false` | Turns the bridge on. |
| `bind` | `0.0.0.0:6262` | Where to listen. `0.0.0.0:6262` exposes on the LAN; `127.0.0.1:6262` is loopback only (useful with a reverse proxy). |
| `tls.certificate` / `tls.key` | unset | Path to your own cert + key (PEM). Skip these and hyprpilot generates a self-signed cert for you. |
| `tls.sans` | unset | Names / IPs the auto-generated cert covers. Set this if you have a stable DNS name pointing at the daemon. |

## Certificates

You have two choices:

**Auto-generated self-signed.** No setup needed. The daemon writes a cert on first start covering loopback, your hostname, and every LAN address it sees. Devices show the self-signed warning the first time and remember the cert after that. If your IP changes, hyprpilot detects it and regenerates — devices warn again.

**Bring your own.** Drop a real cert at `~/.config/hyprpilot/remote-cert.pem` (and the key alongside) and point `[remote.tls]` at them. Use this if you have a Let's Encrypt cert, an internal CA, or you want a cert that never rotates. Hyprpilot reads the files as-is — no warnings, no regeneration.

If you have a stable hostname (a hosts-file entry, internal DNS, etc.) but don't want to wrangle a real cert, set `tls.sans = ["myhost.example.com"]`. The auto-generated cert sticks to that name, doesn't rotate when your IP moves, and you bookmark `https://myhost.example.com:6262/` once.

To force a fresh cert: delete `remote-cert.pem`, `remote-key.pem`, and `remote-cert.sans` under `~/.local/state/hyprpilot/` and restart.

## Troubleshooting

**The bridge isn't responding.** Check `~/.local/state/hyprpilot/logs/hyprpilot.log.*` for a line confirming the listener came up. A bad config warns there and leaves the rest of the daemon running.

**`curl -k https://<your-ip>:6262/healthz`** should print `ok`. If `curl` can't connect at all, the bind address is wrong, the firewall is blocking the port, or another process owns it.

**The cert warning won't go away.** Self-signed certs always warn the first time a device sees them. Click through and the warning sticks for that device. To get rid of it for good, [bring your own cert](#certificates) signed by a CA the device already trusts.

**The pair screen never opens on my desktop.** The desktop overlay has to be visible to receive the pair request. Pop it open with your bind (`Super+Space` by default) before opening the URL on the phone.
