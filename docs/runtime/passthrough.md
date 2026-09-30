---
title: HTTP Passthrough
order: 65
next: false
---

# {{ $frontmatter.title }}

`hyprpilot mcp passthrough` turns HTTP endpoints into MCP tools. You declare each tool in config — its name, description, input schema, url and a static body — and the server lists exactly those. A call POSTs the static body, with the call's arguments laid over it, to the tool's url and hands the response body back verbatim.

<!-- more -->

The server knows nothing about what sits behind a url. A decision service, a build API, a webhook: each is a config entry, not a release.

## Declaring a tool

```toml
[mcp.passthrough]
enabled = true
timeout_seconds = 120

[[mcp.passthrough.tools]]
name = "decide"
description = "Ask the decision model a yes/no question about the current task. Returns its JSON verdict."
url = "https://decide.internal.example/v1/decide"
body = { model = "decision-small", stream = false }
input_schema = { type = "object", properties = { question = { type = "string" }, context = { type = "string" } }, required = ["question"] }
```

A call `decide { "question": "ship it?" }` POSTs

```json
{ "model": "decision-small", "stream": false, "question": "ship it?" }
```

with `Content-Type: application/json`, and the tool result is whatever the endpoint answered, byte for byte.

## The contract

| Upstream                  | Tool result                                        |
| ------------------------- | -------------------------------------------------- |
| `2xx`                     | The response body, verbatim, as text.              |
| Any other status          | `isError`, text `<url> returned <status>: <body>`. |
| Unreachable, or timed out | `isError`, text `<url> unreachable: <cause>`.      |
| Tool name not declared    | JSON-RPC error — the call never leaves the server. |

- **An argument wins over the static body** on a key collision; the merge is one level deep, key by key.
- **The input schema is advertised, not enforced.** The upstream owns what its arguments mean, so the server forwards what the client sent.
- **The result is text only.** Parsing the body into structured content would reshape a response the server does not understand.

## Gating

**Off by default.** The server is injected only when `mcp.passthrough.enabled` is `true` **and** at least one tool is declared — a passthrough with nothing to call serves nothing. Nothing is seeded that points anywhere; an upstream is something you choose. Scope it to the profiles that need it with a `$match`ed [patch](../config/patches).

Enabling it inherits `autoAcceptTools = ['*']` from the `mcp` block unless you set `mcp.passthrough.autoAcceptTools`, so every declared tool lands auto-approved. Tighten it there when an endpoint has side effects.

::: warning Tools ride the sidecar's argv

The launcher hands each tool to the sidecar as a `--tool '<json>'` argument, the same way skill roots travel. A process's argv is world-readable through `/proc/<pid>/cmdline`, so **do not put credentials in `body` or the url**. There is no header support; front an authenticated API with something that holds the secret itself.

:::

## Running it by hand

The same tools, without a launcher, as JSON:

```sh
hyprpilot mcp passthrough \
  --tool '{"name":"decide","inputSchema":{"type":"object"},"url":"http://127.0.0.1:8080/decide","body":{"stream":false}}'
```

Each `--tool` is validated exactly like a config entry and a bad one fails startup. It serves over [HTTP](./mcp-http) like the other servers with `--transport http --listen <addr>`. Field reference: [Config → `mcp.passthrough`](../config/mcp#mcp-passthrough).
