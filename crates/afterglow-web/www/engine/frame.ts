// Deterministic frame order — the per-frame function that orchestrates
// worker polling, structural changes, transform sync, and GPU uploads
// in a fixed order so promise/microtask timing never affects the render phase.

import type { RenderFrame } from './types.ts';
import type { VirtualPageRequest } from './virtual-texture.ts';
import { EnginePhase, type EngineMemory } from './engine-memory.ts';
import { BudgetDecision, FrameBudget, FrameStage } from './frame-budget.ts';

export interface FrameRenderAdapter {
  prepareFrame(frame: RenderFrame): void;
}

/**
 * A worker input interface — the render adapter doesn't depend on the
 * transport (RPC, SAB, ring buffers). It just drains data synchronously.
 */
export interface RenderWorkerInput {
  /** Poll for completed async worker calls (resolves pending promises). */
  poll(): void;
  /** Drain any structural commands (spawn/despawn/reparent) from workers. */
  drainStructuralCommands?(adapter: FrameRenderAdapter): void;
  /** Drain physics pose batches and apply to ECS. */
  drainPoseBatches?(adapter: FrameRenderAdapter): void;
}

export interface FrameVirtualTextureStore {
  recordFrameTime(frameTimeMs: number): void;
  processFeedback(feedback: ReadonlyMap<unknown, VirtualPageRequest>): void;
  poll(): void;
}

/** Optional VT input — if present, VT feedback is processed each frame. */
export interface VTInput {
  /** The virtual texture store. */
  store: FrameVirtualTextureStore;
  /** Feedback from the previous frame's feedback pass (page requests). */
  feedback: ReadonlyMap<unknown, VirtualPageRequest>;
  /** Latest rAF presentation interval in milliseconds, for VT auto-tuning. */
  frameTimeMs?: number;

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
  adapter: FrameRenderAdapter,
  vtInput?: VTInput,
  memory?: EngineMemory,
  budget?: FrameBudget,
): void {
  // Rewind fixed frame scratch and establish cumulative stage deadlines before
  // any engine system can consume frame capacity.
  if (memory && memory.phase !== EnginePhase.GameplaySealed && memory.phase !== EnginePhase.LoadingScreen)
    throw new Error('EngineMemory must be sealed before frame orchestration');
  memory?.beginFrame();
  budget?.beginFrame(frame.frameId, frame.deltaSeconds * 1000);

  // 1. Make completed worker data visible.
  if (workerInput) {
    budget?.beginStage(FrameStage.WorkerPoll, true);
    workerInput.poll();
    budget?.endStage(FrameStage.WorkerPoll);
  }

  // 2. VT: process feedback from previous frame (1-frame latency).
  //    [IDTECH] Section 3.4: "it is typically fine to use a frame old data"
  if (vtInput) {
    budget?.beginStage(FrameStage.VirtualTexture, true);
    if (vtInput.frameTimeMs !== undefined) vtInput.store.recordFrameTime(vtInput.frameTimeMs);
    vtInput.store.processFeedback(vtInput.feedback);
    vtInput.store.poll();
    budget?.endStage(FrameStage.VirtualTexture);
  }

  // 2. Apply structural worker commands on the ECS/main thread.
  if (workerInput?.drainStructuralCommands &&
      (!budget || budget.beginStage(FrameStage.StructuralCommands) === BudgetDecision.Run)) {
    workerInput.drainStructuralCommands(adapter);
    budget?.endStage(FrameStage.StructuralCommands);
  }

  // 3. Resolve bitECS deferred query removals once.
  // (Imported dynamically to avoid circular dep in the adapter)
  // commitRemovals is called inside adapter.prepareFrame().

  // 4. Ingest worker value outputs.
  if (workerInput?.drainPoseBatches &&
      (!budget || budget.beginStage(FrameStage.PoseBatches) === BudgetDecision.Run)) {
    workerInput.drainPoseBatches(adapter);
    budget?.endStage(FrameStage.PoseBatches);
  }

  // 5-9. Structural reconciliation, hierarchy rebuild, transform sync,
  // unique proxy sync, GPU upload flush. Rendering preparation is required;
  // overruns are recorded but never leave GPU state half-committed.
  budget?.beginStage(FrameStage.RenderPrepare, true);
  adapter.prepareFrame(frame);
  budget?.endStage(FrameStage.RenderPrepare);
}
