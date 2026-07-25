import { PhysicsClient } from "../../workers/physics.client.ts";

const output = document.getElementById("out");
function print(message: string): void {
  if (output) output.textContent += `${message}\n`;
}
print("=== Typed Cross-Thread Worker Benchmark (Web Worker + SAB ring) ===");
print(
  "Service RPC includes typed postcard encode/decode and cross-thread notification.\n",
);
const client = await PhysicsClient.spawn({
  workerWasmUrl: "physics_worker.wasm",
});
try {
  const sizes = new Uint32Array([1, 4, 16, 64, 256, 1024, 4096, 16384]);
  const iterations = 1000,
    dt = 0.016,
    roundedDt = Math.fround(dt);
  const warmup = new Float32Array(64);
  for (let index = 0; index < warmup.length; index++) warmup[index] = index;
  for (let index = 0; index < 100; index++) await client.step(warmup, dt);
  print("  f32 count   payload   latency    bandwidth    valid/total");
  for (const count of sizes) {
    const input = new Float32Array(count);
    for (let index = 0; index < count; index++) input[index] = index;
    let elapsed = 0,
      valid = 0;
    for (let iteration = 0; iteration < iterations; iteration++) {
      const started = performance.now();
      const result = await client.step(input, dt);
      elapsed += performance.now() - started;
      if (
        result.length === count &&
        result.every(
          (value, index) =>
            Math.abs(value - Math.fround((input[index] ?? 0) + roundedDt)) <
            1e-6,
        )
      )
        valid++;
    }
    const latency = (elapsed * 1000) / iterations;
    const bandwidth = (count * 4 * iterations * 2) / (elapsed / 1000) / 1048576;
    print(
      `  ${String(count).padStart(9)}  ${String(count * 4).padStart(6)} B  ${latency.toFixed(1).padStart(7)} µs  ${bandwidth.toFixed(1).padStart(8)} MiB/s  ${valid}/${iterations} ${valid === iterations ? "OK" : "PARTIAL"}`,
    );
  }
  print("\nDone.");
} finally {
  client.close();
}
