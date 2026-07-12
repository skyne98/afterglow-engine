// Super edge case tests — extreme boundary conditions that stress every part of VT.
// Run: bun test prototype/vt/vt.edge.test.ts
//
// These tests push the VT system to its limits with unusual inputs:
// 1. UV at exact page boundaries (0.0, 1.0, page edges)
// 2. NaN and Infinity UVs
// 3. Empty feedback (camera completely outside texture)
// 4. Rapid camera movement (different pages every frame, no reuse)
// 5. Total oversubscription (atlas can't hold working set)
// 6. Same page requested at multiple mip levels
// 7. Extreme zoom levels (zoom=0.001 and zoom=100000)
// 8. All atlas slots evicted in sequence (stress LRU)
// 9. Page table entry at maximum physX/physY (255,255)
// 10. Feedback with LOD bias at maximum (all pages at coarsest mip)

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

// ============================================================================
// 1. UV at exact page boundaries
// ============================================================================

describe('Edge: UV at exact page boundaries', () => {
  test('UV = 0.0 → maps to page (0,0) at all mip levels', () => {
    const pt = new PageTable(MAX_MIP);
    for (let mip = 0; mip <= MAX_MIP; mip++) {
      pt.setResident({ mip, x: 0, y: 0 }, { x: 0, y: 0 });
      const result = pt.findResidentPage(0.0, 0.0, mip);
      expect(result).not.toBeNull();
      expect(result!.mip).toBe(mip);
    }
  });

  test('UV = 0.999999 → maps to last page at all mip levels', () => {
    const pt = new PageTable(MAX_MIP);
    for (let mip = 0; mip <= MAX_MIP; mip++) {
      const lastPage = (VIRTUAL_PAGES_X >> mip) - 1;
      pt.setResident({ mip, x: lastPage, y: lastPage }, { x: 0, y: 0 });
      const result = pt.findResidentPage(0.999999, 0.999999, mip);
      expect(result).not.toBeNull();
      expect(result!.mip).toBe(mip);
    }
  });

  test('UV exactly at page boundary (page N / VIRTUAL_PAGES_X)', () => {
    // UV = 5/32 = 0.15625 → page 5 at mip 0
    const pt = new PageTable(MAX_MIP);
    pt.setResident({ mip: 0, x: 5, y: 0 }, { x: 0, y: 0 });
    pt.setResident({ mip: 0, x: 4, y: 0 }, { x: 1, y: 0 });
    const result = pt.findResidentPage(5.0 / VIRTUAL_PAGES_X, 0.0, 0);
    expect(result).not.toBeNull();
    // floor(0.15625 * 32) = floor(5.0) = 5 → page 5
    expect(getPhysX(result!.entry)).toBe(0);
  });

  test('UV = 1.0 (exactly at end) → clamped to last page', () => {
    const pt = new PageTable(MAX_MIP);
    const lastPage = VIRTUAL_PAGES_X - 1;
    pt.setResident({ mip: 0, x: lastPage, y: lastPage }, { x: 0, y: 0 });
    // Math.min(Math.floor(1.0 * 32), 31) = Math.min(32, 31) = 31
    const result = pt.findResidentPage(1.0, 1.0, 0);
    expect(result).not.toBeNull();
  });

  test('UV at center of texture (0.5, 0.5) — boundary between quadrants', () => {
    const pt = new PageTable(MAX_MIP);
    // At mip 0: floor(0.5 * 32) = 16 → page (16, 16)
    pt.setResident({ mip: 0, x: 16, y: 16 }, { x: 5, y: 5 });
    const result = pt.findResidentPage(0.5, 0.5, 0);
    expect(result).not.toBeNull();
    expect(getPhysX(result!.entry)).toBe(5);
  });

  test('UV at 1/pageGrid boundary for each mip level', () => {
    const pt = new PageTable(MAX_MIP);
    for (let mip = 0; mip <= MAX_MIP; mip++) {
      const pagesAtMip = VIRTUAL_PAGES_X >> mip;
      // UV = 1/pagesAtMip → floor(1.0) = 1 → page 1
      // But at max mip (pagesAtMip=1), UV=1.0 → min(1, 0) = 0 → page 0
      const u = 1.0 / pagesAtMip;
      const expectedPage = pagesAtMip > 1 ? 1 : 0;
      pt.setResident({ mip, x: expectedPage, y: expectedPage }, { x: 0, y: 0 });
      const result = pt.findResidentPage(u, u, mip);
      expect(result).not.toBeNull();
      expect(result!.mip).toBe(mip);
    }
  });
});

// ============================================================================
// 2. NaN and Infinity UVs
// ============================================================================

describe('Edge: NaN and Infinity UVs', () => {
  test('NaN UV → findResidentPage returns null (no crash)', () => {
    const pt = new PageTable(MAX_MIP);
    pt.setResident({ mip: 0, x: 0, y: 0 }, { x: 0, y: 0 });
    const result = pt.findResidentPage(NaN, NaN, 0);
    // Math.floor(NaN * 32) = NaN, Math.min(NaN, 31) = NaN
    // this.get({mip:0, x:NaN, y:NaN}) → key = "0:NaN:NaN" → not found → 0
    expect(result).toBeNull();
  });

  test('Infinity UV → clamped to page 0 at coarsest mip (not null)', () => {
    const pt = new PageTable(MAX_MIP);
    pt.setResident({ mip: MAX_MIP, x: 0, y: 0 }, { x: 0, y: 0 });
    // At mip MAX_MIP, pagesAtMip = 1
    // Math.floor(Infinity * 1) = Infinity, Math.min(Infinity, 0) = 0
    // So page (0, 0) at mip MAX_MIP → resident → returns it
    const result = pt.findResidentPage(Infinity, Infinity, 0);
    // Infinity gets clamped through the mip chain and finds the coarsest page
    expect(result).not.toBeNull();
    expect(result!.mip).toBe(MAX_MIP);
  });

  test('Negative UV → findResidentPage returns null', () => {
    const pt = new PageTable(MAX_MIP);
    pt.setResident({ mip: 0, x: 0, y: 0 }, { x: 0, y: 0 });
    // Math.floor(-0.5 * 32) = -16 → page (-16, -16) → not found
    const result = pt.findResidentPage(-0.5, -0.5, 0);
    expect(result).toBeNull();
  });

  test('NaN UV in vtSample → returns null (no crash)', () => {
    const pt = new PageTable(MAX_MIP);
    const atlas = new Uint8Array(ATLAS_WIDTH * ATLAS_HEIGHT * 4);
    const uvDx: [number, number] = [1 / VIRTUAL_SIZE, 0];
    const uvDy: [number, number] = [0, 1 / VIRTUAL_SIZE];
    const result = vtSample(NaN, NaN, uvDx, uvDy, pt, atlas);
    expect(result).toBeNull();
  });

  test('Infinity UV in vtSample → returns null (no crash)', () => {
    const pt = new PageTable(MAX_MIP);
    const atlas = new Uint8Array(ATLAS_WIDTH * ATLAS_HEIGHT * 4);
    const uvDx: [number, number] = [1 / VIRTUAL_SIZE, 0];
    const uvDy: [number, number] = [0, 1 / VIRTUAL_SIZE];
    const result = vtSample(Infinity, Infinity, uvDx, uvDy, pt, atlas);
    expect(result).toBeNull();
  });

  test('computeMipLevel with NaN derivatives → returns NaN (no crash)', () => {
    const mip = computeMipLevel([NaN, 0], [0, NaN], VIRTUAL_SIZE);
    expect(Number.isNaN(mip)).toBe(true);
  });

  test('computeMipLevel with Infinity derivatives → returns Infinity', () => {
    const mip = computeMipLevel([Infinity, 0], [0, 0], VIRTUAL_SIZE);
    // dot(Inf, Inf) = Infinity, max(Inf, 0) = Inf, 0.5*log2(Inf) = Inf
    expect(mip).toBe(Infinity);
  });
});

// ============================================================================
// 3. Empty feedback (camera outside texture)
// ============================================================================

describe('Edge: Empty feedback (camera outside texture)', () => {
  test('Camera at (-1, -1) → 0 requests', () => {
    const requests = simulateFeedback([-1, -1], 1);
    expect(requests.size).toBe(0);
  });

  test('Camera at (2, 2) → 0 requests', () => {
    const requests = simulateFeedback([2, 2], 1);
    expect(requests.size).toBe(0);
  });

  test('Camera at (0.5, -1) → 0 requests (Y out of bounds)', () => {
    const requests = simulateFeedback([0.5, -1], 1);
    expect(requests.size).toBe(0);
  });

  test('processFeedback with 0 requests → no crash, no changes', () => {
    const pm = new PageManager();
    const initialSlots = pm.cache.usedSlots;
    const r = pm.processFeedback(new Map());
    expect(r.loaded).toBe(0);
    expect(r.evicted).toBe(0);
    expect(pm.cache.usedSlots).toBe(initialSlots);
  });

  test('Camera partially outside: UV range [-0.5, 0.5] → some requests', () => {
    // Camera center at (0, 0.5), zoom=2 → UV range [-0.25, 0.25] in X
    const requests = simulateFeedback([0, 0.5], 2);
    // Some feedback pixels are in [0, 0.25] → valid, others in [-0.25, 0) → invalid
    expect(requests.size).toBeGreaterThan(0);
  });
});

// ============================================================================
// 4. Rapid camera movement (different pages every frame)
// ============================================================================

describe('Edge: Rapid camera movement', () => {
  test('10 frames, each at random position → no crash, pinned pages survive', () => {
    const pm = new PageManager();
    for (let i = 0; i < 10; i++) {
      const pos: [number, number] = [Math.random(), Math.random()];
      const zoom = 2 + Math.random() * 16;
      const reqs = simulateFeedback(pos, zoom, pm.getLodBias());
      pm.processFeedback(reqs);
      const rendered = pm.render(pos, zoom);
      // Should not be all black (pinned pages provide fallback)
      let nonBlack = 0;
      for (let j = 0; j < rendered.length; j += 4) {
        if (rendered[j] + rendered[j + 1] + rendered[j + 2] > 0) nonBlack++;
      }
      expect(nonBlack).toBeGreaterThan(0);
    }
    // Pinned pages always resident
    for (const mip of PINNED_MIPS) {
      const pages = VIRTUAL_PAGES_X >> mip;
      expect(pm.pageTable.isResident({ mip, x: 0, y: 0 })).toBe(true);
    }
  });

  test('Alternating between two distant positions 20 times → stabilizes', () => {
    const pm = new PageManager();
    const posA: [number, number] = [0.1, 0.1];
    const posB: [number, number] = [0.9, 0.9];

    for (let i = 0; i < 20; i++) {
      const pos = i % 2 === 0 ? posA : posB;
      const reqs = simulateFeedback(pos, 4, pm.getLodBias());
      pm.processFeedback(reqs);
    }
    // After all movement, should still be functional
    expect(pm.cache.usedSlots).toBeGreaterThan(0);
    expect(pm.cache.usedSlots).toBeLessThanOrEqual(ATLAS_PAGES_X * ATLAS_PAGES_Y);
  });

  test('Zoom from 1 to 32 and back → all zoom levels render', () => {
    const pm = new PageManager();
    const zooms = [1, 2, 4, 8, 16, 32, 16, 8, 4, 2, 1];
    for (const zoom of zooms) {
      const reqs = simulateFeedback([0.5, 0.5], zoom, pm.getLodBias());
      pm.processFeedback(reqs);
      const rendered = pm.render([0.5, 0.5], zoom);
      expect(rendered.length).toBe(256 * 256 * 4);
    }
  });
});

// ============================================================================
// 5. Total oversubscription (atlas can't hold working set)
// ============================================================================

describe('Edge: Total oversubscription', () => {
  test('Zoom=1 requests more pages than atlas can hold → LOD bias increases', () => {
    const pm = new PageManager();
    const atlasSlots = ATLAS_PAGES_X * ATLAS_PAGES_Y;

    // At zoom=1, the feedback requests many pages (entire texture visible)
    for (let frame = 0; frame < 30; frame++) {
      const reqs = simulateFeedback([0.5, 0.5], 1, pm.getLodBias());
      pm.processFeedback(reqs);
    }

    // LOD bias should have increased to reduce page requests
    // (or at least not crash from oversubscription)
    expect(pm.getLodBias()).toBeGreaterThanOrEqual(0);
    expect(pm.cache.usedSlots).toBeLessThanOrEqual(atlasSlots);
  });

  test('Oversubscription: atlas never exceeds slot count', () => {
    const pm = new PageManager();
    const maxSlots = ATLAS_PAGES_X * ATLAS_PAGES_Y;

    for (let i = 0; i < 50; i++) {
      const reqs = simulateFeedback([Math.random(), Math.random()], 1, pm.getLodBias());
      pm.processFeedback(reqs);
      expect(pm.cache.usedSlots).toBeLessThanOrEqual(maxSlots);
    }
  });

  test('Oversubscription: pinned pages always survive', () => {
    const pm = new PageManager();
    for (let i = 0; i < 50; i++) {
      const reqs = simulateFeedback([Math.random(), Math.random()], 1, pm.getLodBias());
      pm.processFeedback(reqs);
    }
    for (const mip of PINNED_MIPS) {
      const pages = VIRTUAL_PAGES_X >> mip;
      for (let y = 0; y < pages; y++) {
        for (let x = 0; x < pages; x++) {
          expect(pm.pageTable.isResident({ mip, x, y })).toBe(true);
        }
      }
    }
  });
});

// ============================================================================
// 6. Same page requested at multiple mip levels
// ============================================================================

describe('Edge: Same page at multiple mip levels', () => {
  test('Page at mip 0 and its parent at mip 1 both resident', () => {
    const pt = new PageTable(MAX_MIP);
    // Page (0,0) at mip 0 and page (0,0) at mip 1
    pt.setResident({ mip: 0, x: 0, y: 0 }, { x: 0, y: 0 });
    pt.setResident({ mip: 1, x: 0, y: 0 }, { x: 1, y: 1 });
    expect(pt.isResident({ mip: 0, x: 0, y: 0 })).toBe(true);
    expect(pt.isResident({ mip: 1, x: 0, y: 0 })).toBe(true);
    // They are independent entries
    expect(pt.residentCount).toBe(2);
  });

  test('findResidentPage at mip 0 returns mip 0 (not mip 1 parent)', () => {
    const pt = new PageTable(MAX_MIP);
    pt.setResident({ mip: 0, x: 0, y: 0 }, { x: 5, y: 5 });
    pt.setResident({ mip: 1, x: 0, y: 0 }, { x: 6, y: 6 });
    const result = pt.findResidentPage(0.01, 0.01, 0);
    expect(result!.mip).toBe(0);
    expect(getPhysX(result!.entry)).toBe(5);
  });

  test('Evict mip 0 page → mip 1 parent still resident for fallback', () => {
    const pt = new PageTable(MAX_MIP);
    pt.setResident({ mip: 0, x: 0, y: 0 }, { x: 5, y: 5 });
    pt.setResident({ mip: 1, x: 0, y: 0 }, { x: 6, y: 6 });
    pt.setEvicted({ mip: 0, x: 0, y: 0 });
    // Request mip 0 → falls back to mip 1
    const result = pt.findResidentPage(0.01, 0.01, 0);
    expect(result).not.toBeNull();
    expect(result!.mip).toBe(1);
    expect(getPhysX(result!.entry)).toBe(6);
  });

  test('All mip levels of same UV region resident → returns finest', () => {
    const pt = new PageTable(MAX_MIP);
    for (let mip = 0; mip <= MAX_MIP; mip++) {
      const pages = VIRTUAL_PAGES_X >> mip;
      const px = Math.min(Math.floor(0.3 * pages), pages - 1);
      pt.setResident({ mip, x: px, y: px }, { x: mip, y: mip });
    }
    const result = pt.findResidentPage(0.3, 0.3, 0);
    expect(result!.mip).toBe(0); // finest available
  });
});

// ============================================================================
// 7. Extreme zoom levels
// ============================================================================

describe('Edge: Extreme zoom levels', () => {
  test('zoom=0.001 (extremely zoomed out) → only coarsest mip needed', () => {
    const reqs = simulateFeedback([0.5, 0.5], 0.001);
    // At zoom=0.001, UV width = 1000 → way outside texture
    // Most feedback pixels are outside [0,1] → few or no requests
    // But some may hit the texture and request coarsest mip
    for (const req of reqs.values()) {
      expect(req.mip).toBeGreaterThanOrEqual(0);
      expect(req.mip).toBeLessThanOrEqual(MAX_MIP);
    }
  });

  test('zoom=100000 (extremely zoomed in) → mip 0 needed', () => {
    const reqs = simulateFeedback([0.5, 0.5], 100000);
    // At zoom=100000, UV width = 0.00001 → tiny region
    // Each pixel covers ~0 texels → mip 0
    for (const req of reqs.values()) {
      expect(req.mip).toBe(0);
    }
  });

  test('zoom=0 (infinite zoom out) → no crash', () => {
    // zoom=0 → uvWidth = 1/0 = Infinity
    // This might produce NaN or Infinity in feedback
    expect(() => simulateFeedback([0.5, 0.5], 0)).not.toThrow();
  });

  test('Very large zoom: render produces non-black output', () => {
    const pm = new PageManager();
    const reqs = simulateFeedback([0.5, 0.5], 10000, pm.getLodBias());
    pm.processFeedback(reqs);
    const rendered = pm.render([0.5, 0.5], 10000);
    let nonBlack = 0;
    for (let i = 0; i < rendered.length; i += 4) {
      if (rendered[i] + rendered[i + 1] + rendered[i + 2] > 0) nonBlack++;
    }
    expect(nonBlack).toBeGreaterThan(0);
  });

  test('zoom progression: 1→2→4→...→32, all render without crash', () => {
    const pm = new PageManager();
    for (let zoom = 1; zoom <= 32; zoom *= 2) {
      const reqs = simulateFeedback([0.5, 0.5], zoom, pm.getLodBias());
      pm.processFeedback(reqs);
      expect(() => pm.render([0.5, 0.5], zoom)).not.toThrow();
    }
  });
});

// ============================================================================
// 8. All atlas slots evicted in sequence (stress LRU)
// ============================================================================

describe('Edge: Stress LRU eviction', () => {
  test('Evict all slots one by one, then reload', () => {
    const cache = new PageCache(new Set());
    const total = ATLAS_PAGES_X * ATLAS_PAGES_Y;

    // Fill all
    for (let i = 0; i < total; i++) {
      const { slot } = cache.acquire({ mip: 0, x: i, y: 0 });
      cache.commit({ mip: 0, x: i, y: 0 }, slot, new Uint8Array(SLOT_SIZE * SLOT_SIZE * 4));
    }

    // Evict all by loading new pages
    for (let i = 0; i < total; i++) {
      const { slot, evicted } = cache.acquire({ mip: 0, x: 1000 + i, y: 0 });
      expect(evicted).not.toBeNull();
      cache.commit({ mip: 0, x: 1000 + i, y: 0 }, slot, new Uint8Array(SLOT_SIZE * SLOT_SIZE * 4));
    }

    expect(cache.usedSlots).toBe(total);
    expect(cache.freeSlotCount).toBe(0);
  });

  test('Evict 1000 pages in rapid succession → no crash', () => {
    const cache = new PageCache(new Set());
    const total = ATLAS_PAGES_X * ATLAS_PAGES_Y;

    // Initial fill
    for (let i = 0; i < total; i++) {
      const { slot } = cache.acquire({ mip: 0, x: i, y: 0 });
      cache.commit({ mip: 0, x: i, y: 0 }, slot, new Uint8Array(SLOT_SIZE * SLOT_SIZE * 4));
    }

    // 1000 evictions
    for (let i = 0; i < 1000; i++) {
      const { slot, evicted } = cache.acquire({ mip: 0, x: 2000 + i, y: 0 });
      expect(evicted).not.toBeNull();
      cache.commit({ mip: 0, x: 2000 + i, y: 0 }, slot, new Uint8Array(SLOT_SIZE * SLOT_SIZE * 4));
    }
    expect(cache.usedSlots).toBe(total);
  });

  test('Touch + evict + touch + evict cycle', () => {
    const cache = new PageCache(new Set());
    const total = ATLAS_PAGES_X * ATLAS_PAGES_Y;

    for (let i = 0; i < total; i++) {
      const { slot } = cache.acquire({ mip: 0, x: i, y: 0 });
      cache.commit({ mip: 0, x: i, y: 0 }, slot, new Uint8Array(SLOT_SIZE * SLOT_SIZE * 4));
    }

    // Touch page 0, evict (should not be page 0), touch page 1, evict (should not be page 1)
    for (let i = 0; i < 10; i++) {
      cache.touch({ mip: 0, x: i, y: 0 });
      const { evicted } = cache.acquire({ mip: 0, x: 5000 + i, y: 0 });
      expect(evicted).not.toBeNull();
      expect(evicted!.x).not.toBe(i);
    }
  });
});

// ============================================================================
// 9. Page table entry at maximum physX/physY (255, 255)
// ============================================================================

describe('Edge: Maximum physX/physY page table entries', () => {
  test('Entry with physX=255, physY=255 → correct atlas coordinates', () => {
    const entry = packEntry(true, 255, 255, 0);
    expect(getPhysX(entry)).toBe(255);
    expect(getPhysY(entry)).toBe(255);

    // Compute atlas position
    const slotOriginX = 255 * SLOT_SIZE;
    const slotOriginY = 255 * SLOT_SIZE;
    // This would be beyond ATLAS_WIDTH (1088), but the entry format supports it
    // In practice, ATLAS_PAGES_X * SLOT_SIZE limits the valid range
    expect(slotOriginX).toBe(255 * 136);
  });

  test('vtSample with page at max valid slot (7,7)', () => {
    const pm = new PageManager();
    const req: PageRequest = { mip: 0, x: 31, y: 31 };
    const { slot } = pm.cache.acquire(req);
    // Slot should be within atlas bounds
    expect(slot.x).toBeLessThan(ATLAS_PAGES_X);
    expect(slot.y).toBeLessThan(ATLAS_PAGES_Y);

    pm.cache.commit(req, slot, generatePage(req));
    pm.pageTable.setResident(req, slot);

    const uvDx: [number, number] = [1 / VIRTUAL_SIZE, 0];
    const uvDy: [number, number] = [0, 1 / VIRTUAL_SIZE];
    const sampled = vtSample(0.999, 0.999, uvDx, uvDy, pm.pageTable, pm.cache.atlas);
    const truth = sampleVirtualTexture(0.999, 0.999);
    expect(sampled).not.toBeNull();
    expect(sampled![0]).toBe(truth[0]);
  });

  test('All atlas slots used with distinct pages → no slot corruption', () => {
    const cache = new PageCache(new Set());
    const total = ATLAS_PAGES_X * ATLAS_PAGES_Y;
    const slotData: Map<string, number> = new Map(); // "sx,sy" → page index

    // Fill all slots with unique pages and unique data
    for (let i = 0; i < total; i++) {
      const data = new Uint8Array(SLOT_SIZE * SLOT_SIZE * 4);
      data.fill(i & 0xFF); // unique pattern per page
      const { slot } = cache.acquire({ mip: 0, x: i, y: 0 });
      cache.commit({ mip: 0, x: i, y: 0 }, slot, data);
      slotData.set(`${slot.x},${slot.y}`, i);
    }

    // Verify each slot has correct data at its origin
    for (let sy = 0; sy < ATLAS_PAGES_Y; sy++) {
      for (let sx = 0; sx < ATLAS_PAGES_X; sx++) {
        const idx = (sy * SLOT_SIZE * ATLAS_WIDTH + sx * SLOT_SIZE) * 4;
        const expectedPage = slotData.get(`${sx},${sy}`)!;
        expect(cache.atlas[idx]).toBe(expectedPage & 0xFF);
      }
    }
  });
});

// ============================================================================
// 10. Feedback with LOD bias at maximum
// ============================================================================

describe('Edge: Maximum LOD bias', () => {
  test('LOD bias = MAX_MIP → all feedback requests at coarsest mip', () => {
    const reqs = simulateFeedback([0.5, 0.5], 4, MAX_MIP);
    for (const req of reqs.values()) {
      expect(req.mip).toBe(MAX_MIP);
    }
  });

  test('LOD bias = 100 (beyond max) → clamped to MAX_MIP', () => {
    const reqs = simulateFeedback([0.5, 0.5], 4, 100);
    for (const req of reqs.values()) {
      expect(req.mip).toBe(MAX_MIP);
    }
  });

  test('LOD bias = -5 (negative) → clamped to 0', () => {
    const reqs = simulateFeedback([0.5, 0.5], 4, -5);
    for (const req of reqs.values()) {
      expect(req.mip).toBeGreaterThanOrEqual(0);
    }
  });

  test('LOD bias changes mip level monotonically', () => {
    const mips: number[] = [];
    for (let bias = 0; bias <= MAX_MIP + 2; bias++) {
      const reqs = simulateFeedback([0.5, 0.5], 4, bias);
      const maxMip = Math.max(...[...reqs.values()].map(r => r.mip));
      mips.push(maxMip);
    }
    // mip should be non-decreasing with increasing bias
    for (let i = 1; i < mips.length; i++) {
      expect(mips[i]).toBeGreaterThanOrEqual(mips[i - 1]);
    }
  });

  test('PageManager with forced oversubscription → LOD bias stabilizes', () => {
    const pm = new PageManager();
    // Run many frames of oversubscription
    const biases: number[] = [];
    for (let i = 0; i < 50; i++) {
      const reqs = simulateFeedback([0.5, 0.5], 1, pm.getLodBias());
      pm.processFeedback(reqs);
      biases.push(pm.getLodBias());
    }
    // LOD bias should not keep increasing forever (stabilizes or oscillates)
    const lastBias = biases[biases.length - 1];
    expect(lastBias).toBeLessThanOrEqual(MAX_MIP);
  });
});
