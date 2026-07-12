// Node test: loads the afterglow-texture WASM module, runs tests + benchmarks.
//
//   node --test crates/afterglow-texture/tests/wasm.test.mjs

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const testDir = fileURLToPath(new URL('.', import.meta.url));
const wasmPath = testDir + '../../../target/wasm32-unknown-unknown/wasm-dev/afterglow_texture.wasm';

async function loadWasm() {
  const wasmBytes = readFileSync(wasmPath);
  const memory = new WebAssembly.Memory({ shared: true, initial: 256, maximum: 1024 });
  const { instance } = await WebAssembly.instantiate(wasmBytes, {
    env: {
      memory,
      performance_now: () => performance.now(),
      notify_worker: () => {},
    },
  });
  return instance.exports;
}

test('WASM: all texture operations work', async () => {
  const exports = await loadWasm();
  const passed = exports.afterglow_texture_test();
  assert.ok(passed >= 9, `expected ≥9 checks passed, got ${passed}`);
  console.log(`  ✅ ${passed} texture checks passed in WASM`);
});

test('WASM: benchmark mip generation (256×256)', async () => {
  const exports = await loadWasm();
  const timeUs = exports.afterglow_texture_bench_mips();
  const count = exports.bench_mips_count();
  console.log(`  generate_mips: ${count} levels for 256×256 in ${timeUs.toFixed(0)}µs`);
  assert.ok(timeUs > 0, 'time should be positive');
  assert.ok(count >= 9, '256×256 should have ≥9 mip levels');
});
