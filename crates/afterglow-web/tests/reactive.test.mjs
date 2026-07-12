// Tests for the reactive ref system.
//
//   node --test crates/afterglow-web/tests/reactive.test.mjs

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { execSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const testDir = fileURLToPath(new URL('.', import.meta.url));
const crateDir = testDir + '../';

function transpile(tsRelPath) {
  const srcPath = crateDir + tsRelPath;
  const outPath = `/tmp/${tsRelPath.split('/').pop().replace('.ts', '.test.js')}`;
  execSync(`bun build ${srcPath} --outfile ${outPath} --target browser`, { stdio: 'pipe' });
  const src = readFileSync(outPath, 'utf8');
  return import('data:text/javascript;base64,' + Buffer.from(src, 'utf8').toString('base64'));
}

// --- tests ---

test('ref: get/set value', async () => {
  const { ref } = await transpile('www/engine/reactive.ts');
  const count = ref(0);
  assert.equal(count.value, 0);
  count.value = 42;
  assert.equal(count.value, 42);
});

test('ref: same value does not trigger', async () => {
  const { ref, effect, pendingEffectCount, flushEffects, clearEffects } = await transpile('www/engine/reactive.ts');
  const val = ref(1);
  let runs = 0;
  effect(() => { val.value; runs++; });
  assert.equal(runs, 1); // initial run
  clearEffects();
  val.value = 1; // same value — no trigger
  assert.equal(pendingEffectCount(), 0);
});

test('ref: trigger() forces effects even if value unchanged', async () => {
  const { ref, effect, flushEffects, pendingEffectCount } = await transpile('www/engine/reactive.ts');
  const obj = ref({ x: 1 });
  let runs = 0;
  let lastX = 0;
  effect(() => { lastX = obj.value.x; runs++; });
  assert.equal(runs, 1);
  // Deep mutation — .value unchanged (same object ref), so no auto-trigger.
  obj.value.x = 2;
  assert.equal(pendingEffectCount(), 0);
  // Force trigger.
  obj.trigger();
  assert.equal(pendingEffectCount(), 1);
  flushEffects();
  assert.equal(runs, 2);
  assert.equal(lastX, 2);
});

test('effect: runs immediately and re-runs on dependency change', async () => {
  const { ref, effect, flushEffects } = await transpile('www/engine/reactive.ts');
  const a = ref(1);
  const b = ref(2);
  let sum = 0;
  let runs = 0;

  effect(() => { sum = a.value + b.value; runs++; });
  assert.equal(runs, 1);
  assert.equal(sum, 3);

  a.value = 10;
  flushEffects();
  assert.equal(runs, 2);
  assert.equal(sum, 12);

  b.value = 20;
  flushEffects();
  assert.equal(runs, 3);
  assert.equal(sum, 30);
});

test('effect: only re-runs when tracked deps change (not untracked refs)', async () => {
  const { ref, effect, flushEffects } = await transpile('www/engine/reactive.ts');
  const tracked = ref(0);
  const untracked = ref(0);
  let runs = 0;

  effect(() => { tracked.value; runs++; }); // only reads `tracked`
  assert.equal(runs, 1);

  untracked.value = 99; // not tracked — should not queue
  flushEffects();
  assert.equal(runs, 1);

  tracked.value = 1; // tracked — should queue
  flushEffects();
  assert.equal(runs, 2);
});

test('effect: cleanup stops re-runs', async () => {
  const { ref, effect, flushEffects } = await transpile('www/engine/reactive.ts');
  const val = ref(0);
  let runs = 0;
  const cleanup = effect(() => { val.value; runs++; });
  assert.equal(runs, 1);

  val.value = 1;
  flushEffects();
  assert.equal(runs, 2);

  cleanup();

  val.value = 2;
  flushEffects();
  assert.equal(runs, 2); // no re-run after cleanup
});

test('effect: multiple effects on same ref all re-run', async () => {
  const { ref, effect, flushEffects } = await transpile('www/engine/reactive.ts');
  const val = ref(0);
  let runs1 = 0, runs2 = 0;

  effect(() => { val.value; runs1++; });
  effect(() => { val.value; runs2++; });
  assert.equal(runs1, 1);
  assert.equal(runs2, 1);

  val.value = 1;
  flushEffects();
  assert.equal(runs1, 2);
  assert.equal(runs2, 2);
});

test('effect: deduplication — same effect queued once', async () => {
  const { ref, effect, flushEffects, pendingEffectCount } = await transpile('www/engine/reactive.ts');
  const a = ref(0);
  const b = ref(0);
  let runs = 0;

  effect(() => { a.value; b.value; runs++; });
  assert.equal(runs, 1);

  a.value = 1;
  b.value = 2;
  // Both writes queue the same effect — should be deduplicated.
  assert.equal(pendingEffectCount(), 1);
  flushEffects();
  assert.equal(runs, 2); // runs once, not twice
});

test('computed: lazy evaluation and dependency tracking', async () => {
  const { ref, computed } = await transpile('www/engine/reactive.ts');
  const a = ref(2);
  const b = ref(3);
  const sum = computed(() => a.value + b.value);

  assert.equal(sum.value, 5);

  a.value = 10;
  assert.equal(sum.value, 13);

  b.value = 20;
  assert.equal(sum.value, 30);
});

test('computed: progressive LOD — picks highest available', async () => {
  const { ref, computed, flushEffects } = await transpile('www/engine/reactive.ts');
  const lod0 = ref('fallback');
  const lod1 = ref(null);
  const lod2 = ref(null);

  const best = computed(() => lod2.value ?? lod1.value ?? lod0.value);
  assert.equal(best.value, 'fallback');

  lod1.value = 'medium';
  flushEffects();
  assert.equal(best.value, 'medium');

  lod2.value = 'high';
  flushEffects();
  assert.equal(best.value, 'high');

  // Higher LOD removed — falls back.
  lod2.value = null;
  flushEffects();
  assert.equal(best.value, 'medium');
});

test('flushEffects: batches and deduplicates', async () => {
  const { ref, effect, flushEffects, pendingEffectCount, clearEffects } = await transpile('www/engine/reactive.ts');
  const val = ref(0);
  let runs = 0;

  effect(() => { val.value; runs++; });
  clearEffects(); // clear initial run's queue (none)
  assert.equal(runs, 1);

  val.value = 1;
  val.value = 2;
  val.value = 3;
  assert.equal(pendingEffectCount(), 1); // deduplicated

  flushEffects();
  assert.equal(runs, 2);
  assert.equal(pendingEffectCount(), 0);
});

test('flushEffects: cascading effects (effect A queues effect B)', async () => {
  const { ref, effect, flushEffects } = await transpile('www/engine/reactive.ts');
  const trigger = ref(0);
  const downstream = ref(0);
  let downstreamRuns = 0;

  // Effect A: writes to `downstream` when `trigger` changes.
  effect(() => {
    trigger.value;
    downstream.value = trigger.value * 10;
  });

  // Effect B: reads `downstream`.
  effect(() => {
    downstream.value;
    downstreamRuns++;
  });

  const initialRuns = downstreamRuns;

  trigger.value = 5;
  flushEffects(); // should run A, which queues B, which also runs
  assert.ok(downstreamRuns > initialRuns, 'cascading effect ran');
  assert.equal(downstream.value, 50);
});

test('infinite loop protection: effect that writes to its own dep', async () => {
  const { ref, effect, flushEffects } = await transpile('www/engine/reactive.ts');
  const val = ref(0);
  let runs = 0;

  effect(() => {
    val.value; // reads
    val.value = val.value + 1; // writes — re-queues itself
    runs++;
  });
  // The effect runs once initially. Then flushEffects should detect the
  // self-requeue and stop after the iteration limit.
  flushEffects();
  // It should terminate (not hang), and runs should be bounded.
  assert.ok(runs < 2000, `runs=${runs} — terminated within limits`);
});

test('dispose: ref.dispose() removes tracked dependencies', async () => {
  const { ref, effect, flushEffects } = await transpile('www/engine/reactive.ts');
  const val = ref(0);
  let runs = 0;

  const cleanup = effect(() => { val.value; runs++; });
  assert.equal(runs, 1);

  val.value = 1;
  flushEffects();
  assert.equal(runs, 2);

  // Dispose the ref — removes all tracked deps.
  val.dispose();
  val.value = 2;
  flushEffects();
  assert.equal(runs, 2); // no more runs after dispose

  // Cleanup the effect too (no-op after dispose, but shouldn't crash).
  cleanup();
});

test('DX: asset swap pattern — ref + effect + flush', async () => {
  const { ref, effect, flushEffects } = await transpile('www/engine/reactive.ts');

  // Simulate: texture starts null, effect swaps it into material when loaded.
  const texture = ref(null);
  const material = { map: null };
  let swapCount = 0;

  effect(() => {
    if (texture.value) {
      material.map = texture.value;
      swapCount++;
    }
  });

  assert.equal(material.map, null);
  assert.equal(swapCount, 0);

  // Asset loads — set the ref.
  texture.value = { uuid: 'tex1' };
  // Effect hasn't run yet (queued, not flushed).
  assert.equal(material.map, null);
  assert.equal(swapCount, 0);

  // Flush — material swaps here, between frames.
  flushEffects();
  assert.equal(material.map.uuid, 'tex1');
  assert.equal(swapCount, 1);

  // Progressive LOD: higher quality texture streams in.
  texture.value = { uuid: 'tex1_hq' };
  flushEffects();
  assert.equal(material.map.uuid, 'tex1_hq');
  assert.equal(swapCount, 2);
});
