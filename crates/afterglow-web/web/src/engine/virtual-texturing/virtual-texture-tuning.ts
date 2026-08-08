import { defineResource } from '../core/resource.ts';

// ============================================================================
// Central VT upload tuning
// ============================================================================

/** Bootstrap-owned fixed storage for asynchronous VT page generations. */
export interface VirtualTextureRuntimeCapacities {
  maxPendingPages: number;
  maxPendingBytes: number;
}

/** Bootstrap-only policy for bounded per-frame atlas/page-table commits. */
export interface VirtualTextureTuningConfig {
  minUploadsPerPoll: number;
  baselineUploadsPerPoll: number;
  maxUploadsPerPoll: number;
  minUploadBudgetMs: number;
  baselineUploadBudgetMs: number;
  maxUploadBudgetMs: number;
  uploadBudgetStepMs: number;
  targetFrameMs: number;
  overloadMultiplier: number;
  overloadSamples: number;
  sampleWindow: number;
  stableWindowsBeforeProbe: number;
  probeCooldownWindows: number;
  /** Cap the physical atlas dimension (texels) below the device's
   *  maxTextureDimension2D. Useful on iGPUs where the default (max 2D
   *  dimension, e.g. 16384 -> ~1 GB atlas) blows the small VRAM carve-out
   *  and saturates shared memory bandwidth. 0 = use device max. */
  atlasMaxDimension?: number;
}

export const DEFAULT_VIRTUAL_TEXTURE_TUNING: Readonly<VirtualTextureTuningConfig> = {
  minUploadsPerPoll: 1,
  baselineUploadsPerPoll: 2,
  maxUploadsPerPoll: 4,
  minUploadBudgetMs: 0.10,
  baselineUploadBudgetMs: 0.20,
  maxUploadBudgetMs: 0.35,
  uploadBudgetStepMs: 0.05,
  targetFrameMs: 1000 / 60,
  overloadMultiplier: 1.25,
  overloadSamples: 2,
  sampleWindow: 15,
  stableWindowsBeforeProbe: 1,
  probeCooldownWindows: 60,
};

/**
 * Central, allocation-free VT upload tuner. It discovers throughput only under
 * real backlog: after several clean windows it probes one bounded step above
 * the configured device caps. A repeated presentation miss rolls any promoted
 * setting back immediately to the independently validated baseline and applies
 * a cooldown, so powerful devices can climb while
 * weaker devices do not continuously oscillate around a failing cap.
 */
export class VirtualTextureTuning {
  readonly minUploadsPerPoll: number;
  readonly baselineUploadsPerPoll: number;
  readonly maxUploadsPerPoll: number;
  readonly minUploadBudgetMs: number;
  readonly baselineUploadBudgetMs: number;
  readonly maxUploadBudgetMs: number;
  readonly uploadBudgetStepMs: number;
  readonly targetFrameMs: number;
  readonly overloadFrameMs: number;
  readonly overloadSamples: number;
  readonly sampleWindow: number;
  readonly stableWindowsBeforeProbe: number;
  readonly probeCooldownWindows: number;
  /** Physical atlas dimension cap (0 = device max). See VirtualTextureTuningConfig. */
  readonly atlasMaxDimension: number;
  uploadsPerPoll: number;
  uploadBudgetMs: number;
  bestSafeUploadsPerPoll: number;
  bestSafeUploadBudgetMs: number;
  private samples = 0;
  private overloaded = 0;
  private stableWindows = 0;
  private cooldownWindows = 0;
  private probing = false;
  downshifts = 0;
  recoveries = 0;
  probes = 0;
  probeRejections = 0;

  constructor(config: Readonly<Partial<VirtualTextureTuningConfig>> = DEFAULT_VIRTUAL_TEXTURE_TUNING) {
    const cfg: Readonly<VirtualTextureTuningConfig> = { ...DEFAULT_VIRTUAL_TEXTURE_TUNING, ...config };
    const integers = [cfg.minUploadsPerPoll, cfg.baselineUploadsPerPoll,
      cfg.maxUploadsPerPoll, cfg.overloadSamples, cfg.sampleWindow,
      cfg.stableWindowsBeforeProbe, cfg.probeCooldownWindows];
    if (integers.some(value => !Number.isInteger(value) || value < 1) ||
        cfg.minUploadsPerPoll > cfg.baselineUploadsPerPoll ||
        cfg.baselineUploadsPerPoll > cfg.maxUploadsPerPoll ||
        !Number.isFinite(cfg.minUploadBudgetMs) || !Number.isFinite(cfg.baselineUploadBudgetMs) ||
        !Number.isFinite(cfg.maxUploadBudgetMs) || cfg.minUploadBudgetMs <= 0 ||
        cfg.minUploadBudgetMs > cfg.baselineUploadBudgetMs ||
        cfg.baselineUploadBudgetMs > cfg.maxUploadBudgetMs ||
        !Number.isFinite(cfg.uploadBudgetStepMs) || cfg.uploadBudgetStepMs <= 0 ||
        !Number.isFinite(cfg.targetFrameMs) || cfg.targetFrameMs <= 0 ||
        !Number.isFinite(cfg.overloadMultiplier) || cfg.overloadMultiplier <= 1)
      throw new RangeError('invalid virtual-texture tuning configuration');
    this.minUploadsPerPoll = cfg.minUploadsPerPoll;
    this.baselineUploadsPerPoll = cfg.baselineUploadsPerPoll;
    this.maxUploadsPerPoll = cfg.maxUploadsPerPoll;
    this.minUploadBudgetMs = cfg.minUploadBudgetMs;
    this.baselineUploadBudgetMs = cfg.baselineUploadBudgetMs;
    this.maxUploadBudgetMs = cfg.maxUploadBudgetMs;
    this.uploadBudgetStepMs = cfg.uploadBudgetStepMs;
    this.targetFrameMs = cfg.targetFrameMs;
    this.overloadFrameMs = cfg.targetFrameMs * cfg.overloadMultiplier;
    this.overloadSamples = cfg.overloadSamples;
    this.sampleWindow = cfg.sampleWindow;
    this.stableWindowsBeforeProbe = cfg.stableWindowsBeforeProbe;
    this.probeCooldownWindows = cfg.probeCooldownWindows;
    this.atlasMaxDimension = cfg.atlasMaxDimension ?? 0;
    this.uploadsPerPoll = cfg.baselineUploadsPerPoll;
    this.uploadBudgetMs = cfg.baselineUploadBudgetMs;
    this.bestSafeUploadsPerPoll = cfg.baselineUploadsPerPoll;
    this.bestSafeUploadBudgetMs = cfg.baselineUploadBudgetMs;
  }

  private resetWindow(): void {
    this.samples = 0;
    this.overloaded = 0;
  }

  private lowerOneStep(): void {
    if (this.uploadsPerPoll > this.minUploadsPerPoll) {
      this.uploadsPerPoll--;
    } else {
      this.uploadBudgetMs = Math.max(this.minUploadBudgetMs, this.uploadBudgetMs - this.uploadBudgetStepMs);
    }
    this.bestSafeUploadsPerPoll = this.uploadsPerPoll;
    this.bestSafeUploadBudgetMs = this.uploadBudgetMs;
    this.downshifts++;
  }

  private probeOneStep(): boolean {
    if (this.uploadsPerPoll < this.maxUploadsPerPoll) {
      this.uploadsPerPoll++;
    } else if (this.uploadBudgetMs < this.maxUploadBudgetMs) {
      this.uploadBudgetMs = Math.min(this.maxUploadBudgetMs, this.uploadBudgetMs + this.uploadBudgetStepMs);
    } else {
      return false;
    }
    this.probing = true;
    this.probes++;
    return true;
  }

  // @hot-no-alloc-begin VirtualTextureTuning.recordFrameTime
  recordFrameTime(frameMs: number, backlog: number): void {
    if (!Number.isFinite(frameMs) || frameMs <= 0) {
      this.stableWindows = 0;
      this.resetWindow();
      return;
    }
    // Preserve evidence across short empty gaps: gameplay streaming arrives in
    // bursts, and resetting here prevented any burst from calibrating the cap.
    if (backlog <= 0) return;
    this.samples++;
    if (frameMs > this.overloadFrameMs) this.overloaded++;
    if (this.samples < this.sampleWindow) return;

    if (this.overloaded >= this.overloadSamples) {
      // The bootstrap baseline was independently validated. Any promoted cap
      // that starts missing presentation deadlines abandons its whole probe
      // ladder at once; only a failing baseline tightens one step further.
      if (this.probing || this.uploadsPerPoll > this.baselineUploadsPerPoll ||
          this.uploadBudgetMs > this.baselineUploadBudgetMs) {
        this.uploadsPerPoll = this.baselineUploadsPerPoll;
        this.uploadBudgetMs = this.baselineUploadBudgetMs;
        this.bestSafeUploadsPerPoll = this.baselineUploadsPerPoll;
        this.bestSafeUploadBudgetMs = this.baselineUploadBudgetMs;
        this.probing = false;
        this.probeRejections++;
      } else {
        this.lowerOneStep();
      }
      this.stableWindows = 0;
      this.cooldownWindows = this.probeCooldownWindows;
    } else if (this.overloaded === 0) {
      if (this.probing) {
        this.bestSafeUploadsPerPoll = this.uploadsPerPoll;
        this.bestSafeUploadBudgetMs = this.uploadBudgetMs;
        this.probing = false;
        this.recoveries++;
        this.stableWindows = 0;
      } else if (this.cooldownWindows > 0) {
        this.cooldownWindows--;
        this.stableWindows = 0;
      } else {
        this.stableWindows++;
        if (this.stableWindows >= this.stableWindowsBeforeProbe) {
          this.probeOneStep();
          this.stableWindows = 0;
        }
      }
    } else {
      this.stableWindows = 0;
    }
    this.resetWindow();
  }
  // @hot-no-alloc-end VirtualTextureTuning.recordFrameTime
}

export const VirtualTextureTuningRes = defineResource<VirtualTextureTuning>(
  'virtualTextureTuning',
  () => new VirtualTextureTuning(),
);

