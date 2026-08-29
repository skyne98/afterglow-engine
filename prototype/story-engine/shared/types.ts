/**
 * Shared story-engine types.
 *
 * This module is the single source of truth for the wire contract between the
 * Bun server and the Vue web client, and for the SQLite row shapes. It is
 * plain TypeScript with no runtime dependencies so both runtimes can import it.
 */

// ---------------------------------------------------------------------------
// NPC definition
// ---------------------------------------------------------------------------

export interface NpcDef {
  id: string;
  name: string;
  /** Short one-line persona shown in the roster. */
  tagline?: string;
  /** Separator-filled biography given to the model as context. */
  biography?: string;
  /** Personality traits, comma-separated. */
  personality?: string;
  /** Adjunct facts about the world the NPC knows. */
  knowledge?: string;
  /** The extra system-turn text glued before the biography. */
  system_prompt?: string;
  /** Free-form auxiliary JSON the game code owns. */
  data: Record<string, unknown>;
  created_at: string;
  updated_at: string;
}

// ---------------------------------------------------------------------------
// Dialogue
// ---------------------------------------------------------------------------

export type MessageRole = "system" | "assistant" | "user" | "story";

export interface DialogueMessage {
  id: string;
  conversation_id: string;
  role: MessageRole;
  content: string;
  /** Optional structured payload for story events (role = "story"). */
  data?: Record<string, unknown>;
  created_at: string;
}

export interface Conversation {
  id: string;
  npc_id: string;
  session_id: string;
  title?: string;
  created_at: string;
  updated_at: string;
}

// ---------------------------------------------------------------------------
// Memory
// ---------------------------------------------------------------------------

export type MemoryKind = "fact" | "relationship" | "event" | "preference";

export interface MemoryRecord {
  id: string;
  npc_id: string;
  kind: MemoryKind;
  content: string;
  /** 0..1 salience used to rank entries for the model context window. */
  importance: number;
  created_at: string;
  last_access: string;
}

// ---------------------------------------------------------------------------
// Story events
// ---------------------------------------------------------------------------

export interface StoryEvent {
  id: string;
  type: string;
  /** npc_id the event concerns, when applicable. */
  npc_id?: string;
  payload: Record<string, unknown>;
  created_at: string;
}

// ---------------------------------------------------------------------------
// Wire messages (client <-> server)
// ---------------------------------------------------------------------------

export type ClientMessage =
  | { type: "npc.list" }
  | { type: "npc.get"; id: string }
  | { type: "npc.create"; npc: NpcDef }
  | { type: "chat.start"; npc_id: string; session_id?: string }
  | { type: "chat.send"; conversation_id: string; content: string }
  | { type: "chat.history"; conversation_id: string; before_id?: string; limit?: number }
  | { type: "memory.list"; npc_id: string };

export type ServerMessage =
  | { type: "npc.list.result"; items: NpcDef[] }
  | { type: "npc.get.result"; npc: NpcDef | null }
  | { type: "npc.created"; npc: NpcDef }
  | { type: "chat.started"; conversation: Conversation }
  | { type: "token"; conversation_id: string; delta: string }
  | { type: "message"; message: DialogueMessage }
  | { type: "chat.history.result"; messages: DialogueMessage[] }
  | { type: "memory.list.result"; items: MemoryRecord[] }
  | { type: "story"; event: StoryEvent }
  | { type: "error"; message: string };
