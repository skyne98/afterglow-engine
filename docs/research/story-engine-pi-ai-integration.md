# Story Engine — pi-ai / makora DeepSeek integration

Date: 2026-08-10
Status: implemented in `prototype/story-engine/`
Crates/packages: `@earendil-works/pi-ai`, `@earendil-works/pi-agent-core`,
`@earendil-works/pi-coding-agent`

## Goal

The story-engine prototype (Bun server + Vue 3 + SQLite, under
`prototype/story-engine/`) drives LLM-backed NPC dialogue. The user asked to
use the same LLM pi uses in this shell — the **makora** provider, DeepSeek
**DeepSeek-V4-Flash** — with **pi-ai as the client**, rather than a hand-rolled
OpenAI HTTP client.

## Findings

### Provider facts (from the installed pi-makora-provider extension)

- `PROVIDER_ID = "makora"`.
- `BASE_URL = https://inference.makora.com/v1`.
- API key: `~/.pi/agent/auth.json` `"makora"` entry, or the
  `MAKORA_OPTIMIZE_TOKEN` env var.
- Model `deepseek-ai/DeepSeek-V4-Flash` is OpenAI-completions compatible:
  `reasoning: false`, no `chat_template_kwargs`, no `assistantReasoningField`.
  The makora extension passes such models through the plain OpenAI completions
  reader unchanged.
- The makora provider is **registered by an extension**, not built into pi-ai's
  provider catalog. `ModelRuntime.create()` alone does not list `makora`; the
  provider registers when a resource loader binds the makora extension
  (discovered from the agent dir).

### Direct `streamSimple` path is broken on this endpoint

Calling `streamSimple` from `@earendil-works/pi-ai/compat` with a hand-built
`api: "openai-completions"`, `baseUrl: inference.makora.com/v1` model throws
inside the reader:

```
undefined is not an object (evaluating 'block.name.length')
```

The stream emits an `error` event instead of text. Root cause not fully
isolated in pi-ai internals; instrumenting the `catch` in
`openAI-completions.js` shows the error is converted to an error event
elsewhere, not in that function's own catch. Rather than debug library
internals, the adapter uses the supported higher-level path (below), which is
exactly what the makora extension is built and tested against.

### Working path: AgentSession

Create a `ModelRuntime`, load a `DefaultResourceLoader` from the agent dir
(registers the makora extension), create an `AgentSession` with `noTools:
"all"`, `thinkingLevel: "off"`, then `session.setModel(
modelRuntime.getModel("makora", "deepseek-ai/DeepSeek-V4-Flash") )`. Streaming
text arrives as `message_update` / `text_delta` events.

Verified live: one prompt returned `hello world`; an NPC character prompt
produced an in-character reply over WebSocket with tokens streamed to the Vue
client and the reply persisted to SQLite.

### System prompt must go through the loader

The session resets `agent.state.systemPrompt` to its base prompt on every turn
(unless an extension supplies one via `emitBeforeAgentStart`). So the NPC
character prompt is set with `DefaultResourceLoader({ systemPromptOverride:
() => characterPrompt })`, and `cwd` is a neutral temp dir so the repo's
`AGENTS.md` is not injected into the NPC context.

## Design in the prototype

- `server/src/llm/pi.ts` — `PiLlm` (shared `ModelRuntime`) and `SessionChat`
  (one `AgentSession` per conversation, `send()` returns the full reply and
  streams deltas). `PiLlm` is a lazy singleton, initialized at server boot.
- `server/src/npc-engine.ts` — framework core calls `PiChat.send`; the live
  session is per conversation and holds the NPC's streaming context, while
  SQLite remains the durable store of npc/conversation/message/memory/events.

## Revisit

- Memory refresh into the live session prompt (currently a snapshot taken at
  conversation creation).
- Conversation resume after server restart (replay SQLite history into a fresh
  session).
- Re-check `streamSimple` upstream fix; prefer it over a full AgentSession if
  it ever works for this endpoint (lighter, stateless per-turn calls).
