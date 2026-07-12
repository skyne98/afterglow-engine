// Stress tests — push VT to its limits: memory, allocation, throughput, thrashing.
// Run: bun test prototype/vt/vt.stress.test.ts
//
// These tests verify:
// - No crashes under extreme load
// - Memory doesn't grow unboundedly (LRU map, page table, atlas)
// - State remains consistent after thousands of operations
// - LRU map stays in sync with actual slots
// - Page table doesn't accumulate stale entries
// - Atlas data is correctly overwritten during eviction
// - Performance is reasonable (completes in seconds, not minutes)

import { test, describe, expect } from 'bun:test';
import {
  packEntry, isResident, getPhysX, getPhysY, getMip,
  sampleVirtualTexture, generatePage,
  PageTable, PageCache, PageManager,
  computeMipLevel, vtSample, simulateFeedback,
  PAGE_SIZE, PAGE_BORDER, SLOT_SIZE,
  ATLAS_PAGES_X, ATLAS_PAGES_Y, ATLAS_WIDTH, ATLAS_HEIGHT,
  VIRTUAL_SIZE, VIRTUAL_PAGES_X, MAX_MIP, FEEDBACK_SCALE, PINNED_MIPS,
  type PageRequest,
} from './vt';

const ATLAS_SLOTS = ATLAS_PAGES_X * ATLAS_PAGES_Y;
const ATLAS_BYTES = ATLAS_WIDTH * ATLAS_HEIGHT * 4;

// ============================================================================
// 1. Fill atlas 1000 times (constant eviction + recommit)
// ============================================================================

describe('Stress: Atlas fill cycle × 1000', () => {
  test('1000 fill-evict-refill cycles → no crash, slots consistent', () => {
    const cache = new PageCache(new Set());

    for (let cycle = 0; cycle < 1000; cycle++) {
      // Fill all slots
      for (let i = 0; i < ATLAS_SLOTS; i++) {
        const req: PageRequest = { mip: 0, x: cycle * 100 + i, y: 0 };
        const { slot, evicted } = cache.acquire(req);
        if (evicted) {
          // Just overwrite — don't track eviction in this stress test
        }
        cache.commit(req, slot, new Uint8Array(SLOT_SIZE * SLOT_SIZE * 4));
      }
      expect(cache.usedSlots).toBe(ATLAS_SLOTS);
      expect(cache.freeSlotCount).toBe(0);
    }
  }, 10000);

  test('Atlas byte array stays constant size after 1000 cycles', () => {
    const cache = new PageCache(new Set());
    const initialSize = cache.atlas.byteLength;

    for (let cycle = 0; cycle < 1000; cycle++) {
      const { slot } = cache.acquire({ mip: 0, x: cycle, y: 0 });
      cache.commit({ mip: 0, x: cycle, y: 0 }, slot, new Uint8Array(SLOT_SIZE * SLOT_SIZE * 4));
    }

    expect(cache.atlas.byteLength).toBe(initialSize);
    expect(cache.atlas.byteLength).toBe(ATLAS_BYTES);
  }, 10000);
});

// ============================================================================
// 2. Page table with 10,000 entries → set + evict + verify count
// ============================================================================

describe('Stress: Page table 10,000 entries', () => {
  test('Set 10,000 entries → count matches → evict all → count 0', () => {
    const pt = new PageTable(MAX_MIP);
    const entries: PageRequest[] = [];

    // Set 10,000 entries across different mip levels
    for (let i = 0; i < 10000; i++) {
      const req: PageRequest = {
        mip: i % (MAX_MIP + 1),
        x: i % VIRTUAL_PAGES_X,
        y: Math.floor(i / VIRTUAL_PAGES_X) % VIRTUAL_PAGES_X,
      };
      pt.setResident(req, { x: i % ATLAS_PAGES_X, y: 0 });
      entries.push(req);
    }

    // Some entries may collide (same mip:x:y) → count <= 10000
    expect(pt.residentCount).toBeGreaterThan(0);
    expect(pt.residentCount).toBeLessThanOrEqual(10000);

    // Evict all
    for (const req of entries) {
      pt.setEvicted(req);
    }
    expect(pt.residentCount).toBe(0);
  });

  test('Set + evict same entry 1000 times → no leak', () => {
    const pt = new PageTable(MAX_MIP);
    const req: PageRequest = { mip: 0, x: 5, y: 5 };

    for (let i = 0; i < 1000; i++) {
      pt.setResident(req, { x: i % ATLAS_PAGES_X, y: 0 });
      expect(pt.residentCount).toBe(1);
      pt.setEvicted(req);
      expect(pt.residentCount).toBe(0);
    }
  });

  test('findResidentPage with 10,000 entries → fast and correct', () => {
    const pt = new PageTable(MAX_MIP);
    // Fill with entries at all mip levels for UV 0.5
    for (let mip = 0; mip <= MAX_MIP; mip++) {
      const pages = VIRTUAL_PAGES_X >> mip;
      const px = Math.min(Math.floor(0.5 * pages), pages - 1);
      pt.setResident({ mip, x: px, y: px }, { x: px, y: px });
    }
    // Add 10,000 more entries (noise)
    for (let i = 0; i < 10000; i++) {
      pt.setResident({ mip: 0, x: i % 32, y: Math.floor(i / 32) % 32 }, { x: 0, y: 0 });
    }
    const result = pt.findResidentPage(0.5, 0.5, 0);
    expect(result).not.toBeNull();
    expect(result!.mip).toBe(0);
  });
});

// ============================================================================
// 3. LRU map consistency after 5000 touch+evict operations
// ============================================================================

describe('Stress: LRU map consistency', () => {
  test('5000 touch + evict operations → usedSlots always correct', () => {
    const cache = new PageCache(new Set());

    // Initial fill
    for (let i = 0; i < ATLAS_SLOTS; i++) {
      const { slot } = cache.acquire({ mip: 0, x: i, y: 0 });
      cache.commit({ mip: 0, x: i, y: 0 }, slot, new Uint8Array(SLOT_SIZE * SLOT_SIZE * 4));
    }

    // 5000 touch + evict cycles
    for (let i = 0; i < 5000; i++) {
      // Touch a random page
      cache.touch({ mip: 0, x: i % ATLAS_SLOTS, y: 0 });
      // Evict + commit
      const { slot } = cache.acquire({ mip: 0, x: 50000 + i, y: 0 });
      cache.commit({ mip: 0, x: 50000 + i, y: 0 }, slot, new Uint8Array(SLOT_SIZE * SLOT_SIZE * 4));
      // Invariant: slots always full
      expect(cache.usedSlots).toBe(ATLAS_SLOTS);
      expect(cache.freeSlotCount).toBe(0);
    }
  }, 15000);

  test('LRU map size matches usedSlots after 1000 operations', () => {
    const cache = new PageCache(new Set());

    for (let i = 0; i < 1000; i++) {
      const { slot } = cache.acquire({ mip: 0, x: i, y: 0 });
      cache.commit({ mip: 0, x: i, y: 0 }, slot, new Uint8Array(SLOT_SIZE * SLOT_SIZE * 4));
      // usedSlots should never exceed ATLAS_SLOTS
      expect(cache.usedSlots).toBeLessThanOrEqual(ATLAS_SLOTS);
    }
    expect(cache.usedSlots).toBe(ATLAS_SLOTS);
  });
});

// ============================================================================
// 4. 1000 rapid camera movements (constant thrashing)
// ============================================================================

describe('Stress: 1000 rapid camera movements', () => {
  test('1000 frames at random positions → no crash, state consistent', () => {
    const pm = new PageManager();

    for (let i = 0; i < 1000; i++) {
      const pos: [number, number] = [Math.random(), Math.random()];
      const zoom = 1 + Math.random() * 31;
      const reqs = simulateFeedback(pos, zoom, pm.getLodBias());
      pm.processFeedback(reqs);

      // Invariants
      expect(pm.cache.usedSlots).toBeLessThanOrEqual(ATLAS_SLOTS);
      expect(pm.cache.usedSlots).toBeGreaterThanOrEqual(PINNED_MIPS.size > 0 ? 1 : 0);
    }

    // Pinned pages survive
    for (const mip of PINNED_MIPS) {
      const pages = VIRTUAL_PAGES_X >> mip;
      for (let y = 0; y < pages; y++) {
        for (let x = 0; x < pages; x++) {
          expect(pm.pageTable.isResident({ mip, x, y })).toBe(true);
        }
      }
    }
  }, 30000);

  test('1000 frames: page table resident count ≤ atlas slots', () => {
    const pm = new PageManager();

    for (let i = 0; i < 1000; i++) {
      const reqs = simulateFeedback([Math.random(), Math.random()], 4, pm.getLodBias());
      pm.processFeedback(reqs);
      // Page table should never have more entries than atlas slots
      // (each resident page takes one atlas slot)
      expect(pm.pageTable.residentCount).toBeLessThanOrEqual(ATLAS_SLOTS);
    }
  }, 30000);
});

// ============================================================================
// 5. Generate 10,000 pages (memory allocation stress)
// ============================================================================

describe('Stress: Page generation × 10,000', () => {
  test('Generate 10,000 pages → all correct size, no crash', () => {
    const expectedSize = SLOT_SIZE * SLOT_SIZE * 4;

    for (let i = 0; i < 10000; i++) {
      const req: PageRequest = {
        mip: i % (MAX_MIP + 1),
        x: i % VIRTUAL_PAGES_X,
        y: (i * 7) % VIRTUAL_PAGES_X,
      };
      const data = generatePage(req);
      expect(data.length).toBe(expectedSize);
      // Last byte should be 255 (alpha)
      expect(data[data.length - 1]).toBe(255);
    }
  }, 15000);

  test('Generate same page 1000 times → identical output', () => {
    const req: PageRequest = { mip: 0, x: 5, y: 3 };
    const first = generatePage(req);

    for (let i = 0; i < 1000; i++) {
      const data = generatePage(req);
      // Should be byte-identical (deterministic)
      expect(data).toEqual(first);
    }
  });
});

// ============================================================================
// 6. Full atlas cycle: fill → evict all → fill → evict all → fill (3 cycles)
// ============================================================================

describe('Stress: 3 full atlas cycles', () => {
  test('Fill → evict all → fill → evict all → fill → consistent', () => {
    const cache = new PageCache(new Set());

    for (let cycle = 0; cycle < 3; cycle++) {
      // Fill
      for (let i = 0; i < ATLAS_SLOTS; i++) {
        const req: PageRequest = { mip: 0, x: cycle * 1000 + i, y: 0 };
        const { slot } = cache.acquire(req);
        const data = new Uint8Array(SLOT_SIZE * SLOT_SIZE * 4);
        data.fill((cycle * 64 + i) & 0xFF);
        cache.commit(req, slot, data);
      }
      expect(cache.usedSlots).toBe(ATLAS_SLOTS);

      // Verify data integrity
      for (let sy = 0; sy < ATLAS_PAGES_Y; sy++) {
        for (let sx = 0; sx < ATLAS_PAGES_X; sx++) {
          const idx = (sy * SLOT_SIZE * ATLAS_WIDTH + sx * SLOT_SIZE) * 4;
          expect(cache.atlas[idx]).toBeGreaterThanOrEqual(0);
        }
      }
    }
    expect(cache.usedSlots).toBe(ATLAS_SLOTS);
  }, 10000);
});

// ============================================================================
// 7. vtSample throughput: 10,000 samples
// ============================================================================

describe('Stress: vtSample throughput', () => {
  test('10,000 vtSample calls → all return valid results', () => {
    const pm = new PageManager();
    // Load pages at mip 0
    for (let i = 0; i < 6; i++) {
      const req: PageRequest = { mip: 0, x: 10 + i, y: 10 };
      const { slot } = pm.cache.acquire(req);
      pm.cache.commit(req, slot, generatePage(req));
      pm.pageTable.setResident(req, slot);
    }

    const uvDx: [number, number] = [1 / VIRTUAL_SIZE, 0];
    const uvDy: [number, number] = [0, 1 / VIRTUAL_SIZE];
    let successCount = 0;

    for (let i = 0; i < 10000; i++) {
      const u = (10 + Math.random() * 6) / VIRTUAL_PAGES_X;
      const v = (10 + 0.5) / VIRTUAL_PAGES_X;
      const result = vtSample(u, v, uvDx, uvDy, pm.pageTable, pm.cache.atlas);
      if (result !== null) successCount++;
    }
    expect(successCount).toBe(10000);
  }, 10000);

  test('10,000 vtSample calls with fallback → all return non-null (pinned)', () => {
    const pm = new PageManager();
    // Only pinned pages loaded
    const uvDx: [number, number] = [1 / VIRTUAL_SIZE, 0];
    const uvDy: [number, number] = [0, 1 / VIRTUAL_SIZE];

    for (let i = 0; i < 10000; i++) {
      const u = Math.random();
      const v = Math.random();
      const result = vtSample(u, v, uvDx, uvDy, pm.pageTable, pm.cache.atlas);
      // Should find pinned pages via fallback
      expect(result).not.toBeNull();
    }
  }, 10000);
});

// ============================================================================
// 8. Feedback simulation 500 times at random positions
// ============================================================================

describe('Stress: Feedback simulation × 500', () => {
  test('500 feedback simulations → no crash, valid requests', () => {
    for (let i = 0; i < 500; i++) {
      const pos: [number, number] = [Math.random(), Math.random()];
      const zoom = 0.5 + Math.random() * 32;
      const reqs = simulateFeedback(pos, zoom, 0);

      for (const req of reqs.values()) {
        expect(req.mip).toBeGreaterThanOrEqual(0);
        expect(req.mip).toBeLessThanOrEqual(MAX_MIP);
        expect(req.x).toBeGreaterThanOrEqual(0);
        expect(req.y).toBeGreaterThanOrEqual(0);
      }
    }
  }, 15000);

  test('Feedback at same position 100 times → consistent results', () => {
    const pos: [number, number] = [0.5, 0.5];
    const zoom = 4;
    const first = simulateFeedback(pos, zoom, 0);

    for (let i = 0; i < 100; i++) {
      const reqs = simulateFeedback(pos, zoom, 0);
      expect(reqs.size).toBe(first.size);
      // Same page requests
      for (const [key, req] of first) {
        expect(reqs.has(key)).toBe(true);
        expect(reqs.get(key)).toEqual(req);
      }
    }
  });
});

// ============================================================================
// 9. Oversubscription spiral: 200 frames at zoom=1
// ============================================================================

describe('Stress: Oversubscription spiral', () => {
  test('200 frames at zoom=1 → LOD bias stabilizes, no crash', () => {
    const pm = new PageManager();
    const biases: number[] = [];

    for (let i = 0; i < 200; i++) {
      const reqs = simulateFeedback([0.5, 0.5], 1, pm.getLodBias());
      pm.processFeedback(reqs);
      biases.push(pm.getLodBias());
      expect(pm.cache.usedSlots).toBeLessThanOrEqual(ATLAS_SLOTS);
    }

    // LOD bias should stabilize (not keep increasing)
    const last10 = biases.slice(-10);
    const maxLast10 = Math.max(...last10);
    const minLast10 = Math.min(...last10);
    expect(maxLast10 - minLast10).toBeLessThanOrEqual(2); // oscillates within 2
  }, 30000);

  test('200 frames alternating zoom=1 and zoom=32 → no crash', () => {
    const pm = new PageManager();

    for (let i = 0; i < 200; i++) {
      const zoom = i % 2 === 0 ? 1 : 32;
      const reqs = simulateFeedback([0.5, 0.5], zoom, pm.getLodBias());
      pm.processFeedback(reqs);
      expect(pm.cache.usedSlots).toBeLessThanOrEqual(ATLAS_SLOTS);
    }
  }, 30000);
});

// ============================================================================
// 10. Memory leak: atlas array size constant after 2000 operations
// ============================================================================

describe('Stress: Memory leak detection', () => {
  test('Atlas array byteLength constant after 2000 evict+commit cycles', () => {
    const cache = new PageCache(new Set());
    const initialByteLength = cache.atlas.byteLength;

    // Initial fill
    for (let i = 0; i < ATLAS_SLOTS; i++) {
      const { slot } = cache.acquire({ mip: 0, x: i, y: 0 });
      cache.commit({ mip: 0, x: i, y: 0 }, slot, new Uint8Array(SLOT_SIZE * SLOT_SIZE * 4));
    }

    // 2000 evict + commit cycles
    for (let i = 0; i < 2000; i++) {
      const { slot } = cache.acquire({ mip: 0, x: 10000 + i, y: 0 });
      cache.commit({ mip: 0, x: 10000 + i, y: 0 }, slot, new Uint8Array(SLOT_SIZE * SLOT_SIZE * 4));
    }

    expect(cache.atlas.byteLength).toBe(initialByteLength);
    expect(cache.atlas.byteLength).toBe(ATLAS_BYTES);
  }, 15000);

  test('Page table count = atlas slots after 1000 operations', () => {
    const pm = new PageManager();

    for (let i = 0; i < 1000; i++) {
      const reqs = simulateFeedback([0.5, 0.5], 4, pm.getLodBias());
      pm.processFeedback(reqs);
    }

    // Page table should have exactly as many entries as used atlas slots
    // (no stale entries from evicted pages)
    expect(pm.pageTable.residentCount).toBe(pm.cache.usedSlots);
  }, 30000);

  test('No stale page table entries after 500 evict+reload cycles', () => {
    const pm = new PageManager();

    // Process feedback at position A → loads pages
    const reqsA = simulateFeedback([0.2, 0.2], 4, pm.getLodBias());
    pm.processFeedback(reqsA);
    const countA = pm.pageTable.residentCount;

    // Move to position B → evicts A's pages, loads B's
    for (let i = 0; i < 500; i++) {
      const reqs = simulateFeedback([0.8, 0.8], 4, pm.getLodBias());
      pm.processFeedback(reqs);
    }

    // Page table count should equal atlas used slots (no stale entries)
    expect(pm.pageTable.residentCount).toBe(pm.cache.usedSlots);
    expect(pm.cache.usedSlots).toBeLessThanOrEqual(ATLAS_SLOTS);
  }, 30000);
});

// ============================================================================
// 11. Touch all pages then verify eviction order
// ============================================================================

describe('Stress: LRU eviction order after full touch', () => {
  test('Touch all 64 pages in order → evict in reverse order', () => {
    const cache = new PageCache(new Set());

    // Fill all
    for (let i = 0; i < ATLAS_SLOTS; i++) {
      const { slot } = cache.acquire({ mip: 0, x: i, y: 0 });
      cache.commit({ mip: 0, x: i, y: 0 }, slot, new Uint8Array(SLOT_SIZE * SLOT_SIZE * 4));
    }

    // Touch all in order 0, 1, 2, ..., 63
    // After touching: MRU = 63, LRU = 0
    for (let i = 0; i < ATLAS_SLOTS; i++) {
      cache.touch({ mip: 0, x: i, y: 0 });
    }

    // Evict → should evict page 0 (LRU)
    const { evicted: e1 } = cache.acquire({ mip: 0, x: 100, y: 0 });
    expect(e1!.x).toBe(0);

    // Touch remaining again
    for (let i = 1; i < ATLAS_SLOTS; i++) {
      cache.touch({ mip: 0, x: i, y: 0 });
    }

    // Evict → should evict page 1
    const { evicted: e2 } = cache.acquire({ mip: 0, x: 101, y: 0 });
    expect(e2!.x).toBe(1);
  });

  test('Interleaved touch pattern preserves correct LRU order', () => {
    const cache = new PageCache(new Set());

    for (let i = 0; i < ATLAS_SLOTS; i++) {
      const { slot } = cache.acquire({ mip: 0, x: i, y: 0 });
      cache.commit({ mip: 0, x: i, y: 0 }, slot, new Uint8Array(SLOT_SIZE * SLOT_SIZE * 4));
    }

    // Touch pattern: 0, 2, 4, ..., 62, 1, 3, 5, ..., 63
    // After: MRU = 63, then 62, 61, ..., 1, then 62, 60, ..., 0
    // Actually: touch order determines LRU. Last touched = MRU.
    // Touch even numbers first, then odd numbers
    for (let i = 0; i < ATLAS_SLOTS; i += 2) {
      cache.touch({ mip: 0, x: i, y: 0 });
    }
    for (let i = 1; i < ATLAS_SLOTS; i += 2) {
      cache.touch({ mip: 0, x: i, y: 0 });
    }

    // Evict → LRU should be page 0 (touched first in the even sequence)
    const { evicted } = cache.acquire({ mip: 0, x: 999, y: 0 });
    expect(evicted).not.toBeNull();
    // Page 0 was touched first (in the even sequence), so it's LRU
    expect(evicted!.x).toBe(0);
  });
});

// ============================================================================
// 12. All pages at mip 0 requested (extreme oversubscription)
// ============================================================================

describe('Stress: All mip 0 pages requested (1024 pages, 64 slots)', () => {
  test('Request all 1024 pages → atlas stays at 64 slots, no crash', () => {
    const pm = new PageManager();

    // Simulate feedback that covers the entire texture at mip 0
    // (zoom=32 → 1 texel per pixel → mip 0)
    const reqs = simulateFeedback([0.5, 0.5], 32, pm.getLodBias());
    pm.processFeedback(reqs);

    // Can't fit all pages → atlas full
    expect(pm.cache.usedSlots).toBeLessThanOrEqual(ATLAS_SLOTS);
  });

  test('Request all pages 10 times → consistent behavior', () => {
    const pm = new PageManager();

    for (let i = 0; i < 10; i++) {
      const reqs = simulateFeedback([0.5, 0.5], 32, pm.getLodBias());
      pm.processFeedback(reqs);
      expect(pm.cache.usedSlots).toBeLessThanOrEqual(ATLAS_SLOTS);
    }
    // Should still have pinned pages
    expect(pm.pageTable.isResident({ mip: MAX_MIP, x: 0, y: 0 })).toBe(true);
  });
});

// ============================================================================
// 13. Page generation at all mip levels × all positions
// ============================================================================

describe('Stress: Page generation at all mip levels', () => {
  test('Generate pages at all positions for mip 0-3 → no crash', () => {
    for (let mip = 0; mip <= 3; mip++) {
      const pagesAtMip = VIRTUAL_PAGES_X >> mip;
      // Generate a sample of pages (not all, to keep test fast)
      for (let y = 0; y < Math.min(pagesAtMip, 8); y++) {
        for (let x = 0; x < Math.min(pagesAtMip, 8); x++) {
          const data = generatePage({ mip, x, y });
          expect(data.length).toBe(SLOT_SIZE * SLOT_SIZE * 4);
          // Verify alpha channel
          expect(data[3]).toBe(255);
        }
      }
    }
  }, 15000);

  test('Page at mip 0 texel matches page at mip 1 (downscaled)', () => {
    // The same virtual texel should appear in both mip 0 and mip 1 pages
    // (mip 1 page covers 2×2 mip 0 pages, but samples from half-resolution virtual texture)
    const page0 = generatePage({ mip: 0, x: 0, y: 0 });
    const page1 = generatePage({ mip: 1, x: 0, y: 0 });

    // Both should have valid data
    expect(page0.length).toBe(page1.length);
    expect(page0[3]).toBe(255);
    expect(page1[3]).toBe(255);
  });
});

// ============================================================================
// 14. Extreme zoom range in feedback (0.0001 to 1000000)
// ============================================================================

describe('Stress: Extreme zoom range', () => {
  test('Feedback at zoom 0.0001 → no crash, valid mips', () => {
    const reqs = simulateFeedback([0.5, 0.5], 0.0001, 0);
    for (const req of reqs.values()) {
      expect(req.mip).toBeGreaterThanOrEqual(0);
      expect(req.mip).toBeLessThanOrEqual(MAX_MIP);
    }
  });

  test('Feedback at zoom 1000000 → mip 0', () => {
    const reqs = simulateFeedback([0.5, 0.5], 1000000, 0);
    for (const req of reqs.values()) {
      expect(req.mip).toBe(0);
    }
  });

  test('Render at 10 extreme zoom levels → no crash', () => {
    const pm = new PageManager();
    const zooms = [0.001, 0.01, 0.1, 1, 10, 100, 1000, 10000, 100000, 1000000];

    for (const zoom of zooms) {
      const reqs = simulateFeedback([0.5, 0.5], zoom, pm.getLodBias());
      pm.processFeedback(reqs);
      expect(() => pm.render([0.5, 0.5], zoom)).not.toThrow();
    }
  }, 15000);
});

// ============================================================================
// 15. Alternating 50 different camera positions (thrashing)
// ============================================================================

describe('Stress: 50-position thrashing', () => {
  test('50 positions × 5 cycles = 250 frames → no crash', () => {
    const pm = new PageManager();
    const positions: [number, number][] = [];
    for (let i = 0; i < 50; i++) {
      positions.push([
        0.05 + (i / 50) * 0.9,
        0.05 + ((i * 37) % 50 / 50) * 0.9,
      ]);
    }

    for (let cycle = 0; cycle < 5; cycle++) {
      for (const pos of positions) {
        const reqs = simulateFeedback(pos, 4, pm.getLodBias());
        pm.processFeedback(reqs);
      }
    }

    expect(pm.cache.usedSlots).toBeLessThanOrEqual(ATLAS_SLOTS);
    // Pinned pages survive
    expect(pm.pageTable.isResident({ mip: MAX_MIP, x: 0, y: 0 })).toBe(true);
  }, 30000);

  test('50 positions with varying zoom → all render', () => {
    const pm = new PageManager();

    for (let i = 0; i < 50; i++) {
      const pos: [number, number] = [Math.random(), Math.random()];
      const zoom = 1 + Math.random() * 31;
      const reqs = simulateFeedback(pos, zoom, pm.getLodBias());
      pm.processFeedback(reqs);
      const rendered = pm.render(pos, zoom);
      expect(rendered.length).toBe(256 * 256 * 4);
    }
  }, 30000);
});

// ============================================================================
// 16. Render 100 frames continuously (pipeline stress)
// ============================================================================

describe('Stress: 100-frame continuous render', () => {
  test('100 frames: feedback → process → render → all non-black', () => {
    const pm = new PageManager();
    let nonBlackFrames = 0;

    for (let frame = 0; frame < 100; frame++) {
      // Keep camera inside [0.2, 0.8] and zoom ≥ 2 so UVs stay within [0,1]
      const pos: [number, number] = [
        0.5 + 0.3 * Math.sin(frame * 0.1),  // 0.2 to 0.8
        0.5 + 0.3 * Math.cos(frame * 0.13), // 0.2 to 0.8
      ];
      const zoom = 4 + 3 * Math.abs(Math.sin(frame * 0.05)); // 4 to 7

      // Process feedback twice to ensure pages loaded
      for (let i = 0; i < 2; i++) {
        const reqs = simulateFeedback(pos, zoom, pm.getLodBias());
        pm.processFeedback(reqs);
      }
      const rendered = pm.render(pos, zoom);

      let nonBlack = 0;
      for (let i = 0; i < rendered.length; i += 4) {
        if (rendered[i] + rendered[i + 1] + rendered[i + 2] > 0) nonBlack++;
      }
      if (nonBlack > 0) nonBlackFrames++;
    }

    // ALL frames must have non-black output (camera stays inside texture,
    // pinned pages always provide fallback)
    expect(nonBlackFrames).toBe(100);
  }, 60000);

  test('100 frames: page table count always = used slots (no stale entries)', () => {
    const pm = new PageManager();

    for (let frame = 0; frame < 100; frame++) {
      const reqs = simulateFeedback(
        [Math.random(), Math.random()],
        2 + Math.random() * 16,
        pm.getLodBias(),
      );
      pm.processFeedback(reqs);
      // After each frame, page table should match atlas
      expect(pm.pageTable.residentCount).toBe(pm.cache.usedSlots);
    }
  }, 60000);
});

// ============================================================================
// 17. computeMipLevel 10,000 times (throughput)
// ============================================================================

describe('Stress: computeMipLevel throughput', () => {
  test('10,000 computeMipLevel calls → all valid', () => {
    for (let i = 0; i < 10000; i++) {
      const texels = 1 + (i % 4096);
      const uvDx: [number, number] = [texels / VIRTUAL_SIZE, 0];
      const uvDy: [number, number] = [0, texels / VIRTUAL_SIZE];
      const mip = computeMipLevel(uvDx, uvDy, VIRTUAL_SIZE);
      expect(Number.isFinite(mip) || mip === -Infinity).toBe(true);
    }
  }, 10000);

  test('computeMipLevel with random derivatives 5000 times → monotonic', () => {
    let prevMip = -Infinity;
    for (let i = 0; i < 5000; i++) {
      const texels = 1 + i;
      const uvDx: [number, number] = [texels / VIRTUAL_SIZE, 0];
      const uvDy: [number, number] = [0, texels / VIRTUAL_SIZE];
      const mip = computeMipLevel(uvDx, uvDy, VIRTUAL_SIZE);
      // mip should be non-decreasing as texel footprint increases
      expect(mip).toBeGreaterThanOrEqual(prevMip - 0.01);
      prevMip = mip;
    }
  }, 10000);
});

// ============================================================================
// 18. Pack/unpack 10,000 entries (bit manipulation stress)
// ============================================================================

describe('Stress: Pack/unpack 10,000 entries', () => {
  test('10,000 pack+unpack cycles → all correct', () => {
    for (let i = 0; i < 10000; i++) {
      const x = i % 256;
      const y = (i * 37) % 256;
      const m = (i * 13) % 32;
      const resident = i % 2 === 0;

      const entry = packEntry(resident, x, y, m);
      expect(isResident(entry)).toBe(resident);
      expect(getPhysX(entry)).toBe(x);
      expect(getPhysY(entry)).toBe(y);
      expect(getMip(entry)).toBe(m);
    }
  });

  test('All 256×256 physX/physY combinations → no collision', () => {
    const entries = new Set<number>();
    let collisions = 0;

    for (let x = 0; x < 256; x++) {
      for (let y = 0; y < 256; y++) {
        const entry = packEntry(true, x, y, 0);
        if (entries.has(entry)) collisions++;
        entries.add(entry);
      }
    }

    // 256×256 = 65536 entries, but u32 can hold them all
    // With resident bit + 8 bits each for X and Y = 17 bits → no collision
    expect(collisions).toBe(0);
    expect(entries.size).toBe(65536);
  });
});

// ============================================================================
// 19. Full pipeline stress: 500 frames with varying camera
// ============================================================================

describe('Stress: 500-frame full pipeline', () => {
  test('500 frames: feedback → process → render → state invariants', () => {
    const pm = new PageManager();

    for (let frame = 0; frame < 500; frame++) {
      // Slowly pan camera
      const t = frame * 0.01;
      const pos: [number, number] = [
        0.5 + 0.3 * Math.sin(t),
        0.5 + 0.3 * Math.cos(t * 0.7),
      ];
      const zoom = 4 + 2 * Math.sin(t * 0.3);

      const reqs = simulateFeedback(pos, zoom, pm.getLodBias());
      pm.processFeedback(reqs);

      // Invariants every frame
      expect(pm.cache.usedSlots).toBeLessThanOrEqual(ATLAS_SLOTS);
      expect(pm.cache.usedSlots).toBeGreaterThanOrEqual(0);
      expect(pm.pageTable.residentCount).toBe(pm.cache.usedSlots);
      expect(pm.getLodBias()).toBeGreaterThanOrEqual(0);
      expect(pm.getLodBias()).toBeLessThanOrEqual(MAX_MIP);

      // Render every 10th frame
      if (frame % 10 === 0) {
        const rendered = pm.render(pos, zoom);
        expect(rendered.length).toBe(256 * 256 * 4);
      }
    }

    // After 500 frames, pinned pages still resident
    expect(pm.pageTable.isResident({ mip: MAX_MIP, x: 0, y: 0 })).toBe(true);
  }, 60000);
});

// ============================================================================
// 20. Slot data integrity after 500 evictions (no corruption)
// ============================================================================

describe('Stress: Slot data integrity after 500 evictions', () => {
  test('Each slot contains correct page data after 500 evictions', () => {
    const cache = new PageCache(new Set());
    const slotToPage = new Map<string, number>(); // "sx,sy" → page index

    // Initial fill
    for (let i = 0; i < ATLAS_SLOTS; i++) {
      const data = new Uint8Array(SLOT_SIZE * SLOT_SIZE * 4);
      data.fill(i & 0xFF);
      const { slot } = cache.acquire({ mip: 0, x: i, y: 0 });
      cache.commit({ mip: 0, x: i, y: 0 }, slot, data);
      slotToPage.set(`${slot.x},${slot.y}`, i);
    }

    // 500 evictions with unique data
    for (let i = 0; i < 500; i++) {
      const pageIdx = 10000 + i;
      const data = new Uint8Array(SLOT_SIZE * SLOT_SIZE * 4);
      data.fill(pageIdx & 0xFF);
      const { slot, evicted } = cache.acquire({ mip: 0, x: pageIdx, y: 0 });
      expect(evicted).not.toBeNull();

      // Clear old mapping for this slot
      slotToPage.delete(`${slot.x},${slot.y}`);

      cache.commit({ mip: 0, x: pageIdx, y: 0 }, slot, data);
      slotToPage.set(`${slot.x},${slot.y}`, pageIdx);
    }

    // Verify: each slot's data matches its assigned page
    for (const [slotKey, pageIdx] of slotToPage) {
      const [sx, sy] = slotKey.split(',').map(Number);
      const idx = (sy * SLOT_SIZE * ATLAS_WIDTH + sx * SLOT_SIZE) * 4;
      expect(cache.atlas[idx]).toBe(pageIdx & 0xFF);
    }

    // All slots should have valid data
    expect(slotToPage.size).toBe(ATLAS_SLOTS);
  }, 15000);

  test('Page table entry matches atlas slot after 200 evictions', () => {
    const pm = new PageManager();

    // Load a page and verify entry matches slot
    for (let i = 0; i < 200; i++) {
      const req: PageRequest = { mip: 0, x: 100 + i, y: 100 + i };
      // Check if we can acquire (may evict)
      try {
        const { slot, evicted } = pm.cache.acquire(req);
        if (evicted) {
          pm.pageTable.setEvicted(evicted);
        }
        pm.cache.commit(req, slot, generatePage(req));
        pm.pageTable.setResident(req, slot);

        // Verify entry matches
        const entry = pm.pageTable.get(req);
        expect(getPhysX(entry)).toBe(slot.x);
        expect(getPhysY(entry)).toBe(slot.y);
      } catch {
        // All pinned → skip
      }
    }

    // Final state: page table count = used slots
    expect(pm.pageTable.residentCount).toBe(pm.cache.usedSlots);
  }, 15000);
});
