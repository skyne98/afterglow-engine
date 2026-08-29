<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch, watchEffect } from "vue";
import { Application, Container, Graphics, Text } from "pixi.js";
import { StoryWorld } from "../world/story-world";
import type { Entity, Kind, Tool } from "../sandbox/model";

/**
 * The combined world view. The sandbox is where "we" exist: the player is a
 * `player` entity, real NPCs are `npc` entities bound to server records, and
 * the docked chat panel sends messages that the target NPC replies to (streamed
 * into the log and a live bubble). Everything is driven by the reactive
 * `StoryWorld.model`; Pixi just mirrors it.
 */

const TILE = 48;
const COLORS: Record<Kind, number> = { player: 0x7ef08e, npc: 0x5b9dff, prop: 0x8dd45b };
const KIND_LABEL_UI: Record<Kind, string> = { player: "You", npc: "NPC", prop: "Prop" };

const world = new StoryWorld();
const model = world.model;
const host = ref<HTMLElement | null>(null);
const logEl = ref<HTMLElement | null>(null);
const chatInput = ref("");
const status = ref("You are the green player. Click an NPC or walk near one, then type to talk.");

interface Display {
  root: Container;
  tile: Graphics;
  label: Text;
  bubble: Container;
  bubbleBg: Graphics;
  bubbleLabel: Text;
  wantBubble: boolean;
  bubbleAlpha: number;
  bubbleOx: number;
  bubbleOy: number;
}

let app: Application | null = null;
let grid: Graphics;
let entityLayer: Container;
let bubbleLayer: Container;
const display = new Map<number, Display>();
let dragId: number | null = null;
let dragMoved = false;
let lastW = 0;
let lastH = 0;
let disposed = false;

function maxCol(): number {
  return Math.max(1, Math.floor((app?.screen.width ?? 0) / TILE) - 1);
}
function maxRow(): number {
  return Math.max(1, Math.floor((app?.screen.height ?? 0) / TILE) - 1);
}

function drawGrid() {
  if (!app) return;
  const w = Math.ceil(app.screen.width / TILE) + 1;
  const h = Math.ceil(app.screen.height / TILE) + 1;
  grid.clear();
  grid.rect(0, 0, app.screen.width, app.screen.height).fill(0x14161b);
  for (let c = 0; c <= w; c++) grid.moveTo(c * TILE, 0).lineTo(c * TILE, h * TILE);
  for (let r = 0; r <= h; r++) grid.moveTo(0, r * TILE).lineTo(w * TILE, r * TILE);
  grid.stroke({ width: 1, color: 0x2a2f3a, alpha: 0.5 });
  lastW = app.screen.width;
  lastH = app.screen.height;
}

function paintEntity(e: Entity, sel: boolean, d: Display) {
  const size = TILE - 8;
  const inset = 4;
  d.tile.clear();
  if (e.kind === "player") {
    d.tile.circle(TILE / 2, TILE / 2, (size + 2) / 2).fill(COLORS.player);
  } else {
    d.tile.roundRect(inset, inset, size, size, 6).fill(COLORS[e.kind]);
  }
  d.tile.stroke({
    width: sel ? 3 : 1,
    color: e.kind === "player" ? (sel ? 0xffd75b : 0xffffff) : sel ? 0xffd75b : 0xffffff,
    alpha: sel ? 1 : e.kind === "player" ? 0.9 : 0.7,
  });
  d.label.text = e.kind === "player" ? "You" : `${KIND_LABEL_UI[e.kind]} ${e.id}`;
  d.label.anchor.set(0.5, 0);
  d.label.position.set(TILE / 2, inset + size + 2);
}

function layoutBubble(e: Entity, d: Display, text: string) {
  d.bubbleLabel.text = text;
  const w = Math.min(140, Math.max(36, d.bubbleLabel.width + 16));
  const h = d.bubbleLabel.height + 8;
  d.bubbleBg.clear();
  d.bubbleBg.roundRect(0, 0, w, h, 8).fill(0x0f1116).stroke({ width: 2, color: COLORS[e.kind] });
  d.bubbleLabel.position.set((w - d.bubbleLabel.width) / 2, 4);
  d.bubbleOx = (TILE - w) / 2;
  d.bubbleOy = -h - 2;
}

function createDisplay(e: Entity): Display {
  const tile = new Graphics();
  const label = new Text({ text: "", style: { fontSize: 11, fill: 0xffffff } });
  const root = new Container();
  root.eventMode = "static";
  root.cursor = "pointer";
  root.addChild(tile, label);
  entityLayer.addChild(root);

  const bubbleLabel = new Text({
    text: "",
    style: { fontSize: 12, fill: 0xffffff, wordWrap: true, wordWrapWidth: 140 },
  });
  const bubbleBg = new Graphics();
  const bubble = new Container();
  bubble.addChild(bubbleBg, bubbleLabel);
  bubble.visible = false;
  bubbleLayer.addChild(bubble);

  return { root, tile, label, bubble, bubbleBg, bubbleLabel, wantBubble: false, bubbleAlpha: 0, bubbleOx: 0, bubbleOy: 0 };
}

function reconcile() {
  for (const e of model.entities) {
    let d = display.get(e.id);
    if (!d) {
      d = createDisplay(e);
      display.set(e.id, d);
    }
    d.root.position.set(e.col * TILE, e.row * TILE);
    d.root.zIndex = e.row;
    paintEntity(e, e.id === model.selectedId, d);

    const wants = e.bubble !== null && e.kind !== "prop";
    if (wants) layoutBubble(e, d, e.bubble as string);
    d.wantBubble = wants;
  }
  for (const [id, d] of display) {
    if (!model.entities.some((x) => x.id === id)) {
      entityLayer.removeChild(d.root);
      bubbleLayer.removeChild(d.bubble);
      d.root.destroy();
      display.delete(id);
    }
  }
}

function tickBubbles() {
  for (const d of display.values()) {
    const target = d.wantBubble ? 1 : 0;
    d.bubbleAlpha += (target - d.bubbleAlpha) * 0.2;
    if (Math.abs(target - d.bubbleAlpha) < 0.02) d.bubbleAlpha = target;
    if (d.bubbleAlpha <= 0.02) {
      d.bubble.visible = false;
      continue;
    }
    d.bubble.position.set(d.root.position.x + d.bubbleOx, d.root.position.y + d.bubbleOy);
    d.bubble.alpha = d.bubbleAlpha;
    d.bubble.visible = true;
  }
}

// -- Input -------------------------------------------------------------------

function cellFromEvent(ev: PointerEvent): { col: number; row: number } | null {
  if (!app) return null;
  const canvas = app.canvas as HTMLCanvasElement;
  const rect = canvas.getBoundingClientRect();
  const scaleX = app.screen.width / rect.width;
  const scaleY = app.screen.height / rect.height;
  const x = (ev.clientX - rect.left) * scaleX;
  const y = (ev.clientY - rect.top) * scaleY;
  if (x < 0 || y < 0) return null;
  return { col: Math.min(maxCol(), Math.floor(x / TILE)), row: Math.min(maxRow(), Math.floor(y / TILE)) };
}

function onPointerDown(ev: PointerEvent) {
  const cell = cellFromEvent(ev);
  if (!cell) return;
  ev.preventDefault();
  try {
    (ev.target as HTMLElement).setPointerCapture?.(ev.pointerId);
  } catch {
    /* non-capturable / synthetic events */
  }

  if (model.tool === "remove") {
    if (model.entityAt(cell.col, cell.row) && model.entityAt(cell.col, cell.row)!.kind !== "player") {
      model.removeAt(cell.col, cell.row);
      status.value = `Removed entity at (${cell.col}, ${cell.row}).`;
    }
    return;
  }

  const existing = model.entityAt(cell.col, cell.row);
  if (existing) {
    model.select(existing.id);
    dragId = existing.id;
    dragMoved = false;
    status.value = `${existing.kind === "player" ? "You" : KIND_LABEL_UI[existing.kind] + " " + existing.id} selected at (${existing.col}, ${existing.row}).`;
    return;
  }

  const e = model.place(cell.col, cell.row);
  if (e && e.kind !== "player") {
    status.value = `Placed ${KIND_LABEL_UI[e.kind]} ${e.id} at (${e.col}, ${e.row}).`;
    if (e.kind === "npc") {
      void model.speak(e.id, world.talkTarget() ? "Hello." : "Hello.", 2000);
    }
  }
}

function onPointerMove(ev: PointerEvent) {
  if (dragId === null) return;
  const cell = cellFromEvent(ev);
  if (!cell) return;
  if (model.move(dragId, cell.col, cell.row)) dragMoved = true;
}

function onPointerUp(ev: PointerEvent) {
  if (dragId !== null) {
    const e = model.entities.find((x) => x.id === dragId);
    if (e && dragMoved) {
      status.value = `${e.kind === "player" ? "You moved" : `Moved ${KIND_LABEL_UI[e.kind]} ${e.id}`} to (${e.col}, ${e.row}).`;
    }
  }
  dragId = null;
  dragMoved = false;
  try {
    (ev.target as HTMLElement).releasePointerCapture?.(ev.pointerId);
  } catch {
    /* ignore */
  }
}

function selectTool(t: Tool) {
  model.tool = t;
  dragId = null;
}

function sendChat() {
  const text = chatInput.value;
  if (!text.trim() || world.streamingActive) return;
  chatInput.value = "";
  world.send(text);
}

onMounted(async () => {
  if (!host.value) return;
  app = new Application();
  await app.init({ resizeTo: host.value, background: 0x14161b, antialias: true });
  host.value.appendChild(app.canvas);

  grid = new Graphics();
  entityLayer = new Container();
  entityLayer.sortableChildren = true;
  bubbleLayer = new Container();
  app.stage.addChild(grid, entityLayer, bubbleLayer);

  const canvas = app.canvas as HTMLCanvasElement;
  canvas.style.width = "100%";
  canvas.style.height = "100%";
  canvas.addEventListener("pointerdown", onPointerDown);
  canvas.addEventListener("pointermove", onPointerMove);
  canvas.addEventListener("pointerup", onPointerUp);
  canvas.addEventListener("pointercancel", onPointerUp);

  drawGrid();
  void world.seed(maxCol(), maxRow());

  watchEffect(() => reconcile());

  app.ticker.add(() => {
    if (disposed) return;
    tickBubbles();
    if (app && (app.screen.width !== lastW || app.screen.height !== lastH)) {
      drawGrid();
      for (const e of [...model.entities]) {
        const cc = Math.min(maxCol(), Math.max(0, e.col));
        const rr = Math.min(maxRow(), Math.max(0, e.row));
        if (cc !== e.col || rr !== e.row) model.move(e.id, cc, rr);
      }
    }
  });
  requestAnimationFrame(() => {
    if (app) app.renderer.resize(host.value!.clientWidth, host.value!.clientHeight);
  });
});

watch(
  () => world.log.length,
  () => {
    requestAnimationFrame(() => {
      if (logEl.value) logEl.value.scrollTop = logEl.value.scrollHeight;
    });
  },
);

onBeforeUnmount(() => {
  disposed = true;
  app?.destroy(true, { children: true });
  app = null;
});
</script>

<template>
  <div class="world">
    <div class="toolbar">
      <button :class="{ active: model.tool === 'npc' }" @click="selectTool('npc')">🧙 NPC</button>
      <button :class="{ active: model.tool === 'prop' }" @click="selectTool('prop')">🪵 Prop</button>
      <button :class="{ active: model.tool === 'remove' }" @click="selectTool('remove')">✕ Remove</button>
      <button class="subtle" @click="model.clear()">Clear</button>
      <span class="hint">{{ status }}</span>
    </div>

    <div class="body">
      <div ref="host" class="canvas-host"></div>

      <aside class="chat">
        <h3>Log</h3>
        <div ref="logEl" class="log">
          <div v-if="!world.log.length" class="empty">No dialogue yet. Type below to speak to the nearest NPC.</div>
          <div v-for="l in world.log" :key="l.id" class="line" :class="l.role">
            <span class="who">{{ l.role === "player" ? "You" : l.speaker }}</span>
            <p>{{ l.text }}<span v-if="l.streaming" class="caret">▌</span></p>
          </div>
        </div>
        <form class="composer" @submit.prevent="sendChat">
          <input
            v-model="chatInput"
            placeholder="Say something…"
            :disabled="world.streamingActive"
          />
          <button type="submit" class="primary" :disabled="world.streamingActive">Send</button>
        </form>
      </aside>
    </div>
  </div>
</template>

<style scoped>
.world {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}
.toolbar {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 16px;
  border-bottom: 1px solid #ffffff22;
  flex-wrap: wrap;
}
.toolbar button.active {
  background: var(--btn-accent-soft);
  border-color: var(--btn-accent);
  color: #cfe3ff;
}
.toolbar .hint {
  margin-left: auto;
  font-size: 0.85em;
  color: #aaa;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.body {
  flex: 1;
  min-height: 0;
  display: flex;
}
.canvas-host {
  flex: 1;
  min-height: 0;
  position: relative;
  overflow: hidden;
}
.canvas-host :deep(canvas) {
  display: block;
}
.chat {
  width: 300px;
  border-left: 1px solid #ffffff22;
  display: flex;
  flex-direction: column;
  min-height: 0;
}
.chat h3 {
  margin: 0;
  padding: 10px 14px;
  font-size: 0.95em;
  border-bottom: 1px solid #ffffff22;
}
.log {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 12px 14px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.log .empty {
  color: #888;
  font-size: 0.85em;
}
.log .line {
  max-width: 92%;
}
.log .line.player {
  align-self: flex-end;
  text-align: right;
}
.log .line.npc {
  align-self: flex-start;
}
.log .who {
  display: block;
  font-size: 0.72em;
  opacity: 0.7;
  margin-bottom: 2px;
}
.log .line p {
  margin: 0;
  padding: 8px 10px;
  border-radius: 10px;
  background: #ffffff12;
  white-space: pre-wrap;
  word-break: break-word;
}
.log .line.player p {
  background: #1f6feb;
}
.caret {
  animation: blink 1s steps(1) infinite;
}
@keyframes blink {
  50% {
    opacity: 0;
  }
}
.composer {
  display: flex;
  gap: 6px;
  padding: 10px;
  border-top: 1px solid #ffffff22;
}
.composer input {
  flex: 1;
  min-width: 0;
  height: var(--btn-h);
  padding: 0 12px;
  border-radius: var(--btn-radius);
  border: 1px solid var(--btn-border);
  background: var(--btn-bg);
  color: inherit;
}
.composer input:focus-visible {
  outline: 2px solid var(--btn-accent);
  outline-offset: -1px;
}
.composer input:disabled {
  opacity: 0.6;
}
.composer button {
  flex: 0 0 auto;
}
</style>
