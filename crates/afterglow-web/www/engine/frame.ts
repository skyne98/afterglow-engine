// Deterministic frame order — the per-frame function that orchestrates
// worker polling, structural changes, transform sync, and GPU uploads
// in a fixed order so promise/microtask timing never affects the render phase.

import type { RenderAdapter } from './render-adapter.js';
import type { RenderFrame } from './types.js';

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

/**
 * Prepare one afterglow frame in deterministic order:
 *
 * 1. Poll workers (resolve pending async calls)
 * 2. Apply structural commands (spawn/despawn from workers)
 * 3. Commit bitECS deferred query removals
 * 4. Ingest worker value outputs (physics poses)
 * 5. Flush structural changes (attach/detach render proxies)
 * 6. Rebuild hierarchy if topology changed
 * 7. Sync transforms (batched raw math → GPU buffers)
 * 8. Sync unique proxies (lights, skinned meshes)
 * 9. Flush coalesced GPU uploads
 *
 * After this returns, the host calls `renderer.render()`.
 */
export function prepareAfterglowFrame(
  frame: RenderFrame,
  workerInput: RenderWorkerInput | null,
  adapter: RenderAdapter,
): void {
  // 1. Make completed worker data visible.
  if (workerInput) {
    workerInput.poll();
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
