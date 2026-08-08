// Stage-1 batching ceiling probe.
//
// Reads K contiguous pages (a segment of a mip row) in ONE fetch+Range request,
// to prove how much of the per-request overhead is amortizable by batching.
// The .big stores pages contiguously within a mip (row-major), so a mip-row
// segment is one contiguous byte range.
//
// If throughput scales with K, batching is the right fix (#2). If it flatlines,
// the bottleneck is per-byte (V8/arrayBuffer/HTTP-body), not per-request.
//
// Run (server must be up):
//   nix-shell -p deno --run "deno run --allow-net --allow-read --allow-env \
//     scripts/bench/bench-stage1-batched.ts"

import {
  createFetchRangeLoader,
  readBigHeader,
} from '../../crates/afterglow-web/web/src/engine/assets/asset-range.ts';
import { findVTPageChunk } from '../../crates/afterglow-web/web/src/engine/assets/big-format.ts';

const BASE_URL = process.env.BENCH_BASE_URL ?? 'http://127.0.0.1:8787/';
const CONTAINER = process.env.BENCH_CONTAINER ?? 'dungeon.big';
const ASSET = process.env.BENCH_ASSET ?? 'Rock064_Color.png';
const CONCURRENCY = Number(process.env.BENCH_C ?? 16);
const KS = [1, 2, 4, 8, 16, 32, 64];

interface Run { offset: number; size: number; pages: number; }

function pct(sorted: number[], q: number): number {
  if (!sorted.length) return 0;
  return sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * q))];
}
function fmtBytes(b: number): string {
  if (b >= 1 << 20) return (b / (1 << 20)).toFixed(1) + ' MiB';
  if (b >= 1 << 10) return (b / (1 << 10)).toFixed(1) + ' KiB';
  return b + ' B';
}

async function concurrent(tasks: (() => Promise<void>)[], c: number): Promise<void> {
  let next = 0;
  await Promise.all(Array.from({ length: c }, async () => {
    while (true) {
      const i = next++;
      if (i >= tasks.length) return;
      await tasks[i]();
    }
  }));
}

async function main() {
  const source = createFetchRangeLoader(BASE_URL);
  const header = await readBigHeader(source, CONTAINER, 2 * 1024 * 1024);
  const asset = header.assets.find(a => a.name === ASSET)?.virtualTexture;
  if (!asset) throw new Error(`asset ${ASSET} not found`);

  // Build contiguous runs: for each mip, each row, the row is a contiguous span
  // of pagesX pages. Verify contiguity (next page offset == prev offset + size).
  const rows: { offset: number; pageSize: number; pages: number }[] = [];
  for (const m of asset.mips) {
    for (let y = 0; y < m.pagesY; y++) {
      const first = y * m.pagesX;
      const off0 = Number(m.offset) + m.pageSizes.slice(0, first).reduce((s, v) => s + v, 0);
      const sz = m.pageSizes[first];
      // verify contiguity across the row
      let ok = true, o = off0;
      for (let x = 0; x < m.pagesX; x++) {
        const idx = first + x;
        if (m.pageSizes[idx] !== sz) { ok = false; break; }
        o += m.pageSizes[idx];
      }
      if (ok && sz > 0) rows.push({ offset: off0, pageSize: sz, pages: m.pagesX });
    }
  }
  const totalRunBytes = rows.reduce((s, r) => s + r.pageSize * r.pages, 0);
  console.log(`asset=${ASSET} rows=${rows.length} (${fmtBytes(totalRunBytes)} contiguous)`);

  // For each K, build a read list of fixed count. Each "read" fetches K contiguous
  // pages from a random row (clamped to the row length). Total bytes scales with K,
  // so compare per-request latency AND MiB/s.
  const READS = 1024;
  console.log(`\nconcurrency=${CONCURRENCY}, ${READS} requests per K\n`);
  console.log('K (pages) | req bytes | total bytes | MiB/s  | per-req p50 | per-page p50');
  console.log('----------+-----------+--------------+--------+-------------+--------------');

  // Warm.
  await concurrent(rows.slice(0, 64).map(r => () => source.read(CONTAINER, r.offset, r.pageSize * Math.min(8, r.pages))), CONCURRENCY);

  for (const K of KS) {
    const runs: Run[] = [];
    for (let i = 0; i < READS; i++) {
      const row = rows[Math.floor(Math.random() * rows.length)];
      const k = Math.min(K, row.pages);
      const maxStart = row.pages - k;
      const start = maxStart > 0 ? Math.floor(Math.random() * maxStart) : 0;
      runs.push({ offset: row.offset + start * row.pageSize, size: row.pageSize * k, pages: k });
    }
    const latencies: number[] = [];
    let bytes = 0;
    const t0 = performance.now();
    await concurrent(runs.map(r => async () => {
      const r0 = performance.now();
      const buf = await source.read(CONTAINER, r.offset, r.size);
      latencies.push(performance.now() - r0);
      bytes += buf.byteLength;
    }), CONCURRENCY);
    const elapsed = performance.now() - t0;
    latencies.sort((a, b) => a - b);
    const mibs = bytes / (1 << 20) / (elapsed / 1000);
    const perReqP50 = pct(latencies, 0.5);
    const perPageP50 = perReqP50 / K;
    console.log(
      `${String(K).padStart(9)} | ${fmtBytes(runs[0].size).padStart(9)} | ${fmtBytes(bytes).padStart(12)} | ` +
      `${mibs.toFixed(1).padStart(6)} | ${perReqP50.toFixed(2).padStart(9)}ms | ${perPageP50.toFixed(2).padStart(7)}ms`,
    );
  }
}

main().catch(e => { console.error(e); process.exit(1); });
