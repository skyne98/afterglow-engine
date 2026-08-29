/**
 * SandboxModel — the reactive data model behind the story world.
 *
 * This is the single source of truth for world state. It is plain reactive
 * data (Vue `reactive`): `entities` is a reactive array of simple data objects
 * (kind, cell, optional npc binding + bubble), plus the active tool and
 * selection. Any code can read or mutate it and the Pixi renderer (and any UI)
 * stays in sync with no manual events.
 *
 * The model has no rendering or network knowledge: `place`/`move` reason only
 * about grid cells and collisions, `speak`/`setBubble` manage bubble lifetimes,
 * and dialogue is handled by the world layer above it.
 */
import { reactive } from "vue";

/** `player` is the "you" entity that exists in the world. */
export type Kind = "player" | "npc" | "prop";
/** Tools for hand-placing entities; the player is not a hand-placed kind. */
export type Tool = "npc" | "prop" | "remove";

export interface Entity {
  id: number;
  kind: Kind;
  col: number;
  row: number;
  /** Server NPC id this entity is bound to (npc kind only). */
  npcId?: string;
  /** Display name (npc binding or generated). */
  name?: string;
  /** Current speech-bubble text, or null when silent. Reactive. */
  bubble: string | null;
}

export interface SandboxModel {
  tool: Tool;
  selectedId: number | null;
  entities: Entity[];

  entityAt(col: number, row: number): Entity | undefined;
  /** Generic spawner; returns undefined when the cell is occupied. */
  addEntity(kind: Kind, col: number, row: number, meta?: { npcId?: string; name?: string }): Entity | undefined;
  /** Place an entity of the current tool kind. */
  place(col: number, row: number): Entity | undefined;
  remove(id: number): void;
  removeAt(col: number, row: number): void;
  select(id: number | null): void;
  move(id: number, col: number, row: number): boolean;
  clear(): void;
  /** Reactively set or clear an entity's bubble without a timed lifecycle. */
  setBubble(id: number, text: string | null): void;
  /** Show a bubble for `durationMs`, then hide it. Resolves when done. */
  speak(id: number, text: string, durationMs?: number): Promise<void>;
}

/** How long the hide fade-out runs; the renderer uses the same value. */
export const BUBBLE_FADE_MS = 250;

function sleep(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}

export function createSandboxModel(): SandboxModel {
  const byKey = new Map<string, Entity>();
  const speakGen = new Map<number, number>();
  let nextId = 1;
  const key = (c: number, r: number): string => `${c},${r}`;

  let model: SandboxModel;
  model = reactive<SandboxModel>({
    tool: "npc",
    selectedId: null,
    entities: [],

    entityAt: (col, row): Entity | undefined => byKey.get(key(col, row)),

    addEntity: (kind, col, row, meta): Entity | undefined => {
      if (model.entityAt(col, row)) return undefined;
      model.entities.push({
        id: nextId++,
        kind,
        col,
        row,
        ...(meta?.npcId ? { npcId: meta.npcId } : {}),
        ...(meta?.name ? { name: meta.name } : {}),
        bubble: null,
      });
      const e = model.entities[model.entities.length - 1];
      byKey.set(key(col, row), e);
      model.selectedId = e.id;
      return e;
    },

    place: (col, row): Entity | undefined => model.addEntity(model.tool as Kind, col, row),

    remove: (id): void => {
      const e = model.entities.find((x) => x.id === id);
      if (!e) return;
      model.entities.splice(model.entities.indexOf(e), 1);
      byKey.delete(key(e.col, e.row));
      speakGen.delete(id);
      if (model.selectedId === id) model.selectedId = null;
    },

    removeAt: (col, row): void => {
      const e = byKey.get(key(col, row));
      if (e) model.remove(e.id);
    },

    select: (id): void => {
      model.selectedId = id;
    },

    move: (id, col, row): boolean => {
      const e = model.entities.find((x) => x.id === id);
      if (!e) return false;
      const occupant = byKey.get(key(col, row));
      if (occupant && occupant.id !== id) return false; // collision: blocked
      if (e.col === col && e.row === row) return false; // no change
      byKey.delete(key(e.col, e.row));
      e.col = col;
      e.row = row;
      byKey.set(key(col, row), e);
      return true;
    },

    clear: (): void => {
      model.entities.splice(0);
      byKey.clear();
      speakGen.clear();
      model.selectedId = null;
    },

    setBubble: (id, text): void => {
      const e = model.entities.find((x) => x.id === id);
      if (!e) return;
      e.bubble = text;
    },

    speak: (id, text, durationMs = 3000): Promise<void> => {
      const e = model.entities.find((x) => x.id === id);
      if (!e) return Promise.resolve();
      const gen = (speakGen.get(id) ?? 0) + 1;
      speakGen.set(id, gen);
      e.bubble = text;
      return (async () => {
        await sleep(durationMs);
        // A newer speak on this entity supersedes this one; do not clear it.
        if (speakGen.get(id) !== gen) return;
        e.bubble = null;
        await sleep(BUBBLE_FADE_MS);
      })();
    },
  }) as SandboxModel;

  return model;
}
