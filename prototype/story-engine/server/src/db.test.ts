import { describe, expect, test } from "bun:test";
import { Database } from "bun:sqlite";
import { applyMigrations, appliedMigrations } from "./migrations";
import { MIGRATIONS_DIR } from "./db";
import { NpcEngine } from "./npc-engine";

function memDb(): Database {
  return new Database(":memory:");
}

describe("migrations", () => {
  test("Schema migrations apply and are recorded", () => {
    const db = memDb();
    const ran = applyMigrations(db, MIGRATIONS_DIR);
    expect(ran.length).toBeGreaterThan(0);
    expect(ran).toContain("0001_init.sql");

    // Tables exist.
    const tables = db
      .query<{ name: string }, []>(
        `SELECT name FROM sqlite_master WHERE type='table'`,
      )
      .all()
      .map((r) => r.name);
    for (const t of ["npcs", "conversations", "messages", "memory", "story_events", "_schema_migrations"]) {
      expect(tables).toContain(t);
    }

    // Re-running applies nothing new.
    const again = applyMigrations(db, MIGRATIONS_DIR);
    expect(again).toEqual([]);
    expect(appliedMigrations(db).size).toBe(ran.length);
    db.close();
  });
});

describe("NpcEngine CRUD", () => {
  test("Upsert, list, and get an NPC", () => {
    const db = memDb();
    applyMigrations(db, MIGRATIONS_DIR);
    const e = new NpcEngine(db);

    e.upsertNpc({
      id: "n1",
      name: "Mira",
      tagline: "A dockside smuggler",
      biography: "Mira runs the night market.",
      personality: "wry, guarded, shrewd",
      data: { faction: "Guild" },
    });

    const list = e.listNpcs();
    expect(list).toHaveLength(1);
    expect(list[0].name).toBe("Mira");
    expect(list[0].data).toEqual({ faction: "Guild" });

    const got = e.getNpc("n1");
    expect(got?.tagline).toBe("A dockside smuggler");
    expect(e.getNpc("missing")).toBeNull();
    db.close();
  });

  test("Conversation, messages, and memory persist", () => {
    const db = memDb();
    applyMigrations(db, MIGRATIONS_DIR);
    const e = new NpcEngine(db);

    e.upsertNpc({ id: "n2", name: "Kov", data: {} });
    const conv = e.startConversation("n2");
    expect(conv.npc_id).toBe("n2");

    const userMsg = e.insertMessage(conv.id, "user", "Hello there.");
    const asstMsg = e.insertMessage(conv.id, "assistant", "Well met.");
    const hist = e.history(conv.id);
    expect(hist.map((m) => m.role)).toEqual(["user", "assistant"]);
    expect(userMsg.conversation_id).toBe(conv.id);

    e.addMemory("n2", "event", "Player greeted Kov warmly.", 0.6);
    const mem = e.listMemory("n2");
    expect(mem).toHaveLength(1);
    expect(mem[0].kind).toBe("event");
    expect(e.memoryContext("n2")).toContain("greeted Kov");
    expect(e.memoryContext("missing")).toBe("");
    db.close();
  });

  test("Story events are emitted to the sink", () => {
    const db = memDb();
    applyMigrations(db, MIGRATIONS_DIR);
    const e = new NpcEngine(db);
    const seen: string[] = [];
    e.onEvent = (ev) => seen.push(ev.type);

    e.emitEvent("scene.entered", "n2", { room: "tavern" });
    expect(seen).toEqual(["scene.entered"]);
    db.close();
  });

  test("characterPrompt assembles identity and memory", () => {
    const db = memDb();
    applyMigrations(db, MIGRATIONS_DIR);
    const e = new NpcEngine(db);

    e.upsertNpc({
      id: "n3",
      name: "Orin",
      biography: "Orin tends the tavern hearth.",
      personality: "warm, chatty",
      system_prompt: "Only ever speak in short lines.",
      data: {},
    });
    e.addMemory("n3", "event", "The player asked about the cider.", 0.7);

    const p = e.characterPrompt(e.getNpc("n3")!);
    expect(p).toContain("Orin tends the tavern hearth.");
    expect(p).toContain("warm, chatty");
    expect(p).toContain("Only ever speak in short lines.");
    expect(p).toContain("asked about the cider");
    db.close();
  });
});
