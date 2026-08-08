export const MAX_MIP = 10;
export const SCORE_COVERAGE_CAP = 255;
const MAX_PIXEL_PERCEPTUAL_WEIGHT = 15;
export const MAX_PERCEPTUAL_WEIGHT = SCORE_COVERAGE_CAP * MAX_PIXEL_PERCEPTUAL_WEIGHT;
export const MAX_PAGE_SCORE = SCORE_COVERAGE_CAP * (MAX_PIXEL_PERCEPTUAL_WEIGHT + 7);
const IMPORTANCE_LEVEL_MAX = 24;
const IMPORTANCE_BUCKET_COUNT = IMPORTANCE_LEVEL_MAX + 1;
const MAX_SCORE_EXPONENT = 31 - Math.clz32(MAX_PAGE_SCORE);
const TOP_SCORE_BASE = 1 << MAX_SCORE_EXPONENT;
const TOP_SCORE_SPLIT = TOP_SCORE_BASE + Math.ceil((MAX_PAGE_SCORE - TOP_SCORE_BASE + 1) / 2);
const FOCUS_IMPORTANCE_BUCKET_MAX = 12;
export const PAGE_KIND_PRIORITY_COUNT = 2;
export const MATERIAL_CHANNEL_PRIORITY_COUNT = 3;
export const PRIORITY_LANE_COUNT =
  IMPORTANCE_BUCKET_COUNT * PAGE_KIND_PRIORITY_COUNT * MATERIAL_CHANNEL_PRIORITY_COUNT;

export type PageBatchTier = 'urgent' | 'focus' | 'peripheral';

export interface PageRequest {
  mip: number;
  x: number;
  y: number;
  tail?: boolean;
  batchTier?: PageBatchTier;
  pinned?: boolean;
}

export interface VirtualPageRequest extends PageRequest {
  textureId?: number;
  path: string;
  screenPriority?: number;
  coverage?: number;
  perceptualWeight?: number;
  residentMipGap?: number;
  channelPriority?: number;
  priorityTier?: number;
}

export interface CachedPage extends VirtualPageRequest {
  cacheKey: number;
  pinned: boolean;
}

export function packedPageCoordinates(
  textureId: number,
  mip: number,
  x: number,
  y: number,
  tail = false,
): number {
  const local = tail
    ? 0x10000000
    : ((mip & 0x3f) | ((x & 0x7ff) << 6) | ((y & 0x7ff) << 17)) >>> 0;
  return textureId * 0x20000000 + local;
}

export function packedPageIdentity(textureId: number, request: PageRequest): number {
  return packedPageCoordinates(
    textureId,
    request.mip,
    request.x,
    request.y,
    request.tail,
  );
}

/** Fixed two-buckets-per-octave perceptual importance; zero is highest. */
export function perceptualImportanceBucket(weight: number): number {
  const bounded = Math.max(1, Math.min(MAX_PAGE_SCORE, Math.floor(weight)));
  const exponent = 31 - Math.clz32(bounded);
  let level: number;
  if (exponent === 0) level = 0;
  else if (exponent < MAX_SCORE_EXPONENT)
    level = exponent * 2 - 1 + ((bounded >>> (exponent - 1)) & 1);
  else level = bounded < TOP_SCORE_SPLIT ? IMPORTANCE_LEVEL_MAX - 1 : IMPORTANCE_LEVEL_MAX;
  return IMPORTANCE_LEVEL_MAX - level;
}

export function sourcePerceptualWeight(source: VirtualPageRequest): number {
  const coverage = Math.min(SCORE_COVERAGE_CAP, source.coverage ?? 1);
  const centerCloseness = 7 - Math.min(7, (source.screenPriority ?? 255) >>> 5);
  return Math.min(
    MAX_PERCEPTUAL_WEIGHT,
    source.perceptualWeight ?? coverage * (1 + centerCloseness),
  );
}

export function perceptualPriority(
  perceptualWeight: number,
  coverage: number,
  residentMipGap: number,
  parent: boolean,
  channelPriority: number,
): number {
  const pageWeight = Math.min(
    MAX_PAGE_SCORE,
    perceptualWeight + Math.min(SCORE_COVERAGE_CAP, coverage) * residentMipGap,
  );
  const bucket = perceptualImportanceBucket(pageWeight);
  const pageKind = parent ? 0 : 1;
  return (bucket * PAGE_KIND_PRIORITY_COUNT + pageKind) *
    MATERIAL_CHANNEL_PRIORITY_COUNT + channelPriority;
}

export function pageBatchTier(parent: boolean, priority: number): PageBatchTier {
  if (parent) return 'urgent';
  const importanceBucket = Math.floor(
    priority / (PAGE_KIND_PRIORITY_COUNT * MATERIAL_CHANNEL_PRIORITY_COUNT),
  );
  return importanceBucket <= FOCUS_IMPORTANCE_BUCKET_MAX ? 'focus' : 'peripheral';
}
