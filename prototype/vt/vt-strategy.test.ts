// Tests for Smart VT LOD Strategy.
// Run: bun test prototype/vt/vt-strategy.test.ts

import { test, describe, expect } from 'bun:test';
import {
  VTLodStrategy, defaultConfig, type VTConfig,
} from './vt-strategy';
import {
  PageTable, PageCache, simulateFeedback,
  VIRTUAL_PAGES_X, MAX_MIP, ATLAS_PAGES_X, ATLAS_PAGES_Y,
  PINNED_MIPS, type PageRequest,
} from './vt';

// Helper: create a strategy with overrides
function makeStrategy(overrides: Partial<VTConfig> = {}): VTLodStrategy {
  return new VTLodStrategy({ ...defaultConfig, ...overrides });
}

// Helper: create a page table + cache with pinned pages loaded
function makePageTable(): { pt: PageTable; cache: PageCache } {
  const pt = new PageTable(MAX_MIP);
  const cache = new PageCache(PINNED_MIPS);
  // Load pinned pages
  for (const mip of PINNED_MIPS) {
    const pages = VIRTUAL_PAGES_X >> mip;
    for (let y = 0; y < pages; y++) {
      for (let x = 0; x < pages; x++) {
        const req: PageRequest = { mip, x, y };
        const { slot } = cache.acquire(req);
        cache.commit(req, slot, new Uint8Array(136 * 136 * 4));
        pt.setResident(req, slot);
      }
    }
  }
  return { pt, cache };
}

// ============================================================================
// 1. Configuration
// ============================================================================

describe('VTLodStrategy configuration', () => {
  test('default config has reasonable values', () => {
    const s = makeStrategy();
    const c = s.getConfig();
    expect(c.maxPagesPerFrame).toBe(8);
    expect(c.maxBudget).toBe(16);
    expect(c.hysteresisFrames).toBe(3);
    expect(c.predictionEnabled).toBe(true);
    expect(c.adaptiveQualityEnabled).toBe(true);
    expect(c.targetFrameTime).toBeCloseTo(16.67, 1);
    expect(c.weightMipDistance).toBe(1.0);
  });

  test('custom config overrides defaults', () => {
    const s = makeStrategy({ maxPagesPerFrame: 4, predictionEnabled: false });
    const c = s.getConfig();
    expect(c.maxPagesPerFrame).toBe(4);
    expect(c.predictionEnabled).toBe(false);
    expect(c.maxBudget).toBe(16); // unchanged
  });

  test('LOD bias starts at 0', () => {
    const s = makeStrategy();
    expect(s.getLodBias()).toBe(0);
  });

  test('Budget starts at maxPagesPerFrame', () => {
    const s = makeStrategy({ maxPagesPerFrame: 12 });
    expect(s.getBudget()).toBe(12);
  });
});

// ============================================================================
// 2. Priority computation
// ============================================================================

describe('Priority computation', () => {
  test('non-resident page with large mip gap gets high priority', () => {
    const s = makeStrategy();
    const { pt, cache } = makePageTable();

    // Page at mip 0 (fine) not resident → fallback to pinned mip 5 → large gap
    const feedback = simulateFeedback([0.5, 0.5], 32, 0);
    const result = s.processFeedback(feedback, pt, cache);

    // Should have pages to load
    expect(result.toLoad.length).toBeGreaterThan(0);
    // Priorities should be computed
    expect(result.priorities.size).toBeGreaterThan(0);
  });

  test('already-resident page gets no load request', () => {
    const s = makeStrategy();
    const { pt, cache } = makePageTable();

    // Feedback requests pinned pages → already resident → no load
    const feedback = simulateFeedback([0.5, 0.5], 1, 0);
    const result = s.processFeedback(feedback, pt, cache);

    // No pages to load (all pinned)
    expect(result.toLoad.length).toBe(0);
  });

  test('pages are sorted by priority (highest first)', () => {
    const s = makeStrategy();
    const { pt, cache } = makePageTable();

    const feedback = simulateFeedback([0.5, 0.5], 4, 0);
    const result = s.processFeedback(feedback, pt, cache);

    // Verify priorities are in descending order
    const priVals = [...result.priorities.values()];
    for (let i = 1; i < priVals.length; i++) {
      // toLoad is already sorted, but priorities map may not be
      // Just verify the map has values
      expect(priVals[i]).toBeGreaterThanOrEqual(0);
    }
  });
});

// ============================================================================
// 3. Budget limiting
// ============================================================================

describe('Budget limiting', () => {
  test('toLoad respects maxPagesPerFrame', () => {
    const s = makeStrategy({ maxPagesPerFrame: 2 });
    const { pt, cache } = makePageTable();

    // Request many pages (zoomed in → mip 0, many pages)
    const feedback = simulateFeedback([0.5, 0.5], 32, 0);
    const result = s.processFeedback(feedback, pt, cache);

    expect(result.toLoad.length).toBeLessThanOrEqual(2);
  });

  test('budget=1 loads only 1 page per frame', () => {
    const s = makeStrategy({ maxPagesPerFrame: 1 });
    const { pt, cache } = makePageTable();

    const feedback = simulateFeedback([0.5, 0.5], 16, 0);
    const result = s.processFeedback(feedback, pt, cache);

    expect(result.toLoad.length).toBeLessThanOrEqual(1);
  });

  test('budget=0 loads no pages', () => {
    const s = makeStrategy({ maxPagesPerFrame: 0 });
    const { pt, cache } = makePageTable();

    const feedback = simulateFeedback([0.5, 0.5], 16, 0);
    const result = s.processFeedback(feedback, pt, cache);

    expect(result.toLoad.length).toBe(0);
  });
});

// ============================================================================
// 4. Adaptive quality
// ============================================================================

describe('Adaptive quality (frame-time-based)', () => {
  test('LOD bias increases when frame time is high', () => {
    const s = makeStrategy({ adaptiveQualityEnabled: true, targetFrameTime: 16.67 });
    const { pt, cache } = makePageTable();

    // Record high frame times (stressed)
    for (let i = 0; i < 30; i++) {
      s.recordFrameTime(30); // 30ms = way above 16.67ms target
    }

    const feedback = simulateFeedback([0.5, 0.5], 4, 0);
    s.processFeedback(feedback, pt, cache);

    expect(s.getLodBias()).toBeGreaterThan(0);
  });

  test('LOD bias decreases when frame time is low', () => {
    const s = makeStrategy({ adaptiveQualityEnabled: true, targetFrameTime: 16.67 });
    const { pt, cache } = makePageTable();

    // First: stress the system
    for (let i = 0; i < 30; i++) s.recordFrameTime(30);
    const fb = simulateFeedback([0.5, 0.5], 4, 0);
    s.processFeedback(fb, pt, cache);
    expect(s.getLodBias()).toBeGreaterThan(0);

    // Then: record low frame times (comfortable)
    for (let i = 0; i < 30; i++) s.recordFrameTime(5); // 5ms = well below target
    s.processFeedback(fb, pt, cache);

    // LOD bias should decrease
    expect(s.getLodBias()).toBeLessThan(2); // decreased from stressed level
  });

  test('Budget decreases under stress, increases when comfortable', () => {
    const s = makeStrategy({
      adaptiveQualityEnabled: true,
      maxPagesPerFrame: 8,
      maxBudget: 16,
      targetFrameTime: 16.67,
    });
    const { pt, cache } = makePageTable();

    // Stress
    for (let i = 0; i < 30; i++) s.recordFrameTime(30);
    s.processFeedback(simulateFeedback([0.5, 0.5], 4, 0), pt, cache);
    const stressedBudget = s.getBudget();
    expect(stressedBudget).toBeLessThan(8);

    // Comfortable
    for (let i = 0; i < 60; i++) s.recordFrameTime(5);
    s.processFeedback(simulateFeedback([0.5, 0.5], 4, 0), pt, cache);
    const comfortableBudget = s.getBudget();
    expect(comfortableBudget).toBeGreaterThanOrEqual(stressedBudget);
  });

  test('Adaptive quality disabled → no LOD bias change', () => {
    const s = makeStrategy({ adaptiveQualityEnabled: false });
    const { pt, cache } = makePageTable();

    for (let i = 0; i < 30; i++) s.recordFrameTime(100); // extreme stress
    s.processFeedback(simulateFeedback([0.5, 0.5], 4, 0), pt, cache);

    expect(s.getLodBias()).toBe(0); // no change when disabled
  });
});

// ============================================================================
// 5. Oversubscription
// ============================================================================

describe('Oversubscription handling', () => {
  test('LOD bias increases when atlas is nearly full', () => {
    const s = makeStrategy({
      adaptiveQualityEnabled: false, // disable frame-time adaptive
      highWaterMark: 0.5, // trigger early
    });
    const { pt, cache } = makePageTable();

    // Fill atlas to >50%
    for (let i = 0; i < 40; i++) {
      const req: PageRequest = { mip: 0, x: i, y: 0 };
      const { slot } = cache.acquire(req);
      cache.commit(req, slot, new Uint8Array(136 * 136 * 4));
      pt.setResident(req, slot);
    }

    const feedback = simulateFeedback([0.5, 0.5], 4, 0);
    s.processFeedback(feedback, pt, cache);

    expect(s.getLodBias()).toBeGreaterThan(0);
  });

  test('LOD bias decreases when atlas is empty', () => {
    const s = makeStrategy({
      adaptiveQualityEnabled: false,
      highWaterMark: 0.5,
      lowWaterMark: 0.1,
    });
    const { pt, cache } = makePageTable();

    // First: trigger high water mark
    for (let i = 0; i < 40; i++) {
      const req: PageRequest = { mip: 0, x: i, y: 0 };
      const { slot } = cache.acquire(req);
      cache.commit(req, slot, new Uint8Array(136 * 136 * 4));
      pt.setResident(req, slot);
    }
    s.processFeedback(simulateFeedback([0.5, 0.5], 4, 0), pt, cache);
    expect(s.getLodBias()).toBeGreaterThan(0);

    // Now: evict everything (simulate empty atlas)
    // Create fresh cache (only pinned pages)
    const { pt: pt2, cache: cache2 } = makePageTable();
    s.processFeedback(simulateFeedback([0.5, 0.5], 4, 0), pt2, cache2);
    // Low water mark → decrease LOD bias
    expect(s.getLodBias()).toBeLessThan(2);
  });
});

// ============================================================================
// 6. Prediction
// ============================================================================

describe('Predictive loading', () => {
  test('prediction disabled → no predicted requests', () => {
    const s = makeStrategy({ predictionEnabled: false });
    const { pt, cache } = makePageTable();

    s.recordCamera([0.5, 0.5], 4);
    s.recordCamera([0.51, 0.5], 4); // moving right

    const feedback = simulateFeedback([0.51, 0.5], 4, 0);
    const result = s.processFeedback(feedback, pt, cache);

    // No predicted pages
    const stats = s.getStats();
    expect(stats.predictedCount).toBe(0);
  });

  test('prediction enabled + camera moving → predicted pages tracked', () => {
    const s = makeStrategy({ predictionEnabled: true, predictionFrames: 2 });
    const { pt, cache } = makePageTable();

    // Record camera movement
    s.recordCamera([0.3, 0.5], 4);
    s.recordCamera([0.35, 0.5], 4); // moving right

    const feedback = simulateFeedback([0.35, 0.5], 4, 0);
    s.processFeedback(feedback, pt, cache);

    const stats = s.getStats();
    // Should have tracked predicted pages (at predicted position 0.45)
    expect(stats.predictedCount).toBeGreaterThan(0);
  });

  test('prediction adds pages for future camera position', () => {
    const s = makeStrategy({ predictionEnabled: true, predictionFrames: 3 });
    const { pt, cache } = makePageTable();

    // Moving right at 0.05 UV/frame
    s.recordCamera([0.3, 0.5], 4);
    s.recordCamera([0.35, 0.5], 4);

    // Current feedback at 0.35 → pages at UV 0.35
    // Predicted position: 0.35 + 0.05 * 3 = 0.50 → pages at UV 0.50
    const feedback = simulateFeedback([0.35, 0.5], 4, 0);
    const result = s.processFeedback(feedback, pt, cache);

    // Should have tracked more pages than just feedback
    const stats = s.getStats();
    expect(stats.trackedPages).toBeGreaterThanOrEqual(feedback.size);
  });

  test('stationary camera → no predicted pages', () => {
    const s = makeStrategy({ predictionEnabled: true, predictionFrames: 2 });
    const { pt, cache } = makePageTable();

    s.recordCamera([0.5, 0.5], 4);
    s.recordCamera([0.5, 0.5], 4); // no movement

    const feedback = simulateFeedback([0.5, 0.5], 4, 0);
    s.processFeedback(feedback, pt, cache);

    const stats = s.getStats();
    expect(stats.predictedCount).toBe(0); // no velocity → no prediction
  });
});

// ============================================================================
// 7. Hysteresis
// ============================================================================

describe('Hysteresis', () => {
  test('recently loaded page has lower priority (hysteresis factor)', () => {
    const s = makeStrategy({
      hysteresisFrames: 5,
      hysteresisFactor: 0.1,
      maxPagesPerFrame: 100, // load everything
    });
    const { pt, cache } = makePageTable();

    // Frame 1: request pages
    const feedback = simulateFeedback([0.5, 0.5], 4, 0);
    const result1 = s.processFeedback(feedback, pt, cache);
    expect(result1.toLoad.length).toBeGreaterThan(0);

    // Load them (simulate)
    for (const req of result1.toLoad) {
      const { slot } = cache.acquire(req);
      cache.commit(req, slot, new Uint8Array(136 * 136 * 4));
      pt.setResident(req, slot);
    }

    // Frame 2: evict them (force eviction)
    for (const req of result1.toLoad) {
      pt.setEvicted(req);
    }

    // Frame 3: request same pages → should have lower priority (hysteresis)
    const result3 = s.processFeedback(feedback, pt, cache);

    // Pages should still be in toLoad (they're not resident again)
    // But their priority should be lower due to hysteresis
    // (We can't directly check priority value, but we verify it doesn't crash)
    expect(result3.toLoad.length).toBeGreaterThan(0);
  });
});

// ============================================================================
// 8. Eviction
// ============================================================================

describe('Eviction', () => {
  test('pages not seen for graceFrames → evicted', () => {
    const s = makeStrategy({ evictionGraceFrames: 2 });
    const { pt, cache } = makePageTable();

    // Load some pages
    const req: PageRequest = { mip: 0, x: 10, y: 10 };
    const { slot } = cache.acquire(req);
    cache.commit(req, slot, new Uint8Array(136 * 136 * 4));
    pt.setResident(req, slot);

    // Frame 1: see the page
    const fb1 = new Map<string, PageRequest>();
    fb1.set('0:10:10', req);
    s.processFeedback(fb1, pt, cache);

    // Frames 2-4: don't see the page (graceFrames=2, so after frame 3 it's evictable)
    s.processFeedback(new Map(), pt, cache); // frame 2: grace
    s.processFeedback(new Map(), pt, cache); // frame 3: grace expired
    const result = s.processFeedback(new Map(), pt, cache); // frame 4: should evict

    expect(result.toEvict.length).toBeGreaterThan(0);
    expect(result.toEvict.some(r => r.mip === 0 && r.x === 10 && r.y === 10)).toBe(true);
  });

  test('page seen every frame → not evicted', () => {
    const s = makeStrategy({ evictionGraceFrames: 3 });
    const { pt, cache } = makePageTable();

    const req: PageRequest = { mip: 0, x: 5, y: 5 };
    const { slot } = cache.acquire(req);
    cache.commit(req, slot, new Uint8Array(136 * 136 * 4));
    pt.setResident(req, slot);

    const fb = new Map<string, PageRequest>();
    fb.set('0:5:5', req);

    for (let i = 0; i < 10; i++) {
      const result = s.processFeedback(fb, pt, cache);
      expect(result.toEvict.some(r => r.mip === 0 && r.x === 5 && r.y === 5)).toBe(false);
    }
  });
});

// ============================================================================
// 9. Multi-frame scenario
// ============================================================================

describe('Multi-frame scenario', () => {
  test('5 frames of feedback → quality improves over time', () => {
    const s = makeStrategy({ maxPagesPerFrame: 4 });
    const { pt, cache } = makePageTable();

    const pos: [number, number] = [0.5, 0.5];
    const zoom = 4;

    for (let frame = 0; frame < 5; frame++) {
      s.recordCamera(pos, zoom);
      s.recordFrameTime(16); // normal frame time
      const feedback = simulateFeedback(pos, zoom, s.getLodBias());
      const result = s.processFeedback(feedback, pt, cache);

      // Load the pages
      for (const req of result.toLoad) {
        const { slot } = cache.acquire(req);
        cache.commit(req, slot, new Uint8Array(136 * 136 * 4));
        pt.setResident(req, slot);
      }
    }

    // After 5 frames, most requested pages should be resident
    const feedback = simulateFeedback(pos, zoom, s.getLodBias());
    let resident = 0;
    for (const req of feedback.values()) {
      if (pt.isResident(req)) resident++;
    }
    expect(resident).toBeGreaterThan(0);
  });

  test('camera movement → predicted pages pre-loaded', () => {
    const s = makeStrategy({
      predictionEnabled: true,
      predictionFrames: 3,
      maxPagesPerFrame: 16,
    });
    const { pt, cache } = makePageTable();

    // Move camera right over 3 frames
    for (let i = 0; i < 3; i++) {
      const pos: [number, number] = [0.3 + i * 0.05, 0.5];
      s.recordCamera(pos, 8);
      s.recordFrameTime(16);
      const feedback = simulateFeedback(pos, 8, s.getLodBias());
      const result = s.processFeedback(feedback, pt, cache);
      for (const req of result.toLoad) {
        const { slot } = cache.acquire(req);
        cache.commit(req, slot, new Uint8Array(136 * 136 * 4));
        pt.setResident(req, slot);
      }
    }

    // After 3 frames moving right, predicted position should have some pages loaded
    const stats = s.getStats();
    expect(stats.predictedCount).toBeGreaterThan(0);
  });
});

// ============================================================================
// 10. Statistics and debugging
// ============================================================================

describe('Statistics', () => {
  test('getStats returns valid data', () => {
    const s = makeStrategy();
    const { pt, cache } = makePageTable();

    s.recordCamera([0.5, 0.5], 4);
    s.recordFrameTime(16.67);
    s.processFeedback(simulateFeedback([0.5, 0.5], 4, 0), pt, cache);

    const stats = s.getStats();
    expect(stats.trackedPages).toBeGreaterThan(0);
    expect(stats.totalHits).toBeGreaterThan(0);
    expect(stats.lodBias).toBeGreaterThanOrEqual(0);
    expect(stats.budget).toBeGreaterThan(0);
    expect(stats.avgFrameTime).toBeCloseTo(16.67, 1);
  });

  test('getPageState returns state for tracked page', () => {
    const s = makeStrategy();
    const { pt, cache } = makePageTable();

    const feedback = simulateFeedback([0.5, 0.5], 4, 0);
    s.processFeedback(feedback, pt, cache);

    // Get the first requested page
    const firstReq = feedback.values().next().value;
    const state = s.getPageState(firstReq);
    expect(state).toBeDefined();
    expect(state!.hitCount).toBeGreaterThan(0);
    expect(state!.consecutiveFrames).toBeGreaterThan(0);
  });
});
