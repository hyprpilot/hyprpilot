---
title: Remote bridge
order: 6
---

# Remote bridge

Open hyprpilot on your phone. Or your tablet. Or another laptop. Same overlay, same chat, same agent — driven from any browser on the LAN.

## What it's for

- You're at the desktop and you stand up. Pick your phone off the charger, open the bookmark, keep typing.
- A tablet on the side of the desk while the laptop is mid-build, watching the agent chew through a task.
- Quick prompts from another machine without having to SSH in and start a CLI.

It's not a viewer or a mirror. The phone drives the agent the same way the desktop does — every palette, every command, every keyboard hint that's been turned into a tap target.

## Pairing — what you'll actually see

Hyprpilot doesn't keep a list of "trusted devices". Every time a phone connects, you confirm it once. Like Bluetooth or AirDrop.

### 1. Open the URL

On the phone: open `https://<your-host>:6262/`. The browser warns about the self-signed certificate the first time — trust it.

You see a screen on the phone showing a 4-word code and a QR. Something like:

```
ocean · velvet · iron · prairie
```

### 2. Open the overlay on the desktop

If it's not already up, pop it (`Super+Space`, the system tray, or whichever bind you use). A modal appears with a different 4-word code and its own QR.

The phone code and the desktop code are different on purpose. Each side shows its own code; the other side has to confirm it.

### 3. Confirm — pick whichever is easiest

You have three ways to do it:

- **Type.** Read the desktop's four words off the desktop screen, type them into the phone. Or read the phone's four words off the phone, type them into the desktop modal.
- **Scan from desktop.** Click *scan* on the desktop modal. Hold the phone up to the laptop webcam. The desktop reads the QR off the phone screen, the four words appear in the input, click *confirm*.
- **Scan from phone.** Tap *scan desktop QR* on the phone. Point the phone at the desktop modal. The phone reads it and submits automatically — no extra tap.

The last one is usually the easiest: rear-camera phones are higher resolution than laptop webcams.

### 4. You're in

The pair screen on the phone disappears, the modal on the desktop closes, the chat surface comes up. Both ends are now driving the same agent.

### What if I get it wrong?

You have 60 seconds and 3 tries. Wrong code three times and the phone has to reload the page to start over. Same if you walk away — the connection times out cleanly.

### Reconnecting

Phone screen locks, you come back, browser disconnects and reconnects — no re-pair. Hyprpilot remembers the device for the lifetime of the daemon. Restart the daemon and everyone re-pairs from scratch (one re-pair per device, not per session).

## What works on mobile

Everything the desktop does:

- Chat / composer / palette
- Permission prompts (allow / deny right on the phone)
- Tool runs, terminals, attachments
- Multiple instances, model + profile switching

The chrome adapts to touch:

- Keyboard hints become tap buttons
- Pill rows scroll horizontally instead of clipping
- Tap the dimmed area outside the palette to dismiss it
- The palette opens via the menu button in the header (top-right)

## What you give up

- **No public-internet exposure.** Hyprpilot is built for the LAN or a Tailscale / WireGuard tailnet. There's no rate limiting, no IP allowlist, no DDoS shield.
- **No stored device list.** If your daemon restarts every day, you re-pair every day. By design — there's no "trust this device" toggle to manage.
- **No mDNS / Bonjour discovery.** Bookmark the URL once. If your IP rotates, re-bookmark.

## Setup

```toml
[remote]
enabled = true
```

Restart, point the phone at `https://<your-host>:6262/`, pair. See [Configuration → Remote bridge](../configuration/remote) for the certificate options and bind address knobs.
