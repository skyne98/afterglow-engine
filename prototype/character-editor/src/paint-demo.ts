/* libmypaint NG paint demo — thin client.
 * The engine runs in paint-engine-worker.ts (Web Worker with pthreads).
 * This module captures input, manages DOM/UI, and forwards to the worker.
 */
import { decodeZip, encodeStoredZip, text, utf8 } from './openraster.ts';

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const canvas = $<HTMLCanvasElement>('paint');
const statusEl = $('status'), logEl = $('logs'), hudEl = $<HTMLDivElement>('hud');

let worker: Worker | null = null;
let engineState: any = null;
let ready = false;
let exportSeq = 0;
let pendingTiles: ((v: { data: ArrayBuffer[]; scale: number }) => void) | null = null;

let strokeActive = false, activePid: number | null = null, panPid: number | null = null;
let lastPX = 0, lastPY = 0;
const docSize = { width: 2048, height: 2048 };
const view = { zoom: 1, rotationDegrees: 0, mirror: false, panX: 0, panY: 0 };
let dispW = canvas.width, dispH = canvas.height;
const ui = { radius: 14, hardness: 0.6, opacity: 1.0, color: '#4ecdc4' };

type BrushPreset = { id: string; name: string; group: string; brush: string; preview: string };
const brushGrid = $<HTMLDivElement>('brushGrid'), layerList = $<HTMLDivElement>('layerList'), groupList = $<HTMLDivElement>('groupList');
let selectedGroupId = -1, selectedBrushId = '', selectedBrushJson = '';
const brushButtons = new Map<string, HTMLButtonElement>();
const layerModes = ['Normal','Multiply','Screen','Overlay','Darken','Lighten','Hard Light','Soft Light','Burn','Dodge','Difference','Exclusion','Hue','Saturation','Color','Luminosity','Plus','Destination In','Destination Out','Source Atop','Destination Atop','Pigment'];

function log(m: string) { logEl.textContent += m + '\n'; logEl.scrollTop = logEl.scrollHeight; }
function send(m: any, t?: Transferable[]) { if (t) worker?.postMessage(m, t); else worker?.postMessage(m); }
function sendCfg(s: [string, number][]) { send({ cmd: 'config', settings: s }); }
function brushUrl(p: string) { return `/mypaint/brushes/${p.split('/').map(encodeURIComponent).join('/')}`; }
function hexRgb(v: string): [number, number, number] { const n = parseInt(v.slice(1), 16); return [((n>>16)&255)/255, ((n>>8)&255)/255, (n&255)/255]; }
function rgbHsv(r: number, g: number, b: number): [number, number, number] { const mx = Math.max(r,g,b), mn = Math.min(r,g,b), d = mx-mn; let h = 0; if (d) { if (mx===r) h=((g-b)/d)%6; else if (mx===g) h=(b-r)/d+2; else h=(r-g)/d+4; h/=6; if (h<0) h+=1; } return [h, mx===0?0:d/mx, mx]; }
function applyBrushColor() { const [r,g,b] = hexRgb(ui.color); const h = rgbHsv(r**2.2, g**2.2, b**2.2); sendCfg([['color_h',h[0]],['color_s',h[1]],['color_v',h[2]]]); }
function applyBrushOverrides() { sendCfg([['radius_logarithmic', Math.log(ui.radius)],['hardness', ui.hardness],['opaque_multiply', ui.opacity]]); }
function applyBgColor() { const [r,g,b] = hexRgb(($('backgroundColor') as HTMLInputElement).value); send({ cmd: 'setBackground', r, g, b }); }
function ensureBrush() { if (selectedBrushJson) { send({ cmd: 'loadBrush', json: selectedBrushJson }); applyBrushColor(); applyBrushOverrides(); } }

function applyView() {
  canvas.style.transform = `translate(${view.panX}px,${view.panY}px) scale(${view.zoom}) rotate(${view.rotationDegrees}deg) scaleX(${view.mirror?-1:1})`;
  canvas.style.imageRendering = view.zoom >= 2.5 ? 'pixelated' : 'auto';
  const zEl = $('viewZoom') as HTMLInputElement; zEl.value = String(view.zoom); $('zoomVal').textContent = view.zoom.toFixed(2); $('mirrorBtn').classList.toggle('active', view.mirror);
  send({ cmd: 'setView', zoom: view.zoom });
}
function pointerModel(e: PointerEvent): [number, number] {
  const r = canvas.getBoundingClientRect(), cx = r.left + r.width * 0.5, cy = r.top + r.height * 0.5;
  const t = new DOMMatrix(getComputedStyle(canvas).transform);
  const lt = new DOMMatrix([t.a, t.b, t.c, t.d, 0, 0]).inverse();
  const l = new DOMPoint(e.clientX - cx, e.clientY - cy).matrixTransform(lt);
  const cw = canvas.offsetWidth, ch = canvas.offsetHeight;
  const dx = (l.x + cw * 0.5) * (dispW / cw), dy = (l.y + ch * 0.5) * (dispH / ch);
  return [dx * (docSize.width / dispW), dy * (docSize.height / dispH)];
}
let lastSampleTime = 0;
const MIN_SAMPLE_MS = 4;
function sendSample(e: PointerEvent, pressure: number) {
  const now = Number.isFinite(e.timeStamp) ? e.timeStamp : performance.now();
  if (now - lastSampleTime < MIN_SAMPLE_MS) return;
  lastSampleTime = now;
  const [x, y] = pointerModel(e);
  const xt = Math.max(-1, Math.min(1, (e.tiltX / 90) || 0)), yt = Math.max(-1, Math.min(1, (e.tiltY / 90) || 0));
  send({ cmd: 'strokeSample', x, y, pressure, xtilt: xt, ytilt: yt, time: now, zoom: view.zoom, rotation: view.rotationDegrees * Math.PI / 180, barrel: 0.5 });
}
function beginStrokeAt(e: PointerEvent) {
  if (!ready) return; ensureBrush();
  lastSampleTime = 0;
  const [x, y] = pointerModel(e);
  send({ cmd: 'beginStroke', x, y, xtilt: (e.tiltX/90)||0, ytilt: (e.tiltY/90)||0, zoom: view.zoom, rotation: view.rotationDegrees * Math.PI / 180, barrel: 0.5 });
  sendSample(e, e.pointerType === 'mouse' ? 0.5 : Math.max(0, Math.min(1, e.pressure)));
}
function endStroke(e: PointerEvent) {
  if (e.pointerId === panPid) { panPid = null; return; }
  if (!strokeActive || activePid !== e.pointerId) return; strokeActive = false; activePid = null; lastSampleTime = 0; sendSample(e, 0); send({ cmd: 'commit' });
}
canvas.addEventListener('pointerdown', e => { if (!ready) return; if (e.button === 1) { panPid = e.pointerId; lastPX = e.clientX; lastPY = e.clientY; try { canvas.setPointerCapture?.(e.pointerId); } catch {} return; } if (e.button !== 0) return; strokeActive = true; activePid = e.pointerId; beginStrokeAt(e); try { canvas.setPointerCapture?.(e.pointerId); } catch {} });
canvas.addEventListener('pointermove', e => { if (e.pointerId === panPid) { view.panX += e.clientX - lastPX; view.panY += e.clientY - lastPY; lastPX = e.clientX; lastPY = e.clientY; applyView(); return; } if (e.pointerId !== activePid || !strokeActive) return; const c = e.pointerType === 'mouse' ? (e.buttons & 1) !== 0 : e.pressure > 0; if (!c) { strokeActive = false; activePid = null; send({ cmd: 'commit' }); return; } sendSample(e, e.pointerType === 'mouse' ? 0.5 : Math.max(0, Math.min(1, e.pressure))); });
canvas.addEventListener('pointerup', endStroke); canvas.addEventListener('pointercancel', endStroke);
window.addEventListener('pointerup', endStroke); window.addEventListener('pointercancel', endStroke);
window.addEventListener('blur', () => { strokeActive = false; activePid = null; panPid = null; if (ready) send({ cmd: 'commit' }); });

function refreshLayers() { if (!engineState) return; layerList.replaceChildren(); groupList.replaceChildren();
  for (let l = engineState.layers.length - 1; l >= 0; l--) { const info = engineState.layers[l]; const row = document.createElement('div'); row.className = 'layer-row';
    const btn = document.createElement('button'); btn.type = 'button'; btn.textContent = `Layer ${l+1}`; btn.classList.toggle('active', info.active); btn.onclick = () => send({ cmd: 'layer', op: 'setActive', layer: l });
    const up = document.createElement('button'); up.type = 'button'; up.className = 'order'; up.textContent = '↑'; up.onclick = () => send({ cmd: 'layer', op: 'move', layer: l, value: 1 });
    const dn = document.createElement('button'); dn.type = 'button'; dn.className = 'order'; dn.textContent = '↓'; dn.onclick = () => send({ cmd: 'layer', op: 'move', layer: l, value: -1 });
    const vis = document.createElement('input'); vis.type = 'checkbox'; vis.checked = info.visible !== 0; vis.onchange = () => send({ cmd: 'layer', op: 'setVisible', layer: l, value: vis.checked ? 1 : 0 });
    const mode = document.createElement('select'); for (let m = 0; m < layerModes.length; m++) { const o = document.createElement('option'); o.value = String(m); o.textContent = layerModes[m]; mode.append(o); } mode.value = String(info.mode); mode.onchange = () => send({ cmd: 'layer', op: 'setMode', layer: l, value: Number(mode.value) });
    const grp = document.createElement('select'); const root = document.createElement('option'); root.value = '-1'; root.textContent = 'Root'; grp.append(root); for (const g of engineState.groups) if (g.alive) { const o = document.createElement('option'); o.value = String(g.id); o.textContent = `G${g.id+1}`; grp.append(o); } grp.value = String(info.group); grp.onchange = () => send({ cmd: 'layer', op: 'setGroup', layer: l, value: Number(grp.value) });
    const op = document.createElement('input'); op.type = 'range'; op.min = '0'; op.max = '1'; op.step = '0.01'; op.value = String(info.opacity); op.oninput = () => send({ cmd: 'layer', op: 'setOpacity', layer: l, value: Number(op.value) });
    row.append(up, dn, btn, vis, grp, mode, op); layerList.append(row); }
  for (let g = engineState.groups.length - 1; g >= 0; g--) { const info = engineState.groups[g]; if (!info.alive) continue; const row = document.createElement('div'); row.className = 'group-row';
    const up = document.createElement('button'); up.type = 'button'; up.className = 'order'; up.textContent = '↑'; up.onclick = () => send({ cmd: 'group', op: 'move', group: g, value: 1 });
    const dn = document.createElement('button'); dn.type = 'button'; dn.className = 'order'; dn.textContent = '↓'; dn.onclick = () => send({ cmd: 'group', op: 'move', group: g, value: -1 });
    const btn = document.createElement('button'); btn.type = 'button'; btn.textContent = `Group ${g+1}`; btn.classList.toggle('active', selectedGroupId === g); btn.onclick = () => { selectedGroupId = g; refreshLayers(); };
    const vis = document.createElement('input'); vis.type = 'checkbox'; vis.checked = info.visible !== 0; vis.onchange = () => send({ cmd: 'group', op: 'setVisible', group: g, value: vis.checked ? 1 : 0 });
    const par = document.createElement('select'); const root = document.createElement('option'); root.value = '-1'; root.textContent = 'Root'; par.append(root); for (const p of engineState.groups) if (p.id !== g && p.alive) { const o = document.createElement('option'); o.value = String(p.id); o.textContent = `G${p.id+1}`; par.append(o); } par.value = String(info.parent); par.onchange = () => send({ cmd: 'group', op: 'setParent', group: g, value: Number(par.value) });
    const mode = document.createElement('select'); for (let m = 0; m < layerModes.length; m++) { const o = document.createElement('option'); o.value = String(m); o.textContent = layerModes[m]; mode.append(o); } mode.value = String(info.mode); mode.onchange = () => send({ cmd: 'group', op: 'setMode', group: g, value: Number(mode.value) });
    const op = document.createElement('input'); op.type = 'range'; op.min = '0'; op.max = '1'; op.step = '0.01'; op.value = String(info.opacity); op.oninput = () => send({ cmd: 'group', op: 'setOpacity', group: g, value: Number(op.value) });
    const pass = document.createElement('input'); pass.type = 'checkbox'; pass.checked = info.passThrough !== 0; pass.onchange = () => send({ cmd: 'group', op: 'setPassThrough', group: g, value: pass.checked ? 1 : 0 });
    const iso = document.createElement('input'); iso.type = 'checkbox'; iso.checked = info.isolated !== 0; iso.onchange = () => send({ cmd: 'group', op: 'setIsolated', group: g, value: iso.checked ? 1 : 0 });
    row.append(up, dn, btn, vis, par, mode, op, pass, iso); groupList.append(row); }
}
function renderCatalog(brushes: BrushPreset[]) { brushGrid.replaceChildren(); brushButtons.clear(); let last = ''; for (const b of brushes) { if (b.group !== last) { const h = document.createElement('h3'); h.textContent = b.group; brushGrid.append(h); last = b.group; } const btn = document.createElement('button'); btn.type = 'button'; btn.className = 'brush-item'; const img = document.createElement('img'); img.src = brushUrl(b.preview); img.alt = ''; const sp = document.createElement('span'); sp.textContent = b.name; btn.append(img, sp); btn.onclick = () => void selectBrush(b); brushButtons.set(b.id, btn); brushGrid.append(btn); } }
async function selectBrush(b: BrushPreset) { const r = await fetch(brushUrl(b.brush)); if (!r.ok) return; const json = await r.text(); selectedBrushId = b.id; selectedBrushJson = json; try { const root = JSON.parse(json); const rest = Number(root?.settings?.restore_color?.base_value ?? 0); if (rest > 0) { const s = [Number(root?.settings?.color_h?.base_value ?? 0), Number(root?.settings?.color_s?.base_value ?? 0), Number(root?.settings?.color_v?.base_value ?? 0)]; const a = hexRgb(ui.color).map(v => v ** 2.2); const f = Math.max(0, Math.min(1, rest)); const r2 = a[0]*(1-f)+s[0]*f, g2 = a[1]*(1-f)+s[1]*f, b2 = a[2]*(1-f)+s[2]*f; ui.color = `#${Math.round(r2**(1/2.2)*255).toString(16).padStart(2,'0')}${Math.round(g2**(1/2.2)*255).toString(16).padStart(2,'0')}${Math.round(b2**(1/2.2)*255).toString(16).padStart(2,'0')}`; ($('color') as HTMLInputElement).value = ui.color; } } catch {} send({ cmd: 'loadBrush', json }); applyBrushColor(); applyBrushOverrides(); for (const [id, btn] of brushButtons) btn.classList.toggle('selected', id === selectedBrushId); statusEl.textContent = `Ready — ${b.name}.`; }
async function loadCatalog() { const r = await fetch('/mypaint/brushes.json'); if (!r.ok) throw new Error('Cannot load brush catalog.'); const m = await r.json() as { count: number; brushes: BrushPreset[] }; renderCatalog(m.brushes); const init = m.brushes.find(b => b.id === 'classic/brush') ?? m.brushes[0]; if (init) await selectBrush(init); log(`${m.count} brushes loaded.`); }

// Export
function reqTiles(layerId: number | null): Promise<{ data: ArrayBuffer[]; scale: number }> { const id = ++exportSeq; return new Promise(res => { pendingTiles = (v) => res(v); send({ cmd: 'exportTiles', layerId, id }); }); }
async function canvasPng(c: HTMLCanvasElement): Promise<Uint8Array> { const b = await new Promise<Blob|null>(r => c.toBlob(r, 'image/png')); if (!b) throw new Error('PNG failed.'); return new Uint8Array(await b.arrayBuffer()); }
async function renderPng(layerId: number | null): Promise<Uint8Array> { if (!engineState) throw new Error('No engine.'); const sc = engineState.displayScale; const ow = Math.ceil(engineState.width / sc), oh = Math.ceil(engineState.height / sc); const out = document.createElement('canvas'); out.width = ow; out.height = oh; const g = out.getContext('2d', { alpha: true })!; const { data, scale } = await reqTiles(layerId); const cols = Math.ceil(ow / (64 * scale)); const img = new ImageData(64, 64); for (let i = 0; i < data.length; i++) { const tx = i % cols, ty = Math.floor(i / cols); img.data.set(new Uint8Array(data[i])); g.putImageData(img, tx * 64, ty * 64); } return canvasPng(out); }
function dl(data: Uint8Array, name: string, type: string) { const b = new Blob([data as BlobPart], { type }); const u = URL.createObjectURL(b); const a = document.createElement('a'); a.href = u; a.download = name; a.click(); setTimeout(() => URL.revokeObjectURL(u), 1000); }
async function exportPng() { dl(await renderPng(null), 'afterglow-paint.png', 'image/png'); }
function compOp(m: number) { const n = ['svg:src-over','svg:multiply','svg:screen','svg:overlay','svg:darken','svg:lighten','svg:hard-light','svg:soft-light','svg:color-burn','svg:color-dodge','svg:difference','svg:exclusion','svg:hue','svg:saturation','svg:color','svg:luminosity','svg:plus','svg:src-in','svg:src-out','svg:src-atop','svg:dst-atop','svg:src-over']; return n[m] ?? 'svg:src-over'; }
function buildStackXml(): string { if (!engineState) return ''; const lc = engineState.layers.length, gc = engineState.groups.length; const lX = (l: number, i: string) => `${i}<layer name="Layer ${l+1}" src="data/layer-${l}.png" opacity="${engineState.layers[l].opacity}" visibility="${engineState.layers[l].visible !== 0 ? 'visible' : 'hidden'}" composite-op="${compOp(engineState.layers[l].mode)}" />`; const gX = (g: number, i: string) => { const lines = [`${i}<stack name="Group ${g+1}" opacity="${engineState.groups[g].opacity}" visibility="${engineState.groups[g].visible !== 0 ? 'visible' : 'hidden'}" composite-op="${compOp(engineState.groups[g].mode)}">`]; for (let c = g - 1; c >= 0; c--) if (engineState.groups[c].alive && engineState.groups[c].parent === g && c !== g) lines.push(gX(c, i + '  ')); for (let l = lc - 1; l >= 0; l--) if (engineState.layers[l].group === g) lines.push(lX(l, i + '  ')); lines.push(`${i}</stack>`); return lines.join('\n'); }; const lines = ['<?xml version="1.0" encoding="UTF-8"?>', `<image version="0.0" w="${docSize.width}" h="${docSize.height}" name="Afterglow">`, '  <stack name="Afterglow">']; for (let g = gc - 1; g >= 0; g--) if (engineState.groups[g].alive && engineState.groups[g].parent < 0) lines.push(gX(g, '    ')); for (let l = lc - 1; l >= 0; l--) if (engineState.layers[l].group < 0) lines.push(lX(l, '    ')); lines.push('  </stack>', '</image>'); return lines.join('\n'); }
function buildMeta(): string { if (!engineState) return '{}'; return JSON.stringify({ width: docSize.width, height: docSize.height, layers: engineState.layers.map((l: any) => ({ id: l.id, group: l.group, visible: l.visible, opacity: l.opacity, mode: l.mode })), groups: engineState.groups.filter((g: any) => g.alive).map((g: any) => ({ id: g.id, parent: g.parent, visible: g.visible, opacity: g.opacity, mode: g.mode, passThrough: g.passThrough, isolated: g.isolated })) }); }
async function exportOra() { if (!engineState) return; const e = [{ name: 'mimetype', data: utf8('image/openraster') }, { name: 'stack.xml', data: utf8(buildStackXml()) }, { name: 'mergedimage.png', data: await renderPng(null) }, { name: 'data/metadata.json', data: utf8(buildMeta()) }]; for (let l = 0; l < engineState.layers.length; l++) e.push({ name: `data/layer-${l}.png`, data: await renderPng(l) }); dl(encodeStoredZip(e), 'afterglow-paint.ora', 'image/openraster'); }
async function importOra(file: File) { if (!ready) return; const entries = await decodeZip(await file.arrayBuffer()); const mb = entries.get('data/metadata.json'); const meta = mb ? JSON.parse(text(mb)) as any : null; const st = entries.get('stack.xml'); const stT = st ? text(st) : ''; const w = meta?.width ?? Number(stT.match(/\bw="(\d+)"/)?.[1] ?? docSize.width), h = meta?.height ?? Number(stT.match(/\bh="(\d+)"/)?.[1] ?? docSize.height); const merged = entries.get('mergedimage.png') ?? entries.get('data/layer-0.png'); if (!merged) throw new Error('No image.'); resetDoc(w, h); send({ cmd: 'clearBackground' }); send({ cmd: 'clear' }); const layers = meta?.layers ?? [{ id: 0, group: -1, visible: 1, mode: 0 }]; for (const l of layers) { if (l.id > 0) send({ cmd: 'layer', op: 'create', layer: l.id }); send({ cmd: 'layer', op: 'setVisible', layer: l.id, value: l.visible !== 0 ? 1 : 0 }); send({ cmd: 'layer', op: 'setOpacity', layer: l.id, value: Number(l.opacity) || 1 }); send({ cmd: 'layer', op: 'setMode', layer: l.id, value: Number(l.mode) || 0 }); } const ic = await imgCanvas(merged); writeImg(ic, 0); for (const l of layers) { if (l.id === 0) continue; const d = entries.get(`data/layer-${l.id}.png`); if (d) writeImg(await imgCanvas(d), l.id); } for (const g of meta?.groups ?? []) { send({ cmd: 'group', op: 'create', group: g.id }); send({ cmd: 'group', op: 'setVisible', group: g.id, value: g.visible !== 0 ? 1 : 0 }); send({ cmd: 'group', op: 'setOpacity', group: g.id, value: Number(g.opacity) || 0 }); send({ cmd: 'group', op: 'setMode', group: g.id, value: Number(g.mode) || 0 }); send({ cmd: 'group', op: 'setPassThrough', group: g.id, value: g.passThrough ? 1 : 0 }); send({ cmd: 'group', op: 'setIsolated', group: g.id, value: g.isolated ? 1 : 0 }); send({ cmd: 'group', op: 'setParent', group: g.id, value: Number(g.parent) }); } for (const l of layers) if (l.group !== undefined) send({ cmd: 'layer', op: 'setGroup', layer: l.id, value: Number(l.group) }); send({ cmd: 'layer', op: 'setActive', layer: 0 }); }
async function imgCanvas(d: Uint8Array): Promise<HTMLCanvasElement> { const bm = await createImageBitmap(new Blob([d as BlobPart], { type: 'image/png' })); const c = document.createElement('canvas'); c.width = bm.width; c.height = bm.height; c.getContext('2d', { alpha: true })!.drawImage(bm, 0, 0); bm.close(); return c; }
function writeImg(ic: HTMLCanvasElement, layer: number) { const g = ic.getContext('2d', { alpha: true })!; const img = g.getImageData(0, 0, ic.width, ic.height); const tile = new Uint8Array(64 * 64 * 4); for (let ty = 0; ty < Math.ceil(ic.height / 64); ty++) for (let tx = 0; tx < Math.ceil(ic.width / 64); tx++) { tile.fill(0); for (let y = 0; y < 64; y++) { const sy = ty * 64 + y; if (sy >= img.height) continue; for (let x = 0; x < 64; x++) { const sx = tx * 64 + x; if (sx >= img.width) continue; tile.set(img.data.subarray((sy * img.width + sx) * 4, (sy * img.width + sx) * 4 + 4), (y * 64 + x) * 4); } } send({ cmd: 'writeTile', layer, tx, ty, data: tile.slice().buffer }, [tile.slice().buffer]); } }
function resetDoc(w: number, h: number) { if (w < 64 || h < 64 || w > 16384 || h > 16384) return; docSize.width = w; docSize.height = h; const r = Math.max(w, h) / 4096; const ds = r <= 1 ? 1 : r <= 2 ? 2 : 4; dispW = Math.ceil(w / ds); dispH = Math.ceil(h / ds); canvas.style.width = `${dispW}px`; canvas.style.height = `${dispH}px`; send({ cmd: 'init', width: w, height: h }); }

worker = new Worker(new URL('./paint-engine-worker.ts', import.meta.url), { type: 'module' });
(window as any).probe = (y: number) => { worker.postMessage({ cmd: 'probe', id: Math.floor(Math.random() * 1e9), y }); };
worker.onmessage = (e: MessageEvent) => { const m = e.data;
  switch (m.type) {
    case 'ready': ready = true; refreshLayers(); statusEl.textContent = 'Ready — choose a brush or draw.'; break;
    case 'state': engineState = m.state; if (engineState) { dispW = Math.ceil(engineState.width / engineState.displayScale); dispH = Math.ceil(engineState.height / engineState.displayScale); canvas.style.width = `${dispW}px`; canvas.style.height = `${dispH}px`; } refreshLayers(); break;
    case 'status': statusEl.textContent = m.text; break;
    case 'log': log(m.text); break;
    case 'stats': hudEl.textContent = `queue  ${m.queued} sp\nbrush  ${m.brushMs.toFixed(1)} ms\nrender ${m.renderMs.toFixed(1)} ms\ninput  ${m.sps}/s`; break;
    case 'tiles': if (pendingTiles) { const r = pendingTiles; pendingTiles = null; r({ data: m.data, scale: m.scale }); } break;
    case 'probeResult': (window as any).__probeResult = m; break;
  }
};
worker.onerror = (e) => { log(`Worker error: ${e.message}`); statusEl.textContent = 'Engine error.'; };

async function init() { statusEl.textContent = 'Loading brush engine…'; log('loading brush engine…');
  const off = canvas.transferControlToOffscreen(); send({ cmd: 'init', width: docSize.width, height: docSize.height, canvas: off }, [off]);
  try { await loadCatalog(); } catch (e) { log(`Brush catalog error: ${(e as Error).message}`); } refreshLayers();
}
applyView(); init().catch(e => { statusEl.textContent = 'Engine failed to load: ' + (e as Error).message; log('ERROR: ' + ((e as Error).stack || (e as Error).message)); });

// UI bindings
['radius','hardness','opacity'].forEach(k => { const i = $(k) as HTMLInputElement; const a = () => { (ui as any)[k] = Number(i.value); $(`${k}Val`).textContent = i.value; applyBrushOverrides(); }; i.addEventListener('input', a); a(); });
$('color').addEventListener('input', e => { ui.color = (e.target as HTMLInputElement).value; applyBrushColor(); applyBrushOverrides(); });
$('viewZoom').addEventListener('input', e => { view.zoom = Number((e.target as HTMLInputElement).value); applyView(); });
$('rotateLeftBtn').addEventListener('click', () => { view.rotationDegrees = (view.rotationDegrees + 90) % 360; applyView(); });
$('rotateRightBtn').addEventListener('click', () => { view.rotationDegrees = (view.rotationDegrees + 270) % 360; applyView(); });
$('mirrorBtn').addEventListener('click', () => { view.mirror = !view.mirror; applyView(); });
$('resetViewBtn').addEventListener('click', () => { Object.assign(view, { zoom: 1, rotationDegrees: 0, mirror: false, panX: 0, panY: 0 }); applyView(); });
canvas.addEventListener('wheel', e => { e.preventDefault(); view.zoom = Math.max(0.1, Math.min(8, view.zoom * (e.deltaY < 0 ? 1.1 : 0.9))); applyView(); }, { passive: false });
$('frameEnabled').addEventListener('change', e => canvas.classList.toggle('frame-visible', (e.target as HTMLInputElement).checked));
$('clearBtn').addEventListener('click', () => send({ cmd: 'clear' }));
$('backgroundColor').addEventListener('input', applyBgColor);
$('undoBtn').addEventListener('click', () => send({ cmd: 'undo' }));
$('redoBtn').addEventListener('click', () => send({ cmd: 'redo' }));
$('newDocumentBtn').addEventListener('click', () => resetDoc(Number(($('documentWidth') as HTMLInputElement).value), Number(($('documentHeight') as HTMLInputElement).value)));
$('exportPngBtn').addEventListener('click', () => void exportPng().catch(e => statusEl.textContent = `PNG export failed: ${(e as Error).message}`));
$('exportOraBtn').addEventListener('click', () => void exportOra().catch(e => statusEl.textContent = `ORA export failed: ${(e as Error).message}`));
$('importOraInput').addEventListener('change', e => { const f = (e.target as HTMLInputElement).files?.[0]; if (f) void importOra(f).then(() => statusEl.textContent = 'OpenRaster imported.').catch(er => statusEl.textContent = `ORA import failed: ${(er as Error).message}`); });
$('addLayerBtn').addEventListener('click', () => send({ cmd: 'layer', op: 'create', layer: 0 }));
$('deleteLayerBtn').addEventListener('click', () => { if (engineState) send({ cmd: 'layer', op: 'delete', layer: engineState.activeLayer }); });
$('addGroupBtn').addEventListener('click', () => send({ cmd: 'group', op: 'create', group: 0 }));
$('deleteGroupBtn').addEventListener('click', () => { if (selectedGroupId >= 0) { send({ cmd: 'group', op: 'delete', group: selectedGroupId }); selectedGroupId = -1; } });
$('strokeBtn').addEventListener('click', () => { if (!ready) return; ensureBrush(); const y = docSize.height / 2, x0 = docSize.width * 0.15; send({ cmd: 'beginStroke', x: x0, y, xtilt: 0, ytilt: 0, zoom: view.zoom, rotation: view.rotationDegrees * Math.PI / 180, barrel: 0.5 }); for (let i = 1; i <= 10; i++) send({ cmd: 'strokeSample', x: x0 + (docSize.width * 0.6) * (i / 10), y, pressure: 0.5, xtilt: 0, ytilt: 0, time: i * 16, zoom: view.zoom, rotation: view.rotationDegrees * Math.PI / 180, barrel: 0.5 }); send({ cmd: 'commit' }); statusEl.textContent = 'Test stroke drawn.'; });
