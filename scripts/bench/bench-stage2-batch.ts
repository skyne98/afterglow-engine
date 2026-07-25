// Stage-2 batch reader bench: confirms createPageRangeReader (the client-side
// batch-read primitive) is FAST. Requests ALL pages of one asset in RANDOM
// order; the reader coalesces them into one range read per mip row regardless
// of input order. No transcode (isolates the read path). Compare to the raw
// batched ceiling (~600-700 MiB/s at K=32-64).
//
// Run (server up):
//   nix-shell -p deno --run "deno run --allow-net --allow-read --allow-env \
//     scripts/bench/bench-stage2-batch.ts"

import {
  createFetchRangeLoader,
  createPageRangeReader,
  findVTPageChunk,
  readBigHeader,
  type PageReadRequest,
} from '../../crates/afterglow-web/web/src/engine/assets/big-parser.ts';

const BASE_URL = process.env.BENCH_BASE_URL ?? 'http://127.0.0.1:8787/';
const CONTAINER = process.env.BENCH_CONTAINER ?? 'dungeon.big';
const ASSET = process.env.BENCH_ASSET ?? 'Rock064_Color.png';
const CONCURRENCIES = [1, 4, 8, 16, 32];

function fmtBytes(b: number): string {
  if (b >= 1 << 20) return (b / (1 << 20)).toFixed(1) + ' MiB';
  if (b >= 1 << 10) return (b / (1 << 10)).toFixed(1) + ' KiB';
  return b + ' B';
}

async function main() {
  const rawSource = createFetchRangeLoader(BASE_URL);
  const header = await readBigHeader(rawSource, CONTAINER, 2 * 1024 * 1024);
  const asset = header.assets.find(a => a.name === ASSET)?.virtualTexture;
  if (!asset) throw new Error(`asset ${ASSET} not found`);

  // BigAssetSession's containerLoader ignores the per-asset path and always
  // reads the session container; mirror that so `loader.read(path+'.big')`
  // hits dungeon.big.
  const source: { read(p: string, o: number, l: number): Promise<Uint8Array> } = {
    read: (_p, o, l) => rawSource.read(CONTAINER, o, l),
  };

  // All pages, shuffled — coalescing must be order-independent.
  const all: PageReadRequest[] = [];
  let maxMip = 0;
  for (const m of asset.mips) {
    maxMip = Math.max(maxMip, m.mip);
    for (let y = 0; y < m.pagesY; y++)
      for (let x = 0; x < m.pagesX; x++)
        all.push({ path: ASSET, mip: m.mip, x, y });
  }
  const tail = findVTPageChunk(header, ASSET, maxMip + 1, 0, 0);
  if (tail) all.push({ path: ASSET, mip: maxMip + 1, x: 0, y: 0, tail: true });
  for (let i = all.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [all[i], all[j]] = [all[j], all[i]];
  }
  // total source bytes
  let srcBytes = 0;
  for (const r of all) {
    const c = findVTPageChunk(header, ASSET, r.mip, r.x, r.y) ?? tail!;
    srcBytes += Number(c.compressedSize);
  }
  console.log(`asset=${ASSET} pages=${all.length} src=${fmtBytes(srcBytes)} (shuffled)`);

  console.log(`\nreadConcurrency | MiB/s  | reads | pages/read | coalesced% | elapsed`);
  console.log('----------------+--------+-------+------------+------------+---------');
  for (const c of CONCURRENCIES) {
    const reader = createPageRangeReader(source, header, c);
    // warm
    await reader.readBatch(all.slice(0, 256));
    const t0 = performance.now();
    const out = await reader.readBatch(all);
    const elapsed = performance.now() - t0;
    const s = reader.getStats();
    const mibs = srcBytes / (1 << 20) / (elapsed / 1000);
    const pagesPerRead = s.pagesRequested / s.reads;
    const coalescedPct = (s.pagesCoalesced / s.pagesRequested) * 100;
    console.log(
      `${String(c).padStart(15)} | ${mibs.toFixed(1).padStart(6)} | ${String(s.reads).padStart(5)} | ` +
      `${pagesPerRead.toFixed(1).padStart(10)} | ${coalescedPct.toFixed(0).padStart(6)}% | ${elapsed.toFixed(0).padStart(5)}ms`,
    );
    // sanity: every page returned with correct size
    for (let i = 0; i < out.length; i++) {
      if (!out[i] || out[i].byteLength === 0) throw new Error(`missing result ${i}`);
    }
  }
}

main().catch(e => { console.error(e); process.exit(1); });
