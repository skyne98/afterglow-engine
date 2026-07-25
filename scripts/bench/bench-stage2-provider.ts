// Stage-1 provider-level bench: exercises the REAL createPageDataProvider
// (with a no-op transcoder) to confirm the read-ahead batching delivers the
// batching ceiling inside engine code, not just at the raw fetch layer.
//
// The no-op transcoder returns a valid BC7 frame instantly, isolating the
// read + read-ahead path from CPU transcode (stage 6). Source bytes = sum of
// page compressedSizes (what the serving layer actually moved).
//
// Run (server up):
//   nix-shell -p deno --run "deno run --allow-net --allow-read --allow-env \
//     scripts/bench/bench-stage2-provider.ts"

import {
  createFetchRangeLoader,
  createPageDataProvider,
  findVTPageChunk,
  readBigHeader,
} from '../../crates/afterglow-web/web/src/engine/assets/big-parser.ts';

const BASE_URL = process.env.BENCH_BASE_URL ?? 'http://127.0.0.1:8787/';
const CONTAINER = process.env.BENCH_CONTAINER ?? 'dungeon.big';
const ASSET = process.env.BENCH_ASSET ?? 'Rock064_Color.png';
const CONCURRENCIES = [1, 4, 8, 16];
const FORMAT_BC7 = 0;
const SLOT_BYTES = 34 * 34 * 16; // 18496

interface PageReq { mip: number; x: number; y: number; tail?: boolean; srcBytes: number; }

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
    while (true) { const i = next++; if (i >= tasks.length) return; await tasks[i](); }
  }));
}

async function main() {
  const source = createFetchRangeLoader(BASE_URL);
  const header = await readBigHeader(source, CONTAINER, 2 * 1024 * 1024);
  const asset = header.assets.find(a => a.name === ASSET)?.virtualTexture;
  if (!asset) throw new Error(`asset ${ASSET} not found`);

  // Enumerate every page in ROW ORDER (maximizes read-ahead hits: requesting
  // (x,y) batches (x+1..K,y), so the next requests hit the read-ahead cache).
  const reqs: PageReq[] = [];
  let maxMip = 0;
  for (const m of asset.mips) {
    maxMip = Math.max(maxMip, m.mip);
    for (let y = 0; y < m.pagesY; y++)
      for (let x = 0; x < m.pagesX; x++) {
        const c = findVTPageChunk(header, ASSET, m.mip, x, y)!;
        reqs.push({ mip: m.mip, x, y, srcBytes: Number(c.compressedSize) });
      }
  }
  const tail = findVTPageChunk(header, ASSET, maxMip + 1, 0, 0);
  if (tail) reqs.push({ mip: maxMip + 1, x: 0, y: 0, tail: true, srcBytes: Number(tail.compressedSize) });
  const totalSrc = reqs.reduce((s, r) => s + r.srcBytes, 0);
  console.log(`asset=${ASSET} pages=${reqs.length} src=${fmtBytes(totalSrc)}`);

  // No-op transcoder: returns a valid BC7 frame instantly (isolates read path).
  const NOOP_FRAME = (() => {
    const f = new Uint8Array(16 + SLOT_BYTES);
    const v = new DataView(f.buffer);
    v.setUint32(0, 1, true);   // count
    v.setUint32(4, 136, true); // width
    v.setUint32(8, 136, true); // height
    v.setUint32(12, SLOT_BYTES, true);
    return f;
  })();
  const noopWorkers = Array.from({ length: 4 }, () => ({
    async transcode(): Promise<Uint8Array> { return NOOP_FRAME; },
  }));

  // BigAssetSession's containerLoader ignores the per-asset path and always
  // reads from the session container; mirror that so `loader.read(path+'.big')`
  // hits dungeon.big.
  const containerLoader = {
    read: (_path: string, offset: number, len: number) => source.read(CONTAINER, offset, len),
    load: (p: string) => source.load(p),
    size: (p: string) => source.size(p),
  };
  const provider = createPageDataProvider(containerLoader, header, noopWorkers, FORMAT_BC7);

  // Warm a few rows.
  await concurrent(reqs.slice(0, 128).map(r => () => provider(ASSET, r)), 8);

  console.log(`\nconcurrency | pages | src bytes  | MiB/s  | per-page p50 | per-page p99`);
  console.log('------------+-------+------------+--------+--------------+--------------');
  for (const c of CONCURRENCIES) {
    const latencies: number[] = [];
    let srcBytes = 0;
    const t0 = performance.now();
    await concurrent(reqs.map(r => async () => {
      const r0 = performance.now();
      await provider(ASSET, r);
      latencies.push(performance.now() - r0);
      srcBytes += r.srcBytes;
    }), c);
    const elapsed = performance.now() - t0;
    latencies.sort((a, b) => a - b);
    const mibs = srcBytes / (1 << 20) / (elapsed / 1000);
    console.log(
      `${String(c).padStart(11)} | ${String(reqs.length).padStart(5)} | ${fmtBytes(srcBytes).padStart(10)} | ` +
      `${mibs.toFixed(1).padStart(6)} | ${pct(latencies, 0.5).toFixed(2).padStart(8)}ms | ${pct(latencies, 0.99).toFixed(2).padStart(8)}ms`,
    );
  }
  const s = provider.getStats();
  console.log(`\nsource range reads: ${s.reads} for ${reqs.length} pages ` +
    `(read-ahead ratio ~${(reqs.length / s.reads).toFixed(1)} pages/read)`);
}

main().catch(e => { console.error(e); process.exit(1); });
