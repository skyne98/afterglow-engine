import assert from 'node:assert/strict';

let snapshot = null;
let focused = 0;
let pointerLocks = 0;
let canvasRaster = null;
let rasterCalls = 0;
let operationLog = [];
let redrawNotifications = 0;
let encodedRaster = null;
const nodeByAttributeId = (id) => snapshot.nodes.find((node) =>
  node.attributes?.some((attribute) => attribute.localName === 'id' && attribute.value === id)
);
const nodeByName = (name) => snapshot.nodes.find((node) => node.localName === name);
const rect = (id) => {
  const button = nodeByAttributeId('button');
  const box = nodeByAttributeId('box');
  if (id === button?.id) return { x: 10, y: 10, width: 80, height: 30 };
  if (id === box?.id) return { x: 20, y: 60, width: 100, height: 40 };
  return { x: 0, y: 0, width: 800, height: 500 };
};

Object.defineProperty(globalThis, 'navigator', { value: {}, writable: true, configurable: true });
globalThis.__documentHTML = `<!doctype html><html><body>
  <button id="button">Button</button><input id="check" type="checkbox">
  <div id="box"></div><canvas id="canvas" width="2" height="1"></canvas>
</body></html>`;
globalThis.__exampleURL = 'file:///tmp/test.html';
globalThis.__viewportWidth = 800;
globalThis.__viewportHeight = 500;
globalThis.Deno = { core: { ops: {
  op_probe_log() {},
  op_browser_document_dirty() { redrawNotifications++; },
  op_sync_browser_document(_epoch, value) { operationLog.push('sync'); snapshot = value; },
  op_browser_media_query_matches(query) {
    return query === '(min-width: 800px)' || query === 'screen and (orientation: landscape)';
  },
  op_browser_computed_property(_id, property) {
    return property.startsWith('padding-') ? '0px' : property.startsWith('overflow-') ? 'visible' : '';
  },
  op_browser_rect(id) { return rect(id); },
  op_browser_box_metrics(id) {
    const value = rect(id);
    return {
      clientWidth: value.width, clientHeight: value.height, clientLeft: 0, clientTop: 0,
      offsetWidth: value.width, offsetHeight: value.height, offsetLeft: value.x, offsetTop: value.y,
      offsetParent: null, scrollWidth: value.width, scrollHeight: value.height,
      scrollLeft: 0, scrollTop: 0,
    };
  },
  op_browser_hit_test() { return nodeByAttributeId('button').id; },
  op_browser_hit_tests() {
    return [nodeByAttributeId('button').id, nodeByName('body').id, nodeByName('html').id];
  },
  op_browser_intersection(id) {
    return {
      intersectionRect: rect(id),
      rootBounds: { x: 0, y: 0, width: 800, height: 500 },
    };
  },
  op_browser_set_focus(id) { const changed = focused !== id; focused = id; return changed; },
  op_browser_set_pointer_state() { return false; },
  op_browser_set_scroll() { return false; },
  op_request_pointer_lock() { pointerLocks++; },
  op_exit_pointer_lock() { pointerLocks--; },
  op_set_fetch_state() {}, op_set_loaded_asset_bytes() {},
  op_create_capture_canvas() { return {}; }, op_bind_canvas_node() {}, op_resize_canvas() {},
  op_browser_set_canvas_raster(id, width, height, data) {
    assert.ok(data instanceof Uint8Array, 'native byte ops need Uint8Array, not Uint8ClampedArray');
    operationLog.push('raster');
    rasterCalls++;
    canvasRaster = { id, width, height, data: [...data] };
  },
  op_encode_png(width, height, data) {
    encodedRaster = { width, height, data: [...data] };
    return new Uint8Array([137, 80, 78, 71]);
  },
} } };

await import('../dom_setup.ts');

const { reactive } = await import('../../../prototype/character-editor/node_modules/@vue/reactivity/dist/reactivity.esm-bundler.js');
for (const value of [document, document.body, document.createElement('input'), document.createTextNode('text')]) {
  assert.notEqual(Object.prototype.toString.call(value), '[object Object]');
  assert.equal(reactive(value), value, 'Vue must retain DOM node identity');
}

document.body.setAttribute('data-sync-test', 'first');
__syncBrowserDocument();
const syncCount = operationLog.filter((op) => op === 'sync').length;
const redrawCount = redrawNotifications;
await Promise.resolve();
__syncBrowserDocument();
assert.equal(operationLog.filter((op) => op === 'sync').length, syncCount,
  'an empty observer callback after takeRecords must not synchronize the DOM again');
assert.equal(redrawNotifications, redrawCount, 'empty observer callbacks do not request redraws');
document.body.setAttribute('data-sync-test', 'second');
await Promise.resolve();
assert.ok(redrawNotifications > redrawCount, 'DOM changes request a redraw before snapshot synchronization');
assert.equal(operationLog.filter(op => op === 'sync').length, syncCount);
__syncBrowserDocument();
assert.equal(operationLog.filter((op) => op === 'sync').length, syncCount + 1,
  'a later mutation must still synchronize the DOM');

document.body.style.cssText = 'transform:translate(0px,-200%)';
__syncBrowserDocument();
document.body.style.transform = 'translate(33px,32px)';
__syncBrowserDocument();
assert.ok(nodeByName('body').attributes.find(attribute => attribute.localName === 'style').value.includes('translate(33px,32px)'),
  'style property changes must reach the native snapshot');
document.body.style.removeProperty('transform');
__syncBrowserDocument();
assert.equal(nodeByName('body').attributes.find(attribute => attribute.localName === 'style').value, '');

const eventStart = performance.now();
const pointerTimestamp = new PointerEvent('pointermove').timeStamp;
assert.ok(pointerTimestamp >= eventStart && pointerTimestamp <= performance.now(),
  'pointer timestamps use the monotonic performance clock, not the Unix epoch');

assert.equal(matchMedia('(min-width: 800px)').matches, true);
assert.equal(matchMedia('(max-width: 20px)').matches, false);
const canvas = document.getElementById('canvas');
const canvas2d = canvas.getContext('2d');
canvas2d.fillStyle = '#ff0000';
canvas2d.fillRect(0, 0, 2, 1);
canvas2d.commit();
assert.equal(canvasRaster.id > 0, true);
assert.deepEqual({ ...canvasRaster, id: 1 }, {
  id: 1,
  width: 2,
  height: 1,
  data: [255, 0, 0, 255, 255, 0, 0, 255],
});
assert.deepEqual(operationLog.slice(-2), ['sync', 'raster']);
const png = await new Promise((resolve) => canvas.toBlob(resolve, 'image/png'));
assert.equal(png.type, 'image/png');
assert.deepEqual([...new Uint8Array(await png.arrayBuffer())], [137, 80, 78, 71]);
assert.deepEqual(encodedRaster, { width: 2, height: 1, data: canvasRaster.data });
const offscreen = document.createElement('canvas');
offscreen.width = 2;
offscreen.height = 1;
const offscreen2d = offscreen.getContext('2d');
offscreen2d.fillStyle = '#00ff00';
offscreen2d.fillRect(0, 0, 2, 1);
const callsBeforeOffscreenCommit = rasterCalls;
offscreen2d.commit();
assert.equal(rasterCalls, callsBeforeOffscreenCommit);
assert.equal(document.elementFromPoint(12, 12).id, 'button');
assert.deepEqual(document.elementsFromPoint(12, 12).map((element) => element.localName), ['button', 'body', 'html']);

const button = document.getElementById('button');
const focusEvents = [];
button.addEventListener('focus', () => focusEvents.push('focus'));
button.focus();
assert.equal(document.activeElement, button);
assert.equal(focused !== 0, true);
assert.deepEqual(focusEvents, ['focus']);

const checkbox = document.getElementById('check');
checkbox.addEventListener('click', (event) => event.preventDefault(), { once: true });
checkbox.click();
assert.equal(checkbox.checked, false);
checkbox.click();
assert.equal(checkbox.checked, true);
assert.equal(checkbox.hasAttribute('checked'), false);
assert.equal(document.querySelector(':checked'), checkbox);

let resizeEntries = null;
const resizeObserver = new ResizeObserver((entries) => { resizeEntries = entries; });
resizeObserver.observe(document.getElementById('box'));
await Promise.resolve();
await Promise.resolve();
assert.equal(resizeEntries.length, 1);
assert.equal(resizeEntries[0].contentRect.width, 100);

let intersectionEntries = null;
const intersectionObserver = new IntersectionObserver((entries) => { intersectionEntries = entries; });
intersectionObserver.observe(document.getElementById('box'));
await Promise.resolve();
await Promise.resolve();
assert.equal(intersectionEntries.length, 1);
assert.equal(intersectionEntries[0].isIntersecting, true);
assert.equal(intersectionEntries[0].intersectionRatio, 1);

const pointerEvents = [];
button.addEventListener('pointerdown', () => pointerEvents.push('pointerdown'));
button.addEventListener('mousedown', () => pointerEvents.push('mousedown'));
button.addEventListener('click', () => pointerEvents.push('click'));
__dispatchBrowserPointerEvent('pointerdown', { clientX: 12, clientY: 12, pointerId: 1 });
__dispatchBrowserPointerEvent('pointerup', { clientX: 12, clientY: 12, pointerId: 1 });
assert.deepEqual(pointerEvents, ['pointerdown', 'mousedown', 'click']);

const box = document.getElementById('box');
let lockedMoves = 0;
box.addEventListener('mousemove', () => { lockedMoves++; });
await box.requestPointerLock({ unadjustedMovement: true });
assert.equal(document.pointerLockElement, box);
assert.equal(pointerLocks, 1);
// The native cursor can remain over another node while locked; relative motion
// must still target the lock element.
__dispatchBrowserPointerEvent('pointermove', {
  clientX: 12, clientY: 12, movementX: 4, movementY: -2, pointerId: 1,
});
assert.equal(lockedMoves, 1);
box.exitPointerLock();
assert.equal(document.pointerLockElement, null);
assert.equal(pointerLocks, 0);

const ops = Deno.core.ops;
const range = document.createElement('input');
range.id = 'range';
range.type = 'range';
range.setAttribute('min', '0');
range.setAttribute('max', '10');
range.setAttribute('step', '0.5');
range.value = '0';
document.body.append(range);
ops.op_browser_hit_test = () => nodeByAttributeId('range').id;
const oldRect = ops.op_browser_rect;
ops.op_browser_rect = (id) => id === nodeByAttributeId('range').id
  ? { x: 0, y: 0, width: 112, height: 16 } : oldRect(id);
let rangeInputs = 0, rangeChanges = 0;
range.addEventListener('input', () => rangeInputs++);
range.addEventListener('change', () => rangeChanges++);
__dispatchBrowserPointerEvent('pointerdown', { pointerId: 5, clientX: 56, clientY: 8 });
assert.equal(range.value, '5');
assert.equal(rangeInputs, 1);
assert.equal(rangeChanges, 0);
__dispatchBrowserPointerEvent('pointermove', { pointerId: 5, clientX: 500, clientY: 100 });
__dispatchBrowserPointerEvent('pointerup', { pointerId: 5, clientX: 500, clientY: 100 });
assert.equal(range.value, '10');
assert.equal(rangeChanges, 1);
range.focus();
__dispatchBrowserKeyboardEvent('keydown', { key: 'ArrowLeft' });
assert.equal(range.value, '9.5');
__dispatchBrowserKeyboardEvent('keydown', { key: 'Home' });
assert.equal(range.value, '0');
range.disabled = true;
__dispatchBrowserPointerEvent('pointerdown', { pointerId: 6, clientX: 106, clientY: 8 });
__dispatchBrowserPointerEvent('pointerup', { pointerId: 6, clientX: 106, clientY: 8 });
assert.equal(range.value, '0');
range.disabled = false;
range.addEventListener('pointerdown', event => event.preventDefault(), { once: true });
__dispatchBrowserPointerEvent('pointerdown', { pointerId: 7, clientX: 106, clientY: 8 });
__dispatchBrowserPointerEvent('pointerup', { pointerId: 7, clientX: 106, clientY: 8 });
assert.equal(range.value, '0');

const scroller = document.createElement('div');
scroller.id = 'scroller';
const clippedButton = document.createElement('button');
clippedButton.id = 'clippedButton';
scroller.append(clippedButton);
document.body.append(scroller);
let scrollTop = 0;
const oldMetrics = ops.op_browser_box_metrics;
ops.op_browser_box_metrics = (id) => {
  const metrics = oldMetrics(id);
  if (id === nodeByAttributeId('scroller').id) return { ...metrics, clientHeight: 100, scrollHeight: 300, scrollTop };
  if (id === nodeByAttributeId('clippedButton').id) return { ...metrics, clientWidth: 20, scrollWidth: 100 };
  return metrics;
};
ops.op_browser_hit_test = () => nodeByAttributeId('clippedButton').id;
ops.op_browser_computed_property = (id, property) => property.startsWith('overflow-')
  ? id === nodeByAttributeId('scroller').id ? 'auto' : 'hidden' : '';
ops.op_browser_set_scroll = (id, _x, y) => {
  assert.equal(id, nodeByAttributeId('scroller').id, 'wheel input must skip clipped labels');
  scrollTop = Math.max(0, Math.min(200, y));
  return true;
};
__dispatchBrowserWheelEvent({ clientX: 1, clientY: 1, deltaY: 120, deltaMode: 0 });
assert.equal(scrollTop, 120);
clippedButton.addEventListener('wheel', event => event.preventDefault(), { once: true });
__dispatchBrowserWheelEvent({ clientX: 1, clientY: 1, deltaY: 120, deltaMode: 0 });
assert.equal(scrollTop, 120);

ops.op_browser_hit_test = () => nodeByAttributeId('button').id;
__syncBrowserDocument();
let syncsAfterPointerMutation = 0;
let mouseEvent;
button.addEventListener('pointermove', () => {
  button.setAttribute('data-pointer-mutation', 'changed');
  syncsAfterPointerMutation = operationLog.filter(op => op === 'sync').length;
}, { once: true });
button.addEventListener('mousemove', event => { mouseEvent = event; }, { once: true });
__dispatchBrowserPointerEvent('pointermove', { pointerId: 25, clientX: 35, clientY: 26 });
assert.equal(mouseEvent.clientX, 35);
assert.equal(operationLog.filter(op => op === 'sync').length, syncsAfterPointerMutation,
  'unused event offsets must not force layout after a pointer listener changes the DOM');
assert.equal(mouseEvent.offsetX, 25);
assert.equal(mouseEvent.offsetY, 16);
assert.equal(operationLog.filter(op => op === 'sync').length, syncsAfterPointerMutation + 1,
  'reading event offsets must use the current target geometry');

ops.op_browser_set_scroll = () => false;
const select = document.createElement('select');
select.innerHTML = '<option value="off">Off</option><option disabled value="bad">Disabled</option><optgroup disabled><option value="group">Group</option></optgroup><option value="on">On</option>';
document.body.append(select);
const nativeSelectedValues = () => {
  __syncBrowserDocument();
  return snapshot.nodes.filter(node => node.localName === 'option' && node.attributes.some(attr => attr.localName === 'selected'))
    .map(node => node.attributes.find(attr => attr.localName === 'value')?.value);
};
assert.deepEqual(nativeSelectedValues(), ['off'], 'the native label uses the first option without a selected attribute');
assert.equal(select.querySelector('[selected]'), null, 'native selectedness must not change source attributes');
const selectEvents = [];
select.addEventListener('input', () => selectEvents.push('input'));
select.addEventListener('change', () => selectEvents.push('change'));
select.focus();
__dispatchBrowserKeyboardEvent('keydown', { key: 'ArrowDown' });
assert.equal(select.value, 'on', 'keyboard selection skips disabled options and groups');
assert.deepEqual(nativeSelectedValues(), ['on'], 'the native label follows the new selection');
assert.deepEqual(selectEvents, ['input', 'change']);
select.addEventListener('keydown', event => event.preventDefault(), { once: true });
__dispatchBrowserKeyboardEvent('keydown', { key: 'Home' });
assert.equal(select.value, 'on');
select.click();
assert.ok(document.querySelector('[role=listbox]'));
__dispatchBrowserKeyboardEvent('keydown', { key: 'Home' });
assert.equal(select.value, 'on', 'popup navigation does not commit before Enter');
__dispatchBrowserKeyboardEvent('keydown', { key: 'Escape' });
assert.equal(document.querySelector('[role=listbox]'), null);
assert.equal(select.value, 'on');
__dispatchBrowserKeyboardEvent('keydown', { key: 'Enter' });
const rows = document.querySelectorAll('[role=option]');
rows[1].click();
assert.ok(document.querySelector('[role=listbox]'), 'disabled options cannot close or change the popup');
rows[0].click();
assert.equal(select.value, 'off');
assert.equal(document.querySelector('[role=listbox]'), null);
assert.equal(document.activeElement, select);
assert.deepEqual(selectEvents, ['input', 'change', 'input', 'change']);
select.click();
__dispatchBrowserKeyboardEvent('keydown', { key: 'End' });
__dispatchBrowserKeyboardEvent('keydown', { key: 'Enter' });
assert.equal(select.value, 'on');
assert.equal(document.querySelector('[role=listbox]'), null);
select.click();
__dispatchBrowserKeyboardEvent('keydown', { key: 'Tab' });
assert.equal(document.querySelector('[role=listbox]'), null);
select.disabled = true;
select.click();
assert.equal(document.querySelector('[role=listbox]'), null);
select.focus();
__dispatchBrowserKeyboardEvent('keydown', { key: 'Home' });
assert.equal(select.value, 'on');
select.remove();
const groupedSelect = document.createElement('select');
groupedSelect.innerHTML = '<optgroup label="Group"><option value="nested">Nested</option><option value="next">Next</option></optgroup>';
document.body.append(groupedSelect);
assert.deepEqual(nativeSelectedValues(), ['nested'], 'the default selection can be inside an optgroup');
groupedSelect.remove();

const eventParent = document.createElement('div');
const eventChild = document.createElement('button');
eventParent.append(eventChild);
document.body.append(eventParent);
const phaseOrder = [];
for (const capture of [false, true]) {
  for (const [name, target] of [['parent', eventParent], ['child', eventChild]]) {
    target.addEventListener('phase-check', function (event) {
      assert.equal(this, target);
      assert.equal(event.target, eventChild);
      phaseOrder.push([name, capture, event.eventPhase]);
    }, capture);
  }
}
eventChild.dispatchEvent(new Event('phase-check', { bubbles: true }));
assert.deepEqual(phaseOrder, [['parent', true, 1], ['child', true, 2], ['child', false, 2], ['parent', false, 3]]);
phaseOrder.length = 0;
eventChild.dispatchEvent(new Event('phase-check'));
assert.deepEqual(phaseOrder, [['parent', true, 1], ['child', true, 2], ['child', false, 2]]);
let calls = 0;
const sameListener = () => calls++;
eventChild.addEventListener('identity', sameListener, true);
eventChild.addEventListener('identity', sameListener, false);
eventChild.removeEventListener('identity', sameListener, true);
eventChild.dispatchEvent(new Event('identity'));
assert.equal(calls, 1);
const abort = new AbortController();
eventChild.addEventListener('abort-check', sameListener, { signal: abort.signal });
abort.abort();
eventChild.dispatchEvent(new Event('abort-check'));
assert.equal(calls, 1);
eventChild.addEventListener('passive-check', event => event.preventDefault(), { passive: true });
assert.equal(eventChild.dispatchEvent(new Event('passive-check', { cancelable: true })), true);
eventParent.remove();

const clickControl = document.createElement('button');
clickControl.id = 'pointer-defaults';
document.body.append(clickControl);
Deno.core.ops.op_browser_hit_test = () => nodeByAttributeId('pointer-defaults').id;
const pointerOrder = [];
clickControl.addEventListener('pointerdown', event => { event.preventDefault(); clickControl.setPointerCapture(event.pointerId); });
for (const type of ['mousedown', 'mousemove', 'mouseup', 'lostpointercapture', 'click', 'auxclick']) {
  clickControl.addEventListener(type, () => pointerOrder.push(type));
}
__dispatchBrowserPointerEvent('pointerdown', { pointerId: 5, button: 0 });
__dispatchBrowserPointerEvent('pointermove', { pointerId: 5 });
__dispatchBrowserPointerEvent('pointerup', { pointerId: 5, button: 0 });
assert.deepEqual(pointerOrder, ['lostpointercapture', 'click'], 'canceled pointerdown must retain click after capture release');
pointerOrder.length = 0;
__dispatchBrowserPointerEvent('pointerdown', { pointerId: 5, button: 2 });
__dispatchBrowserPointerEvent('pointerup', { pointerId: 5, button: 2 });
assert.deepEqual(pointerOrder, ['lostpointercapture', 'auxclick']);
clickControl.remove();

const numericValue = document.createElement('input');
numericValue.type = 'number';
for (const invalid of ['0x10', '+1', '1e999', 'NaN', ' 2', '1.']) {
  numericValue.value = invalid;
  assert.equal(numericValue.value, '', `invalid numeric assignment: ${invalid}`);
}
numericValue.value = '-1.25e2';
assert.equal(numericValue.value, '-1.25e2');
const text = document.createElement('input');
text.defaultValue = 'initial';
assert.equal(text.value, 'initial');
text.value = 'old';
text.defaultValue = 'later';
assert.equal(text.value, 'old');
assert.equal(text.getAttribute('value'), 'later', 'live edits must not change the default attribute');
document.body.append(text);
let textCalls = 0;
Deno.core.ops.op_browser_text_input = (_id, action) => {
  if (action.action === 'query') return { value: text.value, anchor: text.value.length, focus: text.value.length };
  textCalls++;
  assert.equal(action.action, 'key');
  return { value: 'new', anchor: 3, focus: 3 };
};
const textEvents = [];
for (const type of ['keydown', 'beforeinput', 'input', 'change']) text.addEventListener(type, (event) => {
  textEvents.push([type, event.inputType ?? null, text.value]);
});
text.focus();
text.addEventListener('keydown', event => event.preventDefault(), { once: true });
__dispatchBrowserKeyboardEvent('keydown', { key: 'a' });
assert.equal(textCalls, 0, 'keydown cancellation must stop native editing');
text.addEventListener('beforeinput', event => event.preventDefault(), { once: true });
__dispatchBrowserKeyboardEvent('keydown', { key: 'a' });
assert.equal(textCalls, 0, 'beforeinput cancellation must stop native editing');
textEvents.length = 0;
__dispatchBrowserKeyboardEvent('keydown', { key: 'a' });
assert.equal(text.value, 'new');
assert.deepEqual(textEvents, [['keydown', null, 'old'], ['beforeinput', 'insertText', 'old'], ['input', 'insertText', 'new']]);
text.blur();
assert.deepEqual(textEvents.at(-1), ['change', null, 'new']);
text.focus();
text.setAttribute('readonly', '');
__dispatchBrowserKeyboardEvent('keydown', { key: 'b' });
assert.equal(textCalls, 1, 'readonly must stop native editing');
text.removeAttribute('readonly');
text.addEventListener('beforeinput', () => text.remove(), { once: true });
__dispatchBrowserKeyboardEvent('keydown', { key: 'b' });
assert.equal(textCalls, 1, 'beforeinput removal must stop native editing');

const editable = document.createElement('textarea');
document.body.append(editable);
editable.value = 'a😀b';
editable.focus();
let editor = { value: editable.value, anchor: 1, focus: 3 };
let composeRange = null;
Deno.core.ops.op_browser_text_input = (_id, action) => {
  editor.value = editable.value;
  if (action.action === 'select') { editor.anchor = action.anchor; editor.focus = action.focus; }
  if (action.action === 'imeCancel' || action.action === 'imeCommit') {
    if (composeRange) {
      editor.value = editor.value.slice(0, composeRange[0]) + editor.value.slice(composeRange[1]);
      editor.anchor = editor.focus = composeRange[0];
      composeRange = null;
    }
  }
  if (action.action === 'insert' || action.action === 'imeCommit' || action.action === 'imePreedit') {
    const start = composeRange?.[0] ?? Math.min(editor.anchor, editor.focus), end = composeRange?.[1] ?? Math.max(editor.anchor, editor.focus);
    editor.value = editor.value.slice(0, start) + action.key + editor.value.slice(end);
    editor.anchor = editor.focus = start + action.key.length;
    if (action.action === 'imePreedit') composeRange = [start, editor.focus];
  }
  return { ...editor };
};
let clipboard = '', rejectClipboard = false, releaseClipboard = null;
Deno.core.ops.op_browser_clipboard = async (write, value) => {
  if (rejectClipboard) throw new Error('clipboard unavailable');
  if (releaseClipboard) await new Promise(resolve => { releaseClipboard = resolve; });
  if (write) clipboard = value;
  return write ? '' : clipboard;
};
const settleClipboard = async () => { for (let i = 0; i < 12; i++) await Promise.resolve(); };
__dispatchBrowserKeyboardEvent('keydown', { key: 'c', ctrlKey: true });
await settleClipboard();
assert.equal(clipboard, '😀');
assert.equal(editable.value, 'a😀b');
__dispatchBrowserKeyboardEvent('keydown', { key: 'x', ctrlKey: true });
await settleClipboard();
assert.equal(editable.value, 'ab');
__dispatchBrowserKeyboardEvent('keydown', { key: 'z', ctrlKey: true });
assert.equal(editable.value, 'a😀b');
assert.equal(editor.anchor, 1);
assert.equal(editor.focus, 3);
__dispatchBrowserKeyboardEvent('keydown', { key: 'z', ctrlKey: true, shiftKey: true });
assert.equal(editable.value, 'ab');
clipboard = 'é\r\n中';
__dispatchBrowserKeyboardEvent('keydown', { key: 'v', ctrlKey: true });
await settleClipboard();
assert.equal(editable.value, 'aé\n中b');
editable.addEventListener('beforeinput', event => event.preventDefault(), { once: true });
__dispatchBrowserKeyboardEvent('keydown', { key: 'z', ctrlKey: true });
assert.equal(editable.value, 'aé\n中b', 'canceled undo must retain the history cursor');
__dispatchBrowserKeyboardEvent('keydown', { key: 'z', ctrlKey: true });
assert.equal(editable.value, 'ab');
editable.value = 'script';
__dispatchBrowserKeyboardEvent('keydown', { key: 'z', ctrlKey: true });
assert.equal(editable.value, 'script', 'script assignment clears obsolete text history');
editor.anchor = 0; editor.focus = 6;
rejectClipboard = true;
const originalError = console.error;
let clipboardErrors = 0;
console.error = () => clipboardErrors++;
__dispatchBrowserKeyboardEvent('keydown', { key: 'x', ctrlKey: true });
await settleClipboard();
console.error = originalError;
assert.equal(editable.value, 'script', 'failed clipboard writes must not cut text');
assert.equal(clipboardErrors, 1);
rejectClipboard = false;
releaseClipboard = true;
__dispatchBrowserKeyboardEvent('keydown', { key: 'v', ctrlKey: true });
editable.value = 'changed';
releaseClipboard(); releaseClipboard = null;
await settleClipboard();
assert.equal(editable.value, 'changed', 'stale clipboard reads must not replace newer text');
await navigator.clipboard.writeText('script access');
assert.equal(await navigator.clipboard.readText(), 'script access');

editable.value = 'before';
editor.anchor = 0; editor.focus = 6;
const compositionEvents = [];
for (const type of ['compositionstart', 'compositionupdate', 'compositionend']) editable.addEventListener(type, event => compositionEvents.push([type, event.data]));
__dispatchBrowserImeEvent({ phase: 'preedit', data: 'に', cursor: [3, 3] });
assert.equal(editable.value, 'に');
let composingKey = null;
editable.addEventListener('keydown', event => composingKey = event.isComposing, { once: true });
__dispatchBrowserKeyboardEvent('keydown', { key: 'Enter' });
assert.equal(composingKey, true);
assert.equal(editable.value, 'に');
__dispatchBrowserImeEvent({ phase: 'preedit', data: '日本', cursor: [6, 6] });
__dispatchBrowserImeEvent({ phase: 'commit', data: '日本語' });
assert.equal(editable.value, '日本語');
assert.deepEqual(compositionEvents, [['compositionstart', ''], ['compositionupdate', 'に'], ['compositionupdate', '日本'], ['compositionend', '日本語']]);
__dispatchBrowserKeyboardEvent('keydown', { key: 'z', ctrlKey: true });
assert.equal(editable.value, 'before', 'one undo must remove the complete composition');
assert.equal(editor.anchor, 0);
assert.equal(editor.focus, 6);
__dispatchBrowserImeEvent({ phase: 'preedit', data: '中', cursor: [3, 3] });
editable.blur();
assert.equal(editable.value, 'before', 'focus loss must cancel uncommitted composition');
editable.focus();
editable.setAttribute('readonly', '');
__dispatchBrowserImeEvent({ phase: 'preedit', data: 'x', cursor: [1, 1] });
assert.equal(editable.value, 'before');
editable.removeAttribute('readonly');
__dispatchBrowserImeEvent({ phase: 'preedit', data: '中', cursor: [3, 3] });
editable.value = 'new script value';
editable.blur();
assert.equal(editable.value, 'new script value', 'composition cancellation must not restore obsolete script text');
editable.focus();
editable.type = 'password';
clipboard = 'password paste';
editor.anchor = 0; editor.focus = editable.value.length;
__dispatchBrowserKeyboardEvent('keydown', { key: 'v', ctrlKey: true });
await settleClipboard();
assert.equal(editable.value, 'password paste', 'password controls must permit paste');
editable.type = 'text';

const samples = Array.from({ length: 64 }, (_, i) => ({ clientX: i, clientY: i * 2, timeStamp: 10 + i }));
let retained = null;
button.addEventListener('pointermove', event => retained = event.getCoalescedEvents(), { once: true });
Deno.core.ops.op_browser_hit_test = () => nodeByAttributeId('button').id;
__dispatchBrowserPointerEvent('pointermove', { pointerId: 1, clientX: 63, clientY: 126, timeStamp: 73, coalescedEvents: samples });
assert.equal(retained.length, 64);
assert.deepEqual(retained.map(event => [event.clientX, event.clientY, event.timeStamp]), samples.map(sample => [sample.clientX, sample.clientY, sample.timeStamp]));
assert.equal(new PointerEvent('pointermove').getCoalescedEvents().length, 0);
let canceledOnBlur = 0;
button.addEventListener('pointercancel', () => canceledOnBlur++);
button.addEventListener('pointerdown', event => button.setPointerCapture(event.pointerId), { once: true });
__dispatchBrowserPointerEvent('pointerdown', { pointerId: 1 });
globalThis.dispatchEvent(new Event('blur'));
assert.equal(canceledOnBlur, 1);
assert.equal(button.hasPointerCapture(1), false);

console.log('dom_setup API tests passed');
