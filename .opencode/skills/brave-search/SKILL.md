---
name: brave-search
description: Search the web using the bx CLI (Brave Search API). Use for querying docs, errors, news, images, and more.
---

## Setup

The `bx` CLI is installed via cargo (`cargo install brave-search-cli`). API key is configured globally at `~/.config/brave-search/config.json`.

## How to search

Use these subcommands depending on the goal:

| Need | Command | Why |
|---|---|---|
| Look up docs, errors, code patterns | `bx context "query" --max-tokens 4096` | Pre-extracted text with token budget |
| Synthesized explanation | `bx answers "question"` | AI-generated answer with citations |
| Site-specific search (e.g. `site:`) | `bx web "site:docs.rs query"` | Supports search operators |
| Find discussions/forums | `bx web "query" --result-filter discussions` | Filter by result type |
| Latest versions/releases | `bx news "query" --freshness pd` | Past day news |
| Security vulnerabilities | `bx context "CVE query"` | CVE details |
| Image search | `bx images "query"` | Images with thumbnails |
| Filter/boost domains | `bx context "query" --include-site docs.rs --exclude-site w3schools.com` | Domain allow/deny |

## Default command

`bx "query"` runs `bx context` by default. Always pipe JSON output through `jq` for structured access, e.g. `bx web "query" | jq '.web.results[].url'`.

## Best practices

- Use `bx web` when you need raw search results with metadata.
- Use `bx context` for LLM-friendly extracted content with snippets.
- Use `bx answers` for direct AI-synthesized responses.
- Always set `--max-tokens` when using `context` to control output size.
- Prefer `--include-site`/`--exclude-site` over inline Goggles for simple domain filtering.
