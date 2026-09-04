export const PAINT_MEMORY_BLOCK_MIB = 64;
export const PAINT_MEMORY_FALLBACK_MIB = 1024;
export const PAINT_MEMORY_MAX_MIB = 2048;
export const PAINT_HISTORY_MIB = 64;
export const PAINT_SAFETY_MIB = 128;
export const PAINT_TILE_WORKER_MIB = 8;
export const PAINT_TILE_BYTES = 64 * 64 * 4 * 2;
export const PAINT_INITIAL_TILES = 4096;
export const PAINT_GROW_TILES = PAINT_MEMORY_BLOCK_MIB * 1024 * 1024 / PAINT_TILE_BYTES;

export type PaintMemoryPolicy = {
  totalMiB: number;
  initialTiles: number;
  maximumTiles: number;
  growTiles: number;
};

function blockFloor(value: number): number {
  return Math.floor(value / PAINT_MEMORY_BLOCK_MIB) * PAINT_MEMORY_BLOCK_MIB;
}

export function paintMemoryLimitMiB(deviceMemoryGiB?: number, overrideMiB?: number): number {
  const detected = Number.isFinite(deviceMemoryGiB) && deviceMemoryGiB! > 0
    ? deviceMemoryGiB! * 1024 * 0.25
    : PAINT_MEMORY_FALLBACK_MIB;
  const selected = Number.isFinite(overrideMiB) && overrideMiB! > 0 ? overrideMiB! : detected;
  return Math.max(PAINT_MEMORY_BLOCK_MIB, Math.min(PAINT_MEMORY_MAX_MIB, blockFloor(selected)));
}

export function paintTileWorkerCount(logicalProcessors = 8): number {
  return Math.max(1, Math.min(16, Math.floor(logicalProcessors / 2) - 1));
}

export function paintMemoryPolicy(totalMiB: number, workerCount: number): PaintMemoryPolicy {
  const fixedMiB = PAINT_HISTORY_MIB + PAINT_SAFETY_MIB + workerCount * PAINT_TILE_WORKER_MIB;
  const tileMiB = totalMiB - fixedMiB;
  const maximumTiles = tileMiB < PAINT_MEMORY_BLOCK_MIB
    ? 0
    : Math.floor(tileMiB * 1024 * 1024 / PAINT_TILE_BYTES);
  return {
    totalMiB,
    initialTiles: Math.min(PAINT_INITIAL_TILES, maximumTiles),
    maximumTiles,
    growTiles: Math.min(PAINT_GROW_TILES, maximumTiles),
  };
}
