// Deterministic frame order — the per-frame function that orchestrates
// worker polling, structural changes, transform sync, and GPU uploads
// in a fixed order so promise/microtask timing never affects the render phase.

import type { RenderAdapter } from './render-adapter.js';
import type { RenderFrame } from './types.js';
import type { VirtualTextureStore } from './virtual-texture.js';

/**
 * A worker input interface — the render adapter doesn't depend on the
 * transport (RPC, SAB, ring buffers). It just drains data synchronously.
 */
export interface RenderWorkerInput {
  /** Poll for completed async worker calls (resolves pending promises). */
  poll(): void;
  /** Drain any structural commands (spawn/despawn/reparent) from workers. */
  drainStructuralCommands?(adapter: RenderAdapter): void;
  /** Drain physics pose batches and apply to ECS. */
  drainPoseBatches?(adapter: RenderAdapter): void;
}

/** Optional VT input — if present, VT feedback is processed each frame. */
export interface VTInput {
  /** The virtual texture store. */
  store: VirtualTextureStore;
  /** Feedback from the previous frame's feedback pass (page requests). */
  feedback: Map<string, { mip: number; x: number; y: number }>;
  /** Camera position for prediction. */
  cameraPos?: [number, number];
  /** Camera zoom for prediction. */
  cameraZoom?: number;
  /** Measured frame time in ms (for adaptive quality). */
  frameTime?: number;
}

/**
 * Prepare one afterglow frame in deterministic order:
 *
 * 1. Poll workers (resolve pending async calls)
 * 2. VT: process feedback from previous frame (1-frame latency)
 *    — read back feedback buffer, analyze, load pages, update atlas + page table
 * 3. Apply structural commands (spawn/despawn from workers)
 * 4. Commit bitECS deferred query removals
 * 5. Ingest worker value outputs (physics poses)
 * 6. Flush structural changes (attach/detach render proxies)
 * 7. Rebuild hierarchy if topology changed
 * 8. Sync transforms (batched raw math → GPU buffers)
 * 9. Sync unique proxies (lights, skinned meshes)
 * 10. Flush coalesced GPU uploads
 *
 * After this returns, the host calls `renderer.render()`
 * which includes the VT feedback pass at 1/8 resolution.
 */
export function prepareAfterglowFrame(
  frame: RenderFrame,
  workerInput: RenderWorkerInput | null,
  adapter: RenderAdapter,
  vtInput?: VTInput,
): void {
  // 1. Make completed worker data visible.
  if (workerInput) {
    workerInput.poll();
  }

  // 2. VT: process feedback from previous frame (1-frame latency).
  //    [IDTECH] Section 3.4: "it is typically fine to use a frame old data"
  if (vtInput) {
    vtInput.store.poll();
    if (vtInput.frameTime !== undefined) {
      vtInput.store.recordFrameTime(vtInput.frameTime);
    }
    if (vtInput.cameraPos && vtInput.cameraZoom) {
      vtInput.store.recordCamera(vtInput.cameraPos, vtInput.cameraZoom);
    }
    vtInput.store.processFeedback(vtInput.feedback, vtInput.cameraPos, vtInput.cameraZoom);
  }

  // 2. Apply structural worker commands on the ECS/main thread.
  if (workerInput?.drainStructuralCommands) {
    workerInput.drainStructuralCommands(adapter);
  }

  // 3. Resolve bitECS deferred query removals once.
  // (Imported dynamically to avoid circular dep in the adapter)
  // commitRemovals is called inside adapter.prepareFrame().

  // 4. Ingest worker value outputs.
  if (workerInput?.drainPoseBatches) {
    workerInput.drainPoseBatches(adapter);
  }

  // 5-9. Structural reconciliation, hierarchy rebuild, transform sync,
  // unique proxy sync, GPU upload flush.
  adapter.prepareFrame(frame);
}
