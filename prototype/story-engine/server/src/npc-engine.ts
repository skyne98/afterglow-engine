/**
 * NpcEngine — the story-engine framework core.
 *
 * Owns NPC identity, long-term memory (SQLite), the dialogue loop (context
 * assembly -> LLM call -> response), and emission of story events that the
 * frontend or game code subscribes to. It is transport-agnostic: the WebSocket
 * server and any future engine adapter call the same methods.
 */
import { randomUUID } from "node:crypto";
import type { Database } from "bun:sqlite";
import { PiLlm, type PiChat } from "./llm/pi";
import type {
  Conversation,
  DialogueMessage,
  MemoryKind,
  MemoryRecord,
  NpcDef,
  StoryEvent,
} from "../../shared/types";

export type EventSink = (event: StoryEvent) => void;

export class NpcEngine {
  /** Optional pi-ai adapter; required only to run the dialogue loop (turn). */
  llm?: PiLlm;
  /** Live per-conversation sessions created by the LLM adapter. */
  private readonly chats = new Map<string, PiChat>();
  private readonly db: Database;
  /** Callers may attach a sink to receive emitted story events. */
  onEvent: EventSink = () => {};

  constructor(db: Database, llm?: PiLlm) {
    this.db = db;
    this.llm = llm;
  }

  // -- NPC CRUD -------------------------------------------------------------

  listNpcs(): NpcDef[] {
    return this.db
      .query<Row, []>(`SELECT * FROM npcs ORDER BY name`)
      .all()
      .map(rowToNpc);
  }

  getNpc(id: string): NpcDef | null {
    const row = this.db
      .query<Row, [string]>(`SELECT * FROM npcs WHERE id = ?`)
      .get(id);
    return row ? rowToNpc(row) : null;
  }

  upsertNpc(npc: Omit<NpcDef, "created_at" | "updated_at">): NpcDef {
    const now = new Date().toISOString();
    const existing = this.db
      .query<Row, [string]>(`SELECT created_at FROM npcs WHERE id = ?`)
      .get(npc.id);
    const createdAt = existing?.created_at ?? now;
    this.db
      .query(
        `INSERT INTO npcs
           (id, name, tagline, biography, personality, knowledge, system_prompt, data, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
           name=excluded.name, tagline=excluded.tagline, biography=excluded.biography,
           personality=excluded.personality, knowledge=excluded.knowledge,
           system_prompt=excluded.system_prompt, data=excluded.data, updated_at=excluded.updated_at`,
      )
      .run(
        npc.id,
        npc.name,
        npc.tagline ?? null,
        npc.biography ?? null,
        npc.personality ?? null,
        npc.knowledge ?? null,
        npc.system_prompt ?? null,
        JSON.stringify(npc.data ?? {}),
        createdAt,
        now,
      );
    return this.getNpc(npc.id)!;
  }

  // -- Conversations --------------------------------------------------------

  startConversation(npcId: string, sessionId?: string): Conversation {
    const conv: Conversation = {
      id: randomUUID(),
      npc_id: npcId,
      session_id: sessionId ?? randomUUID(),
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
    };
    this.db
      .query(
        `INSERT INTO conversations (id, npc_id, session_id, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?)`,
      )
      .run(conv.id, conv.npc_id, conv.session_id, conv.created_at, conv.updated_at);
    return conv;
  }

  getConversation(id: string): Conversation | null {
    const row = this.db
      .query<Record<string, unknown>, [string]>(
        `SELECT * FROM conversations WHERE id = ?`,
      )
      .get(id);
    return row ? (row as unknown as Conversation) : null;
  }

  history(conversationId: string, limit = 200): DialogueMessage[] {
    return this.db
      .query<MsgRow, [string, number]>(
        `SELECT * FROM messages WHERE conversation_id = ? ORDER BY created_at ASC LIMIT ?`,
      )
      .all(conversationId, limit)
      .map(msgRowToMsg);
  }

  insertMessage(
    conversationId: string,
    role: DialogueMessage["role"],
    content: string,
    data?: Record<string, unknown>,
  ): DialogueMessage {
    const msg: DialogueMessage = {
      id: randomUUID(),
      conversation_id: conversationId,
      role,
      content,
      created_at: new Date().toISOString(),
      ...(data ? { data } : {}),
    };
    this.db
      .query(
        `INSERT INTO messages (id, conversation_id, role, content, data, created_at)
         VALUES (?, ?, ?, ?, ?, ?)`,
      )
      .run(
        msg.id,
        msg.conversation_id,
        msg.role,
        msg.content,
        JSON.stringify(data ?? {}),
        msg.created_at,
      );
    this.db
      .query(`UPDATE conversations SET updated_at = ? WHERE id = ?`)
      .run(msg.created_at, conversationId);
    return msg;
  }

  // -- Memory ---------------------------------------------------------------

  addMemory(
    npcId: string,
    kind: MemoryKind,
    content: string,
    importance = 0.5,
  ): MemoryRecord {
    const rec: MemoryRecord = {
      id: randomUUID(),
      npc_id: npcId,
      kind,
      content,
      importance,
      created_at: new Date().toISOString(),
      last_access: new Date().toISOString(),
    };
    this.db
      .query(
        `INSERT INTO memory (id, npc_id, kind, content, importance, created_at, last_access)
         VALUES (?, ?, ?, ?, ?, ?, ?)`,
      )
      .run(rec.id, rec.npc_id, rec.kind, rec.content, rec.importance, rec.created_at, rec.last_access);
    return rec;
  }

  listMemory(npcId: string, limit = 50): MemoryRecord[] {
    const rows = this.db
      .query<MemRow, [string, number]>(
        `SELECT * FROM memory WHERE npc_id = ? ORDER BY importance DESC, created_at ASC LIMIT ?`,
      )
      .all(npcId, limit);
    for (const r of rows) {
      this.db
        .query(`UPDATE memory SET last_access = ? WHERE id = ?`)
        .run(new Date().toISOString(), r.id);
    }
    return rows.map(memRowToMem);
  }

  /** Ranked memory text for prompt assembly (importance-sorted, truncated). */
  memoryContext(npcId: string, limit = 10): string {
    const recs = this.listMemory(npcId, limit);
    if (recs.length === 0) return "";
    return (
      "\n[The character remembers]\n" +
      recs.map((r) => `- (${r.kind}) ${r.content}`).join("\n")
    );
  }

  // -- Story events ---------------------------------------------------------

  emitEvent(type: string, npcId: string | undefined, payload: Record<string, unknown>): StoryEvent {
    const event: StoryEvent = {
      id: randomUUID(),
      type,
      ...(npcId ? { npc_id: npcId } : {}),
      payload,
      created_at: new Date().toISOString(),
    };
    this.db
      .query(
        `INSERT INTO story_events (id, type, npc_id, payload, created_at)
         VALUES (?, ?, ?, ?, ?)`,
      )
      .run(event.id, event.type, event.npc_id ?? null, JSON.stringify(event.payload), event.created_at);
    this.onEvent(event);
    return event;
  }

  // -- Dialogue loop --------------------------------------------------------

  /**
   * Build the character system prompt: system_prompt, biography, personality,
   * knowledge, and a snapshot of ranked memory. The session keeps its own
   * live conversation history, so this is the static identity chunk.
   */
  characterPrompt(npc: NpcDef): string {
    const blocks: string[] = [];
    if (npc.biography) blocks.push(`Role: ${npc.name}. ${npc.biography}`);
    if (npc.personality) blocks.push(`Personality: ${npc.personality}`);
    if (npc.knowledge) blocks.push(`Knowledge: ${npc.knowledge}`);
    const memory = this.memoryContext(npc.id);
    if (memory) blocks.push(memory);
    return (
      (npc.system_prompt ? `${npc.system_prompt}\n\n` : "") +
      "You are an in-game character. Stay in character. Answer the player " +
      "directly as the character, no narration, no brackets.\n\n" +
      blocks.join("\n\n")
    );
  }

  /**
   * Run one turn: persist the player message, stream the NPC reply via the
   * LLM, persist the assistant message. `onToken` receives each decoded delta.
   * Returns the assistant message.
   */
  async turn(
    conversationId: string,
    content: string,
    onToken?: (delta: string) => void,
  ): Promise<DialogueMessage> {
    const conv = this.getConversation(conversationId);
    if (!conv) throw new Error(`unknown conversation ${conversationId}`);
    const npc = this.getNpc(conv.npc_id);
    if (!npc) throw new Error(`unknown npc ${conv.npc_id}`);

    this.insertMessage(conversationId, "user", content);

    let chat = this.chats.get(conversationId);
    if (!chat) {
      if (!this.llm) {
        throw new Error("LLM not configured: pass a PiLlm to the engine");
      }
      chat = await this.llm.newConversation(this.characterPrompt(npc));
      this.chats.set(conversationId, chat);
    }

    const reply = await chat.send(content, onToken);

    this.rememberFromTurn(npc.id, content, reply);
    const assistant = this.insertMessage(conversationId, "assistant", reply);
    this.emitEvent("npc.spoke", npc.id, {
      conversation_id: conversationId,
      name: npc.name,
    });
    return assistant;
  }

  /** Close live LLM sessions (e.g. on server shutdown). */
  dispose(): void {
    for (const chat of this.chats.values()) chat.close();
    this.chats.clear();
  }

  /** Simple heuristic memory extraction: summarize the tail once per turn. */
  private rememberFromTurn(npcId: string, userText: string, replyText: string): void {
    const userTail = userText.slice(-160).trim();
    if (userTail.length < 24) return;
    const taken = this.db
      .query<{ c: number }, [string, string]>(
        `SELECT COUNT(*) AS c FROM memory WHERE npc_id = ? AND content LIKE ?`,
      )
      .get(npcId, `%${userTail.slice(0, 40)}%`);
    if ((taken?.c ?? 0) > 0) return;
    this.addMemory(npcId, "event", `The player said: ${userTail}`, 0.4);
  }
}

// -- Row adapters -------------------------------------------------------------

type Row = Record<string, string | number | null> & {
  id: string;
  name: string;
  tagline?: string | null;
  biography?: string | null;
  personality?: string | null;
  knowledge?: string | null;
  system_prompt?: string | null;
  data: string;
  created_at: string;
  updated_at: string;
};

function rowToNpc(r: Row): NpcDef {
  return {
    id: r.id,
    name: r.name,
    ...(r.tagline ? { tagline: r.tagline } : {}),
    ...(r.biography ? { biography: r.biography } : {}),
    ...(r.personality ? { personality: r.personality } : {}),
    ...(r.knowledge ? { knowledge: r.knowledge } : {}),
    ...(r.system_prompt ? { system_prompt: r.system_prompt } : {}),
    data: JSON.parse(r.data),
    created_at: r.created_at,
    updated_at: r.updated_at,
  };
}

type MsgRow = Record<string, unknown> & {
  id: string;
  conversation_id: string;
  role: DialogueMessage["role"];
  content: string;
  data: string;
  created_at: string;
};

function msgRowToMsg(r: MsgRow): DialogueMessage {
  const parsed = JSON.parse(r.data) as Record<string, unknown>;
  return {
    id: r.id,
    conversation_id: r.conversation_id,
    role: r.role,
    content: r.content,
    created_at: r.created_at,
    ...(Object.keys(parsed).length ? { data: parsed } : {}),
  };
}

type MemRow = Record<string, unknown> & {
  id: string;
  npc_id: string;
  kind: MemoryKind;
  content: string;
  importance: number;
  created_at: string;
  last_access: string;
};

function memRowToMem(r: MemRow): MemoryRecord {
  return {
    id: r.id,
    npc_id: r.npc_id,
    kind: r.kind,
    content: r.content,
    importance: Number(r.importance),
    created_at: r.created_at,
    last_access: r.last_access,
  };
}
