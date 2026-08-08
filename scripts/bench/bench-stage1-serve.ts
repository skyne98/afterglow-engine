// Stage-1 (serve) bandwidth benchmark.
//
// Measures the disk→page-byte path using the engine's OWN client code:
//   - createFetchRangeLoader  (the real V8 fetch+Range client used on CEF + web)
//   - readBigHeader           (the real bounded header read + postcard parse)
//   - findVTPageChunk         (the real page-offset resolver)
//
// Serving backend: the engine's own `xtask serve` (crates/afterglow-web
// examples/coep_server.rs = DevAssetServer from dev_server.rs), which is
// pread-backed, single-range, COOP/COEP. This is the public-web fallback path;
// the CEF native `afterglow://` scheme path bottoms out at the same pread and
// cannot be driven outside a running CEF process, but the V8 client code
// (createFetchRangeLoader) is identical on both targets.
//
// Run:
//   nix-shell -p deno --run "deno run --allow-net --allow-read scripts/bench/bench-stage1-serve.ts"
// (server must already be up: `cargo run -p xtask -- serve`)

import {
  createFetchRangeLoader,
  readBigHeader,
} from '../../crates/afterglow-web/web/src/engine/assets/asset-range.ts';
import { findVTPageChunk } from '../../crates/afterglow-web/web/src/engine/assets/big-format.ts';

const BASE_URL = process.env.BENCH_BASE_URL ?? 'http://127.0.0.1:8787/';
const CONTAINER = process.env.BENCH_CONTAINER ?? 'dungeon.big';
const ASSET = process.env.BENCH_ASSET ?? 'Rock064_Color.png';
const READS_PER_SWEEP = Number(process.env.BENCH_READS ?? 4096);
const CONCURRENCIES = [1, 2, 4, 8, 16, 32, 64];

interface PageLoc { offset: number; size: number; }

function pct(sorted: number[], q: number): number {
  if (sorted.length === 0) return 0;
  const i = Math.min(sorted.length - 1, Math.floor(sorted.length * q));
  return sorted[i];
}

function fmtBytes(b: number): string {
  if (b >= 1 << 20) return (b / (1 << 20)).toFixed(1) + ' MiB';
  if (b >= 1 << 10) return (b / (1 << 10)).toFixed(1) + ' KiB';
  return b + ' B';
}

async function concurrent(
  tasks: (() => Promise<void>)[],
  concurrency: number,
): Promise<void> {
  let next = 0;
  const workers = Array.from({ length: concurrency }, async () => {
    while (true) {
      const i = next++;
      if (i >= tasks.length) return;
      await tasks[i]();
    }
  });
  await Promise.all(workers);
}

async function main() {
  const source = createFetchRangeLoader(BASE_URL);
  console.log(`reading header: ${BASE_URL}${CONTAINER}`);
  const header = await readBigHeader(source, CONTAINER, 2 * 1024 * 1024);
  console.log(
    `  v${header.version} dataOffset=${header.dataOffset} assets=${header.assets.length}`,
  );

  // Enumerate every page of one VT asset across all mips + tail.
  const pages: PageLoc[] = [];
  let maxMip = 0;
  for (const a of header.assets) {
    if (a.name !== ASSET || !a.virtualTexture) continue;
    for (const m of a.virtualTexture.mips) {
      maxMip = Math.max(maxMip, m.mip);
      for (let y = 0; y < m.pagesY; y++) {
        for (let x = 0; x < m.pagesX; x++) {
          const c = findVTPageChunk(header, ASSET, m.mip, x, y);
          if (c) pages.push({ offset: Number(c.offset), size: Number(c.compressedSize) });
        }
      }
    }
  }
  const tail = findVTPageChunk(header, ASSET, maxMip + 1, 0, 0);
  if (tail) pages.push({ offset: Number(tail.offset), size: Number(tail.compressedSize) });

  const totalPayload = pages.reduce((s, p) => s + p.size, 0);
  const sizes = new Set(pages.map(p => p.size));
  console.log(
    `  asset=${ASSET} pages=${pages.length} payload=${fmtBytes(totalPayload)} ` +
    `distinctSizes=[${[...sizes].join(',')}]`,
  );
  if (pages.length === 0) throw new Error('no pages resolved');

  // Build a randomized read list of fixed length so every concurrency level
  // moves the same byte volume (fair comparison).
  const readList: PageLoc[] = [];
  for (let i = 0; i < READS_PER_SWEEP; i++) {
    readList.push(pages[Math.floor(Math.random() * pages.length)]);
  }
  const sweepBytes = readList.reduce((s, p) => s + p.size, 0);
  console.log(
    `\nsweep: ${READS_PER_SWEEP} reads, ${fmtBytes(sweepBytes)} per concurrency level\n`,
  );

  // Warm pass: populate OS page cache + server fd so the sweep measures the
  // sustained serving-layer + V8 ceiling, not cold-disk first-touch.
  console.log('warm pass...');
  await concurrent(
    readList.map(p => () => source.read(CONTAINER, p.offset, p.size)),
    16,
  );

  console.log('\nconcurrency | reads | bytes     | MiB/s  | p50    | p90    | p99    | max');
  console.log('------------+-------+-----------+--------+--------+--------+--------+--------');
  for (const c of CONCURRENCIES) {
    const latencies: number[] = [];
    let bytes = 0;
    const t0 = performance.now();
    await concurrent(
      readList.map(p => async () => {
        const r0 = performance.now();
        const buf = await source.read(CONTAINER, p.offset, p.size);
        latencies.push(performance.now() - r0);
        bytes += buf.byteLength;
      }),
      c,
    );
    const elapsed = performance.now() - t0;
    latencies.sort((a, b) => a - b);
    const mibs = bytes / (1 << 20) / (elapsed / 1000);
    console.log(
      `${String(c).padStart(11)} | ${String(readList.length).padStart(5)} | ` +
      `${fmtBytes(bytes).padStart(9)} | ${mibs.toFixed(1).padStart(6)} | ` +
      `${pct(latencies, 0.5).toFixed(2).padStart(6)}ms | ` +
      `${pct(latencies, 0.9).toFixed(2).padStart(6)}ms | ` +
      `${pct(latencies, 0.99).toFixed(2).padStart(6)}ms | ` +
      `${latencies[latencies.length - 1].toFixed(2).padStart(6)}ms`,
    );
  }
  console.log(
    '\nNote: warm (OS page cache hot). Cold-disk throughput requires dropping caches\n' +
    '(echo 3 > /proc/sys/vm/drop_caches as root) and is not measured here.',
  );
}

main().catch(e => {
  console.error(e);
  process.exit(1);
});
