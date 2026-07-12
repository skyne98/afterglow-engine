import * as THREE from 'three';
import { decodeFeedback } from './virtual-texture-feedback.js';
import type { VirtualPageRequest, VirtualTextureStore } from './virtual-texture.js';

/**
 * Reduced-resolution RG32Uint feedback renderer with asynchronous readback.
 * The supplied scene must use feedback materials whose fragment output matches
 * VT_FEEDBACK_WGSL. Readback never blocks the frame that submits it.
 */
export class VirtualTextureFeedbackPass {
  readonly scale: number;
  readonly target: THREE.RenderTarget;
  private width = 1;
  private height = 1;
  private pending = false;
  private completed = new Map<string, VirtualPageRequest>();

  constructor(scale = 0.125) {
    if (!(scale > 0 && scale <= 1)) throw new RangeError('feedback scale must be in (0, 1]');
    this.scale = scale;
    this.target = new THREE.RenderTarget(1, 1, {
      format: THREE.RGIntegerFormat,
      type: THREE.UnsignedIntType,
      minFilter: THREE.NearestFilter,
      magFilter: THREE.NearestFilter,
      generateMipmaps: false,
      depthBuffer: true,
    });
    this.target.texture.name = 'afterglow-vt-feedback-rg32uint';
  }

  resize(displayWidth: number, displayHeight: number): void {
    const width = Math.max(1, Math.ceil(displayWidth * this.scale));
    const height = Math.max(1, Math.ceil(displayHeight * this.scale));
    if (width === this.width && height === this.height) return;
    this.width = width;
    this.height = height;
    this.target.setSize(width, height);
  }

  /** Submit a pass and start readback unless the previous read is still pending. */
  submit(
    renderer: {
      getRenderTarget(): THREE.RenderTarget | null;
      setRenderTarget(target: THREE.RenderTarget | null): void;
      render(scene: THREE.Scene, camera: THREE.Camera): void;
      readRenderTargetPixelsAsync(
        target: THREE.RenderTarget, x: number, y: number, width: number, height: number,
      ): Promise<ArrayBufferView>;
    },
    feedbackScene: THREE.Scene,
    camera: THREE.Camera,
    store: VirtualTextureStore,
  ): boolean {
    if (this.pending) return false;
    const previous = renderer.getRenderTarget();
    renderer.setRenderTarget(this.target);
    renderer.render(feedbackScene, camera);
    renderer.setRenderTarget(previous);

    this.pending = true;
    renderer.readRenderTargetPixelsAsync(this.target, 0, 0, this.width, this.height)
      .then(raw => {
        const words = raw instanceof Uint32Array
          ? raw
          : new Uint32Array(raw.buffer, raw.byteOffset, Math.floor(raw.byteLength / 4));
        const requests = new Map<string, VirtualPageRequest>();
        for (let index = 0; index + 1 < words.length; index += 2) {
          const decoded = decodeFeedback(words[index], words[index + 1]);
          if (!decoded) continue;
          const entry = store.getEntryById(decoded.textureId);
          if (!entry) continue;
          if (decoded.mip > entry.textureMaxMip) continue;
          const tail = decoded.mip > entry.maxMip;
          const pages = tail ? 1 : Math.max(1, entry.pageGrid >> decoded.mip);
          if (decoded.x >= pages || decoded.y >= pages) continue;
          const request = tail
            ? { path: entry.path, mip: entry.tailFirstMip!, x: 0, y: 0, tail: true }
            : { path: entry.path, mip: decoded.mip, x: decoded.x, y: decoded.y };
          requests.set(`${request.path}:${request.mip}:${request.x}:${request.y}`, request);
        }
        this.completed = requests;
      })
      .catch(error => console.error('[VT] feedback readback failed:', error))
      .finally(() => { this.pending = false; });
    return true;
  }

  /** Consume the newest completed readback; subsequent calls return an empty map. */
  consume(): Map<string, VirtualPageRequest> {
    const result = this.completed;
    this.completed = new Map();
    return result;
  }

  dispose(): void {
    this.target.dispose();
    this.completed.clear();
  }
}
