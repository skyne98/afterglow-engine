/**
 * StoryWorld — the combined world + chat layer.
 *
 * Owns the reactive sandbox `model`, the WebSocket `StoryClient`, the chat
 * log, and the dialogue flow. The player exists in the model as a `player`
 * entity; NPC entities carry a server `npcId`. Sending a chat line talks to
 * the target NPC (selected, else nearest to the player), streams the reply
 * into the log and a live bubble, and persists it through the server.
 *
 * The world is transport-bound (uses StoryClient) but the model stays pure.
 */
import { reactive } from "vue";
import { StoryClient } from "../api/ws";
import { createSandboxModel, BUBBLE_FADE_MS, type Entity } from "../sandbox/model";
import type { ServerMessage, NpcDef } from "../api/ws";

export interface ChatLine {
  id: string;
  role: "player" | "npc";
  speaker?: string;
  text: string;
  streaming?: boolean;
}

let lineSeq = 0;
function lineId(): string {
  return `c${++lineSeq}`;
}

export class StoryWorld {
  readonly model = createSandboxModel();
  readonly log = reactive<ChatLine[]>([]) as ChatLine[];

  private readonly client = new StoryClient();
  private readonly convs = new Map<string, string>(); // npcId -> conversation id
  private readonly startPrompt = new Map<string, (id: string) => void>();
  private playerId: number | null = null;
  private streaming: { conversationId: string; entityId: number; marker: ChatLine } | null = null;
  private cols = 16;
  private rows = 10;

  constructor() {
    this.client.onMessage((m) => this.onServer(m));
  }

  get player(): Entity | undefined {
    return this.playerId === null ? undefined : this.model.entities.find((e) => e.id === this.playerId);
  }

  get streamingActive(): boolean {
    return this.streaming !== null;
  }

  /** Place the player at the center and populate real NPCs from the server. */
  async seed(cols: number, rows: number): Promise<void> {
    this.cols = Math.max(4, cols);
    this.rows = Math.max(4, rows);
    if (!this.model.entities.some((e) => e.kind === "player")) {
      this.playerId = this.model.addEntity(
        "player",
        Math.floor(this.cols / 2),
        Math.floor(this.rows / 2),
        { name: "You" },
      )?.id ?? null;
    }
    await this.client.ready();
    this.client.listNpcs();
  }

  /** NPC entities present in the world. */
  npcs(): Entity[] {
    return this.model.entities.filter((e) => e.kind === "npc");
  }

  /** Who the player is talking to: the selected NPC, else the nearest one. */
  talkTarget(): Entity | undefined {
    const sel = this.model.entities.find((e) => e.id === this.model.selectedId && e.kind === "npc");
    if (sel) return sel;
    const p = this.player;
    if (!p) return undefined;
    let best: Entity | undefined;
    let bestD = Infinity;
    for (const e of this.npcs()) {
      const dx = e.col - p.col;
      const dy = e.row - p.row;
      const d = dx * dx + dy * dy;
      if (d < bestD) {
        bestD = d;
        best = e;
      }
    }
    return best;
  }

  /** Place the player's speech bubble and send a line to the target NPC. */
  send(text: string): void {
    const line = text.trim();
    if (!line || this.streamingActive) return;

    const p = this.player;
    if (p) {
      this.model.setBubble(p.id, line);
      setTimeout(() => this.model.setBubble(p.id, null), 2500);
    }

    this.log.push({ id: lineId(), role: "player", speaker: "You", text: line });

    const target = this.talkTarget();
    if (!target) {
      const marker: ChatLine = { id: lineId(), role: "npc", speaker: "World", text: "There is no NPC to talk to.", streaming: false };
      this.log.push(marker);
      return;
    }
    void this.dialogue(target, line);
  }

  private async dialogue(npc: Entity, line: string): Promise<void> {
    const markerRaw: ChatLine = {
      id: lineId(),
      role: "npc",
      speaker: npc.name ?? `NPC ${npc.id}`,
      text: "",
      streaming: true,
    };
    this.log.push(markerRaw);
    // Use the reactive proxy held by the array (not the raw object) so live
    // mutations to `text`/`streaming` actually re-render the chat log.
    const marker = this.log[this.log.length - 1];

    const npcId = await this.ensureBound(npc);
    let conv = this.convs.get(npcId);
    if (!conv) {
      conv = await this.startConversation(npcId);
      this.convs.set(npcId, conv);
    }

    this.streaming = { conversationId: conv, entityId: npc.id, marker };
    this.model.select(npc.id);
    this.client.sendChat(conv, line);
  }

  private async ensureBound(npc: Entity): Promise<string> {
    if (npc.npcId) return npc.npcId;
    const def: NpcDef = {
      id: crypto.randomUUID(),
      name: npc.name ?? `NPC ${npc.id}`,
      data: {},
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
    };
    this.client.createNpc(def);
    npc.npcId = def.id;
    npc.name = def.name;
    return def.id;
  }

  private startConversation(npcId: string): Promise<string> {
    return new Promise((resolve) => {
      this.startPrompt.set(npcId, resolve);
      this.client.startChat(npcId);
    });
  }

  // -- Server wiring --------------------------------------------------------

  private onServer(m: ServerMessage): void {
    switch (m.type) {
      case "npc.list.result":
        this.placeNpcs(m.items);
        break;
      case "npc.created":
        this.model.select(this.model.selectedId); // no-op; keep selection stable
        break;
      case "chat.started": {
        const resolve = this.startPrompt.get(m.conversation.npc_id);
        if (resolve) {
          this.startPrompt.delete(m.conversation.npc_id);
          resolve(m.conversation.id);
        }
        break;
      }
      case "token":
        if (this.streaming && this.streaming.conversationId === m.conversation_id) {
          this.streaming.marker.text += m.delta;
          this.model.setBubble(this.streaming.entityId, this.streaming.marker.text);
        }
        break;
      case "message":
        if (this.streaming && this.streaming.conversationId === m.message.conversation_id) {
          const s = this.streaming;
          this.streaming = null;
          s.marker.text = m.message.content;
          s.marker.streaming = false;
          this.model.setBubble(s.entityId, m.message.content);
          setTimeout(() => {
            this.model.setBubble(s.entityId, null);
          }, 2500 + BUBBLE_FADE_MS);
        }
        break;
      case "error":
        if (this.streaming) {
          this.streaming.marker.text += `\n(error: ${m.message})`;
          this.streaming.marker.streaming = false;
          this.streaming = null;
        }
        break;
      default:
        break;
    }
  }

  private placeNpcs(items: NpcDef[]): void {
    for (const npc of items) {
      const cell = this.freeCellNearPlayer();
      if (!cell) break;
      this.model.addEntity("npc", cell.col, cell.row, { npcId: npc.id, name: npc.name });
    }
  }

  /** Closest free grid cell to the player, so NPCs cluster where we exist. */
  private freeCellNearPlayer(): { col: number; row: number } | undefined {
    const p = this.player;
    if (!p) return undefined;
    let best: { col: number; row: number } | undefined;
    let bestD = Infinity;
    for (let r = 0; r < this.rows; r++) {
      for (let c = 0; c < this.cols; c++) {
        if (c === p.col && r === p.row) continue;
        if (this.model.entityAt(c, r)) continue;
        const dx = c - p.col;
        const dy = r - p.row;
        const d = dx * dx + dy * dy;
        if (d < bestD) {
          bestD = d;
          best = { col: c, row: r };
        }
      }
    }
    return best;
  }
}
