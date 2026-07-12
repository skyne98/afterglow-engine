import type { VirtualTextureStore } from './virtual-texture.js';

/** Stable diagnostic color for a sampled virtual mip. */
export const VT_MIP_DEBUG_WGSL = /* wgsl */ `
fn vtMipDebugColor(mip: u32) -> vec4f {
  let level = f32(mip);
  return vec4f(
    fract(level * 0.37 + 0.15),
    fract(level * 0.61 + 0.35),
    fract(level * 0.83 + 0.55),
    1.0
  );
}
`;

/** Convenience controller used by engine debug panels and demos. */
export class VirtualTextureDebugController {
  constructor(readonly store: VirtualTextureStore) {}

  setSlowMotion(enabled: boolean, pagesPerStep = 1): void {
    this.store.setDebugPageBudget(enabled ? pagesPerStep : null);
  }

  pause(paused: boolean): void { this.store.setDebugPaused(paused); }
  snapshot() { return this.store.getDebugSnapshot(); }
}
