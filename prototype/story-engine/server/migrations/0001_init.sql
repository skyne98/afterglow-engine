-- 0001_init.sql
-- Initial story-engine schema: NPCs, conversations, messages, memory, events.
-- A single transaction runs each migration.

CREATE TABLE IF NOT EXISTS npcs (
  id          TEXT PRIMARY KEY,
  name        TEXT NOT NULL,
  tagline     TEXT,
  biography   TEXT,
  personality TEXT,
  knowledge   TEXT,
  system_prompt TEXT,
  data        TEXT NOT NULL DEFAULT '{}',  -- JSON document owned by game code
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS conversations (
  id          TEXT PRIMARY KEY,
  npc_id      TEXT NOT NULL REFERENCES npcs(id) ON DELETE CASCADE,
  session_id  TEXT NOT NULL,
  title       TEXT,
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_conversations_npc ON conversations(npc_id, created_at);

CREATE TABLE IF NOT EXISTS messages (
  id              TEXT PRIMARY KEY,
  conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
  role            TEXT NOT NULL CHECK (role IN ('system','assistant','user','story')),
  content         TEXT NOT NULL,
  data            TEXT NOT NULL DEFAULT '{}',  -- optional JSON payload for role='story'
  created_at      TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_messages_conv ON messages(conversation_id, created_at);

CREATE TABLE IF NOT EXISTS memory (
  id          TEXT PRIMARY KEY,
  npc_id      TEXT NOT NULL REFERENCES npcs(id) ON DELETE CASCADE,
  kind        TEXT NOT NULL CHECK (kind IN ('fact','relationship','event','preference')),
  content     TEXT NOT NULL,
  importance  REAL NOT NULL DEFAULT 0.5,
  created_at  TEXT NOT NULL,
  last_access TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_memory_npc ON memory(npc_id, importance);

CREATE TABLE IF NOT EXISTS story_events (
  id          TEXT PRIMARY KEY,
  type        TEXT NOT NULL,
  npc_id      TEXT REFERENCES npcs(id) ON DELETE SET NULL,
  payload     TEXT NOT NULL DEFAULT '{}',
  created_at  TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_story_events_type ON story_events(type, created_at);
