// Node test: loads the afterglow-meshopt WASM module, runs all tests,
// and benchmarks simplify + encode/decode speed + effectiveness.
//
//   node --test crates/afterglow-meshopt/tests/wasm.test.mjs

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const testDir = fileURLToPath(new URL('.', import.meta.url));
const wasmPath = testDir + '../../../target/wasm32-unknown-unknown/wasm-dev/afterglow_meshopt.wasm';

async function loadWasm() {
  const wasmBytes = readFileSync(wasmPath);
  const memory = new WebAssembly.Memory({ shared: true, initial: 256, maximum: 1024 });
  const { instance } = await WebAssembly.instantiate(wasmBytes, {
    env: {
      memory,
      performance_now: () => performance.now(),
    },
  });
  return instance.exports;
}

test('WASM: all meshopt functions work (21 checks)', async () => {
  const exports = await loadWasm();
  const passed = exports.afterglow_meshopt_test();
  assert.ok(passed >= 19, `expected ≥19 checks passed, got ${passed}`);
  console.log(`  ✅ ${passed} meshopt checks passed in WASM`);
});

test('WASM: benchmark simplify (100×100 grid, ~19.6K triangles → ~4.9K)', async () => {
  const exports = await loadWasm();
  const timeUs = exports.afterglow_meshopt_bench_simplify();
  const orig = exports.bench_simplify_orig();
  const result = exports.bench_simplify_result();
  const error = exports.bench_simplify_error();

  console.log(`  simplify: ${orig} → ${result} indices in ${timeUs.toFixed(0)}µs (error: ${error.toFixed(4)})`);
  console.log(`  reduction: ${((1 - result / orig) * 100).toFixed(1)}% triangles removed`);

  assert.ok(timeUs > 0, 'time should be positive');
  assert.ok(result < orig, 'simplified should have fewer indices');
  assert.ok(error < 1.0, 'error should be small');
});

test('WASM: benchmark encode/decode index buffer', async () => {
  const exports = await loadWasm();
  const timeUs = exports.afterglow_meshopt_bench_encode();
  const origBytes = exports.bench_encode_orig_bytes();
  const compressedBytes = exports.bench_encode_compressed_bytes();

  const ratio = compressedBytes / origBytes;
  console.log(`  encode+decode: ${origBytes} → ${compressedBytes} bytes in ${timeUs.toFixed(0)}µs`);
  console.log(`  compression: ${(ratio * 100).toFixed(1)}% of original (${(1 - ratio).toFixed(1).replace(/^/, '0')}x smaller)`);

  assert.ok(timeUs > 0, 'time should be positive');
  assert.ok(compressedBytes < origBytes, 'compressed should be smaller');
  assert.ok(ratio < 1.0, 'compression ratio should be < 1.0');
});
