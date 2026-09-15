import './paint-main.ts';
const reportError = console.error.bind(console);
console.error = (...values: unknown[]) => reportError(...values.map(value => value instanceof Error ? value.stack ?? value.message : value));

const page = window as typeof window & {
  probe: (y: number) => number;
  __probeResult?: { id: number; native?: boolean; colored: number; transparent: number };
};
async function probe() {
  const id = page.probe(0.5);
  const deadline = performance.now() + 30_000;
  while (page.__probeResult?.id !== id) {
    if (performance.now() > deadline) throw new Error('Native paint probe timeout.');
    await new Promise(resolve => setTimeout(resolve, 10));
  }
  return { ...page.__probeResult };
}
function press(id: string) { (document.getElementById(id) as HTMLButtonElement).click(); }
const initial = await probe();
if (!initial.native) throw new Error('The paint probe needs the native target.');
const strokeStart = performance.now();
press('strokeBtn');
const painted = await probe();
const strokeMs = performance.now() - strokeStart;
press('undoBtn');
const undone = await probe();
press('redoBtn');
const redone = await probe();
if (painted.colored <= initial.colored || undone.colored !== initial.colored || redone.colored !== painted.colored) {
  throw new Error(`Native paint history mismatch: ${JSON.stringify({ initial, painted, undone, redone })}`);
}
const canvas = document.getElementById('paint') as HTMLCanvasElement;
press('undoBtn');
await probe();
const rect = canvas.getBoundingClientRect();
const centerX = rect.left + rect.width / 2, centerY = rect.top + rect.height / 2;
const dispatchPointer = (globalThis as typeof globalThis & {
  __dispatchBrowserPointerEvent: (type: string, init: PointerEventInit) => boolean;
}).__dispatchBrowserPointerEvent;
if (document.elementFromPoint(centerX, centerY) !== canvas) throw new Error('Native hit test missed the paint canvas.');
const pointerStart = performance.now();
for (const [type, x, buttons] of [
  ['pointerdown', centerX, 1], ['pointermove', centerX + 100, 1], ['pointerup', centerX + 200, 0],
] as const) {
  dispatchPointer(type, {
    pointerId: 9001, pointerType: 'mouse', button: 0, buttons,
    clientX: x, clientY: centerY, pressure: buttons ? 0.5 : 0,
  });
  await new Promise(resolve => setTimeout(resolve, 10));
}
const pointerPainted = await probe();
const pointerMs = performance.now() - pointerStart;
if (pointerPainted.colored <= initial.colored) throw new Error('Native pointer stroke did not change the canvas.');
press('undoBtn');
const pointerUndone = await probe();
if (pointerUndone.colored !== initial.colored) throw new Error('Native pointer undo did not restore the canvas.');
press('redoBtn');
await probe();
const grid = document.getElementById('brushGrid') as HTMLElement;
const brushes = Array.from(grid.querySelectorAll<HTMLButtonElement>('button'));
const controlsStart = performance.now();
for (let index = 0; index < 6; index++) {
  const brush = brushes[index % 2];
  const brushRect = brush.getBoundingClientRect();
  const x = brushRect.left + brushRect.width / 2, y = brushRect.top + brushRect.height / 2;
  if (!brush.contains(document.elementFromPoint(x, y))) throw new Error('Native hit test missed a brush.');
  const image = brush.querySelector('img')!.getBoundingClientRect();
  const label = brush.querySelector('span')!.getBoundingClientRect();
  if (label.top < image.bottom - 1) throw new Error('The brush label overlaps its image.');
  for (const type of ['pointerdown', 'pointerup']) dispatchPointer(type, {
    pointerId: 9002, pointerType: 'mouse', button: 0, clientX: x, clientY: y,
  });
  const deadline = performance.now() + 3000;
  while (!brush.classList.contains('selected')) {
    if (performance.now() > deadline) throw new Error('Native brush selection did not complete.');
    await new Promise(resolve => setTimeout(resolve, 10));
  }
  if ((await probe()).colored !== pointerPainted.colored) throw new Error('Brush selection changed the canvas.');
}
const range = document.getElementById('radius') as HTMLInputElement;
const rangeRect = range.getBoundingClientRect();
if (rangeRect.height < 12) throw new Error('The native slider has no height.');
const previousRadius = range.value;
for (const type of ['pointerdown', 'pointerup']) dispatchPointer(type, {
  pointerId: 9003, pointerType: 'mouse', button: 0,
  clientX: rangeRect.left + rangeRect.width * 0.75, clientY: rangeRect.top + rangeRect.height / 2,
});
if (range.value === previousRadius) throw new Error('The native slider did not change its value.');
const gridRect = grid.getBoundingClientRect();
(globalThis as typeof globalThis & {
  __dispatchBrowserWheelEvent: (init: WheelEventInit) => boolean;
}).__dispatchBrowserWheelEvent({ clientX: gridRect.left + 20, clientY: gridRect.top + 100, deltaY: 200, deltaMode: 0 });
if (grid.scrollTop <= 0) throw new Error(`The native brush list did not scroll: ${JSON.stringify({
  rect: gridRect, clientHeight: grid.clientHeight, scrollHeight: grid.scrollHeight,
  overflow: getComputedStyle(grid).overflowY,
  target: document.elementFromPoint(gridRect.left + 20, gridRect.top + 100)?.outerHTML.slice(0, 180),
})}`);
const scrollTop = grid.scrollTop;
const scrolledBrush = brushes.find(brush => {
  const rect = brush.getBoundingClientRect();
  return rect.top >= gridRect.top && rect.bottom <= gridRect.bottom;
});
if (!scrolledBrush) throw new Error('The scrolled brush list has no visible brush.');
const scrolledRect = scrolledBrush.getBoundingClientRect();
if (!scrolledBrush.contains(document.elementFromPoint(scrolledRect.left + scrolledRect.width / 2, scrolledRect.top + scrolledRect.height / 2))) {
  throw new Error('The scrolled brush hit test is incorrect.');
}
const clickNative = (element: HTMLElement) => {
  const rect = element.getBoundingClientRect();
  const x = rect.left + rect.width / 2, y = rect.top + rect.height / 2;
  const hit = document.elementFromPoint(x, y);
  if (!element.contains(hit)) throw new Error(`Native control hit test failed: ${JSON.stringify({
    control: element.outerHTML.slice(0, 400), rect, x, y,
    hit: hit?.outerHTML.slice(0, 400), parent: element.parentElement?.getBoundingClientRect(),
    parentStyle: element.parentElement?.getAttribute('style'), pointerEvents: getComputedStyle(element).pointerEvents,
    parentZ: getComputedStyle(element.parentElement!).zIndex, parentPointer: getComputedStyle(element.parentElement!).pointerEvents,
    hits: document.elementsFromPoint(x, y).slice(0, 5).map(node => node.outerHTML.slice(0, 120)),
    bodyStyle: document.body.getAttribute('style'), bodyPointerEvents: getComputedStyle(document.body).pointerEvents,
  })}`);
  for (const type of ['pointerdown', 'pointerup']) dispatchPointer(type, {
    pointerId: 9005, pointerType: 'mouse', button: 0, clientX: x, clientY: y,
  });
};
const dispatchKey = (globalThis as typeof globalThis & {
  __dispatchBrowserKeyboardEvent: (type: string, init: KeyboardEventInit) => boolean;
}).__dispatchBrowserKeyboardEvent;
for (const trigger of Array.from(document.querySelectorAll<HTMLElement>('.menu-bar [aria-haspopup="menu"]'))) {
  clickNative(trigger);
  await new Promise(resolve => setTimeout(resolve, 100));
  const menu = document.querySelector<HTMLElement>('[role="menu"]');
  if (!menu || menu.getBoundingClientRect().height <= 0) throw new Error(`Native menu did not open: ${trigger.textContent}`);
  const menuRect = menu.getBoundingClientRect();
  if (menuRect.top < trigger.getBoundingClientRect().bottom || menuRect.right > innerWidth || menuRect.bottom > innerHeight) {
    throw new Error(`Native menu position is incorrect: ${JSON.stringify({ rect: menuRect, parent: menu.parentElement?.outerHTML.slice(0, 1800), parentRect: menu.parentElement?.getBoundingClientRect(), parentTransform: getComputedStyle(menu.parentElement!).transform, menuTransform: getComputedStyle(menu).transform, trigger: trigger.getBoundingClientRect() })}`);
  }
  console.log('[native-menu]', JSON.stringify({ trigger: trigger.textContent, rect: menu.getBoundingClientRect(), style: menu.getAttribute('style'), active: document.activeElement?.outerHTML.slice(0, 400) }));
  dispatchKey('keydown', { key: 'Escape', code: 'Escape' });
  await new Promise(resolve => setTimeout(resolve, 500));
  if (document.querySelector('[role="menu"]')) throw new Error(`Native menu did not close: ${trigger.textContent}: ${document.querySelector('[role="menu"]')?.outerHTML.slice(0, 600)}`);
}
const viewMenu = Array.from(document.querySelectorAll<HTMLElement>('.menu-bar [aria-haspopup="menu"]')).find(element => element.textContent === 'View')!;
clickNative(viewMenu);
await new Promise(resolve => setTimeout(resolve, 100));
const actualPixels = Array.from(document.querySelectorAll<HTMLElement>('[role="menuitem"]')).find(element => element.textContent?.startsWith('100%'))!;
clickNative(actualPixels);
await new Promise(resolve => setTimeout(resolve, 150));
const expectedPixelZoom = Math.max(0.1, Math.min(8, canvas.width / Math.max(1, canvas.offsetWidth)));
if (document.querySelector('[role="menu"]') || Math.abs(Number((document.getElementById('viewZoom') as HTMLInputElement).value) - expectedPixelZoom) > 0.001) {
  throw new Error(`Native menu item did not activate: ${JSON.stringify({ open: Boolean(document.querySelector('[role="menu"]')), zoom: (document.getElementById('viewZoom') as HTMLInputElement).value, expectedPixelZoom })}`);
}
viewMenu.focus();
dispatchKey('keydown', { key: 'ArrowDown' });
await new Promise(resolve => setTimeout(resolve, 100));
if (!document.querySelector('[role="menu"]')) throw new Error('Native keyboard menu did not open.');
for (const type of ['pointerdown', 'pointerup']) dispatchPointer(type, { pointerId: 9005, pointerType: 'mouse', button: 0, clientX: 16, clientY: innerHeight - 8 });
await new Promise(resolve => setTimeout(resolve, 150));
if (document.querySelector('[role="menu"]')) throw new Error('Native outside click did not close the menu.');
for (const id of ['stabilizerMode', 'layerBlendMode', 'layerParent']) {
  const select = document.getElementById(id) as HTMLSelectElement;
  const visible = Array.from(select.options).filter(option => getComputedStyle(option).display !== 'none');
  if (visible.length !== 1 || !visible[0].textContent?.trim()) throw new Error(`Empty native select: ${id}: ${JSON.stringify(Array.from(select.options).map(option => ({ value: option.value, text: option.textContent, selected: option.selected, display: getComputedStyle(option).display })))}`);
  clickNative(select);
  const popup = document.querySelector<HTMLElement>('[role="listbox"]');
  if (!popup) throw new Error(`Native select did not open: ${id}`);
  // The popup initially scrolls to the selected option, which can be below this row.
  popup.scrollTop = 0;
  const row = popup.children[id === 'stabilizerMode' ? 1 : 0] as HTMLElement;
  clickNative(row);
  if (document.querySelector('[role="listbox"]')) throw new Error(`Native select did not close: ${id}`);
  if (id === 'stabilizerMode' && select.value !== 'string') throw new Error('Native select did not change.');
}
const mode = document.getElementById('stabilizerMode') as HTMLSelectElement;
mode.value = 'off';
mode.dispatchEvent(new Event('change', { bubbles: true }));
const documentPanel = document.getElementById('documentWidth')!.closest('details')!;
if (!documentPanel.hasAttribute('open')) clickNative(documentPanel.querySelector('summary')!);
for (const id of ['documentWidth', 'documentHeight']) {
  const input = document.getElementById(id) as HTMLInputElement;
  if (!(Number(input.value) >= 64)) throw new Error(`Empty native number input: ${id}`);
  const style = getComputedStyle(input);
  const minimumHeight = Number.parseFloat(style.fontSize) + Number.parseFloat(style.paddingTop)
    + Number.parseFloat(style.paddingBottom) + Number.parseFloat(style.borderTopWidth) + Number.parseFloat(style.borderBottomWidth);
  if (!(input.getBoundingClientRect().height >= minimumHeight)) throw new Error(`Clipped native number input: ${id}`);
  clickNative(input);
  dispatchKey('keydown', { key: 'a', code: 'KeyA', ctrlKey: true });
  for (const key of '1024') dispatchKey('keydown', { key });
  if (input.value !== '1024') throw new Error(`Native text replacement failed: ${id}: ${input.value}`);
  dispatchKey('keydown', { key: 'Backspace' });
  if (String(input.value) !== '102') throw new Error(`Native text deletion failed: ${id}`);
  dispatchKey('keydown', { key: 'a', code: 'KeyA', ctrlKey: true });
  for (const key of '2048') dispatchKey('keydown', { key });
  input.blur();
}
const dispatchWheel = (globalThis as typeof globalThis & {
  __dispatchBrowserWheelEvent: (init: WheelEventInit) => boolean;
}).__dispatchBrowserWheelEvent;
const zoom = document.getElementById('viewZoom') as HTMLInputElement;
const initialZoom = Number(zoom.value);
dispatchWheel({ clientX: centerX, clientY: centerY, deltaY: -120 });
if (Number(zoom.value) <= initialZoom) throw new Error('Upward wheel input must increase canvas zoom.');
const increasedZoom = Number(zoom.value);
dispatchWheel({ clientX: centerX, clientY: centerY, deltaY: 120 });
if (Number(zoom.value) >= increasedZoom) throw new Error('Downward wheel input must decrease canvas zoom.');
const hoverButton = document.getElementById('eyedropperToolBtn') as HTMLElement;
dispatchPointer('pointermove', { pointerId: 9005, pointerType: 'mouse', clientX: centerX, clientY: centerY });
await new Promise(resolve => setTimeout(resolve, 300));
const beforeHover = getComputedStyle(hoverButton).backgroundColor;
const hoverRect = hoverButton.getBoundingClientRect();
dispatchPointer('pointermove', { pointerId: 9005, pointerType: 'mouse', clientX: hoverRect.left + 10, clientY: hoverRect.top + 10 });
await new Promise(resolve => setTimeout(resolve, 300));
const afterHover = getComputedStyle(hoverButton).backgroundColor;
if (afterHover === beforeHover) throw new Error(`Native button hover did not change: ${beforeHover}`);
console.log('[native-controls] PASS', JSON.stringify({ clicks: 6, scrollTop, radius: range.value, selects: 3, menus: 3, textFields: 2, beforeHover, afterHover, controlsMs: performance.now() - controlsStart }));
grid.scrollTop = 0;

const textProbe = document.createElement('input');
textProbe.type = 'text';
textProbe.style.cssText = 'position:fixed;left:400px;top:100px;width:280px;height:36px;z-index:9999;background:white;color:black';
document.body.appendChild(textProbe);
textProbe.value = 'before';
textProbe.focus();
textProbe.select();
const dispatchIme = (globalThis as typeof globalThis & {
  __dispatchBrowserImeEvent: (init: { phase: string; data?: string; cursor?: [number, number] }) => void;
}).__dispatchBrowserImeEvent;
dispatchIme({ phase: 'preedit', data: 'に', cursor: [3, 3] });
await new Promise<number>(resolve => requestAnimationFrame(resolve));
if (String(textProbe.value) !== 'に') throw new Error('Native IME preedit is missing.');
dispatchIme({ phase: 'preedit', data: '日本', cursor: [6, 6] });
await new Promise<number>(resolve => requestAnimationFrame(resolve));
dispatchIme({ phase: 'commit', data: '日本語' });
if (String(textProbe.value) !== '日本語') throw new Error('Native IME commit duplicated preedit.');
dispatchKey('keydown', { key: 'z', ctrlKey: true });
if (String(textProbe.value) !== 'before' || textProbe.selectionStart !== 0 || textProbe.selectionEnd !== 6) throw new Error('Native composition undo lost text or selection.');
dispatchKey('keydown', { key: 'z', ctrlKey: true, shiftKey: true });
if (String(textProbe.value) !== '日本語') throw new Error('Native composition redo failed.');
textProbe.select();
dispatchIme({ phase: 'preedit', data: '中', cursor: [3, 3] });
textProbe.blur();
if (String(textProbe.value) !== '日本語') throw new Error('Native IME cancellation lost text.');
textProbe.remove();
console.log('[native-text-composition] PASS');

// These checks measure native rAF production, not physical display timing.
const frameTimes = new Float64Array(120);
for (const mode of ['unchanged', 'hover', 'slider'] as const) {
  let previous = 0;
  const startPresented = performance.now();
  const layerRows = Array.from(document.getElementById('layerList')!.children);
  if (mode === 'slider') dispatchPointer('pointerdown', {
    pointerId: 9004, pointerType: 'mouse', button: 0,
    clientX: rangeRect.left + rangeRect.width / 2, clientY: rangeRect.top + rangeRect.height / 2,
  });
  for (let frame = -10; frame < frameTimes.length; frame++) {
    const now = await new Promise<number>(resolve => requestAnimationFrame(resolve));
    if (frame >= 0) frameTimes[frame] = now - previous;
    previous = now;
    if (mode === 'hover') dispatchPointer('pointermove', {
      pointerId: 9004, pointerType: 'mouse',
      clientX: frame % 2 ? gridRect.left + 35 : rangeRect.left + 35,
      clientY: frame % 2 ? gridRect.top + 70 : rangeRect.top + rangeRect.height / 2,
    });
    if (mode === 'slider') dispatchPointer('pointermove', {
      pointerId: 9004, pointerType: 'mouse', buttons: 1,
      clientX: rangeRect.left + rangeRect.width * (0.1 + ((frame + 10) % 40) / 50),
      clientY: rangeRect.top + rangeRect.height / 2,
    });
  }
  if (mode === 'slider') dispatchPointer('pointerup', {
    pointerId: 9004, pointerType: 'mouse', button: 0,
    clientX: rangeRect.left + rangeRect.width / 2, clientY: rangeRect.top + rangeRect.height / 2,
  });
  if (mode === 'slider') {
    const currentRows = document.getElementById('layerList')!.children;
    if (currentRows.length !== layerRows.length || layerRows.some((row, index) => row !== currentRows[index])) {
      throw new Error('Brush input replaced unchanged layer rows.');
    }
  }
  const sum = frameTimes.reduce((total, value) => total + value, 0);
  frameTimes.sort();
  console.log('[native-ui-cadence]', JSON.stringify({ mode, samples: frameTimes.length,
    fps: 1000 * frameTimes.length / sum, p50: frameTimes[60], p99: frameTimes[118],
    max: frameTimes[119], elapsedMs: performance.now() - startPresented,
  }));
}
const png = await new Promise<Blob | null>(resolve => canvas.toBlob(resolve, 'image/png'));
if (!png || png.size < 8) throw new Error('Native paint PNG is empty.');
const signature = new Uint8Array(await png.slice(0, 8).arrayBuffer());
if (signature.join(',') !== '137,80,78,71,13,10,26,10') throw new Error('Native paint PNG signature is incorrect.');
clickNative(viewMenu);
await new Promise(resolve => setTimeout(resolve, 150));
if (!document.querySelector('[role="menu"]')) throw new Error('The final menu capture is missing.');
console.log(`[native-paint-probe] PASS ${JSON.stringify({ initial, painted, undone, redone, pointerPainted, pointerUndone, strokeMs, pointerMs, pngBytes: png.size })}`);
