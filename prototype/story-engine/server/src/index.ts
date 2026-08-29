/**
 * story-engine Bun server.
 *
 *   HTTP  : health + JSON API (NPC CRUD, chat history, memory)
 *   WS    : streaming dialogue, memory, story events
 *
 * Run with `bun run dev:server` or `bun --watch server/src/index.ts`.
 * The Vue web dev server (vite) proxies /api and /ws here on port 8787.
 */
import { openDb, MIGRATIONS_DIR } from "./db";
import { NpcEngine } from "./npc-engine";
import { PiLlm } from "./llm/pi";
import type { ServerWebSocket } from "bun";
import type {
  ClientMessage,
  ServerMessage,
} from "../../shared/types";

const PORT = Number(process.env.STORY_PORT ?? 8787);
const db = openDb({}, MIGRATIONS_DIR);
const llm = await PiLlm.init();
const engine = new NpcEngine(db, llm);

/** Notify every connected client about story events in real time. */
const clients = new Set<ServerWebSocket>();
engine.onEvent = (event) => {
  const msg: ServerMessage = { type: "story", event };
  for (const ws of clients) ws.send(JSON.stringify(msg));
};

// ---------------------------------------------------------------------------
// WebSocket message routing
// ---------------------------------------------------------------------------

function handleMessage(ws: ServerWebSocket, data: string | Buffer) {
  let req: ClientMessage;
  try {
    req = JSON.parse(String(data)) as ClientMessage;
  } catch {
    send(ws, { type: "error", message: "invalid JSON" });
    return;
  }

  switch (req.type) {
    case "npc.list":
      send(ws, { type: "npc.list.result", items: engine.listNpcs() });
      break;
    case "npc.get":
      send(ws, { type: "npc.get.result", npc: engine.getNpc(req.id) });
      break;
    case "npc.create": {
      const npc = engine.upsertNpc(req.npc);
      send(ws, { type: "npc.created", npc });
      break;
    }
    case "chat.start": {
      const conv = engine.startConversation(req.npc_id, req.session_id);
      send(ws, { type: "chat.started", conversation: conv });
      break;
    }
    case "chat.send": {
      if (!req.content.trim()) return;
      void turn(ws, req.conversation_id, req.content);
      break;
    }
    case "chat.history": {
      const messages = engine.history(req.conversation_id, req.limit ?? 200);
      send(ws, { type: "chat.history.result", messages });
      break;
    }
    case "memory.list":
      send(ws, { type: "memory.list.result", items: engine.listMemory(req.npc_id) });
      break;
    default: {
      const _exhaustive: never = req;
      send(ws, { type: "error", message: "unknown message type" });
      void _exhaustive;
    }
  }
}

async function turn(ws: ServerWebSocket, conversationId: string, content: string) {
  try {
    const assistant = await engine.turn(conversationId, content, (delta) => {
      send(ws, { type: "token", conversation_id: conversationId, delta });
    });
    send(ws, { type: "message", message: assistant });
  } catch (err) {
    send(ws, { type: "error", message: String(err instanceof Error ? err.message : err) });
  }
}

function send(ws: ServerWebSocket, msg: ServerMessage) {
  if (ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify(msg));
}

// ---------------------------------------------------------------------------
// HTTP surface
// ---------------------------------------------------------------------------

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

const server = Bun.serve({
  port: PORT,
  fetch(req, srv) {
    const url = new URL(req.url);

    // Upgrade /ws to a WebSocket for streaming dialogue.
    if (url.pathname === "/ws") {
      if (srv.upgrade(req)) return undefined;
      return json({ error: "upgrade failed" }, 400);
    }

    // JSON API.
    if (req.method === "GET" && url.pathname === "/health") {
      return json({ ok: true });
    }

    if (req.method === "GET" && url.pathname === "/api/npcs") {
      return json({ items: engine.listNpcs() });
    }

    if (req.method === "GET" && url.pathname.startsWith("/api/npcs/")) {
      const id = decodeURIComponent(url.pathname.slice("/api/npcs/".length));
      const npc = engine.getNpc(id);
      return npc ? json({ npc }) : json({ error: "not found" }, 404);
    }

    if (req.method === "POST" && url.pathname === "/api/npcs") {
      return req
        .json()
        .then((npc) => json({ npc: engine.upsertNpc(npc) }, 201))
        .catch(() => json({ error: "bad body" }, 400));
    }

    return json({ error: "not found" }, 404);
  },
  websocket: {
    open(ws) {
      clients.add(ws);
    },
    message(ws, data) {
      handleMessage(ws, data);
    },
    close(ws) {
      clients.delete(ws);
    },
  },
});

console.log(`story-engine server listening on http://localhost:${PORT}`);
console.log(`  WS   ws://localhost:${PORT}/ws`);
console.log(`  HTTP /api/npcs  /health`);
