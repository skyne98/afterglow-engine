import { PhysicsClient } from './physics.client.ts';

const output = document.getElementById('out');
function log(message: string): void { if (output) output.textContent += `${message}\n`; }
function finish(ok: boolean, message: string): void {
  document.title = ok ? 'PASS' : 'FAIL';
  log(`${ok ? 'PASS' : 'FAIL'}: ${message}`);
}
let client: PhysicsClient | null = null;
try {
  log('=== afterglow typed worker round-trip test (Physics.step) ===');
  client = await PhysicsClient.spawn({ workerWasmUrl: 'physics_worker.wasm' });
  const result = await client.step(new Float32Array([0, 1, 2]), 0.5);
  log(`Got [${Array.from(result, value => value.toFixed(3)).join(', ')}]`);
  const ok = result.length === 3 && result.every((value, index) =>
    Math.abs(value - (index + 0.5)) < 1e-6);
  finish(ok, 'Physics.step([0,1,2], 0.5) == [0.5,1.5,2.5]');
} catch (error) {
  log(error instanceof Error ? error.stack ?? error.message : String(error));
  finish(false, error instanceof Error ? error.message : String(error));
} finally {
  client?.close();
}
