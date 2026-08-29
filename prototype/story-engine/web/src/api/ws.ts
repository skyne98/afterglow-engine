import type {
  ClientMessage,
  ServerMessage,
  NpcDef,
  Conversation,
  DialogueMessage,
  MemoryRecord,
  StoryEvent,
} from "../../../shared/types";

/**
 * Thin typed WebSocket client for the story-engine server.
 * Emits decoded ServerMessage objects and exposes small typed helpers.
 *
 * Connects directly to the bun server (default wss/ws port 8787) rather than
 * through the vite proxy: the vite `ws` proxy does not relay upgrades to bun
 * reliably, while HTTP `/api` proxying does work.
 */
const DEFAULT_WS = `ws://${typeof location !== "undefined" ? location.hostname : "127.0.0.1"}:8787/ws`;

export class StoryClient {
  private ws: WebSocket;
  private listeners = new Set<(msg: ServerMessage) => void>();

  constructor(url = DEFAULT_WS) {
    this.ws = new WebSocket(url);
    this.ws.onmessage = (e) => {
      try {
        const msg = JSON.parse(String(e.data)) as ServerMessage;
        for (const l of this.listeners) l(msg);
      } catch {
        /* ignore malformed frames */
      }
    };
  }

  onMessage(fn: (msg: ServerMessage) => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  /** Resolves once the socket is open and the server can accept messages. */
  ready(): Promise<void> {
    return new Promise((resolve) => {
      if (this.ws.readyState === WebSocket.OPEN) {
        resolve();
        return;
      }
      const onOpen = () => {
        this.ws.removeEventListener("open", onOpen);
        resolve();
      };
      this.ws.addEventListener("open", onOpen);
    });
  }

  /** True once the socket is open and the server is ready to accept. */
  get connected(): boolean {
    return this.ws.readyState === WebSocket.OPEN;
  }

  send(msg: ClientMessage): void {
    if (this.ws.readyState === WebSocket.OPEN) this.ws.send(JSON.stringify(msg));
  }

  // Typed helpers
  listNpcs(): void {
    this.send({ type: "npc.list" });
  }
  createNpc(npc: NpcDef): void {
    this.send({ type: "npc.create", npc });
  }
  startChat(npcId: string): void {
    this.send({ type: "chat.start", npc_id: npcId });
  }
  sendChat(conversationId: string, content: string): void {
    this.send({ type: "chat.send", conversation_id: conversationId, content });
  }
  history(conversationId: string): void {
    this.send({ type: "chat.history", conversation_id: conversationId });
  }
  memory(npcId: string): void {
    this.send({ type: "memory.list", npc_id: npcId });
  }
}

export type {
  ServerMessage,
  NpcDef,
  Conversation,
  DialogueMessage,
  MemoryRecord,
  StoryEvent,
};
