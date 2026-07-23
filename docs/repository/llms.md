---
title: Agents
order: 40
next: false
---

# {{ $frontmatter.title }}

The documentation site is published in an LLM-friendly form so AI assistants and agents can consume it directly. It is generated at build time by [`vitepress-plugin-llms`](https://github.com/okineadev/vitepress-plugin-llms), following the [llms.txt](https://llmstxt.org) convention.

<!-- more -->

## Endpoints

- **[`/llms.txt`](https://hyprpilot.kilic.dev/llms.txt)** — an index of the documentation: the site description plus a linked table of contents.
- **[`/llms-full.txt`](https://hyprpilot.kilic.dev/llms-full.txt)** — the entire documentation concatenated into a single Markdown file, to drop into a context window in one shot.
- **Per-page Markdown** — every page is also emitted as raw Markdown at its path with a `.md` suffix (e.g. `/features/profiles.md`), so an agent can fetch just the page it needs.

## Using it with an agent

Point your agent or LLM at whichever endpoint fits the task:

- For a broad overview, or to let the model discover the right page, give it `https://hyprpilot.kilic.dev/llms.txt`.
- To load the whole documentation at once, use `https://hyprpilot.kilic.dev/llms-full.txt`.
- To answer a question about a single topic, fetch that page's Markdown — e.g. `https://hyprpilot.kilic.dev/features/skills.md`.

These are plain Markdown, so any tool that can fetch a URL — a coding agent, a retrieval pipeline, or a chat with browsing — can read them without scraping the rendered HTML.
