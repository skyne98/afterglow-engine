/* libmypaint NG paint demo — thin client.
 * The engine runs in paint-engine-worker.ts (Web Worker with a tile pool).
 * This module captures input, manages DOM/UI, and forwards to the worker.
 */
import { decodeZip, encodeStoredZip, text, utf8 } from './openraster.ts';
import { PaintSession } from './paint-session.ts';
import { openFile, saveFile } from '../../../crates/afterglow-web/web/src/engine/workers/platform-files.ts';
import { resolvePenPressure } from './paint-input.ts';
import { paintMemoryLimitMiB } from './paint-memory.ts';
import { PaintPointerState } from './paint-pointer-state.ts';
import { inversePaintView } from './paint-view.ts';
import { resolvePaintShortcut, type PaintShortcut } from './paint-shortcuts.ts';
import { buildPaintLayerRows, samePaintLayerState, type PaintGroupInfo, type PaintLayerInfo } from './paint-layers.ts';
import { rgbToHex } from './paint-color.ts';
import { StrokeStabilizer, type StrokeStabilizerMode } from './paint-stabilizer.ts';

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const canvas = $<HTMLCanvasElement>('paint');
const statusEl = $('status'), logEl = $('logs'), hudEl = $<HTMLDivElement>('hud');

function newPaintDocumentId(): string {
  if (crypto.randomUUID) return crypto.randomUUID();
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const hex = Array.from(bytes, byte => byte.toString(16).padStart(2, '0')).join('');
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

let worker: PaintSession | null = null;
let paintDocumentId = newPaintDocumentId();
let engineState: any = null;
let ready = false;
let recoveryPending = false;
let resolveInitialReady!: () => void;
let rejectInitialReady!: (error: Error) => void;
const initialReady = new Promise<void>((resolve, reject) => { resolveInitialReady = resolve; rejectInitialReady = reject; });
let resolveRecoveryPrompt!: () => void;
const recoveryPrompt = new Promise<void>(resolve => { resolveRecoveryPrompt = resolve; });
let exportSeq = 0;
let pendingTiles: { id: number; resolve: (v: { data: ArrayBuffer[]; scale: number }) => void; reject: (error: Error) => void } | null = null;

const pointerState = new PaintPointerState();
let lastPX = 0, lastPY = 0;
const docSize = { width: 2048, height: 2048 };
const view = { zoom: 1, rotationDegrees: 0, mirror: false, panX: 0, panY: 0 };
let dispW = canvas.width, dispH = canvas.height;
const ui = { radius: 14, hardness: 0.6, opacity: 1.0, color: '#4ecdc4' };
type ActiveTool = 'brush' | 'eyedropper' | 'hand' | 'rotate' | 'zoom';
let activeTool: ActiveTool = 'brush';
let viewDragMode: 'pan' | 'rotate' | null = null;
let colorPickPointer: number | null = null;
let latestColorPickId = 0;
let spaceHeld = false;
const stabilizer = new StrokeStabilizer();
let stabilizerFrame: number | null = null;
let lastSourceWallTime = 0;
let lastStrokeTime = 0;
let lastStrokePressure = 0.5;
let lastStrokeXTilt = 0;
let lastStrokeYTilt = 0;
let stabilizerMode: StrokeStabilizerMode = 'off';
let stabilizerAmount = 20;
let stabilizerCatchUp = true;
const strokeSampleMessage = {
  cmd: 'strokeSample', x: 0, y: 0, pressure: 0, xtilt: 0, ytilt: 0,
  time: 0, zoom: 1, rotation: 0, barrel: 0.5,
};
const stabilizerModes = new Set<StrokeStabilizerMode>(['off', 'string', 'average', 'exponential', 'inertia']);
try {
  const saved = JSON.parse(localStorage.getItem('afterglow.paintStabilizer') ?? 'null') as
    { mode?: string; amount?: number; catchUp?: boolean } | null;
  if (saved?.mode && stabilizerModes.has(saved.mode as StrokeStabilizerMode)) {
    stabilizerMode = saved.mode as StrokeStabilizerMode;
  }
  if (Number.isFinite(saved?.amount)) stabilizerAmount = Math.max(1, Math.min(100, saved!.amount!));
  if (typeof saved?.catchUp === 'boolean') stabilizerCatchUp = saved.catchUp;
} catch {}
stabilizer.configure(stabilizerMode, stabilizerAmount, stabilizerCatchUp);
const deviceMemoryGiB = (navigator as Navigator & { deviceMemory?: number }).deviceMemory;
let savedPaintMemoryMiB: number | undefined;
let nativeMemoryLimitMiB: number | undefined;
try {
  const native = Number(localStorage.getItem('afterglow.nativePaintMemoryMiB'));
  if (Number.isFinite(native) && native > 0) nativeMemoryLimitMiB = native;
  const saved = Number(localStorage.getItem('afterglow.paintMemoryMiB'));
  if (saved > 0) savedPaintMemoryMiB = saved;
} catch {}
let paintMemoryMiB = paintMemoryLimitMiB(deviceMemoryGiB, savedPaintMemoryMiB);

type BrushPreset = { id: string; name: string; group: string; brush: string; preview: string };
let loadedBrushes: BrushPreset[] = [];
const brushGrid = $<HTMLDivElement>('brushGrid'), layerList = $<HTMLDivElement>('layerList');
let selectedGroupId = -1, selectedBrushId = '', selectedBrushJson = '';
const brushButtons = new Map<string, HTMLButtonElement>();
const layerModes = ['Normal','Multiply','Screen','Overlay','Darken','Lighten','Hard Light','Soft Light','Burn','Dodge','Difference','Exclusion','Hue','Saturation','Color','Luminosity','Plus','Destination In','Destination Out','Source Atop','Destination Atop','Pigment'];

function log(m: string) { logEl.textContent += m + '\n'; logEl.scrollTop = logEl.scrollHeight; }
function send(m: any, t?: Transferable[]) {
  if (recoveryPending && m.cmd !== 'init') return;
  worker?.send(m, t);
}
function releaseCanvasPointer(pointerId: number | null) {
  if (pointerId === null) return;
  try {
    if (canvas.hasPointerCapture?.(pointerId)) canvas.releasePointerCapture(pointerId);
  } catch {}
}
function commitPointerStroke(commit: boolean) {
  if (!commit) return;
  cancelStabilizerCatchUp();
  if (ready && stabilizer.isActive()) sendStabilizedPoint(performance.now(), 0);
  stabilizer.end();
  if (ready) send({ cmd: 'commit' });
}
function clearPointerInput(): boolean {
  const strokePointer = pointerState.strokePointer;
  const panPointer = pointerState.panPointer;
  const pickPointer = colorPickPointer;
  const commit = pointerState.finishForViewChange();
  viewDragMode = null;
  colorPickPointer = null;
  releaseCanvasPointer(strokePointer);
  if (panPointer !== strokePointer) releaseCanvasPointer(panPointer);
  if (pickPointer !== strokePointer && pickPointer !== panPointer) releaseCanvasPointer(pickPointer);
  return commit;
}
function finishInputForViewChange() {
  commitPointerStroke(clearPointerInput());
}
function sendCfg(s: [string, number][]) { send({ cmd: 'config', settings: s }); }
function brushUrl(p: string) { return new URL(`../mypaint/brushes/${p.split('/').map(encodeURIComponent).join('/')}`, document.baseURI).href; }
function hexRgb(v: string): [number, number, number] { const n = parseInt(v.slice(1), 16); return [((n>>16)&255)/255, ((n>>8)&255)/255, (n&255)/255]; }
function rgbHsv(r: number, g: number, b: number): [number, number, number] { const mx = Math.max(r,g,b), mn = Math.min(r,g,b), d = mx-mn; let h = 0; if (d) { if (mx===r) h=((g-b)/d)%6; else if (mx===g) h=(b-r)/d+2; else h=(r-g)/d+4; h/=6; if (h<0) h+=1; } return [h, mx===0?0:d/mx, mx]; }
function applyBrushColor() { const [r,g,b] = hexRgb(ui.color); const h = rgbHsv(r**2.2, g**2.2, b**2.2); sendCfg([['color_h',h[0]],['color_s',h[1]],['color_v',h[2]]]); }
function applyBrushOverrides() { sendCfg([['radius_logarithmic', Math.log(ui.radius)],['hardness', ui.hardness],['opaque_multiply', ui.opacity]]); }
function applyBgColor() { const [r,g,b] = hexRgb(($('backgroundColor') as HTMLInputElement).value); send({ cmd: 'setBackground', r, g, b }); }
function ensureBrush() { if (selectedBrushJson) { send({ cmd: 'loadBrush', json: selectedBrushJson }); applyBrushColor(); applyBrushOverrides(); } }

function applyView() {
  canvas.style.transform = `translate(${view.panX}px,${view.panY}px) scale(${view.zoom}) rotate(${view.rotationDegrees}deg) scaleX(${view.mirror?-1:1})`;
  canvas.style.imageRendering = view.zoom >= 2.5 ? 'pixelated' : 'auto';
  const zEl = $('viewZoom') as HTMLInputElement; zEl.value = String(view.zoom); $('zoomVal').textContent = String(Math.round(view.zoom * 100)); $('mirrorBtn').classList.toggle('active', view.mirror);
  send({ cmd: 'setView', zoom: view.zoom });
}
function setZoom(zoom: number) { finishInputForViewChange(); view.zoom = Math.max(0.1, Math.min(8, zoom)); applyView(); }
function fitView() { finishInputForViewChange(); Object.assign(view, { zoom: 1, panX: 0, panY: 0 }); applyView(); }
function actualPixels() { setZoom(docSize.width / Math.max(1, canvas.offsetWidth)); }
function setTool(tool: ActiveTool) {
  activeTool = tool;
  canvas.dataset.tool = tool;
  delete canvas.dataset.altEyedropper;
  for (const name of ['brush', 'eyedropper', 'hand', 'rotate', 'zoom'] as const) {
    const button = $<HTMLButtonElement>(`${name}ToolBtn`);
    button.classList.toggle('tool-active', name === tool);
    button.setAttribute('aria-pressed', String(name === tool));
  }
}
function updateColorSwatches() {
  const foreground = document.querySelector<HTMLElement>('.foreground-swatch');
  const toolbar = document.querySelector<HTMLElement>('.toolbar-color-swatch');
  const background = document.querySelector<HTMLElement>('.background-swatch');
  if (foreground) foreground.style.background = ui.color;
  if (toolbar) toolbar.style.background = ui.color;
  if (background) background.style.background = $<HTMLInputElement>('backgroundColor').value;
  $<HTMLInputElement>('color').dispatchEvent(new Event('change'));
}
function setForegroundColor(color: string) {
  ui.color = color;
  $<HTMLInputElement>('color').value = color;
  applyBrushColor();
  applyBrushOverrides();
  updateColorSwatches();
}
function requestColorPick(event: PointerEvent) {
  const [x, y] = pointerModel(event);
  send({ cmd: 'pickColor', id: ++latestColorPickId, x, y });
}
function setBrushValue(id: 'radius' | 'hardness' | 'opacity', value: number) {
  const input = $<HTMLInputElement>(id);
  input.value = String(value);
  input.dispatchEvent(new Event('input'));
}
function cycleBrush(offset: number) {
  if (loadedBrushes.length === 0) return;
  const index = loadedBrushes.findIndex((brush) => brush.id === selectedBrushId);
  const next = (Math.max(0, index) + offset + loadedBrushes.length) % loadedBrushes.length;
  void selectBrush(loadedBrushes[next]);
}
function pointerModel(e: PointerEvent): [number, number] {
  const r = canvas.getBoundingClientRect(), cx = r.left + r.width * 0.5, cy = r.top + r.height * 0.5;
  const [x, y] = inversePaintView(e.clientX - cx, e.clientY - cy, view.zoom, view.rotationDegrees, view.mirror);
  const cw = canvas.offsetWidth, ch = canvas.offsetHeight;
  const dx = (x + cw * 0.5) * (dispW / cw), dy = (y + ch * 0.5) * (dispH / ch);
  return [dx * (docSize.width / dispW), dy * (docSize.height / dispH)];
}
let lastKnownPenPressure = 0.5;
function penPressure(e: PointerEvent, contact = (e.buttons & 1) !== 0): number {
  const p = resolvePenPressure(e.pointerType, e.pressure, contact, lastKnownPenPressure);
  // Exact 0 while the pen is down means "missing", not "no force": on
  // Wayland, a compositor may omit an unchanged pressure from tool frames
  // and Chromium then reports 0 mid-stroke. Hold the last real value
  // (spec default 0.5 before the first real reading).
  if (p > 0) lastKnownPenPressure = p;
  return p;
}
function sendStabilizedPoint(time: number, pressure: number): void {
  lastStrokeTime = Math.max(lastStrokeTime + 0.01, time);
  strokeSampleMessage.x = stabilizer.x;
  strokeSampleMessage.y = stabilizer.y;
  strokeSampleMessage.pressure = pressure;
  strokeSampleMessage.xtilt = lastStrokeXTilt;
  strokeSampleMessage.ytilt = lastStrokeYTilt;
  strokeSampleMessage.time = lastStrokeTime;
  strokeSampleMessage.zoom = view.zoom;
  strokeSampleMessage.rotation = view.rotationDegrees * Math.PI / 180;
  send(strokeSampleMessage);
}
function cancelStabilizerCatchUp(): void {
  if (stabilizerFrame === null) return;
  cancelAnimationFrame(stabilizerFrame);
  stabilizerFrame = null;
}
function runStabilizerCatchUp(time: number): void {
  stabilizerFrame = null;
  if (!pointerState.strokeActive || !stabilizer.canCatchUp()) return;
  if (performance.now() - lastSourceWallTime < 24) {
    stabilizerFrame = requestAnimationFrame(runStabilizerCatchUp);
    return;
  }
  if (!stabilizer.stepCatchUp()) return;
  sendStabilizedPoint(time, lastStrokePressure);
  stabilizerFrame = requestAnimationFrame(runStabilizerCatchUp);
}
function scheduleStabilizerCatchUp(): void {
  if (stabilizerFrame === null && stabilizer.canCatchUp()) {
    stabilizerFrame = requestAnimationFrame(runStabilizerCatchUp);
  }
}
function sendSample(e: PointerEvent, pressure: number, scheduleCatchUp = true): boolean {
  const now = Number.isFinite(e.timeStamp) ? e.timeStamp : performance.now();
  const [x, y] = pointerModel(e);
  lastStrokeXTilt = Math.max(-1, Math.min(1, (e.tiltX / 90) || 0));
  lastStrokeYTilt = Math.max(-1, Math.min(1, (e.tiltY / 90) || 0));
  if (pressure > 0) lastStrokePressure = pressure;
  lastSourceWallTime = performance.now();
  const modelUnitsPerPixel = (docSize.width / dispW) / Math.max(0.1, view.zoom);
  const emitted = stabilizer.sample(x, y, now, modelUnitsPerPixel);
  if (emitted) sendStabilizedPoint(now, pressure);
  if (scheduleCatchUp && pressure > 0) scheduleStabilizerCatchUp();
  return emitted;
}
function beginStrokeAt(e: PointerEvent) {
  if (!ready) return;
  if (e.pointerType !== 'mouse') lastKnownPenPressure = 0.5;
  const [x, y] = pointerModel(e);
  const now = Number.isFinite(e.timeStamp) ? e.timeStamp : performance.now();
  stabilizer.begin(x, y, now);
  lastStrokeTime = now;
  lastStrokePressure = penPressure(e, true);
  send({ cmd: 'beginStroke', x, y, xtilt: (e.tiltX/90)||0, ytilt: (e.tiltY/90)||0, zoom: view.zoom, rotation: view.rotationDegrees * Math.PI / 180, barrel: 0.5 });
  sendSample(e, lastStrokePressure);
}
function endStroke(e: PointerEvent) {
  if (colorPickPointer === e.pointerId) {
    colorPickPointer = null;
    releaseCanvasPointer(e.pointerId);
    return;
  }
  if (pointerState.finishPan(e.pointerId)) {
    viewDragMode = null;
    releaseCanvasPointer(e.pointerId);
    return;
  }
  if (!pointerState.strokeActive || pointerState.strokePointer !== e.pointerId) return;
  cancelStabilizerCatchUp();
  sendSample(e, lastStrokePressure, false);
  if (stabilizer.finishCatchUp()) {
    sendStabilizedPoint(lastStrokeTime + 1000 / 120, lastStrokePressure);
  }
  commitPointerStroke(pointerState.finishStroke(e.pointerId));
  releaseCanvasPointer(e.pointerId);
}
canvas.addEventListener('pointerdown', e => {
  if (!ready) return;
  const viewDrag = e.button === 1 || (e.button === 0 && (spaceHeld || activeTool === 'hand' || activeTool === 'rotate'));
  if (viewDrag) {
    const oldStrokePointer = pointerState.strokePointer;
    const oldPanPointer = pointerState.panPointer;
    const commit = pointerState.beginPan(e.pointerId);
    if (oldStrokePointer !== e.pointerId) releaseCanvasPointer(oldStrokePointer);
    if (oldPanPointer !== oldStrokePointer && oldPanPointer !== e.pointerId) {
      releaseCanvasPointer(oldPanPointer);
    }
    commitPointerStroke(commit);
    viewDragMode = activeTool === 'rotate' && !spaceHeld && e.button === 0 ? 'rotate' : 'pan';
    lastPX = e.clientX;
    lastPY = e.clientY;
    try { canvas.setPointerCapture?.(e.pointerId); } catch {}
    return;
  }
  if (e.button !== 0) return;
  if (activeTool === 'eyedropper' || (activeTool === 'brush' && e.altKey)) {
    colorPickPointer = e.pointerId;
    requestColorPick(e);
    try { canvas.setPointerCapture?.(e.pointerId); } catch {}
    return;
  }
  if (activeTool === 'zoom') {
    setZoom(view.zoom * (e.altKey ? 0.8 : 1.25));
    return;
  }
  if (activeTool !== 'brush') return;
  const oldStrokePointer = pointerState.strokePointer;
  const oldPanPointer = pointerState.panPointer;
  const commit = pointerState.beginStroke(e.pointerId);
  if (oldStrokePointer !== e.pointerId) releaseCanvasPointer(oldStrokePointer);
  if (oldPanPointer !== oldStrokePointer && oldPanPointer !== e.pointerId) {
    releaseCanvasPointer(oldPanPointer);
  }
  commitPointerStroke(commit);
  beginStrokeAt(e);
  try { canvas.setPointerCapture?.(e.pointerId); } catch {}
});
canvas.addEventListener('pointermove', e => {
  if (e.pointerId === colorPickPointer) {
    if ((e.buttons & 1) !== 0) requestColorPick(e);
    else endStroke(e);
    return;
  }
  if (e.pointerId === pointerState.panPointer) {
    if (viewDragMode === 'rotate') {
      view.rotationDegrees = (view.rotationDegrees + (e.clientX - lastPX) * 0.5) % 360;
    } else {
      view.panX += e.clientX - lastPX;
      view.panY += e.clientY - lastPY;
    }
    lastPX = e.clientX;
    lastPY = e.clientY;
    applyView();
    return;
  }
  if (e.pointerId !== pointerState.strokePointer || !pointerState.strokeActive) return;
  // Contact comes from the button bit, never from e.pressure: on some
  // compositors (niri + Chromium wayland), a held constant pressure is omitted
  // from tool frames and the browser then reports pressure 0 mid-stroke.
  const contact = (e.buttons & 1) !== 0;
  if (!contact) {
    endStroke(e);
    return;
  }
  const samples = e.getCoalescedEvents();
  if (samples.length === 0) {
    sendSample(e, penPressure(e, true));
  } else {
    for (const sample of samples) sendSample(sample, penPressure(sample, true));
  }
});
canvas.addEventListener('pointerup', endStroke);
canvas.addEventListener('pointercancel', endStroke);
canvas.addEventListener('lostpointercapture', endStroke);
window.addEventListener('pointerup', endStroke);
window.addEventListener('pointercancel', endStroke);
window.addEventListener('blur', () => {
  spaceHeld = false;
  delete canvas.dataset.spaceHand;
  delete canvas.dataset.altEyedropper;
  finishInputForViewChange();
});
document.addEventListener('visibilitychange', () => {
  if (document.visibilityState === 'hidden') finishInputForViewChange();
});

const icon = {
  eye: '<svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12Z"/><circle cx="12" cy="12" r="3"/></svg>',
  eyeOff: '<svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="m3 3 18 18M10.6 10.6a2 2 0 0 0 2.8 2.8M9.9 4.2A11 11 0 0 1 12 4c6.5 0 10 8 10 8a17 17 0 0 1-2.1 3.2M6.6 6.6C3.7 8.5 2 12 2 12s3.5 8 10 8a10 10 0 0 0 3.4-.6"/></svg>',
  layer: '<svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="m12 2-9 5 9 5 9-5-9-5Z"/><path d="m3 12 9 5 9-5M3 17l9 5 9-5"/></svg>',
  group: '<svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 6h6l2 2h10v11H3Z"/></svg>',
};
function selectedLayerInfo(): PaintLayerInfo | PaintGroupInfo | null {
  if (!engineState) return null;
  const group = engineState.groups.find((item: PaintGroupInfo) => item.id === selectedGroupId && item.alive);
  if (group) return group;
  selectedGroupId = -1;
  return engineState.layers.find((item: PaintLayerInfo) => item.id === engineState.activeLayer) ?? null;
}
function setLayerSelection(kind: 'layer' | 'group', id: number) {
  if (kind === 'group') {
    selectedGroupId = id;
    refreshLayers();
  } else {
    selectedGroupId = -1;
    send({ cmd: 'layer', op: 'setActive', layer: id });
  }
}
function setVisibility(kind: 'layer' | 'group', id: number, visible: boolean) {
  send({ cmd: kind, op: 'setVisible', [kind]: id, value: visible ? 1 : 0 });
}
function sendSelection(op: string, value?: number) {
  if (!engineState) return;
  if (selectedGroupId >= 0) send({ cmd: 'group', op, group: selectedGroupId, value });
  else send({ cmd: 'layer', op, layer: engineState.activeLayer, value });
}
function groupIsInside(groupId: number, possibleParent: number): boolean {
  if (!engineState) return false;
  let next = possibleParent;
  while (next >= 0) {
    if (next === groupId) return true;
    next = engineState.groups.find((group: PaintGroupInfo) => group.id === next)?.parent ?? -1;
  }
  return false;
}
function refreshLayerInspector() {
  const info = selectedLayerInfo();
  const groupSelected = selectedGroupId >= 0;
  const blend = $<HTMLSelectElement>('layerBlendMode');
  const opacity = $<HTMLInputElement>('layerOpacity');
  const parent = $<HTMLSelectElement>('layerParent');
  blend.disabled = !info;
  opacity.disabled = !info;
  parent.disabled = !info;
  if (!info || !engineState) return;
  blend.value = String(info.mode);
  opacity.value = String(info.opacity);
  $('layerOpacityValue').textContent = `${Math.round(info.opacity * 100)}%`;
  parent.replaceChildren(Object.assign(document.createElement('option'), { textContent: 'Canvas', value: '-1' }));
  for (const group of engineState.groups as PaintGroupInfo[]) {
    if (!group.alive || groupSelected && groupIsInside(selectedGroupId, group.id)) continue;
    parent.append(Object.assign(document.createElement('option'), { textContent: `Group ${group.id + 1}`, value: String(group.id) }));
  }
  parent.value = String(groupSelected ? (info as PaintGroupInfo).parent : (info as PaintLayerInfo).group);
  const groupOptions = $<HTMLDivElement>('groupOptions');
  groupOptions.hidden = !groupSelected;
  if (groupSelected) {
    $<HTMLInputElement>('groupPassThrough').checked = (info as PaintGroupInfo).passThrough !== 0;
    $<HTMLInputElement>('groupIsolated').checked = (info as PaintGroupInfo).isolated !== 0;
  }
}
function refreshLayers() {
  if (!engineState) return;
  layerList.replaceChildren();
  const rows = buildPaintLayerRows(engineState.layers, engineState.groups);
  for (const item of rows) {
    const row = document.createElement('div');
    row.className = `layer-tree-row ${item.kind}-row`;
    row.style.setProperty('--indent', `${item.depth * 14}px`);
    const selected = item.kind === 'group' ? selectedGroupId === item.id : selectedGroupId < 0 && item.info.active;
    row.classList.toggle('selected', selected);
    row.classList.toggle('hidden-item', item.info.visible === 0);

    const visibility = document.createElement('button');
    visibility.type = 'button';
    visibility.className = 'layer-visibility';
    visibility.innerHTML = item.info.visible !== 0 ? icon.eye : icon.eyeOff;
    visibility.title = item.info.visible !== 0 ? 'Hide' : 'Show';
    visibility.setAttribute('aria-label', `${visibility.title} ${item.kind}`);
    visibility.onclick = () => setVisibility(item.kind, item.id, item.info.visible === 0);

    const thumbnail = document.createElement('span');
    thumbnail.className = 'layer-thumbnail';
    thumbnail.innerHTML = item.kind === 'group' ? icon.group : icon.layer;

    const name = document.createElement('button');
    name.type = 'button';
    name.className = 'layer-name';
    name.textContent = `${item.kind === 'group' ? 'Group' : 'Layer'} ${item.id + 1}`;
    name.onclick = () => setLayerSelection(item.kind, item.id);
    row.onclick = (event) => { if (event.target === row || thumbnail.contains(event.target as Node)) setLayerSelection(item.kind, item.id); };
    row.append(visibility, thumbnail, name);
    layerList.append(row);
  }
  refreshLayerInspector();
}
function renderCatalog(brushes: BrushPreset[]) { loadedBrushes = brushes; brushGrid.replaceChildren(); brushButtons.clear(); let last = ''; for (const b of brushes) { if (b.group !== last) { const h = document.createElement('h3'); h.textContent = b.group; brushGrid.append(h); last = b.group; } const btn = document.createElement('button'); btn.type = 'button'; btn.className = 'brush-item'; const img = document.createElement('img'); img.src = brushUrl(b.preview); img.alt = ''; const sp = document.createElement('span'); sp.textContent = b.name; btn.append(img, sp); btn.onclick = () => void selectBrush(b); brushButtons.set(b.id, btn); brushGrid.append(btn); } }
async function selectBrush(b: BrushPreset) { const r = await fetch(brushUrl(b.brush)); if (!r.ok) return; const json = await r.text(); selectedBrushId = b.id; selectedBrushJson = json; try { const root = JSON.parse(json); const rest = Number(root?.settings?.restore_color?.base_value ?? 0); if (rest > 0) { const s = [Number(root?.settings?.color_h?.base_value ?? 0), Number(root?.settings?.color_s?.base_value ?? 0), Number(root?.settings?.color_v?.base_value ?? 0)]; const a = hexRgb(ui.color).map(v => v ** 2.2); const f = Math.max(0, Math.min(1, rest)); const r2 = a[0]*(1-f)+s[0]*f, g2 = a[1]*(1-f)+s[1]*f, b2 = a[2]*(1-f)+s[2]*f; ui.color = `#${Math.round(r2**(1/2.2)*255).toString(16).padStart(2,'0')}${Math.round(g2**(1/2.2)*255).toString(16).padStart(2,'0')}${Math.round(b2**(1/2.2)*255).toString(16).padStart(2,'0')}`; ($('color') as HTMLInputElement).value = ui.color; } } catch {} send({ cmd: 'loadBrush', json }); applyBrushColor(); applyBrushOverrides(); updateColorSwatches(); for (const [id, btn] of brushButtons) btn.classList.toggle('selected', id === selectedBrushId); statusEl.textContent = `Ready — ${b.name}.`; }
async function loadCatalog() { const r = await fetch(new URL('../mypaint/brushes.json', document.baseURI)); if (!r.ok) throw new Error('Cannot load brush catalog.'); const m = await r.json() as { count: number; brushes: BrushPreset[] }; renderCatalog(m.brushes); const init = m.brushes.find(b => b.id === 'classic/brush') ?? m.brushes[0]; if (init) await selectBrush(init); log(`${m.count} brushes loaded.`); }

// Export
function reqTiles(layerId: number | null): Promise<{ data: ArrayBuffer[]; scale: number }> {
  if (!ready) return Promise.reject(new Error('The paint document is not ready.'));
  if (pendingTiles) return Promise.reject(new Error('An export is already active.'));
  const id = ++exportSeq;
  return new Promise((resolve, reject) => {
    pendingTiles = { id, resolve, reject };
    try { send({ cmd: 'exportTiles', layerId, id }); }
    catch (error) { pendingTiles = null; reject(error); }
  });
}
async function canvasPng(c: HTMLCanvasElement): Promise<Uint8Array> { const b = await new Promise<Blob|null>(r => c.toBlob(r, 'image/png')); if (!b) throw new Error('PNG failed.'); return new Uint8Array(await b.arrayBuffer()); }
async function renderPng(layerId: number | null): Promise<Uint8Array> { if (!engineState) throw new Error('No engine.');
  const ow = engineState.width, oh = engineState.height;
  const out = document.createElement('canvas'); out.width = ow; out.height = oh; const g = out.getContext('2d', { alpha: true })!;
  const { data, scale } = await reqTiles(layerId); const cols = Math.ceil(ow / (64 * scale)); const img = new ImageData(64, 64);
  for (let i = 0; i < data.length; i++) { const tx = i % cols, ty = Math.floor(i / cols); img.data.set(new Uint8Array(data[i])); g.putImageData(img, tx * 64, ty * 64); } return canvasPng(out); }
async function exportPng() { await saveFile(await renderPng(null), 'afterglow-paint.png', 'image/png'); }
function compOp(m: number) { const n = ['svg:src-over','svg:multiply','svg:screen','svg:overlay','svg:darken','svg:lighten','svg:hard-light','svg:soft-light','svg:color-burn','svg:color-dodge','svg:difference','svg:exclusion','svg:hue','svg:saturation','svg:color','svg:luminosity','svg:plus','svg:src-in','svg:src-out','svg:src-atop','svg:dst-atop','svg:src-over']; return n[m] ?? 'svg:src-over'; }
function buildStackXml(): string { if (!engineState) return ''; const lc = engineState.layers.length, gc = engineState.groups.length; const lX = (l: number, i: string) => `${i}<layer name="Layer ${l+1}" src="data/layer-${l}.png" opacity="${engineState.layers[l].opacity}" visibility="${engineState.layers[l].visible !== 0 ? 'visible' : 'hidden'}" composite-op="${compOp(engineState.layers[l].mode)}" />`; const gX = (g: number, i: string) => { const lines = [`${i}<stack name="Group ${g+1}" opacity="${engineState.groups[g].opacity}" visibility="${engineState.groups[g].visible !== 0 ? 'visible' : 'hidden'}" composite-op="${compOp(engineState.groups[g].mode)}">`]; for (let c = g - 1; c >= 0; c--) if (engineState.groups[c].alive && engineState.groups[c].parent === g && c !== g) lines.push(gX(c, i + '  ')); for (let l = lc - 1; l >= 0; l--) if (engineState.layers[l].group === g) lines.push(lX(l, i + '  ')); lines.push(`${i}</stack>`); return lines.join('\n'); }; const lines = ['<?xml version="1.0" encoding="UTF-8"?>', `<image version="0.0" w="${docSize.width}" h="${docSize.height}" name="Afterglow">`, '  <stack name="Afterglow">']; for (let g = gc - 1; g >= 0; g--) if (engineState.groups[g].alive && engineState.groups[g].parent < 0) lines.push(gX(g, '    ')); for (let l = lc - 1; l >= 0; l--) if (engineState.layers[l].group < 0) lines.push(lX(l, '    ')); lines.push('  </stack>', '</image>'); return lines.join('\n'); }
function buildMeta(): string { if (!engineState) return '{}'; return JSON.stringify({ width: docSize.width, height: docSize.height, layers: engineState.layers.map((l: any) => ({ id: l.id, group: l.group, visible: l.visible, opacity: l.opacity, mode: l.mode })), groups: engineState.groups.filter((g: any) => g.alive).map((g: any) => ({ id: g.id, parent: g.parent, visible: g.visible, opacity: g.opacity, mode: g.mode, passThrough: g.passThrough, isolated: g.isolated })) }); }
async function exportOra() { if (!engineState) return; const e = [{ name: 'mimetype', data: utf8('image/openraster') }, { name: 'stack.xml', data: utf8(buildStackXml()) }, { name: 'mergedimage.png', data: await renderPng(null) }, { name: 'data/metadata.json', data: utf8(buildMeta()) }]; for (let l = 0; l < engineState.layers.length; l++) e.push({ name: `data/layer-${l}.png`, data: await renderPng(l) }); await saveFile(encodeStoredZip(e), 'afterglow-paint.ora', 'image/openraster'); }
async function importOra(file: Blob) { if (!ready) return; const entries = await decodeZip(await file.arrayBuffer()); const mb = entries.get('data/metadata.json'); const meta = mb ? JSON.parse(text(mb)) as any : null; const st = entries.get('stack.xml'); const stT = st ? text(st) : ''; const w = meta?.width ?? Number(stT.match(/\bw="(\d+)"/)?.[1] ?? docSize.width), h = meta?.height ?? Number(stT.match(/\bh="(\d+)"/)?.[1] ?? docSize.height); const merged = entries.get('data/layer-0.png') ?? entries.get('mergedimage.png'); if (!merged) throw new Error('No image.'); resetDoc(w, h); send({ cmd: 'clearBackground' }); send({ cmd: 'clear' }); const layers = meta?.layers ?? [{ id: 0, group: -1, visible: 1, mode: 0 }]; for (const l of layers) { if (l.id > 0) send({ cmd: 'layer', op: 'create', layer: l.id }); send({ cmd: 'layer', op: 'setVisible', layer: l.id, value: l.visible !== 0 ? 1 : 0 }); send({ cmd: 'layer', op: 'setOpacity', layer: l.id, value: Number(l.opacity ?? 1) }); send({ cmd: 'layer', op: 'setMode', layer: l.id, value: Number(l.mode) || 0 }); } const ic = await imgCanvas(merged); await writeImg(ic, 0); for (const l of layers) { if (l.id === 0) continue; const d = entries.get(`data/layer-${l.id}.png`); if (d) await writeImg(await imgCanvas(d), l.id); } for (const g of meta?.groups ?? []) { send({ cmd: 'group', op: 'create', group: g.id }); send({ cmd: 'group', op: 'setVisible', group: g.id, value: g.visible !== 0 ? 1 : 0 }); send({ cmd: 'group', op: 'setOpacity', group: g.id, value: Number(g.opacity) || 0 }); send({ cmd: 'group', op: 'setMode', group: g.id, value: Number(g.mode) || 0 }); send({ cmd: 'group', op: 'setPassThrough', group: g.id, value: g.passThrough ? 1 : 0 }); send({ cmd: 'group', op: 'setIsolated', group: g.id, value: g.isolated ? 1 : 0 }); send({ cmd: 'group', op: 'setParent', group: g.id, value: Number(g.parent) }); } for (const l of layers) if (l.group !== undefined) send({ cmd: 'layer', op: 'setGroup', layer: l.id, value: Number(l.group) }); send({ cmd: 'layer', op: 'setActive', layer: 0 }); }
async function imgCanvas(d: Uint8Array): Promise<HTMLCanvasElement> { const bm = await createImageBitmap(new Blob([d as BlobPart], { type: 'image/png' })); const c = document.createElement('canvas'); c.width = bm.width; c.height = bm.height; c.getContext('2d', { alpha: true })!.drawImage(bm, 0, 0); bm.close(); return c; }
async function writeImg(ic: HTMLCanvasElement, layer: number) { const g = ic.getContext('2d', { alpha: true })!; const img = g.getImageData(0, 0, ic.width, ic.height); const tile = new Uint8Array(64 * 64 * 4); for (let ty = 0; ty < Math.ceil(ic.height / 64); ty++) for (let tx = 0; tx < Math.ceil(ic.width / 64); tx++) { tile.fill(0); for (let y = 0; y < 64; y++) { const sy = ty * 64 + y; if (sy >= img.height) continue; for (let x = 0; x < 64; x++) { const sx = tx * 64 + x; if (sx >= img.width) continue; tile.set(img.data.subarray((sy * img.width + sx) * 4, (sy * img.width + sx) * 4 + 4), (y * 64 + x) * 4); } } const data = tile.slice().buffer; send({ cmd: 'writeTile', layer, tx, ty, data }, [data]); if ((ty * Math.ceil(ic.width / 64) + tx + 1) % 64 === 0) await worker?.flush(); } await worker?.flush(); }
function resetDoc(w: number, h: number) { if (recoveryPending || w < 64 || h < 64 || w > 16384 || h > 16384) return; clearPointerInput(); ready = false; paintDocumentId = newPaintDocumentId(); docSize.width = w; docSize.height = h; const r = Math.max(w, h) / 4096; const ds = r <= 1 ? 1 : r <= 2 ? 2 : 4; dispW = Math.ceil(w / ds); dispH = Math.ceil(h / ds); canvas.style.width = `${dispW}px`; canvas.style.height = `${dispH}px`; send({ cmd: 'init', recovery: 'discard', width: w, height: h, documentId: paintDocumentId, hardwareConcurrency: navigator.hardwareConcurrency, memoryLimitMiB: paintMemoryMiB, nativeMemoryLimitMiB }); }

function chooseRecovery(recovery: 'restore' | 'discard') {
  if (!recoveryPending) return;
  $<HTMLButtonElement>('restorePaintBtn').disabled = true;
  $<HTMLButtonElement>('discardPaintBtn').disabled = true;
  send({ cmd: 'init', recovery, width: docSize.width, height: docSize.height, documentId: paintDocumentId,
    hardwareConcurrency: navigator.hardwareConcurrency, memoryLimitMiB: paintMemoryMiB, nativeMemoryLimitMiB });
}
$('restorePaintBtn').onclick = () => chooseRecovery('restore');
$('discardPaintBtn').onclick = () => chooseRecovery('discard');

const tileMetricKeys = ['residentTiles', 'residentTileLimit', 'maximumResidentTiles'] as const;
worker = new PaintSession(canvas);
(window as any).probe = (y: number) => { const id = ++exportSeq; send({ cmd: 'probe', id, y }); return id; };
worker.onmessage = (e) => { const m = e.data;
  switch (m.type) {
    case 'recoveryRequired':
      ready = false;
      recoveryPending = true;
      $('paintRecovery').style.display = 'block';
      statusEl.textContent = 'Select Restore or Discard.';
      $<HTMLButtonElement>('restorePaintBtn').disabled = false;
      $<HTMLButtonElement>('discardPaintBtn').disabled = false;
      $('restorePaintBtn').focus();
      resolveRecoveryPrompt();
      break;
    case 'ready':
      if (recoveryPending) canvas.focus();
      recoveryPending = false;
      $('paintRecovery').style.display = 'none';
      ready = true; resolveInitialReady(); ensureBrush(); refreshLayers(); statusEl.textContent = 'Ready — choose a brush or draw.'; break;
    case 'state': {
      const previous = engineState;
      engineState = m.state;
      if (!engineState) break;
      if (engineState.nativeMemory && (!previous || previous.nativeMemory?.limitMiB !== engineState.nativeMemory.limitMiB)) {
        const memory = engineState.nativeMemory;
        const input = $<HTMLInputElement>('memoryLimit');
        input.max = String(memory.maximumMiB);
        input.min = String(Math.ceil((memory.reservedBytes / 1048576 + 64) / 64) * 64);
        paintMemoryMiB = memory.limitMiB;
        input.value = String(paintMemoryMiB);
        $('memoryLimitVal').textContent = String(paintMemoryMiB);
      }
      if (!previous || previous.width !== engineState.width || previous.height !== engineState.height || previous.displayScale !== engineState.displayScale) {
        docSize.width = engineState.width;
        docSize.height = engineState.height;
        $<HTMLInputElement>('documentWidth').value = String(docSize.width);
        $<HTMLInputElement>('documentHeight').value = String(docSize.height);
        dispW = Math.ceil(engineState.width / engineState.displayScale);
        dispH = Math.ceil(engineState.height / engineState.displayScale);
        canvas.style.width = `${dispW}px`;
        canvas.style.height = `${dispH}px`;
      }
      for (const key of tileMetricKeys) {
        if (!previous || previous[key] !== engineState[key]) canvas.dataset[key] = String(engineState[key]);
      }
      if (!samePaintLayerState(previous, engineState)) refreshLayers();
      break;
    }
    case 'status': statusEl.textContent = m.text; break;
    case 'log': log(m.text); break;
    case 'stats': hudEl.textContent = `queue   ${m.queued} sp\nactions ${m.deferred}\ntiles   ${m.residentTiles}/${m.residentTileLimit}\nbrush   ${m.brushMs.toFixed(1)} ms\nrender  ${m.renderMs.toFixed(1)} ms\ninput   ${m.sps}/s`; break;
    case 'tiles': if (pendingTiles && pendingTiles.id === m.id) { const request = pendingTiles; pendingTiles = null; request.resolve({ data: m.data, scale: m.scale }); } break;
    case 'probeResult': (window as any).__probeResult = m; break;
    case 'colorPicked':
      if (m.id === latestColorPickId) {
        setForegroundColor(rgbToHex({ r: m.r, g: m.g, b: m.b }));
        statusEl.textContent = `Sampled ${ui.color}.`;
      }
      break;
  }
};
worker.onerror = (e) => {
  ready = false;
  rejectInitialReady(new Error(e.message));
  pendingTiles?.reject(new Error(e.message));
  pendingTiles = null;
  log(`Worker error: ${e.message}`);
  statusEl.textContent = 'Engine error: ' + e.message;
};

async function init() { statusEl.textContent = 'Loading brush engine…'; log('loading brush engine…');
  send({ cmd: 'init', width: docSize.width, height: docSize.height, documentId: paintDocumentId, hardwareConcurrency: navigator.hardwareConcurrency, memoryLimitMiB: paintMemoryMiB, nativeMemoryLimitMiB });
  try { await loadCatalog(); } catch (e) { log(`Brush catalog error: ${(e as Error).message}`); } refreshLayers();
  await initialReady;
}
applyView();
setTool('brush');
updateColorSwatches();
const initTask = init().catch(e => { statusEl.textContent = 'Engine failed to load: ' + (e as Error).message; log('ERROR: ' + ((e as Error).stack || (e as Error).message)); throw e; });

function togglePanels() {
  const workspace = document.querySelector<HTMLElement>('.workspace');
  const hidden = workspace?.classList.toggle('panels-hidden') ?? false;
  $('panelToggleBtn').setAttribute('aria-pressed', String(hidden));
}
function editableTarget(target: EventTarget | null): boolean {
  return target instanceof HTMLInputElement || target instanceof HTMLSelectElement || target instanceof HTMLTextAreaElement || (target instanceof HTMLElement && target.isContentEditable);
}
function runShortcut(action: PaintShortcut) {
  if (action.startsWith('opacity-')) {
    setBrushValue('opacity', Number(action.slice('opacity-'.length)) / 100);
    return;
  }
  switch (action) {
    case 'brush-tool': setTool('brush'); break;
    case 'eyedropper-tool': setTool('eyedropper'); break;
    case 'hand-tool': setTool('hand'); break;
    case 'rotate-tool': setTool('rotate'); break;
    case 'zoom-tool': setTool('zoom'); break;
    case 'brush-smaller': setBrushValue('radius', Math.max(2, ui.radius - 2)); break;
    case 'brush-larger': setBrushValue('radius', Math.min(60, ui.radius + 2)); break;
    case 'hardness-softer': setBrushValue('hardness', Math.max(0, ui.hardness - 0.1)); break;
    case 'hardness-harder': setBrushValue('hardness', Math.min(1, ui.hardness + 0.1)); break;
    case 'default-colors':
      $<HTMLInputElement>('backgroundColor').value = '#ffffff';
      applyBgColor();
      setForegroundColor('#000000');
      break;
    case 'switch-colors': {
      const background = $<HTMLInputElement>('backgroundColor');
      const foreground = ui.color;
      const nextForeground = background.value;
      background.value = foreground;
      applyBgColor();
      setForegroundColor(nextForeground);
      break;
    }
    case 'previous-brush': cycleBrush(-1); break;
    case 'next-brush': cycleBrush(1); break;
    case 'undo': $('undoBtn').click(); break;
    case 'redo': $('redoBtn').click(); break;
    case 'clear-layer': $('clearBtn').click(); break;
    case 'new-document': $('newDocumentBtn').click(); break;
    case 'new-layer': $('addLayerBtn').click(); break;
    case 'open-document': $('openDocumentBtn').click(); break;
    case 'save-document': $('exportOraBtn').click(); break;
    case 'export-png': $('exportPngBtn').click(); break;
    case 'zoom-in': setZoom(view.zoom * 1.1); break;
    case 'zoom-out': setZoom(view.zoom * 0.9); break;
    case 'fit-view': fitView(); break;
    case 'actual-pixels': actualPixels(); break;
    case 'toggle-panels': togglePanels(); break;
  }
}
document.addEventListener('keydown', (event) => {
  const editable = editableTarget(event.target);
  if (event.key === 'Alt' && activeTool === 'brush' && !editable) canvas.dataset.altEyedropper = 'true';
  if (event.key === ' ' && !editable && !event.ctrlKey && !event.metaKey && !event.altKey) {
    event.preventDefault();
    spaceHeld = true;
    canvas.dataset.spaceHand = 'true';
    return;
  }
  const action = resolvePaintShortcut({
    key: event.key,
    code: event.code,
    ctrlKey: event.ctrlKey,
    metaKey: event.metaKey,
    shiftKey: event.shiftKey,
    altKey: event.altKey,
    editable,
  });
  if (!action) return;
  event.preventDefault();
  runShortcut(action);
});
document.addEventListener('keyup', (event) => {
  if (event.key === 'Alt') delete canvas.dataset.altEyedropper;
  if (event.key === ' ') {
    spaceHeld = false;
    delete canvas.dataset.spaceHand;
  }
});

// UI bindings
const stabilizerModeInput = $<HTMLSelectElement>('stabilizerMode');
const stabilizerAmountInput = $<HTMLInputElement>('stabilizerAmount');
const stabilizerCatchUpInput = $<HTMLInputElement>('stabilizerCatchUp');
function updateStabilizerControls(save: boolean): void {
  const requestedMode = stabilizerModeInput.value as StrokeStabilizerMode;
  stabilizerMode = stabilizerModes.has(requestedMode) ? requestedMode : 'off';
  stabilizerAmount = Math.max(1, Math.min(100, Number(stabilizerAmountInput.value) || 20));
  stabilizerCatchUp = stabilizerCatchUpInput.checked;
  stabilizer.configure(stabilizerMode, stabilizerAmount, stabilizerCatchUp);
  canvas.dataset.stabilizerMode = stabilizerMode;
  canvas.dataset.stabilizerAmount = String(stabilizerAmount);
  canvas.dataset.stabilizerCatchUp = String(stabilizerCatchUp);
  stabilizerAmountInput.disabled = stabilizerMode === 'off';
  $('stabilizerAmountVal').textContent = String(stabilizerAmount);
  const supportsCatchUp = stabilizerMode === 'average' || stabilizerMode === 'exponential';
  $('stabilizerCatchUpLabel').hidden = !supportsCatchUp;
  stabilizerCatchUpInput.disabled = !supportsCatchUp;
  if (save) {
    try {
      localStorage.setItem('afterglow.paintStabilizer', JSON.stringify({
        mode: stabilizerMode, amount: stabilizerAmount, catchUp: stabilizerCatchUp,
      }));
    } catch {}
  }
}
function changeStabilizer(): void {
  finishInputForViewChange();
  updateStabilizerControls(true);
}
stabilizerModeInput.value = stabilizerMode;
stabilizerAmountInput.value = String(stabilizerAmount);
stabilizerCatchUpInput.checked = stabilizerCatchUp;
stabilizerModeInput.addEventListener('change', changeStabilizer);
stabilizerAmountInput.addEventListener('input', changeStabilizer);
stabilizerCatchUpInput.addEventListener('change', changeStabilizer);
updateStabilizerControls(false);
['radius','hardness','opacity'].forEach(k => { const i = $(k) as HTMLInputElement; const a = () => { (ui as any)[k] = Number(i.value); $(`${k}Val`).textContent = i.value; applyBrushOverrides(); }; i.addEventListener('input', a); a(); });
$('color').addEventListener('input', e => setForegroundColor((e.target as HTMLInputElement).value));
$('viewZoom').addEventListener('input', e => setZoom(Number((e.target as HTMLInputElement).value)));
$('rotateLeftBtn').addEventListener('click', () => { finishInputForViewChange(); view.rotationDegrees = (view.rotationDegrees + 90) % 360; applyView(); });
$('rotateRightBtn').addEventListener('click', () => { finishInputForViewChange(); view.rotationDegrees = (view.rotationDegrees + 270) % 360; applyView(); });
$('mirrorBtn').addEventListener('click', () => { finishInputForViewChange(); view.mirror = !view.mirror; applyView(); });
$('resetViewBtn').addEventListener('click', () => { finishInputForViewChange(); Object.assign(view, { zoom: 1, rotationDegrees: 0, mirror: false, panX: 0, panY: 0 }); applyView(); });
$('fitViewBtn').addEventListener('click', fitView);
$('actualPixelsBtn').addEventListener('click', actualPixels);
$('zoomOutBtn').addEventListener('click', () => setZoom(view.zoom * 0.9));
$('zoomInBtn').addEventListener('click', () => setZoom(view.zoom * 1.1));
$('brushToolBtn').addEventListener('click', () => setTool('brush'));
$('eyedropperToolBtn').addEventListener('click', () => setTool('eyedropper'));
$('handToolBtn').addEventListener('click', () => setTool('hand'));
$('rotateToolBtn').addEventListener('click', () => setTool('rotate'));
$('zoomToolBtn').addEventListener('click', () => setTool('zoom'));
$('panelToggleBtn').addEventListener('click', togglePanels);
canvas.addEventListener('wheel', e => { e.preventDefault(); setZoom(view.zoom * (e.deltaY < 0 ? 1.1 : 0.9)); }, { passive: false });
$('frameEnabled').addEventListener('change', e => canvas.classList.toggle('frame-visible', (e.target as HTMLInputElement).checked));
$('clearBtn').addEventListener('click', () => send({ cmd: 'clear' }));
$('backgroundColor').addEventListener('input', () => { applyBgColor(); updateColorSwatches(); });
$('undoBtn').addEventListener('click', () => send({ cmd: 'undo' }));
$('redoBtn').addEventListener('click', () => send({ cmd: 'redo' }));
$('memoryLimit').addEventListener('input', e => {
  const input = e.target as HTMLInputElement;
  const native = engineState?.nativeMemory;
  paintMemoryMiB = native
    ? Math.max(Number(input.min), Math.min(native.maximumMiB, Math.floor(Number(input.value) / 64) * 64))
    : paintMemoryLimitMiB(deviceMemoryGiB, Number(input.value));
  if (native) nativeMemoryLimitMiB = paintMemoryMiB;
  input.value = String(paintMemoryMiB);
  $('memoryLimitVal').textContent = String(paintMemoryMiB);
  try { localStorage.setItem(native ? 'afterglow.nativePaintMemoryMiB' : 'afterglow.paintMemoryMiB', String(paintMemoryMiB)); } catch {}
});
($('memoryLimit') as HTMLInputElement).value = String(paintMemoryMiB);
$('memoryLimitVal').textContent = String(paintMemoryMiB);
$('newDocumentBtn').addEventListener('click', () => resetDoc(Number(($('documentWidth') as HTMLInputElement).value), Number(($('documentHeight') as HTMLInputElement).value)));
$('exportPngBtn').addEventListener('click', () => void exportPng().catch(e => statusEl.textContent = `PNG export failed: ${(e as Error).message}`));
$('exportOraBtn').addEventListener('click', () => void exportOra().catch(e => statusEl.textContent = `ORA export failed: ${(e as Error).message}`));
$('openDocumentBtn').addEventListener('click', () => {
  void openFile(['ora']).then(async file => {
    if (file) { await importOra(file); statusEl.textContent = 'OpenRaster imported.'; }
  }).catch(error => { statusEl.textContent = `ORA import failed: ${(error as Error).message}`; });
});
$('layerBlendMode').addEventListener('change', event => sendSelection('setMode', Number((event.target as HTMLSelectElement).value)));
$('layerOpacity').addEventListener('input', event => {
  const value = Number((event.target as HTMLInputElement).value);
  $('layerOpacityValue').textContent = `${Math.round(value * 100)}%`;
  sendSelection('setOpacity', value);
});
$('layerParent').addEventListener('change', event => sendSelection(selectedGroupId >= 0 ? 'setParent' : 'setGroup', Number((event.target as HTMLSelectElement).value)));
$('groupPassThrough').addEventListener('change', event => sendSelection('setPassThrough', (event.target as HTMLInputElement).checked ? 1 : 0));
$('groupIsolated').addEventListener('change', event => sendSelection('setIsolated', (event.target as HTMLInputElement).checked ? 1 : 0));
$('addLayerBtn').addEventListener('click', () => send({ cmd: 'layer', op: 'create', layer: 0 }));
$('addGroupBtn').addEventListener('click', () => send({ cmd: 'group', op: 'create', group: 0 }));
$('moveSelectionUpBtn').addEventListener('click', () => sendSelection('move', 1));
$('moveSelectionDownBtn').addEventListener('click', () => sendSelection('move', -1));
$('deleteSelectionBtn').addEventListener('click', () => {
  if (!engineState) return;
  if (selectedGroupId >= 0) {
    send({ cmd: 'group', op: 'delete', group: selectedGroupId });
    selectedGroupId = -1;
  } else {
    send({ cmd: 'layer', op: 'delete', layer: engineState.activeLayer });
  }
});
$('strokeBtn').addEventListener('click', () => { if (!ready) return; const y = docSize.height / 2, x0 = docSize.width * 0.15; send({ cmd: 'beginStroke', x: x0, y, xtilt: 0, ytilt: 0, zoom: view.zoom, rotation: view.rotationDegrees * Math.PI / 180, barrel: 0.5 }); for (let i = 1; i <= 10; i++) send({ cmd: 'strokeSample', x: x0 + (docSize.width * 0.6) * (i / 10), y, pressure: 0.5, xtilt: 0, ytilt: 0, time: i * 16, zoom: view.zoom, rotation: view.rotationDegrees * Math.PI / 180, barrel: 0.5 }); send({ cmd: 'commit' }); statusEl.textContent = 'Test stroke drawn.'; });

// The native host must complete module startup before it accepts recovery input.
await Promise.race([initTask, recoveryPrompt]);
