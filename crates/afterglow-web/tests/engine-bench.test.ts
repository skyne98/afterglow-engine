// Benchmarks for the render adapter hot paths.
//
//   node --test crates/afterglow-web/tests/engine-bench.test.mjs
//
// Measures:
// 1. composeTransformInto — the batched raw math matrix compose
// 2. multiplyMatricesInto — hierarchy parent × child
// 3. EntityDirtyQueue — mark + clear
// 4. DirtySlotRanges — mark + flush (coalesced upload ranges)
// 5. InstanceShard — allocate + remove (swap-remove)
// 6. Full syncTransforms — the end-to-end per-frame cost

import { test } from 'node:test';
import assert from 'node:assert/strict';

import { createTransformStore } from '../www/engine/components.js';
import { composeTransformInto, multiplyMatricesInto } from '../www/engine/matrix.js';
import { EntityDirtyQueue } from '../www/engine/dirty-queue.js';
import { DirtySlotRanges } from '../www/engine/dirty-ranges.js';
import { RenderDirty, NULL_ENTITY } from '../www/engine/types.js';
import { createTransformStore } from '../www/engine/components.js';

// --- 1b. Inlined gather → branch-free compute (what the render adapter does) ---

// --- 1b. Inlined branch-free compute (render adapter approach) ---
// The dirty queue already provides a compact dirty list — no gather scan needed.

function benchInlinedCompute(count) {
  const transform = createTransformStore(count);
  const out = new Float32Array(count * 16);

  // Pre-build dirty list (what EntityDirtyQueue provides)
  const dirtyCount = Math.floor(count * 0.1);
  const dirtyIds = new Uint32Array(dirtyCount);
  for (let i = 0; i < dirtyCount; i++) dirtyIds[i] = Math.floor(i * (count / dirtyCount));

  return measure(() => {
    // Compute (branch-free, inlined — no function call, no gather scan)
    for (let j = 0; j < dirtyCount; j++) {
      const i = dirtyIds[j];
      const qx = transform.rotationX[i], qy = transform.rotationY[i], qz = transform.rotationZ[i], qw = transform.rotationW[i];
      const x2 = qx + qx, y2 = qy + qy, z2 = qz + qz;
      const xx = qx * x2, xy = qx * y2, xz = qx * z2;
      const yy = qy * y2, yz = qy * z2, zz = qz * z2;
      const wx = qw * x2, wy = qw * y2, wz = qw * z2;
      const sx = transform.scaleX[i], sy = transform.scaleY[i], sz = transform.scaleZ[i];
      const off = i * 16;
      out[off]      = (1 - (yy + zz)) * sx;
      out[off + 1]  = (xy + wz) * sx;
      out[off + 2]  = (xz - wy) * sx;
      out[off + 3]  = 0;
      out[off + 4]  = (xy - wz) * sy;
      out[off + 5]  = (1 - (xx + zz)) * sy;
      out[off + 6]  = (yz + wx) * sy;
      out[off + 7]  = 0;
      out[off + 8]  = (xz + wy) * sz;
      out[off + 9]  = (yz - wx) * sz;
      out[off + 10] = (1 - (xx + yy)) * sz;
      out[off + 11] = 0;
      out[off + 12] = transform.positionX[i];
      out[off + 13] = transform.positionY[i];
      out[off + 14] = transform.positionZ[i];
      out[off + 15] = 1;
    }
  });
}

function measure(fn, iterations = 500) {
  // Warm up
  for (let i = 0; i < 50; i++) fn();
  const times = [];
  for (let i = 0; i < iterations; i++) {
    const t0 = performance.now();
    fn();
    times.push(performance.now() - t0);
  }
  times.sort((a, b) => a - b);
  return {
    median: times[Math.floor(times.length / 2)],
    p99: times[Math.floor(times.length * 0.99)],
    mean: times.reduce((s, t) => s + t, 0) / times.length,
    min: times[0],
  };
}

function print(label, r, count) {
  const pct = (r.median / 16.67 * 100).toFixed(1);
  const per = (r.median * 1000 / count).toFixed(3);
  console.log(`  ${label.padEnd(35)} median ${r.median.toFixed(3)} ms  (${per} µs/ent)  p99 ${r.p99.toFixed(3)} ms  ${pct}% frame`);
}

// --- 1. composeTransformInto ---

function benchCompose(count) {
  const transform = createTransformStore(count);
  const out = new Float32Array(count * 16);

  // Initialize with random values
  for (let i = 0; i < count; i++) {
    transform.positionX[i] = Math.random() * 1000;
    transform.positionY[i] = Math.random() * 1000;
    transform.positionZ[i] = Math.random() * 1000;
    transform.rotationX[i] = Math.random();
    transform.rotationY[i] = Math.random();
    transform.rotationZ[i] = Math.random();
    transform.rotationW[i] = Math.random();
  }

  // Only compose dirty entities (simulate 10% dirty)
  const dirtyCount = Math.floor(count * 0.1);
  const dirty = new Uint32Array(dirtyCount);
  for (let i = 0; i < dirtyCount; i++) dirty[i] = i;

  return measure(() => {
    for (let i = 0; i < dirty.length; i++) {
      const eid = dirty[i];
      composeTransformInto(out, eid * 16, transform, eid);
    }
  });
}

// --- 2. multiplyMatricesInto ---

function benchMultiply(count) {
  const a = new Float32Array(count * 16);
  const b = new Float32Array(count * 16);
  const out = new Float32Array(count * 16);

  for (let i = 0; i < count * 16; i++) {
    a[i] = Math.random();
    b[i] = Math.random();
  }

  return measure(() => {
    for (let i = 0; i < count; i++) {
      multiplyMatricesInto(out, i * 16, a, i * 16, b, i * 16);
    }
  });
}

// --- 3. EntityDirtyQueue ---

function benchDirtyQueue(count) {
  const queue = new EntityDirtyQueue(count + 100);

  return measure(() => {
    for (let i = 0; i < count; i++) {
      queue.mark(i, RenderDirty.Transform);
    }
    queue.clear();
  });
}

// --- 4. DirtySlotRanges ---

function benchDirtyRanges(count, dirtyRatio) {
  const ranges = new DirtySlotRanges(count, 16);
  const dirtyCount = Math.floor(count * dirtyRatio);

  // Simulate a mock attribute with updateRanges
  const mockAttr = {
    needsUpdate: false,
    updateRanges: [],
  };

  return measure(() => {
    // Mark dirty slots (scattered, not contiguous)
    for (let i = 0; i < dirtyCount; i++) {
      ranges.mark(Math.floor(i * (count / dirtyCount)));
    }
    ranges.flush(mockAttr, 16, count);
  });
}

// --- 5. InstanceShard allocate/remove ---

// This needs Three.js (for InstancedMesh), so we skip it in node and
// benchmark it only in the browser demo. But we can benchmark the
// swap-remove logic separately.

function benchSwapRemove(count) {
  // Simulate the swap-remove: copyWithin + bookkeeping
  const slotToEntity = new Uint32Array(count);
  const entityToSlot = new Uint32Array(count);
  const matrixData = new Float32Array(count * 16);

  for (let i = 0; i < count; i++) {
    slotToEntity[i] = i;
    entityToSlot[i] = i;
  }

  // Remove random slots
  const removals = Math.floor(count * 0.1);
  const toRemove = new Uint32Array(removals);
  for (let i = 0; i < removals; i++) {
    toRemove[i] = Math.floor(Math.random() * count);
  }

  return measure(() => {
    let currentCount = count;
    for (let i = 0; i < removals; i++) {
      const slot = toRemove[i] % currentCount;
      const lastSlot = currentCount - 1;
      const movedEntity = slotToEntity[lastSlot];
      currentCount = lastSlot;
      if (slot !== lastSlot) {
        matrixData.copyWithin(slot * 16, lastSlot * 16, lastSlot * 16 + 16);
        slotToEntity[slot] = movedEntity;
        entityToSlot[movedEntity] = slot;
      }
    }
  });
}

// --- Run all benchmarks ---

console.log('=== Render Adapter Hot Path Benchmarks ===\n');

console.log('1. composeTransformInto (function call, 10% dirty, batched)');
for (const c of [10_000, 50_000, 100_000, 500_000, 1_000_000]) {
  print(`${c.toLocaleString()} entities`, benchCompose(c), Math.floor(c * 0.1));
}
console.log();

console.log('1b. Inlined branch-free compute (render adapter — no gather scan)');
for (const c of [10_000, 50_000, 100_000, 500_000, 1_000_000]) {
  print(`${c.toLocaleString()} entities`, benchInlinedCompute(c), Math.floor(c * 0.1));
}
console.log();

console.log('2. multiplyMatricesInto (hierarchy)');
for (const c of [1_000, 10_000, 50_000]) {
  print(`${c.toLocaleString()} children`, benchMultiply(c), c);
}
console.log();

console.log('3. EntityDirtyQueue (mark + clear)');
for (const c of [10_000, 50_000, 100_000]) {
  print(`${c.toLocaleString()} entities`, benchDirtyQueue(c), c);
}
console.log();

console.log('4. DirtySlotRanges (mark + flush, 10% dirty)');
for (const c of [10_000, 50_000, 100_000]) {
  print(`${c.toLocaleString()} slots`, benchDirtyRanges(c, 0.1), Math.floor(c * 0.1));
}
console.log();

console.log('5. Swap-remove (10% removals)');
for (const c of [10_000, 50_000, 100_000]) {
  print(`${c.toLocaleString()} slots`, benchSwapRemove(c), Math.floor(c * 0.1));
}
console.log();

// Verify correctness
test('composeTransformInto produces identity for unit quaternion + unit scale', () => {
  const t = createTransformStore(1);
  const out = new Float32Array(16);
  composeTransformInto(out, 0, t, 0);
  // Identity matrix check
  assert.equal(out[0], 1);
  assert.equal(out[5], 1);
  assert.equal(out[10], 1);
  assert.equal(out[15], 1);
  assert.equal(out[12], 0); // position X = 0
});

test('multiplyMatricesInto: identity × identity = identity', () => {
  const identity = new Float32Array(16);
  identity[0] = identity[5] = identity[10] = identity[15] = 1;
  const out = new Float32Array(16);
  multiplyMatricesInto(out, 0, identity, 0, identity, 0);
  assert.equal(out[0], 1);
  assert.equal(out[5], 1);
  assert.equal(out[10], 1);
  assert.equal(out[15], 1);
});

test('EntityDirtyQueue dedupes and clears', () => {
  const q = new EntityDirtyQueue(100);
  q.mark(5, RenderDirty.Transform);
  q.mark(5, RenderDirty.Appearance); // deduped
  assert.equal(q.count, 1);
  assert.equal(q.flags[5], RenderDirty.Transform | RenderDirty.Appearance);
  q.clear();
  assert.equal(q.count, 0);
  assert.equal(q.flags[5], 0);
});
