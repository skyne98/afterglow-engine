---
name: docs-rs
description: Browse, search, and extract Rust API documentation from docs.rs cleanly using agent-browser.
---

## Setup

Requires `agent-browser` CLI. Open a page and extract content:

```
agent-browser open <url>
agent-browser wait 2000
agent-browser eval "document.querySelector('#main-content').innerText" | jq -r
```

## Direct URL navigation (most reliable)

docs.rs has predictable URLs for any item:

| Item | URL pattern |
|---|---|
| Crate root | `https://docs.rs/<crate>/latest/<crate>/` |
| Module | `https://docs.rs/<crate>/latest/<crate>/<path>/index.html` |
| Struct | `.../struct.<Name>.html` |
| Trait | `.../trait.<Name>.html` |
| Enum | `.../enum.<Name>.html` |
| Function | `.../fn.<Name>.html` |
| Macro | `.../macro.<Name>.html` |
| Type alias | `.../type.<Name>.html` |

Example:
```
agent-browser open https://docs.rs/tokio/latest/tokio/net/struct.TcpStream.html
agent-browser wait 2000
agent-browser eval "document.querySelector('#main-content').innerText" | jq -r
```

Fallback if `#main-content` doesn't exist:
```
agent-browser eval "document.querySelector('main').innerText" | jq -r
```

## Search via URL (when you don't know the exact path)

```
agent-browser open "https://docs.rs/tokio/latest/tokio/?search=TcpStream"
agent-browser wait 2000
agent-browser eval "document.querySelector('.search-results').innerText" | jq -r
```

Then click the first result:
```
agent-browser eval "document.querySelector('.search-results a').click()"
agent-browser wait 2000
agent-browser eval "document.querySelector('#main-content').innerText" | jq -r
```

## Navigate into a module from a crate page

```
agent-browser eval "document.querySelector('a[href*=\"ecs/index.html\"]').click()"
agent-browser wait 1500
```

## Navigation

- Back: `agent-browser back && agent-browser wait 1000`
- Close: `agent-browser close --all`

## Best practices

- Always use `agent-browser wait` after navigation for JS rendering.
- Prefer direct URL navigation when you know the item name and module path.
- Use `?search=` when you need to discover the location of an item.
- Use `#main-content` as the primary selector, fall back to `main`.
- Always `close --all` at the end to clean up.
