// DirtySlotRanges — tracks which instance slots are dirty and coalesces
// upload ranges to avoid thousands of tiny GPU writeBuffer calls.
//
// Uses a bitset (Uint32Array) to track dirty slots. On flush, walks the
// bitset to build contiguous ranges. If fragmentation is high (>25% dirty
// or >maxRanges), falls back to one bounding range.

import * as THREE from 'three/webgpu';
import { NONE_U32 } from '../core/types.ts';

interface UpdateRange {
  start: number;
  count: number;
}

export class DirtySlotRanges {
  private readonly bits: Uint32Array;
  private readonly rangePool: UpdateRange[];
  private dirtyCount = 0;
  private minimumSlot = NONE_U32;
  private maximumSlot = 0;

  constructor(
    capacity: number,
    private readonly maximumRanges = 16,
  ) {
    this.bits = new Uint32Array((capacity + 31) >>> 5);
    this.rangePool = Array.from({ length: maximumRanges }, () => ({ start: 0, count: 0 }));
  }

  // @hot-no-alloc-begin DirtySlotRanges.mark
  mark(slot: number): void {
    const wordIndex = slot >>> 5;
    const bit = 1 << (slot & 31);
    const previous = this.bits[wordIndex];
    if ((previous & bit) !== 0) return;
    this.bits[wordIndex] = previous | bit;
    this.dirtyCount++;
    if (slot < this.minimumSlot) this.minimumSlot = slot;
    if (slot > this.maximumSlot) this.maximumSlot = slot;
  }
  // @hot-no-alloc-end DirtySlotRanges.mark

  // @hot-no-alloc-begin DirtySlotRanges.flush
  flush(attribute: THREE.BufferAttribute, stride: number, activeCount: number): void {
    if (this.dirtyCount === 0 || activeCount === 0) return;

    const updateRanges = attribute.updateRanges as UpdateRange[];
    updateRanges.length = 0;

    const dirtyRatio = this.dirtyCount / activeCount;

    if (dirtyRatio >= 0.25) {
      // High dirty ratio — one bounding range.
      const range = this.rangePool[0];
      range.start = 0;
      range.count = activeCount * stride;
      updateRanges[0] = range;
    } else {
      // Walk the bitset, building contiguous ranges.
      let rangeCount = 0;
      let slot = this.minimumSlot;

      while (slot <= this.maximumSlot) {
        // Skip clean slots.
        while (slot <= this.maximumSlot && !this.isDirty(slot)) slot++;
        if (slot > this.maximumSlot) break;

        const runStart = slot++;
        // Extend while dirty.
        while (slot <= this.maximumSlot && this.isDirty(slot)) slot++;

        if (rangeCount >= this.maximumRanges) {
          // Too fragmented — fall back to one bounding range.
          const range = this.rangePool[0];
          range.start = this.minimumSlot * stride;
          range.count = (this.maximumSlot - this.minimumSlot + 1) * stride;
          updateRanges.length = 1;
          updateRanges[0] = range;
          rangeCount = 1;
          break;
        }

        const range = this.rangePool[rangeCount];
        range.start = runStart * stride;
        range.count = (slot - runStart) * stride;
        updateRanges[rangeCount++] = range;
      }
    }

    attribute.needsUpdate = true;

    // Clear dirty bits.
    const firstWord = this.minimumSlot >>> 5;
    const lastWord = this.maximumSlot >>> 5;
    this.bits.fill(0, firstWord, lastWord + 1);
    this.dirtyCount = 0;
    this.minimumSlot = NONE_U32;
    this.maximumSlot = 0;
  }
  // @hot-no-alloc-end DirtySlotRanges.flush

  private isDirty(slot: number): boolean {
    return (this.bits[slot >>> 5] & (1 << (slot & 31))) !== 0;
  }
}
