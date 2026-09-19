---
title: Serving MCP over HTTP
order: 70
next: false
---

# {{ $frontmatter.title }}

Every in-tree MCP server speaks stdio by default, spawned by the vendor CLI that hyprpilot launched. `--transport http` serves the same surface over a port instead, for clients that are **not** that vendor — another editor, a second machine, a script.

<!-- more -->

```sh
hyprpilot mcp skills  --transport http --listen 127.0.0.1:7777 --skill-dir '{"dir":"~/.config/hyprpilot/skills","ignore":[],"watch":true}'
hyprpilot mcp serve   --transport http --listen 127.0.0.1:7778
hyprpilot mcp harness --transport http --listen 127.0.0.1:7779 --token-file ~/.config/hyprpilot/mcp-token
```

| Flag                      | Meaning                                                                                                               |
| ------------------------- | --------------------------------------------------------------------------------------------------------------------- |
| `--transport stdio\|http` | Default `stdio`. Nothing about the stdio path changes.                                                                |
| `--listen <ADDR>`         | Required with `--transport http`. No default: each server needs its own port, and an ephemeral one addresses nothing. |
| `--token-file <PATH>`     | Bearer token clients must present. Overrides `HYPRPILOT_MCP_TOKEN`.                                                   |
| `--allow-remote`          | Answer requests whose `Host` is not a loopback name.                                                                  |

::: warning This is a server you run, not one hyprpilot starts

The launcher still auto-injects a **stdio** entry for every server your `mcp` config enables, and still starts nothing else. An HTTP server has no supervisor here: you run it, you restart it, and when it dies every harness session dies with it.

:::

## Wiring a client to it

Auto-injection is unchanged, so point clients at the URL yourself — the standard `mcpServers` shape, which all three vendors already understand:

```json
{ "mcpServers": { "skills-http": { "url": "http://127.0.0.1:7777/mcp" } } }
```

::: danger The reserved name replaces your entry

Naming that catalogue entry `hyprpilot-skills` (or whatever `[mcp.<server>] name` resolves to) does **not** work for a hyprpilot-launched session: the launcher drops any configured server matching a reserved name and inserts its own stdio entry at the front. It warns, but the launch still uses stdio.

For a hyprpilot-launched vendor to reach the HTTP server, either use a different catalogue key (as above) or turn the auto-injected one off with `[mcp.skills] enabled = false`.

:::

## Authentication is optional

With no `--token-file` and no `HYPRPILOT_MCP_TOKEN`, the server is **unauthenticated** and the startup log says so at `warn`. That is a supported configuration and it is your call:

- **Every local process can reach a loopback port**, and on the harness that means calling `spawn`, which runs an arbitrary binary as you.
- **Every client shares the harness's sessions.** They all appear in `session_list`, and any client can `session_send`, steer or kill a conversation another one started.

There is no config key for the token. The `mcp` subcommands deliberately never load config — that is what keeps a broken `[[profiles]]` list from killing a sidecar — and there is no `--token <value>` flag either, because argv is world-readable through `/proc`.

The 401 is a bare one. This is a single shared token, not OAuth, so it carries no `WWW-Authenticate` pointing at RFC 9728 resource metadata: advertising a discovery document for an endpoint that does not exist would be worse than saying nothing.

## What the browser guard does

`Origin` validation is **on**, allowing only this server's own origin. A cross-origin `fetch()` from any page you happen to have open is refused with `403` before it reaches a handler:

```console
$ curl -s -o /dev/null -w '%{http_code}\n' -X POST http://127.0.0.1:7777/mcp \
    -H 'Origin: http://evil.example' -H 'Mcp-Method: initialize' -d '{...}'
403
```

`Host` validation is separate and defaults to loopback names. `--allow-remote` widens **both** the reachable bind and that check — without it a non-loopback bind accepts the connection and then refuses every request, which reads as a bug rather than a policy.

## Notifications degrade, and the cache ttl says so

MCP `2026-07-28` removed sessions (SEP-2567), so rmcp serves every request of that revision **statelessly**: the peer a request carries dies with its response. Consequences, all of them deliberate:

| Channel                                                           | Over HTTP                                                                                                                                                                                                         |
| ----------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `resources/updated`, `resources/list_changed` to a **subscriber** | Works. A `subscriptions/listen` stream's sink outlives the request that opened it, and every clone of the handler shares one registry.                                                                            |
| The same, **broadcast** to a non-subscriber                       | Gone. There is no ambient peer to broadcast through.                                                                                                                                                              |
| `notifications/claude/channel` (harness turn finished)            | Gone. Rides a peer captured at startup.                                                                                                                                                                           |
| `notifications/tasks` (SEP-2663)                                  | Gone. Same, and rmcp will not route task notifications through a subscription.                                                                                                                                    |
| SEP-2663 tasks themselves                                         | Only for a client that attaches `io.modelcontextprotocol/clientCapabilities` to each request — statelessness leaves nowhere else to keep it. Otherwise `spawn` returns its ordinary result and no task is minted. |

So **every HTTP result carries `ttlMs: 0`** where the stdio path carries 24 hours. The long ttl is honest only because every mutable surface fires an invalidation the client actually receives; over HTTP a non-subscribing client would be caching for a day against a notification that never comes.

An HTTP harness caller therefore polls `session_status`, or subscribes.

## Building without it

`--transport http` lives behind a cargo feature that is **on by default**. `cargo build --no-default-features` compiles it out; the flag still parses and says what to do:

```console
$ hyprpilot mcp skills --transport http --listen 127.0.0.1:7777 --skill-dir '{...}'
Error: mcp: this hyprpilot was built without the `http` feature, so `--transport http` cannot serve. Rebuild with `--features http` (it is on by default).
```
