import { describe, expect, test } from "bun:test";
import { createSandboxModel } from "./model";

describe("SandboxModel", () => {
  test("place creates entities and selects the new one", () => {
    const m = createSandboxModel();
    m.tool = "npc";
    const a = m.place(2, 3);
    expect(a).toEqual({ id: 1, kind: "npc", col: 2, row: 3, bubble: null });
    expect(m.entities).toHaveLength(1);
    expect(m.selectedId).toBe(1);
    expect(m.entityAt(2, 3)).toBe(a);
  });

  test("addEntity creates a player and carries npc binding meta", () => {
    const m = createSandboxModel();
    const p = m.addEntity("player", 5, 5, { name: "You" });
    expect(p?.kind).toBe("player");
    expect(p?.name).toBe("You");
    const n = m.addEntity("npc", 6, 5, { npcId: "n-1", name: "Mira" });
    expect(n?.npcId).toBe("n-1");
    expect(n?.name).toBe("Mira");
    expect(m.entityAt(6, 5)).toBe(n);
  });

  test("setBubble reactively sets and clears an entity bubble", () => {
    const m = createSandboxModel();
    m.tool = "npc";
    const e = m.place(0, 0)!;
    m.setBubble(e.id, "hello");
    expect(e.bubble).toBe("hello");
    m.setBubble(e.id, null);
    expect(e.bubble).toBeNull();
  });

  test("cannot place two entities on the same cell", () => {
    const m = createSandboxModel();
    m.tool = "npc";
    m.place(0, 0);
    expect(m.place(0, 0)).toBeUndefined();
    expect(m.entities).toHaveLength(1);
  });

  test("remove deletes by id and clears selection", () => {
    const m = createSandboxModel();
    m.tool = "prop";
    const e = m.place(4, 4)!;
    m.remove(e.id);
    expect(m.entities).toHaveLength(0);
    expect(m.entityAt(4, 4)).toBeUndefined();
    expect(m.selectedId).toBeNull();
  });

  test("removeAt deletes by cell", () => {
    const m = createSandboxModel();
    m.tool = "npc";
    m.place(1, 1);
    m.removeAt(1, 1);
    expect(m.entities).toHaveLength(0);
  });

  test("move updates the cell and the index", () => {
    const m = createSandboxModel();
    m.tool = "npc";
    const e = m.place(0, 0)!;
    expect(m.move(e.id, 5, 5)).toBe(true);
    expect(e.col).toBe(5);
    expect(e.row).toBe(5);
    expect(m.entityAt(0, 0)).toBeUndefined();
    expect(m.entityAt(5, 5)).toBe(e);
  });

  test("select picks an entity and null clears it", () => {
    const m = createSandboxModel();
    m.tool = "npc";
    const a = m.place(0, 0)!;
    m.place(1, 1);
    m.select(a.id);
    expect(m.selectedId).toBe(a.id);
    m.select(null);
    expect(m.selectedId).toBeNull();
  });

  test("move is blocked by an occupied cell and is a no-op on same cell", () => {
    const m = createSandboxModel();
    m.tool = "npc";
    const a = m.place(0, 0)!;
    m.tool = "prop";
    const b = m.place(3, 3)!;
    expect(m.move(a.id, 3, 3)).toBe(false);
    expect(a.col).toBe(0);
    expect(m.move(a.id, 0, 0)).toBe(false);
  });

  test("clear removes everything", () => {
    const m = createSandboxModel();
    m.tool = "npc";
    m.place(0, 0);
    m.place(1, 1);
    m.place(2, 2);
    m.clear();
    expect(m.entities).toHaveLength(0);
    expect(m.selectedId).toBeNull();
  });

  test("speak shows a bubble synchronously and clears after the duration", async () => {
    const m = createSandboxModel();
    m.tool = "npc";
    const e = m.place(1, 1)!;
    expect(e.bubble).toBeNull();
    const p = m.speak(e.id, "Hi there", 60);
    expect(e.bubble).toBe("Hi there");
    await p;
    expect(e.bubble).toBeNull();
  });

  test("speak resolves immediately for a missing entity", async () => {
    const m = createSandboxModel();
    await expect(m.speak(999, "x", 50)).resolves.toBeUndefined();
  });

  test("removing an entity cancels its pending speak", async () => {
    const m = createSandboxModel();
    m.tool = "npc";
    const e = m.place(0, 0)!;
    const p = m.speak(e.id, "hi", 200);
    m.remove(e.id);
    await expect(p).resolves.toBeUndefined();
  });
});
