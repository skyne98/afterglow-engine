#!/usr/bin/env bun

import {
  parseBigHeader,
  type BigHeader,
  type VirtualTextureDirectory,
} from '../crates/afterglow-web/web/src/engine/assets/big-format.ts';
import { validateAgtb } from './profile-dungeon-vt.ts';

const HEADER_BYTES = 40;
const RECORD_BYTES = 40;
const DESCRIPTOR_BULK_WAIT = 14;
const DESCRIPTOR_BULK_DISPATCH = 15;
const DESCRIPTOR_SCHEDULER_WAIT = 23;
const PHASE_ASYNC_BEGIN = 4;
const PHASE_ASYNC_END = 5;
const SCHEDULER_ADMITTED = 0;
const URGENT_DEADLINE_NS = 1_000_000;
const FOCUS_DEADLINE_NS = 16_000_000;
const PERIPHERAL_DEADLINE_NS = 64_000_000;
const QUALITY_TIER_BASE = 66;
const CHANNEL_LANES = 22;
const MAX_MIP = 10;

const DUNGEON_TEXTURE_ORDER = [
  'Rock064_Color.png',
  'Rock064_NormalGL.png',
  'Rock064_Masks.png',
  'Ground103_Color.png',
  'Ground103_NormalGL.png',
  'Ground103_Masks.png',
  'PavingStones150_Color.png',
  'PavingStones150_NormalGL.png',
  'PavingStones150_Masks.png',
] as const;

interface TraceRecord {
  timestamp: number;
  correlation: number;
  argument0: number;
  argument1: number;
  descriptor: number;
  phase: number;
}

export interface DecodedPageIdentity {
  textureId: number;
  material: number;
  channel: number;
  mip: number;
  x: number;
  y: number;
  tail: boolean;
}

export interface ReplayRequest extends DecodedPageIdentity {
  key: number;
  detectedAt: number;
  admittedAt: number;
  timestamp: number;
  bytes: number;
  lane: 0 | 1 | 2;
  priority: number;
  sourceOffset: number;
  dispatched: boolean;
}

export interface ReplayBatch {
  lane: 0 | 1 | 2;
  openedAt: number;
  dispatchedAt: number;
  requests: ReplayRequest[];
}

export interface ReplayVariantSummary {
  batches: number;
  meanSpans: number;
  admissionMeanMs: number;
  admissionP95Ms: number;
  admissionP99Ms: number;
  admissionMaxMs: number;
  reorderedRequests: number;
}

export interface DungeonVtReplayReport {
  trace: string;
  successfulPageReads: number;
  canceledBeforeDispatch: number;
  recordedBulkRequests: number;
  replayedBulkRequests: number;
  meanSpansPerRequest: number;
  callerOrderedSourceRuns: number;
  sourceSortedRuns: number;
  sourceRunReductionPercent: number;
  sourceSortedBulkRequests: number;
  prioritySensitivity: {
    caveat: string;
    currentPriorityReschedule: ReplayVariantSummary;
    mipDeficitFirst: ReplayVariantSummary;
    mipDeficitAndChannelAffinity: ReplayVariantSummary;
  };
  conclusion: string;
}

function u64(view: DataView, offset: number): number {
  return view.getUint32(offset, true) + view.getUint32(offset + 4, true) * 0x1_0000_0000;
}

function readRecords(bytes: Uint8Array): TraceRecord[] {
  const header = validateAgtb(bytes);
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const records = new Array<TraceRecord>(header.recordCount);
  for (let index = 0; index < header.recordCount; index++) {
    const base = HEADER_BYTES + index * RECORD_BYTES;
    records[index] = {
      timestamp: u64(view, base),
      correlation: u64(view, base + 8),
      argument0: u64(view, base + 16),
      argument1: u64(view, base + 24),
      descriptor: view.getUint32(base + 32, true),
      phase: view.getUint8(base + 36),
    };
  }
  return records;
}

export function decodePageIdentity(key: number): DecodedPageIdentity {
  const textureId = Math.floor(key / 0x2000_0000);
  const local = key - textureId * 0x2000_0000;
  return {
    textureId,
    material: Math.floor((textureId - 1) / 3),
    channel: (textureId - 1) % 3,
    mip: local & 0x3f,
    x: (local >>> 6) & 0x7ff,
    y: (local >>> 17) & 0x7ff,
    tail: (local & 0x1000_0000) !== 0,
  };
}

function pushMap<T>(map: Map<number, T[]>, key: number, value: T): void {
  const values = map.get(key);
  if (values) values.push(value);
  else map.set(key, [value]);
}

interface SchedulerAdmission {
  detectedAt: number;
  admittedAt: number;
  priority: number;
}

interface BulkOperation {
  key: number;
  timestamp: number;
  completedAt: number;
  bytes: number;
  lane: 0 | 1 | 2;
  dispatched: boolean;
}

function extractOperations(records: readonly TraceRecord[]): {
  admissions: Map<number, SchedulerAdmission[]>;
  bulk: BulkOperation[];
  recordedDispatches: number;
} {
  const schedulerStarts = new Map<number, number>();
  const admissions = new Map<number, SchedulerAdmission[]>();
  const bulkStarts = new Map<number, BulkOperation>();
  const bulk: BulkOperation[] = [];
  let recordedDispatches = 0;

  for (const record of records) {
    if (record.descriptor === DESCRIPTOR_SCHEDULER_WAIT) {
      if (record.phase === PHASE_ASYNC_BEGIN) {
        schedulerStarts.set(record.correlation, record.timestamp);
      } else if (record.phase === PHASE_ASYNC_END) {
        const detectedAt = schedulerStarts.get(record.correlation);
        schedulerStarts.delete(record.correlation);
        if (detectedAt !== undefined && record.argument1 === SCHEDULER_ADMITTED) {
          pushMap(admissions, record.correlation, {
            detectedAt,
            admittedAt: record.timestamp,
            priority: record.argument0,
          });
        }
      }
      continue;
    }
    if (record.descriptor === DESCRIPTOR_BULK_WAIT) {
      if (record.phase === PHASE_ASYNC_BEGIN) {
        const operation: BulkOperation = {
          key: record.correlation,
          timestamp: record.timestamp,
          completedAt: 0,
          bytes: record.argument0,
          lane: record.argument1 === 0 ? 0 : record.argument1 === 1 ? 1 : 2,
          dispatched: false,
        };
        bulkStarts.set(record.correlation, operation);
      } else if (record.phase === PHASE_ASYNC_END) {
        const operation = bulkStarts.get(record.correlation);
        bulkStarts.delete(record.correlation);
        if (operation) {
          operation.completedAt = record.timestamp;
          operation.dispatched = record.argument0 !== 0;
          bulk.push(operation);
        }
      }
      continue;
    }
    if (record.descriptor === DESCRIPTOR_BULK_DISPATCH && record.phase === PHASE_ASYNC_BEGIN)
      recordedDispatches++;
  }
  return { admissions, bulk, recordedDispatches };
}

interface DirectoryIndex {
  directory: VirtualTextureDirectory;
  prefixByMip: Map<number, Float64Array>;
}

function buildDirectoryIndexes(header: BigHeader): DirectoryIndex[] {
  return DUNGEON_TEXTURE_ORDER.map(name => {
    const directory = header.assets.find(asset => asset.name === name)?.virtualTexture;
    if (!directory) throw new Error(`Dungeon VT directory missing: ${name}`);
    const prefixByMip = new Map<number, Float64Array>();
    for (const mip of directory.mips) {
      const prefix = new Float64Array(mip.pageSizes.length + 1);
      for (let index = 0; index < mip.pageSizes.length; index++)
        prefix[index + 1] = prefix[index] + mip.pageSizes[index];
      prefixByMip.set(mip.mip, prefix);
    }
    return { directory, prefixByMip };
  });
}

function sourceOffset(page: DecodedPageIdentity, indexes: readonly DirectoryIndex[]): number {
  const index = indexes[page.textureId - 1];
  if (!index) throw new RangeError(`trace texture ID ${page.textureId} has no Dungeon directory`);
  if (page.tail) {
    if (!index.directory.tail) throw new Error(`texture ${page.textureId} has no mip tail`);
    return Number(index.directory.tail.offset);
  }
  const mip = index.directory.mips.find(candidate => candidate.mip === page.mip);
  const prefix = index.prefixByMip.get(page.mip);
  if (!mip || !prefix || page.x >= mip.pagesX || page.y >= mip.pagesY)
    throw new RangeError(`page outside directory: texture=${page.textureId} mip=${page.mip} (${page.x},${page.y})`);
  const pageIndex = page.y * mip.pagesX + page.x;
  return Number(mip.offset) + prefix[pageIndex];
}

function buildRequests(
  operations: readonly BulkOperation[],
  admissions: Map<number, SchedulerAdmission[]>,
  indexes: readonly DirectoryIndex[],
): ReplayRequest[] {
  const admissionCursor = new Map<number, number>();
  const requests: ReplayRequest[] = [];
  for (const operation of operations) {
    const cursor = admissionCursor.get(operation.key) ?? 0;
    const admission = admissions.get(operation.key)?.[cursor];
    if (!admission) throw new Error(`dispatched page ${operation.key} has no scheduler admission`);
    admissionCursor.set(operation.key, cursor + 1);
    const page = decodePageIdentity(operation.key);
    requests.push({
      ...page,
      key: operation.key,
      detectedAt: admission.detectedAt,
      admittedAt: admission.admittedAt,
      timestamp: operation.timestamp,
      bytes: operation.bytes,
      lane: operation.lane,
      priority: admission.priority,
      sourceOffset: sourceOffset(page, indexes),
      dispatched: operation.dispatched,
    });
  }
  return requests;
}

/** Replay the two non-resetting queue deadlines. The measured traces never
 * saturate the two in-flight/8 MiB dispatch boundary, so service time does not
 * affect batch formation for these captures. */
export function replayBatches(input: readonly ReplayRequest[]): ReplayBatch[] {
  const requests = [...input].sort((left, right) => left.timestamp - right.timestamp);
  const queued: [ReplayRequest[], ReplayRequest[], ReplayRequest[]] = [[], [], []];
  const openedAt = [
    Number.POSITIVE_INFINITY, Number.POSITIVE_INFINITY, Number.POSITIVE_INFINITY,
  ];
  const deadline = [
    Number.POSITIVE_INFINITY, Number.POSITIVE_INFINITY, Number.POSITIVE_INFINITY,
  ];
  const batches: ReplayBatch[] = [];
  let next = 0;
  while (next < requests.length || deadline.some(Number.isFinite)) {
    const arrival = next < requests.length ? requests[next].timestamp : Number.POSITIVE_INFINITY;
    const readyLane: 0 | 1 | 2 = deadline[0] <= deadline[1] && deadline[0] <= deadline[2]
      ? 0 : deadline[1] <= deadline[2] ? 1 : 2;
    if (arrival <= deadline[readyLane]) {
      const request = requests[next++];
      const lane = request.lane;
      queued[lane].push(request);
      if (!Number.isFinite(deadline[lane])) {
        openedAt[lane] = request.timestamp;
        const wait = lane === 0 ? URGENT_DEADLINE_NS :
          lane === 1 ? FOCUS_DEADLINE_NS : PERIPHERAL_DEADLINE_NS;
        deadline[lane] = request.timestamp + wait;
      }
    } else {
      if (queued[readyLane].length !== 0) {
        const dispatched = queued[readyLane].filter(request => request.dispatched);
        if (dispatched.length !== 0) {
          batches.push({
            lane: readyLane,
            openedAt: openedAt[readyLane],
            dispatchedAt: deadline[readyLane],
            requests: dispatched,
          });
        }
        queued[readyLane] = [];
      }
      openedAt[readyLane] = Number.POSITIVE_INFINITY;
      deadline[readyLane] = Number.POSITIVE_INFINITY;
    }
  }
  return batches;
}

export function countSourceRuns(requests: readonly ReplayRequest[], sortBySource: boolean): number {
  const ordered = sortBySource
    ? [...requests].sort((left, right) => left.sourceOffset - right.sourceOffset)
    : requests;
  let runs = 0;
  let previousEnd = -1;
  for (const request of ordered) {
    if (request.sourceOffset !== previousEnd) runs++;
    previousEnd = request.sourceOffset + request.bytes;
  }
  return runs;
}

function virtualFootprintsOverlap(left: ReplayRequest, right: ReplayRequest): boolean {
  if (left.material !== right.material || left.lane !== right.lane || left.channel === right.channel)
    return false;
  if (left.tail || right.tail) return left.tail && right.tail;
  const leftScale = 2 ** left.mip;
  const rightScale = 2 ** right.mip;
  return left.x * leftScale < (right.x + 1) * rightScale &&
    right.x * rightScale < (left.x + 1) * leftScale &&
    left.y * leftScale < (right.y + 1) * rightScale &&
    right.y * rightScale < (left.y + 1) * leftScale;
}

function priority(request: ReplayRequest, reverseMipDepth: boolean): number {
  const tier = request.priority >= QUALITY_TIER_BASE ? 1 : 0;
  const relative = request.priority % QUALITY_TIER_BASE;
  const channel = Math.floor(relative / CHANNEL_LANES);
  const rung = relative % CHANNEL_LANES;
  const qualityDepth = Math.floor(rung / 2);
  const centerBit = rung & 1;
  const depth = reverseMipDepth ? MAX_MIP - qualityDepth : qualityDepth;
  return tier * QUALITY_TIER_BASE + channel * CHANNEL_LANES + depth * 2 + centerBit;
}

/** Sensitivity model only: preserve observed admission opportunities and page
 * set, but choose another already-detected request for each opportunity. The
 * trace omits feedback refreshes and current fallback mip, so this cannot prove
 * a production scheduler result. */
export function rescheduleRequests(
  baseline: readonly ReplayRequest[],
  reverseMipDepth: boolean,
  channelAffinity: boolean,
): ReplayRequest[] {
  const tokens = baseline.map(request => request.timestamp).sort((left, right) => left - right);
  const remaining = [...baseline].sort((left, right) => left.detectedAt - right.detectedAt);
  const pending: ReplayRequest[] = [];
  const output: ReplayRequest[] = [];
  let remainingIndex = 0;
  let lastTimestamp = 0;
  let previous: ReplayRequest | undefined;

  for (const token of tokens) {
    let timestamp = Math.max(token, lastTimestamp);
    while (remainingIndex < remaining.length && remaining[remainingIndex].detectedAt <= timestamp)
      pending.push(remaining[remainingIndex++]);
    if (pending.length === 0 && remainingIndex < remaining.length) {
      timestamp = Math.max(timestamp, remaining[remainingIndex].detectedAt);
      while (remainingIndex < remaining.length && remaining[remainingIndex].detectedAt <= timestamp)
        pending.push(remaining[remainingIndex++]);
    }
    if (pending.length === 0) throw new Error('priority replay exhausted detected requests');
    lastTimestamp = timestamp;

    let selected = 0;
    let selectedPriority = priority(pending[0], reverseMipDepth);
    for (let index = 1; index < pending.length; index++) {
      const candidate = priority(pending[index], reverseMipDepth);
      if (candidate < selectedPriority) {
        selected = index;
        selectedPriority = candidate;
      }
    }
    if (channelAffinity && previous) {
      let affinity = -1;
      let affinityPriority = Number.POSITIVE_INFINITY;
      for (let index = 0; index < pending.length; index++) {
        const candidate = priority(pending[index], reverseMipDepth);
        if (candidate <= selectedPriority + CHANNEL_LANES &&
            virtualFootprintsOverlap(previous, pending[index]) && candidate < affinityPriority) {
          affinity = index;
          affinityPriority = candidate;
        }
      }
      if (affinity >= 0) selected = affinity;
    }
    const request = pending.splice(selected, 1)[0];
    const replayed = { ...request, timestamp };
    output.push(replayed);
    previous = replayed;
  }
  return output;
}

function percentile(values: readonly number[], fraction: number): number {
  if (values.length === 0) return 0;
  const sorted = [...values].sort((left, right) => left - right);
  return sorted[Math.floor((sorted.length - 1) * fraction)] ?? 0;
}

function summarizeVariant(variant: readonly ReplayRequest[], baseline: readonly ReplayRequest[]): ReplayVariantSummary {
  const batches = replayBatches(variant);
  const waits = variant.map(request => request.timestamp - request.detectedAt);
  const mean = waits.reduce((sum, value) => sum + value, 0) / Math.max(1, waits.length);
  return {
    batches: batches.length,
    meanSpans: variant.length / Math.max(1, batches.length),
    admissionMeanMs: mean / 1_000_000,
    admissionP95Ms: percentile(waits, 0.95) / 1_000_000,
    admissionP99Ms: percentile(waits, 0.99) / 1_000_000,
    admissionMaxMs: percentile(waits, 1) / 1_000_000,
    reorderedRequests: variant.reduce(
      (count, request, index) => count + (request.key === baseline[index]?.key ? 0 : 1), 0,
    ),
  };
}

export function analyzeDungeonVtReplay(
  traceName: string,
  traceBytes: Uint8Array,
  header: BigHeader,
): DungeonVtReplayReport {
  const records = readRecords(traceBytes);
  const extracted = extractOperations(records);
  const operations = buildRequests(extracted.bulk, extracted.admissions, buildDirectoryIndexes(header));
  const requests = operations.filter(request => request.dispatched);
  const batches = replayBatches(operations);
  let callerRuns = 0;
  let sortedRuns = 0;
  for (const batch of batches) {
    callerRuns += countSourceRuns(batch.requests, false);
    sortedRuns += countSourceRuns(batch.requests, true);
  }
  const current = rescheduleRequests(requests, false, false);
  const mipDeficit = rescheduleRequests(requests, true, false);
  const grouped = rescheduleRequests(requests, true, true);
  return {
    trace: traceName,
    successfulPageReads: requests.length,
    canceledBeforeDispatch: operations.length - requests.length,
    recordedBulkRequests: extracted.recordedDispatches,
    replayedBulkRequests: batches.length,
    meanSpansPerRequest: requests.length / Math.max(1, batches.length),
    callerOrderedSourceRuns: callerRuns,
    sourceSortedRuns: sortedRuns,
    sourceRunReductionPercent: callerRuns === 0 ? 0 : (callerRuns - sortedRuns) * 100 / callerRuns,
    sourceSortedBulkRequests: batches.length,
    prioritySensitivity: {
      caveat: 'Static sensitivity only: AGTB omits feedback refreshes and current resident fallback mip.',
      currentPriorityReschedule: summarizeVariant(current, requests),
      mipDeficitFirst: summarizeVariant(mipDeficit, requests),
      mipDeficitAndChannelAffinity: summarizeVariant(grouped, requests),
    },
    conclusion: sortedRuns < callerRuns
      ? 'Source sorting reduces underlying adjacent read runs but cannot reduce HTTP/bridge bulk-request count; priority/grouping sensitivity leaves modeled request count unchanged from its control.'
      : 'No tested replay transformation reduces the bulk-request count.',
  };
}

interface Options { trace: string; big: string; output?: string }

function parseOptions(args: readonly string[]): Options {
  let trace = '';
  let big = 'crates/afterglow-web/web/assets/dungeon.big';
  let output: string | undefined;
  for (let index = 0; index < args.length; index++) {
    const option = args[index];
    const value = args[index + 1];
    if (option === '--trace' && value) { trace = value; index++; }
    else if (option === '--big' && value) { big = value; index++; }
    else if (option === '--output' && value) { output = value; index++; }
    else throw new Error(`unknown or incomplete argument: ${option ?? '<missing>'}`);
  }
  if (!trace) throw new Error('--trace is required');
  return { trace, big, output };
}

async function readHeader(path: string): Promise<BigHeader> {
  const file = Bun.file(path);
  const prefix = new Uint8Array(await file.slice(0, 16).arrayBuffer());
  if (prefix.byteLength !== 16) throw new Error('BIG prefix is truncated');
  const dataOffset = Number(new DataView(prefix.buffer, prefix.byteOffset, prefix.byteLength).getBigUint64(8, true));
  const headerBytes = new Uint8Array(await file.slice(0, dataOffset).arrayBuffer());
  return parseBigHeader(headerBytes).header;
}

async function main(): Promise<void> {
  const options = parseOptions(Bun.argv.slice(2));
  const traceBytes = new Uint8Array(await Bun.file(options.trace).arrayBuffer());
  const report = analyzeDungeonVtReplay(options.trace, traceBytes, await readHeader(options.big));
  const json = JSON.stringify(report, null, 2) + '\n';
  if (options.output) await Bun.write(options.output, json);
  else process.stdout.write(json);
}

if (import.meta.main) await main();
