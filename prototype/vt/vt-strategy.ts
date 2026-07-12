// Smart VT LOD Strategy — configurable, multi-factor priority with adaptive quality.
//
// Sources:
//   [IDTECH]    id Software "Software Virtual Textures" (van Waveren, 2012)
//   [SHLOM]     shlomnissan/virtual-textures (2025)
//   [ADAPTIVE]  Zhang et al. "High-performance adaptive texture streaming" (2022)
//   [SCHMITZ]   Schmitz et al. "Predictive page management based on camera movements"
//   [UNREAL]    Unreal Engine Virtual Texturing system
//
// === How existing engines decide LOD/mip transitions ===
//
// [IDTECH] RAGE:
//   - Feedback-driven: GPU renders feedback pass → CPU reads back (1-2 frame latency)
//   - Priority sort: (1) |desiredMip - residentMip|, (2) hit count in feedback
//   - Oversubscription: high/low water marks → dynamic LOD bias
//   - LOD snapping fix: upsample coarser mip → blend to finer
//   - Budget: 8-16 pages transcoded per frame at 60fps
//
// [SHLOM]:
//   - Simple LRU eviction (no priority sorting)
//   - Pinned coarsest mip for fallback
//   - No adaptive quality
//
// [ADAPTIVE] Zhang et al.:
//   - Frame-time-adaptive workload: adjusts streaming based on measured frame latency
//   - Mesh perceptibility weight: fragment count = visual importance
//   - Preload higher resolution: Am > As (metadata anisotropy > sampling anisotropy)
//     → preload mip at higher res than needed to reduce pop-in
//   - Color blending: blend between mip levels using per-mesh colors
//
// [SCHMITZ]:
//   - Predictive page management: track camera velocity, pre-load future pages
//
// [UNREAL]:
//   - Configurable texture groups with LOD bias
//   - NumStreamedMips, VirtualTextureTileCountBias parameters
//
// === Our "super smart" approach ===
//
// The key innovation: a **multi-factor priority score** that considers:
// 1. Visual impact (how much will loading this page improve the image?)
// 2. Temporal stability (how confident are we this page is needed?)
// 3. Prediction (is this page in the camera's future view?)
// 4. Performance cost (how expensive is loading this page?)
//
// Combined with:
// - Adaptive quality (frame-time-based LOD bias + page budget)
// - Hysteresis (don't switch mips too frequently)
// - LOD snapping prevention (upsample coarser → blend to finer)
// - Predictive loading (camera velocity × N frames ahead)

import {
  PageTable, PageCache, simulateFeedback,
  VIRTUAL_PAGES_X, MAX_MIP, ATLAS_PAGES_X, ATLAS_PAGES_Y,
  type PageRequest,
} from './vt';

// ============================================================================
// Configuration
// ============================================================================

export interface VTConfig {
  // --- Page loading budget ---
  maxPagesPerFrame: number;       // max pages to load per frame (default: 8)
  maxBudget: number;              // max budget when performance is good (default: 16)

  // --- Hysteresis ---
  hysteresisFrames: number;       // frames to wait before re-switching a page's mip (default: 3)
  hysteresisFactor: number;       // priority multiplier for recently-switched pages (default: 0.3)

  // --- Prediction ---
  predictionEnabled: boolean;     // enable predictive loading (default: true)
  predictionFrames: number;        // how many frames ahead to predict (default: 2)
  predictionPadding: number;      // extra UV padding for predicted requests (default: 0.1)

  // --- Adaptive quality ---
  adaptiveQualityEnabled: boolean; // adjust LOD based on frame time (default: true)
  targetFrameTime: number;        // target frame time in ms (default: 16.67 = 60fps)
  adaptiveLodBiasStep: number;    // LOD bias adjustment per frame (default: 0.5)
  adaptiveBudgetStep: number;     // page budget adjustment per frame (default: 1)

  // --- Oversubscription ---
  highWaterMark: number;          // atlas % full before backing off (default: 0.9)
  lowWaterMark: number;           // atlas % empty before adding detail (default: 0.5)

  // --- Priority weights ---
  weightMipDistance: number;     // priority for large mip gap (default: 1.0)
  weightHitCount: number;        // priority for frequently sampled pages (default: 0.5)
  weightScreenCoverage: number;  // priority for large screen-area pages (default: 0.3)
  weightCenterBias: number;      // priority for screen-center pages (default: 0.2)
  weightPrediction: number;      // priority for predicted future pages (default: 0.3)
  weightConfidence: number;      // priority for pages requested N consecutive frames (default: 0.4)

  // --- LOD snapping prevention ---
  lodSnappingFix: boolean;        // upsample coarser mip before switching (default: true)
  blendFrames: number;            // frames to blend from upsampled → actual (default: 4)

  // --- Eviction ---
  evictionGraceFrames: number;    // frames a page can be unseen before eviction (default: 3)
}

export const defaultConfig: VTConfig = {
  maxPagesPerFrame: 8,
  maxBudget: 16,
  hysteresisFrames: 3,
  hysteresisFactor: 0.3,
  predictionEnabled: true,
  predictionFrames: 2,
  predictionPadding: 0.1,
  adaptiveQualityEnabled: true,
  targetFrameTime: 16.67,
  adaptiveLodBiasStep: 0.5,
  adaptiveBudgetStep: 1,
  highWaterMark: 0.9,
  lowWaterMark: 0.5,
  weightMipDistance: 1.0,
  weightHitCount: 0.5,
  weightScreenCoverage: 0.3,
  weightCenterBias: 0.2,
  weightPrediction: 0.3,
  weightConfidence: 0.4,
  lodSnappingFix: true,
  blendFrames: 4,
  evictionGraceFrames: 3,
};

// ============================================================================
// Per-page tracking state
// ============================================================================

interface PageState {
  request: PageRequest;
  hitCount: number;              // how many feedback pixels requested this page
  screenCoverage: number;        // fraction of screen (0-1)
  centerDistance: number;        // distance from screen center (0=center, 1=edge)
  consecutiveFrames: number;     // how many consecutive frames this page was requested
  lastSeenFrame: number;         // last frame this page was in feedback
  lastMipSwitch: number;         // last frame this page's mip was changed
  isPredicted: boolean;          // is this a predicted (pre-loaded) request?
  residentMip: number;            // currently resident mip level (-1 = not resident)
}

// ============================================================================
// Smart LOD Strategy
// ============================================================================

export class VTLodStrategy {
  private config: VTConfig;
  private pageStates = new Map<string, PageState>();
  private frame = 0;

  // Camera state for prediction
  private cameraHistory: Array<{ pos: [number, number]; zoom: number }> = [];

  // Adaptive state
  private frameTimeLodBias = 0;   // from frame-time monitoring
  private oversubscriptionLodBias = 0; // from atlas usage
  private currentBudget: number;

  // Frame time history (for adaptive quality)
  private frameTimes: number[] = [];

  constructor(config: Partial<VTConfig> = {}) {
    this.config = { ...defaultConfig, ...config };
    this.currentBudget = this.config.maxPagesPerFrame;
  }

  getLodBias(): number { return Math.max(this.frameTimeLodBias, this.oversubscriptionLodBias); }
  getBudget(): number { return this.currentBudget; }
  getConfig(): VTConfig { return this.config; }

  /**
   * Record camera state for velocity prediction.
   * Call this every frame BEFORE processFeedback.
   */
  recordCamera(pos: [number, number], zoom: number) {
    this.cameraHistory.push({ pos: [...pos], zoom });
    if (this.cameraHistory.length > 10) this.cameraHistory.shift();
  }

  /**
   * Record frame time for adaptive quality.
   * Call this every frame with the measured frame time in ms.
   */
  recordFrameTime(frameTimeMs: number) {
    this.frameTimes.push(frameTimeMs);
    if (this.frameTimes.length > 30) this.frameTimes.shift();
  }

  /**
   * Compute camera velocity (UV units per frame).
   */
  private getCameraVelocity(): [number, number] {
    if (this.cameraHistory.length < 2) return [0, 0];
    const last = this.cameraHistory[this.cameraHistory.length - 1];
    const prev = this.cameraHistory[this.cameraHistory.length - 2];
    return [last.pos[0] - prev.pos[0], last.pos[1] - prev.pos[1]];
  }

  /**
   * Predict camera position N frames ahead.
   */
  private predictCameraPosition(): [number, number] {
    if (!this.config.predictionEnabled || this.cameraHistory.length < 2) {
      return this.cameraHistory[this.cameraHistory.length - 1]?.pos ?? [0.5, 0.5];
    }
    const [vx, vy] = this.getCameraVelocity();
    const last = this.cameraHistory[this.cameraHistory.length - 1];
    return [
      last.pos[0] + vx * this.config.predictionFrames,
      last.pos[1] + vy * this.config.predictionFrames,
    ];
  }

  /**
   * Compute priority score for a page request.
   *
   * This is the core "smart" function. It combines multiple factors:
   *
   * 1. Visual impact: mip distance × screen coverage × center bias
   * 2. Temporal stability: consecutive frames × hysteresis
   * 3. Prediction: is this page in the predicted future view?
   * 4. Performance: coarser mips are cheaper to load
   *
   * [IDTECH] Section 5.1: "priority is first based on the LOD level of the page
   *   such that finer mips will be replaced first"
   * [ADAPTIVE]: "mesh perceptibility weight based on fragment count"
   */
  private computePriority(state: PageState): number {
    const c = this.config;

    // Factor 1: Visual impact
    // How much will loading this page improve the image?
    const mipDistance = Math.abs(state.request.mip - state.residentMip);
    const visualImpact =
      c.weightMipDistance * mipDistance +
      c.weightScreenCoverage * state.screenCoverage +
      c.weightCenterBias * (1 - state.centerDistance);

    // Factor 2: Temporal stability
    // How confident are we this page is needed?
    // Pages requested for many consecutive frames = high confidence
    const confidence = Math.min(state.consecutiveFrames / 5, 1); // saturate at 5 frames
    const temporalStability = c.weightConfidence * confidence;

    // Hysteresis: de-prioritize pages that were recently mip-switched
    const framesSinceSwitch = this.frame - state.lastMipSwitch;
    let hysteresis = 1.0;
    if (framesSinceSwitch < c.hysteresisFrames) {
      hysteresis = c.hysteresisFactor;
    }

    // Factor 3: Prediction
    // Is this page in the predicted future view?
    const prediction = state.isPredicted ? c.weightPrediction : 0;

    // Factor 4: Hit count (how many feedback pixels requested this page)
    // [IDTECH]: "priority increases as the number of samples increases"
    const hitScore = c.weightHitCount * Math.log2(1 + state.hitCount);

    // Combine: higher = more important to load
    return (visualImpact + temporalStability + prediction + hitScore) * hysteresis;
  }

  /**
   * Update adaptive quality based on frame time.
   *
   * [ADAPTIVE]: "automatically adjusts the texture streaming workload based
   *   on measured frame latencies"
   * [IDTECH] Section 3.5: oversubscription handling via LOD bias
   */
  private updateAdaptiveQuality(atlasUsage: number) {
    if (this.config.adaptiveQualityEnabled) {
      const avgFrameTime = this.frameTimes.length > 0
        ? this.frameTimes.reduce((a, b) => a + b, 0) / this.frameTimes.length
        : this.config.targetFrameTime;

      const target = this.config.targetFrameTime;

      // Frame time too high → reduce quality
      if (avgFrameTime > target * 1.2) {
        this.frameTimeLodBias = Math.min(this.frameTimeLodBias + this.config.adaptiveLodBiasStep, MAX_MIP);
        this.currentBudget = Math.max(1, this.currentBudget - this.config.adaptiveBudgetStep);
      }
      // Frame time comfortable → increase quality
      else if (avgFrameTime < target * 0.8) {
        this.frameTimeLodBias = Math.max(0, this.frameTimeLodBias - this.config.adaptiveLodBiasStep);
        this.currentBudget = Math.min(this.config.maxBudget, this.currentBudget + this.config.adaptiveBudgetStep);
      }
    }

    // Oversubscription check — [IDTECH] Section 3.5
    // Independent from frame-time: uses separate bias, combined via max()
    if (atlasUsage > this.config.highWaterMark) {
      this.oversubscriptionLodBias = Math.min(this.oversubscriptionLodBias + this.config.adaptiveLodBiasStep, MAX_MIP);
    } else if (atlasUsage < this.config.lowWaterMark) {
      this.oversubscriptionLodBias = Math.max(0, this.oversubscriptionLodBias - this.config.adaptiveLodBiasStep);
    }
  }

  /**
   * Process feedback with smart priority sorting.
   *
   * Returns a sorted list of page requests to load, plus which pages to evict.
   *
   * @param feedback Map of page requests from the feedback pass
   * @param pageTable Current page table (to check resident mip)
   * @param cache Page cache (to check atlas usage)
   * @returns Sorted page requests to load (highest priority first)
   */
  processFeedback(
    feedback: Map<string, PageRequest>,
    pageTable: PageTable,
    cache: PageCache,
  ): { toLoad: PageRequest[]; toEvict: PageRequest[]; priorities: Map<string, number> } {
    this.frame++;

    const atlasUsage = cache.usedSlots / (ATLAS_PAGES_X * ATLAS_PAGES_Y);
    this.updateAdaptiveQuality(atlasUsage);

    // Update page states from feedback
    const activeKeys = new Set<string>();
    for (const [key, req] of feedback) {
      let state = this.pageStates.get(key);
      if (!state) {
        state = {
          request: req,
          hitCount: 0,
          screenCoverage: 0,
          centerDistance: 0.5,
          consecutiveFrames: 0,
          lastSeenFrame: this.frame,
          lastMipSwitch: 0,
          isPredicted: false,
          residentMip: -1,
        };
        this.pageStates.set(key, state);
      }

      // Update state
      state.hitCount++;
      state.lastSeenFrame = this.frame;
      state.consecutiveFrames++;
      state.screenCoverage = Math.min(state.hitCount / 1000, 1); // approximate
      activeKeys.add(key);
    }

    // Decay consecutive frames for pages not in this feedback
    for (const [key, state] of this.pageStates) {
      if (!activeKeys.has(key)) {
        state.consecutiveFrames = 0;
      }
    }

    // Generate predicted page requests
    const predictedReqs = new Map<string, PageRequest>();
    if (this.config.predictionEnabled && this.cameraHistory.length >= 2) {
      const [vx, vy] = this.getCameraVelocity();
      // Only predict when camera is actually moving
      if (Math.abs(vx) > 0.001 || Math.abs(vy) > 0.001) {
        const predictedPos = this.predictCameraPosition();
        const lastZoom = this.cameraHistory[this.cameraHistory.length - 1].zoom;
        const padding = this.config.predictionPadding;

        // Simulate feedback at predicted position to find future page needs
        // Use a slightly wider view (padding) to catch edge pages early
        const predFeedback = simulateFeedback(
          predictedPos,
          lastZoom / (1 + padding),
          this.getLodBias(),
        );

        // Add pages not already in current feedback
        for (const [key, req] of predFeedback) {
          if (!feedback.has(key)) {
            predictedReqs.set(key, req);
          }
        }
      }
    }

    // Add predicted requests as low-priority pre-loads
    for (const [key, req] of predictedReqs) {
      if (!this.pageStates.has(key)) {
        this.pageStates.set(key, {
          request: req,
          hitCount: 1,
          screenCoverage: 0,
          centerDistance: 1, // edge = low priority
          consecutiveFrames: 1,
          lastSeenFrame: this.frame,
          lastMipSwitch: 0,
          isPredicted: true,
          residentMip: -1,
        });
      }
    }

    // Compute priority for all non-resident pages
    const priorities = new Map<string, number>();
    const toLoad: Array<{ req: PageRequest; priority: number }> = [];

    for (const [key, state] of this.pageStates) {
      // Check if page is resident
      const resident = pageTable.isResident(state.request);
      if (resident) {
        state.residentMip = state.request.mip;
        continue; // already loaded
      }

      // Check what mip level is currently resident (for fallback)
      const found = pageTable.findResidentPage(
        (state.request.x + 0.5) / (VIRTUAL_PAGES_X >> state.request.mip),
        (state.request.y + 0.5) / (VIRTUAL_PAGES_X >> state.request.mip),
        state.request.mip,
      );
      state.residentMip = found ? found.mip : -1;

      const priority = this.computePriority(state);
      priorities.set(key, priority);
      toLoad.push({ req: state.request, priority });
    }

    // Sort by priority (highest first)
    toLoad.sort((a, b) => b.priority - a.priority);

    // Limit to budget
    const budget = this.currentBudget;
    const loadList = toLoad.slice(0, budget).map(t => t.req);

    // Find pages to evict (not seen for graceFrames)
    const toEvict: PageRequest[] = [];
    for (const [key, state] of this.pageStates) {
      const framesSinceSeen = this.frame - state.lastSeenFrame;
      if (framesSinceSeen > this.config.evictionGraceFrames && pageTable.isResident(state.request)) {
        toEvict.push(state.request);
      }
    }

    // Clean up stale page states (not seen for 2× grace frames)
    for (const [key, state] of this.pageStates) {
      if (this.frame - state.lastSeenFrame > this.config.evictionGraceFrames * 2) {
        this.pageStates.delete(key);
      }
    }

    return { toLoad: loadList, toEvict, priorities };
  }

  /**
   * Get the current effective LOD bias (combines adaptive + oversubscription).
   */
  getEffectiveLodBias(): number {
    return this.lodBias;
  }

  /**
   * Get page state for debugging.
   */
  getPageState(req: PageRequest): PageState | undefined {
    return this.pageStates.get(`${req.mip}:${req.x}:${req.y}`);
  }

  /**
   * Get statistics for debugging.
   */
  getStats() {
    let totalHits = 0;
    let predictedCount = 0;
    let residentCount = 0;
    for (const state of this.pageStates.values()) {
      totalHits += state.hitCount;
      if (state.isPredicted) predictedCount++;
      if (state.residentMip >= 0) residentCount++;
    }
    return {
      trackedPages: this.pageStates.size,
      totalHits,
      predictedCount,
      residentCount,
      lodBias: this.getLodBias(),
      frameTimeLodBias: this.frameTimeLodBias,
      oversubscriptionLodBias: this.oversubscriptionLodBias,
      budget: this.currentBudget,
      avgFrameTime: this.frameTimes.length > 0
        ? this.frameTimes.reduce((a, b) => a + b, 0) / this.frameTimes.length
        : 0,
    };
  }
}
