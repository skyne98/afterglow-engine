import * as THREE from 'three';
import { pagesAtMipAxis } from './virtual-texture-layout.ts';
import type { VirtualPageRequest, VirtualTextureStore } from './virtual-texture.ts';

/**
 * Reduced-resolution RG32Uint feedback renderer with asynchronous readback.
 * The supplied scene must use feedback materials whose fragment output matches
 * VT_FEEDBACK_WGSL. Readback never blocks the frame that submits it.
 */
export class VirtualTextureFeedbackPass {
  readonly scale: number;
  /** Actual feedback pixels per physical display pixel, updated by resize(). */
  readonly pixelScale = new THREE.Vector2(1, 1);
  readonly target: THREE.RenderTarget;
  private width = 1;
  private height = 1;
  private pending = false;
  private readonly feedbackMaps = [
    new Map<number, VirtualPageRequest>(),
    new Map<number, VirtualPageRequest>(),
  ];
  private buildMapIndex = 0;
  private completed: Map<number, VirtualPageRequest> | null = null;
  private requestPool: VirtualPageRequest[] = [];
  private readonly seenMips = new Uint8Array(64);
  private readonly latestMips: number[] = [];

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
    if (!(displayWidth > 0 && displayHeight > 0)) throw new RangeError('display dimensions must be positive');
    const width = Math.max(1, Math.ceil(displayWidth * this.scale));
    const height = Math.max(1, Math.ceil(displayHeight * this.scale));
    this.pixelScale.set(width / displayWidth, height / displayHeight);
    if (width === this.width && height === this.height && this.requestPool.length >= width * height) return;
    this.width = width;
    this.height = height;
    this.target.setSize(width, height);
    const capacity = width * height;
    if (this.requestPool.length < capacity) {
      const previous = this.requestPool.length;
      this.requestPool.length = capacity;
      for (let index = previous; index < capacity; index++)
        this.requestPool[index] = { path: '', mip: 0, x: 0, y: 0 };
    }
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
    if (this.pending || this.completed !== null) return false;
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
        const requests = this.feedbackMaps[this.buildMapIndex];
        requests.clear();
        for (let mip = 0; mip < this.seenMips.length; mip++) this.seenMips[mip] = 0;
        let requestCount = 0;
        for (let index = 0; index + 1 < words.length; index += 2) {
          const packed = words[index];
          if ((packed & 0x80000000) === 0) continue;
          const mip = packed & 0x3f;
          const x = (packed >>> 6) & 0x7ff;
          const y = (packed >>> 17) & 0x7ff;
          const entry = store.getEntryById(words[index + 1]);
          if (!entry || mip > entry.textureMaxMip) continue;
          const tail = mip > entry.maxMip;
          const gridWidth = tail ? 1 : pagesAtMipAxis(entry.pageTableLayout.width, mip);
          const gridHeight = tail ? 1 : pagesAtMipAxis(entry.pageTableLayout.baseHeight, mip);
          if (x >= gridWidth || y >= gridHeight) continue;
          const requestMip = tail ? entry.tailFirstMip! : mip;
          const requestX = tail ? 0 : x;
          const requestY = tail ? 0 : y;
          const local = tail
            ? 0x10000000
            : ((requestMip & 0x3f) | ((requestX & 0x7ff) << 6) | ((requestY & 0x7ff) << 17)) >>> 0;
          const key = entry.textureId * 0x20000000 + local;
          const pixel = index >> 1;
          const pixelX = pixel % this.width;
          const pixelY = Math.floor(pixel / this.width);
          const normalizedX = ((pixelX + 0.5) * 2 / this.width) - 1;
          const normalizedY = ((pixelY + 0.5) * 2 / this.height) - 1;
          const screenPriority = Math.min(255, Math.floor(
            (normalizedX * normalizedX + normalizedY * normalizedY) * 128,
          ));
          const existing = requests.get(key);
          if (existing) {
            existing.screenPriority = Math.min(existing.screenPriority ?? 255, screenPriority);
            existing.coverage = Math.min(0xffff, (existing.coverage ?? 1) + 1);
            continue;
          }
          const request = this.requestPool[requestCount++];
          request.textureId = entry.textureId;
          request.path = entry.path;
          request.mip = requestMip;
          request.x = requestX;
          request.y = requestY;
          request.tail = tail ? true : undefined;
          request.screenPriority = screenPriority;
          request.coverage = 1;
          request.priorityTier = undefined;
          requests.set(key, request);
          this.seenMips[mip] = 1;
        }
        this.latestMips.length = 0;
        for (let mip = 0; mip < this.seenMips.length; mip++)
          if (this.seenMips[mip] !== 0) this.latestMips.push(mip);
        this.completed = requests;
        this.buildMapIndex ^= 1;
      })
      .catch(error => console.error('[VT] feedback readback failed:', error))
      .finally(() => { this.pending = false; });
    return true;
  }

  getLatestMips(): readonly number[] { return this.latestMips; }

  /** Consume the newest completed readback; returns null until another readback completes. */
  consume(): Map<number, VirtualPageRequest> | null {
    const result = this.completed;
    this.completed = null;
    return result;
  }

  dispose(): void {
    this.target.dispose();
    this.feedbackMaps[0].clear();
    this.feedbackMaps[1].clear();
    this.completed = null;
    this.requestPool.length = 0;
  }
}
