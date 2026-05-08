---
name: agent-browser
description: Navigate and read websites using the agent-browser CLI. Open pages, extract text/content, take snapshots and screenshots, click elements, fill forms — full browser automation for AI agents.
---

## Setup

Installed globally via `bun install -g agent-browser`. Browser binary (Chrome) auto-installed at `~/.agent-browser/browsers/`.

## Core workflow

1. Open a page: `agent-browser open <url>`
2. Snapshot the page to see interactive elements with refs: `agent-browser snapshot --json`
3. Interact using refs from the snapshot: `agent-browser click @e1`, `agent-browser fill @e2 "text"`
4. Get page text: `agent-browser get text @e1`
5. Screenshot: `agent-browser screenshot [path]`
6. Close when done: `agent-browser close --all`

## Chaining commands

The browser daemon persists between calls, so chain with `&&`:

```
agent-browser open https://example.com && agent-browser snapshot -i && agent-browser get title
```

## Key commands

| Action | Command |
|---|---|
| Open URL | `agent-browser open <url>` |
| Click element | `agent-browser click @e1` |
| Type into field | `agent-browser type @e2 "text"` |
| Clear and fill | `agent-browser fill @e2 "text"` |
| Press key | `agent-browser press Enter` |
| Get snapshot | `agent-browser snapshot --json` |
| Get text | `agent-browser get text @e1` |
| Screenshot | `agent-browser screenshot` |
| Run JS | `agent-browser eval "document.title"` |
| Wait | `agent-browser wait 2000` |
| Scroll | `agent-browser scroll down 500` |
| Close | `agent-browser close --all` |

## Best practices

- Always call `snapshot --json` after page load to get element refs before interacting.
- Use refs (`@e1`, `@e2`) from snapshots for reliable targeting.
- Chain commands with `&&` — the browser stays alive between calls.
- Use `--headed` to see the browser window when debugging.
- Use `get text` to extract visible page content.
- Always `close --all` at the end to clean up.
