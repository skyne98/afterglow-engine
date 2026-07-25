// Stage-2 DIRECT pread bench: bypasses HTTP entirely to find the disk→bytes
// ceiling. Uses the engine's real readBigHeader + findVTPageChunk to resolve
// page offsets, then reads row-runs directly from the file via pread (one fd
// per worker; seek+read). No coep_server, no fetch, no arrayBuffer-over-HTTP.
//
// Isolates: (a) the no-HTTP software ceiling (warm = RAM memcpy speed), and
// (b) actual NVMe speed (cold, after dropping the page cache).
//
// Run:
//   warm: nix-shell -p deno --run "deno run --allow-read --allow-run --allow-env scripts/bench/bench-stage2-direct.ts"
//   cold: BENCH_COLD=1 nix-shell -p deno --run "deno run --allow-read --allow-run --allow-env scripts/bench/bench-stage2-direct.ts"
// (cold needs passwordless sudo for `echo 3 > /proc/sys/vm/drop_caches`)

import {
  createFetchRangeLoader,
  findVTPageChunk,
  readBigHeader,
} from '../../crates/afterglow-web/web/src/engine/assets/big-parser.ts';

const FILE = process.env.BENCH_FILE ?? '/home/fox/dev/afterglow-engine/crates/afterglow-web/web/assets/dungeon.big';
const BASE_URL = process.env.BENCH_BASE_URL ?? 'http://127.0.0.1:8787/';
const CONTAINER = process.env.BENCH_CONTAINER ?? 'dungeon.big';
const ASSET = process.env.BENCH_ASSET ?? 'Rock064_Color.png';
const CONCURRENCIES = [1, 4, 8, 16, 32];
const COLD = process.env.BENCH_COLD === '1';

interface Run { offset: number; size: number; }
function fmtBytes(b: number): string {
  if (b >= 1 << 30) return (b / (1 << 30)).toFixed(2) + ' GiB';
  if (b >= 1 << 20) return (b / (1 << 20)).toFixed(1) + ' MiB';
  if (b >= 1 << 10) return (b / (1 << 10)).toFixed(1) + ' KiB';
  return b + ' B';
}
async function dropCaches(): Promise<number> {
  const cmd = new Deno.Command('sudo', { args: ['sh', '-c', 'echo 3 > /proc/sys/vm/drop_caches'], stdout: 'null', stderr: 'null' });
  const { code } = await cmd.output();
  return code;
}

async function main() {
  // Header via HTTP (tiny, one-time) for page geometry.
  const source = createFetchRangeLoader(BASE_URL);
  const header = await readBigHeader(source, CONTAINER, 2 * 1024 * 1024);
  const asset = header.assets.find(a => a.name === ASSET)?.virtualTexture;
  if (!asset) throw new Error(`asset ${ASSET} not found`);

  // Build contiguous row-runs (same coalescing the batch reader does).
  const runs: Run[] = [];
  let maxMip = 0;
  for (const m of asset.mips) {
    maxMip = Math.max(maxMip, m.mip);
    for (let y = 0; y < m.pagesY; y++) {
      let off = Number(m.offset);
      for (let i = 0; i < y * m.pagesX; i++) off += m.pageSizes[i];
      const sz = m.pageSizes[y * m.pagesX];
      let ok = true, total = 0;
      for (let x = 0; x < m.pagesX; x++) {
        if (m.pageSizes[y * m.pagesX + x] !== sz) { ok = false; break; }
        total += m.pageSizes[y * m.pagesX + x];
      }
      if (ok && sz > 0) runs.push({ offset: off, size: total });
    }
  }
  const tail = findVTPageChunk(header, ASSET, maxMip + 1, 0, 0);
  if (tail) runs.push({ offset: Number(tail.offset), size: Number(tail.compressedSize) });
  const totalBytes = runs.reduce((s, r) => s + r.size, 0);
  console.log(`asset=${ASSET} runs=${runs.length} total=${fmtBytes(totalBytes)} (${COLD ? 'COLD' : 'warm'})`);

  const readAll = async (fds: Deno.FsFile[], concurrency: number) => {
    let next = 0;
    await Promise.all(fds.slice(0, concurrency).map(async (fd) => {
      while (true) {
        const i = next++;
        if (i >= runs.length) return;
        const run = runs[i];
        const buf = new Uint8Array(run.size);
        fd.seekSync(run.offset, Deno.SeekMode.Start);
        let read = 0;
        while (read < run.size) {
          const n = await fd.read(buf.subarray(read));
          if (!n || n === 0) break;
          read += n;
        }
        if (read !== run.size) throw new Error(`short read ${read}/${run.size} @ ${run.offset}`);
      }
    }));
  };

  if (!COLD) {
    // Full warm pass: cache every run once.
    const warmFd = Deno.openSync(FILE, { read: true });
    await readAll([warmFd], 1);
    warmFd.close();
  }

  console.log(`\nworkers | MiB/s   | GiB/s  | reads | elapsed`);
  console.log('--------+---------+--------+-------+---------');
  for (const c of CONCURRENCIES) {
    if (COLD) { const code = await dropCaches(); if (code !== 0) console.log(`  warn: drop_caches exit ${code}`); }
    const fds: Deno.FsFile[] = [];
    for (let i = 0; i < c; i++) fds.push(Deno.openSync(FILE, { read: true }));
    const t0 = performance.now();
    await readAll(fds, c);
    const elapsed = performance.now() - t0;
    for (const fd of fds) fd.close();
    const mibs = totalBytes / (1 << 20) / (elapsed / 1000);
    console.log(
      `${String(c).padStart(7)} | ${mibs.toFixed(1).padStart(7)} | ${(mibs / 1024).toFixed(2).padStart(6)} | ` +
      `${String(runs.length).padStart(5)} | ${elapsed.toFixed(0).padStart(5)}ms`,
    );
  }
}

main().catch(e => { console.error(e); process.exit(1); });
