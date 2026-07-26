import assert from 'node:assert/strict';

let snapshot = null;
let focused = 0;
let pointerLocks = 0;
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
  <div id="box"></div>
</body></html>`;
globalThis.__exampleURL = 'file:///tmp/test.html';
globalThis.__viewportWidth = 800;
globalThis.__viewportHeight = 500;
globalThis.Deno = { core: { ops: {
  op_probe_log() {},
  op_sync_browser_document(_epoch, value) { snapshot = value; },
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
} } };

await import('../dom_setup.ts');

assert.equal(matchMedia('(min-width: 800px)').matches, true);
assert.equal(matchMedia('(max-width: 20px)').matches, false);
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

console.log('dom_setup API tests passed');
