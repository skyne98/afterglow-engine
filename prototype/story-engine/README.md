# Afterglow — Story Engine Prototype

A **Bun server + Vue 3 frontend + SQLite** framework for LLM-driven NPCs.

This is a prototype, not an engine crate: it mirrors the
`prototype/character-editor/` convention and lives entirely under
`prototype/story-engine/`. The framework core is `server/src/npc-engine.ts` —
NPC identity, long-term memory, the dialogue loop, and story events. The
transport (WebSocket) and the Vue UI are thin consumers of that core, so the
core can later be re-hosted by the engine without rewriting.

## Layout

```
shared/types.ts        wire + DB contract (single source of truth)
server/
  src/index.ts         Bun server: HTTP JSON API + WebSocket
  src/db.ts            SQLite open + migration application
  src/migrations.ts    minimal SQL-file migration runner
  src/migrate.ts       CLI: bun run migrate
  src/npc-engine.ts    framework core (NPCs, memory, dialogue loop, events)
  src/llm/client.ts    OpenAI-compatible streaming client (provider-agnostic)
  migrations/0001_init.sql
web/
  src/App.vue            mounts the combined world view
  src/components/Sandbox.vue  combined world: Pixi grid + player/NPCs + chat panel
  src/sandbox/model.ts    reactive model (entities: player/npc/prop, tool, selection, bubbles)
  src/world/story-world.ts  world layer: model + WebSocket + chat log + dialogue flow
  src/api/ws.ts           typed WebSocket client (talks directly to the bun server)
```

## Quickstart

```sh
bun install
bun run migrate    # create server/data/story.db + apply migrations
bun run dev        # server :8787 + vite :5173 (proxies /api and /ws)
```

Open http://localhost:5173, create an NPC, and say hello.

## LLM: makora DeepSeek via pi-ai

The dialogue engine drives the same provider + model pi uses in this shell,
through the pi SDK (`@earendil-works/pi-ai` + `@earendil-works/pi-coding-agent`):

```text
provider   makora        https://inference.makora.com/v1
model      deepseek-ai/DeepSeek-V4-Flash
key        read from ~/.pi/agent/auth.json ("makora" entry)
```

No extra setup is needed if pi is already configured on this machine. To use a
different provider/model or an explicit key path later, extend
`server/src/llm/pi.ts` (`PiLlm.init` options + the makora provider/model id).

**Why the session path, not the raw client:** the raw pi-ai `streamSimple`
path currently throws on this endpoint (`undefined is not an object
(block.name.length)`), so the adapter uses the supported `AgentSession` path,
which handles streaming, auth, and context. See
`docs/research/story-engine-pi-ai-integration.md`.

Each conversation owns a live session. If no LLM is configured, chat returns
an error in the panel; the server, schema, migrations, and UI still work.

## Database & migrations

SQLite via Bun's built-in `bun:sqlite` (no driver). Migrations are ordered SQL
files in `server/migrations/`; each runs in one transaction and is recorded by
file name in `_schema_migrations`, so re-running is a no-op.

```sh
bun run migrate    # apply pending migrations
bun run dev:server # also applies pending migrations on boot
```

Add a migration by creating `NN_name.sql`. The runner applies it on the next
`migrate` or server boot.

## World view (combined sandbox + chat)

The app is one screen: a **top-down sandbox** where the player (you, the green
circle) exists alongside NPCs, and a docked **chat panel** for dialogue.

- Real NPCs are loaded from the server and placed clustered around the player.
- Click an entity to select it and drag to move; the toolbar places new **NPC**
  / **Prop** entities, **Remove** deletes, **Clear** empties the board.
- Type in the chat panel to **speak to the target NPC** — the selected one, or
  the nearest one to your player. The NPC streams its reply into the log and
  shows a live speech bubble; everything is persisted through the server.

Entity kinds: `player` (you), `npc` (bound to a server NPC record via
`npcId`), `prop` (untalkable scenery).

### Reactive data model

All world state lives in a **reactive model** (`web/src/sandbox/model.ts`,
Vue `reactive`): `{ tool, selectedId, entities }`. The Pixi renderer is a thin
mirror — a `watchEffect` reconciles display objects from the `entities` array
on every change, and a per-frame tween animates bubbles in/out.

```ts
model.addEntity("player", col, row, { name });   // spawn any kind
model.place(col, row);       // spawn the current tool's kind
model.move(id, col, row);    // move (false when blocked)
model.remove(id);            // delete
model.setBubble(id, text);   // set/clear a bubble (reactive)
model.speak(id, text, 3000); // timed bubble; resolves when done
```

The model is framework-independent (no rendering or network knowledge) and
unit-tested. The world layer (`web/src/world/story-world.ts`) owns the model,
the WebSocket client, the chat log, and the dialogue flow (choose target,
start a conversation, stream the reply into the log + bubble).

### Dialogue

Sending a line targets the selected NPC, else the nearest one. A server NPC is
provisioned on first contact if the entity is not bound yet, then a
conversation is started and the reply streams back.

The chat panel connects to the bun server's WebSocket **directly**
(`ws://<host>:8787/ws`): vite's `ws` proxy does not relay upgrades to bun
reliably, while HTTP `/api` proxying works, so `/api` goes through vite and
`/ws` goes straight to the server.


## Next steps (framework directions)

- Multi-NPC scenes, talk-to, and shared state between NPCs.
- Richer memory (importance decay, extraction prompts, consolidation).
- Story-event subscriptions driving game state / triggers.
- Per-NPC context budgets and token accounting.
- Authoring UI for NPC definitions and memory review.

## Scripts

```sh
bun run dev         # server + vite together
bun run dev:server  # Bun server only
bun run dev:web     # vite only
bun run migrate     # apply DB migrations
bun run test        # bun test
bun run typecheck   # tsc --noEmit
```
