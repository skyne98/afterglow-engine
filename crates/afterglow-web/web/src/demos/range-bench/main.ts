import { createFetchRangeLoader, readBigHeader } from "../../engine/assets/asset-range.ts";
import { findVTPageChunk } from "../../engine/assets/big-format.ts";
import {
  createSourceSortedPageReader,
  type PageReadRequest,
} from "../../engine/assets/source-sorted-page-reader.ts";

const CONTAINER = "dungeon.big";
const ASSET = "Rock064_Color.png";
const DEFAULT_CONCURRENCY = 16;

export interface RangeBenchResult {
  bytes: number;
  elapsedMs: number;
  mibPerSecond: number;
  pages: number;
  rangeReads: number;
  coalesceMiB: number;
  protocols: string[];
  readConcurrency: number;
}

declare global {
  interface Window {
    runAfterglowRangeBench: () => Promise<RangeBenchResult>;
  }
}

function readConcurrency(): number {
  const value = Number(new URLSearchParams(location.search).get("concurrency"));
  return Number.isSafeInteger(value) && value > 0 && value <= 32
    ? value
    : DEFAULT_CONCURRENCY;
}

function coalesceBytes(): number {
  const value = Number(new URLSearchParams(location.search).get("coalesceMiB"));
  return Number.isSafeInteger(value) && value >= 1 && value <= 16
    ? value * 1024 * 1024
    : 0;
}

function shuffle(requests: PageReadRequest[]): void {
  let state = 0x9e3779b9;
  for (let index = requests.length - 1; index > 0; index--) {
    state ^= state << 13;
    state ^= state >>> 17;
    state ^= state << 5;
    const other = (state >>> 0) % (index + 1);
    const current = requests[index];
    requests[index] = requests[other];
    requests[other] = current;
  }
}

function protocols(): string[] {
  const values = new Set<string>();
  for (const entry of performance.getEntriesByType("resource")) {
    const resource = entry as PerformanceResourceTiming;
    if (new URL(resource.name).pathname.endsWith(`/${CONTAINER}`))
      values.add(resource.nextHopProtocol || "unknown");
  }
  return [...values].sort();
}

function print(result: RangeBenchResult): void {
  const output = document.getElementById("out");
  if (!output) return;
  const aggregation = result.coalesceMiB === 0 ? "rows" : `${result.coalesceMiB} MiB`;
  output.textContent = [
    `asset=${ASSET} pages=${result.pages} source=${(result.bytes / 1048576).toFixed(1)} MiB`,
    `concurrency=${result.readConcurrency} reads=${result.rangeReads} coalesce=${aggregation} protocols=${result.protocols.join(",")}`,
    `${result.mibPerSecond.toFixed(1)} MiB/s in ${result.elapsedMs.toFixed(1)} ms`,
  ].join("\n");
}

async function run(): Promise<RangeBenchResult> {
  const source = createFetchRangeLoader();
  const header = await readBigHeader(source, CONTAINER, 2 * 1024 * 1024);
  const asset = header.assets.find((candidate) => candidate.name === ASSET)?.virtualTexture;
  if (!asset) throw new Error(`virtual texture not found: ${ASSET}`);

  const requests: PageReadRequest[] = [];
  let maxMip = 0;
  for (const mip of asset.mips) {
    maxMip = Math.max(maxMip, mip.mip);
    for (let y = 0; y < mip.pagesY; y++)
      for (let x = 0; x < mip.pagesX; x++)
        requests.push({ path: ASSET, mip: mip.mip, x, y });
  }
  const tail = findVTPageChunk(header, ASSET, maxMip + 1, 0, 0);
  if (tail) requests.push({ path: ASSET, mip: maxMip + 1, x: 0, y: 0, tail: true });
  shuffle(requests);

  let bytes = 0;
  for (const request of requests) {
    const chunk = request.tail
      ? tail
      : findVTPageChunk(header, ASSET, request.mip, request.x, request.y);
    if (!chunk) throw new Error(`virtual texture page not found: ${request.mip}:${request.x}:${request.y}`);
    bytes += Number(chunk.compressedSize);
  }

  const concurrency = readConcurrency();
  const maxCoalesceBytes = coalesceBytes();
  performance.setResourceTimingBufferSize(512);
  performance.clearResourceTimings();
  const startedAt = performance.now();
  let rangeReads = 0;
  if (maxCoalesceBytes === 0) {
    const reader = createSourceSortedPageReader({
      read: (offset, length) => source.read(CONTAINER, offset, length),
      readBulk: ranges => source.readBulk!(CONTAINER, ranges),
    }, header, concurrency);
    const pages = await reader.readBatch(requests);
    for (const page of pages) if (page.byteLength === 0) throw new Error("empty range response");
    rangeReads = reader.getStats().reads;
  } else {
    const ranges: { offset: number; length: number }[] = [];
    for (const mip of asset.mips) {
      let offset = Number(mip.offset), runOffset = offset, runLength = 0;
      for (const length of mip.pageSizes) {
        if (runLength !== 0 && runLength + length > maxCoalesceBytes) {
          ranges.push({ offset: runOffset, length: runLength });
          runOffset = offset;
          runLength = 0;
        }
        runLength += length;
        offset += length;
      }
      if (runLength !== 0) ranges.push({ offset: runOffset, length: runLength });
    }
    if (tail) ranges.push({ offset: Number(tail.offset), length: Number(tail.compressedSize) });
    let next = 0;
    await Promise.all(Array.from({ length: concurrency }, async () => {
      while (true) {
        const index = next++;
        if (index >= ranges.length) return;
        const range = ranges[index];
        const data = await source.read(CONTAINER, range.offset, range.length);
        if (data.byteLength !== range.length) throw new Error("short coalesced range response");
      }
    }));
    rangeReads = ranges.length;
  }
  const elapsedMs = performance.now() - startedAt;
  const result: RangeBenchResult = {
    bytes,
    elapsedMs,
    mibPerSecond: bytes / 1048576 / (elapsedMs / 1000),
    pages: requests.length,
    rangeReads,
    coalesceMiB: maxCoalesceBytes / 1048576,
    protocols: protocols(),
    readConcurrency: concurrency,
  };
  print(result);
  return result;
}

window.runAfterglowRangeBench = run;
