import {
  BULK_RANGE_CAPACITY,
  BULK_RESPONSE_MAX_BYTES,
  estimatedBulkResponseBytes,
  type AssetByteRange,
} from './bulk-range.ts';
import type { BigHeader } from './big-format.ts';
import type { ContainerRangeReader } from './deadline-range-batcher.ts';
import {
  VtPageDirectory,
  type ResolvedVtPage,
  type VtPageAddress,
} from './vt-page-directory.ts';

export type PageReadRequest = VtPageAddress;

export interface SourceSortedPageReaderStats {
  reads: number;
  averageReadMs: number;
  maxReadMs: number;
  batches: number;
  pagesRequested: number;
  /** Pages served by a multi-page coalesced run (not a singleton read). */
  pagesCoalesced: number;
  runs: number;
}

/** Explicit source-sorted diagnostic/tool reader. Production VT intentionally
 * preserves scheduler admission order; this primitive sorts resolved spans and
 * restores caller order without owning transcode or residency policy. */
export interface SourceSortedPageReader {
  readBatch(requests: readonly PageReadRequest[], signal?: AbortSignal): Promise<Uint8Array[]>;
  getStats(): Readonly<SourceSortedPageReaderStats>;
}

type IndexedPage = ResolvedVtPage & { index: number };

/** Create a batch page-range reader over a `.big` container. `readConcurrency`
 * bounds in-flight range reads per `readBatch` call. */
export function createSourceSortedPageReader(
  loader: ContainerRangeReader,
  header: BigHeader,
  readConcurrency = 16,
): SourceSortedPageReader {
  if (!Number.isInteger(readConcurrency) || readConcurrency < 1)
    throw new RangeError('source-sorted page reader concurrency must be positive');
  const directory = new VtPageDirectory(header);
  let reads = 0, totalReadMs = 0, maxReadMs = 0;
  let batches = 0, pagesRequested = 0, pagesCoalesced = 0, runs = 0;
  const stats: SourceSortedPageReaderStats = {
    reads: 0, averageReadMs: 0, maxReadMs: 0,
    batches: 0, pagesRequested: 0, pagesCoalesced: 0, runs: 0,
  };

  const resolve = (request: PageReadRequest, index: number): IndexedPage => ({
    ...directory.resolve(request),
    index,
  });

  /** Split a same-(path,mip,y) group (sorted by x) into maximal contiguous
   *  uniform-size runs; each run is one range read. */
  const coalesce = (group: IndexedPage[]): IndexedPage[][] => {
    const out: IndexedPage[][] = [];
    let runStart = 0;
    for (let i = 1; i <= group.length; i++) {
      const prev = group[i - 1];
      const cur = i < group.length ? group[i] : null;
      const contiguous = cur !== null
        && cur.x === prev.x + 1 && cur.length === prev.length
        && cur.offset === prev.offset + prev.length;
      if (!contiguous) {
        out.push(group.slice(runStart, i));
        runStart = i;
      }
    }
    return out;
  };

  const readBatch = async (
    requests: readonly PageReadRequest[],
    signal?: AbortSignal,
  ): Promise<Uint8Array[]> => {
    if (signal?.aborted) throw new Error('batch read canceled');
    batches++;
    pagesRequested += requests.length;
    const results = new Array<Uint8Array>(requests.length);
    const resolved = requests.map(resolve);
    if (loader.readBulk) {
      // Bulk transports may reorder independent requests. Source order turns
      // adjacent pages into long sequential reads while `page.index` restores
      // the caller's original order without copying payload bytes.
      const ordered = resolved.slice().sort((left, right) => left.offset - right.offset);
      const groups: IndexedPage[][] = [];
      let group: IndexedPage[] = [];
      let ranges: AssetByteRange[] = [];
      for (const page of ordered) {
        const candidate = { offset: page.offset, length: page.length };
        ranges.push(candidate);
        if (ranges.length > BULK_RANGE_CAPACITY ||
            estimatedBulkResponseBytes(ranges) > BULK_RESPONSE_MAX_BYTES) {
          ranges.pop();
          if (group.length === 0) throw new RangeError('one page exceeds bulk response capacity');
          groups.push(group);
          group = [];
          ranges = [candidate];
        }
        group.push(page);
      }
      if (group.length !== 0) groups.push(group);
      const readGroup = async (pages: IndexedPage[]): Promise<void> => {
        if (signal?.aborted) throw new Error('batch read canceled');
        const spans = pages.map(page => ({ offset: page.offset, length: page.length }));
        const readStartedAt = performance.now();
        const parts = await loader.readBulk!(spans);
        const readMs = performance.now() - readStartedAt;
        if (parts.length !== pages.length) throw new Error('bulk page response count mismatch');
        reads++;
        totalReadMs += readMs;
        maxReadMs = Math.max(maxReadMs, readMs);
        runs++;
        if (pages.length > 1) pagesCoalesced += pages.length;
        for (let index = 0; index < pages.length; index++) {
          if (parts[index].byteLength !== pages[index].length)
            throw new Error('bulk page response length mismatch');
          results[pages[index].index] = parts[index];
        }
      };
      let nextGroup = 0;
      const concurrency = Math.min(2, readConcurrency);
      await Promise.all(Array.from({ length: concurrency }, async () => {
        while (true) {
          const index = nextGroup++;
          if (index >= groups.length) return;
          await readGroup(groups[index]);
        }
      }));
      return results;
    }
    // Group by row (path, mip, y) so adjacent-x pages land together; tails apart.
    const groups = new Map<string, IndexedPage[]>();
    for (const r of resolved) {
      const key = r.tail ? `${r.path}:tail:${r.mip}` : `${r.path}:${r.mip}:${r.y}`;
      let g = groups.get(key);
      if (!g) { g = []; groups.set(key, g); }
      g.push(r);
    }
    const allRuns: IndexedPage[][] = [];
    for (const group of groups.values()) {
      if (group[0].tail) {
        for (const r of group) allRuns.push([r]);
      } else {
        group.sort((a, b) => a.x - b.x);
        for (const run of coalesce(group)) allRuns.push(run);
      }
    }
    const readRun = async (run: IndexedPage[]): Promise<void> => {
      const runOffset = run[0].offset;
      let runSize = 0;
      for (const p of run) runSize += p.length;
      const readStartedAt = performance.now();
      const batchData = await loader.read(runOffset, runSize);
      const readMs = performance.now() - readStartedAt;
      reads++;
      totalReadMs += readMs;
      maxReadMs = Math.max(maxReadMs, readMs);
      runs++;
      if (run.length > 1) pagesCoalesced += run.length;
      let rel = 0;
      for (const p of run) {
        results[p.index] = batchData.subarray(rel, rel + p.length);
        rel += p.length;
      }
    };
    let next = 0;
    await Promise.all(Array.from({ length: readConcurrency }, async () => {
      while (true) {
        const i = next++;
        if (i >= allRuns.length) return;
        await readRun(allRuns[i]);
      }
    }));
    return results;
  };

  return {
    readBatch,
    getStats() {
      stats.reads = reads;
      stats.averageReadMs = reads === 0 ? 0 : totalReadMs / reads;
      stats.maxReadMs = maxReadMs;
      stats.batches = batches;
      stats.pagesRequested = pagesRequested;
      stats.pagesCoalesced = pagesCoalesced;
      stats.runs = runs;
      return stats;
    },
  };
}
