// GPUStore — manages GPU resources (textures, buffers) with generation handles.
//
// Creates GPU resources, handles progressive streaming (mip-by-mip texture
// uploads), and tracks their lifecycle. Each resource has a generation number
// that increments when it's updated — consumers check this to know when to
// re-bind.
//
// The store is an ECS Resource — one instance per world, accessed via:
//   const gpu = GPUStoreRes.get(world);
//
// Integration with AssetStore:
//   AssetStore loads + transcodes → raw mip data (Uint8Array)
//   GPUStore creates textures → uploads mips progressively → generation++
//   Consumer checks generation → binds updated texture

import * as THREE from 'three';

import { Resource, defineResource } from './resource.js';

// --- handle types --------------------------------------------------------

/** State of a GPU resource. */
export type GPUState = 'creating' | 'partial' | 'ready' | 'error';

/** A handle to a GPU texture with progressive mip streaming. */
export class GPUTextureHandle {
  /** The live THREE.Texture (or null while creating). */
  texture: THREE.Texture | null = null;
  /** Incremented on each mip upload. */
  generation = 0;
  /** How many mip levels have been uploaded. */
  mipLevelsUploaded = 0;
  /** Total expected mip levels. */
  totalMipLevels: number;
  /** Texture width (mip 0). */
  width: number;
  /** Texture height (mip 0). */
  height: number;
  /** GPU format (e.g. 'bc7-rgba-unorm', 'rgba8unorm'). */
  format: string;
  /** Current state. */
  state: GPUState = 'creating';
  /** Resource name (for lookup). */
  readonly name: string;

  /** @internal */ constructor(name: string, width: number, height: number, format: string, totalMipLevels: number) {
    this.name = name;
    this.width = width;
    this.height = height;
    this.format = format;
    this.totalMipLevels = totalMipLevels;
  }

  get isReady(): boolean { return this.state === 'ready'; }
  get isPartial(): boolean { return this.state === 'partial'; }
}

/** A handle to a GPU buffer. */
export class GPUBufferHandle {
  /** The buffer data (Three.js BufferAttribute or raw). */
  attribute: THREE.BufferAttribute | null = null;
  /** Incremented on each update. */
  generation = 0;
  /** Buffer size in bytes. */
  size: number;
  /** Current state. */
  state: GPUState = 'creating';
  /** Resource name. */
  readonly name: string;

  /** @internal */ constructor(name: string, size: number) {
    this.name = name;
    this.size = size;
  }

  get isReady(): boolean { return this.state === 'ready'; }
}

// --- pending upload ------------------------------------------------------

/** @internal — a pending mip upload. */
interface PendingMipUpload {
  handle: GPUTextureHandle;
  level: number;
  data: Uint8Array;
  width: number;
  height: number;
}

// --- GPUStore ------------------------------------------------------------

/**
 * Manages GPU resources with generation handles and progressive streaming.
 *
 * - **Textures**: create with empty mip chain, upload mips one at a time.
 *   GPU sampler auto-selects the best available level.
 * - **Buffers**: create and upload data.
 * - **Time-sliced**: `poll(maxPerFrame)` limits uploads per frame to avoid hitches.
 */
export class GPUStore {
  /** Texture handles by name. */
  private readonly textures = new Map<string, GPUTextureHandle>();
  /** Buffer handles by name. */
  private readonly buffers = new Map<string, GPUBufferHandle>();
  /** Pending mip uploads (FIFO queue). */
  private readonly pendingUploads: PendingMipUpload[] = [];

  /**
   * Create a streaming texture with an empty mip chain.
   * Returns a handle immediately — the texture starts as null.
   * Call `uploadMip()` to progressively fill mip levels.
   */
  createTexture(
    name: string,
    width: number,
    height: number,
    format: string = 'rgba8unorm',
    totalMipLevels: number = 1,
  ): GPUTextureHandle {
    // Don't create duplicates.
    const existing = this.textures.get(name);
    if (existing) return existing;

    const handle = new GPUTextureHandle(name, width, height, format, totalMipLevels);

    // Create the THREE.Texture with an empty mipmaps array.
    // The texture is created at mip 0 size but starts empty.
    const texture = new THREE.DataTexture(
      new Uint8Array(0), // empty initially
      width,
      height,
      THREE.RGBAFormat,
    );
    texture.generateMipmaps = false;
    texture.mipmaps = [];
    texture.minFilter = THREE.LinearMipmapLinearFilter;
    texture.magFilter = THREE.LinearFilter;
    texture.colorSpace = THREE.SRGBColorSpace;

    handle.texture = texture;
    handle.state = totalMipLevels > 1 ? 'partial' : 'ready';
    this.textures.set(name, handle);

    return handle;
  }

  /**
   * Upload a single mip level to a texture.
   * The data is queued and processed during `poll()` (time-sliced).
   * The handle's generation is incremented when the upload is processed.
   */
  uploadMip(
    name: string,
    level: number,
    data: Uint8Array,
    width: number,
    height: number,
  ): void {
    const handle = this.textures.get(name);
    if (!handle) {
      console.warn(`[gpu] texture "${name}" not found for mip ${level}`);
      return;
    }
    if (!handle.texture) {
      console.warn(`[gpu] texture "${name}" has no THREE.Texture`);
      return;
    }

    this.pendingUploads.push({ handle, level, data, width, height });
  }

  /**
   * Process pending uploads. Call each frame.
   * `maxPerFrame` limits how many mips are uploaded per frame to avoid hitches.
   */
  poll(maxPerFrame: number = 2): void {
    let processed = 0;
    while (processed < maxPerFrame && this.pendingUploads.length > 0) {
      const upload = this.pendingUploads.shift()!;
      this.processUpload(upload);
      processed++;
    }
  }

  /** @internal — process a single mip upload. */
  private processUpload(upload: PendingMipUpload): void {
    const { handle, level, data, width, height } = upload;
    const texture = handle.texture!;

    // Ensure the mipmaps array is large enough.
    while (texture.mipmaps!.length <= level) {
      texture.mipmaps!.push(null as any);
    }

    // Set the mip level data.
    texture.mipmaps![level] = { data, width, height };

    // If this is mip 0, set it as the main image too.
    if (level === 0) {
      texture.image = { data, width, height };
    }

    // Mark for GPU upload.
    texture.needsUpdate = true;

    // Update handle state.
    handle.mipLevelsUploaded++;
    handle.generation++;

    if (handle.mipLevelsUploaded >= handle.totalMipLevels) {
      handle.state = 'ready';
    } else {
      handle.state = 'partial';
    }
  }

  /**
   * Create a buffer resource.
   * Returns a handle — upload data via `uploadBuffer()`.
   */
  createBuffer(name: string, size: number): GPUBufferHandle {
    const existing = this.buffers.get(name);
    if (existing) return existing;

    const handle = new GPUBufferHandle(name, size);
    handle.state = 'ready';
    this.buffers.set(name, handle);
    return handle;
  }

  /**
   * Upload data to a buffer. Creates or updates the BufferAttribute.
   */
  uploadBuffer(name: string, data: ArrayLike<number>, itemSize: number): void {
    const handle = this.buffers.get(name);
    if (!handle) return;

    const array = data instanceof Float32Array ? data : new Float32Array(data);
    handle.attribute = new THREE.BufferAttribute(array, itemSize);
    handle.generation++;
  }

  /** Get a texture handle by name. */
  getTexture(name: string): GPUTextureHandle | undefined {
    return this.textures.get(name);
  }

  /** Get a buffer handle by name. */
  getBuffer(name: string): GPUBufferHandle | undefined {
    return this.buffers.get(name);
  }

  /** Is this texture fully loaded (all mips uploaded)? */
  isTextureReady(name: string): boolean {
    return this.textures.get(name)?.isReady ?? false;
  }

  /** Number of pending uploads. */
  get pendingCount(): number {
    return this.pendingUploads.length;
  }

  /** Number of managed textures. */
  get textureCount(): number {
    return this.textures.size;
  }

  /** Number of managed buffers. */
  get bufferCount(): number {
    return this.buffers.size;
  }

  /** Dispose a specific texture. */
  disposeTexture(name: string): void {
    const handle = this.textures.get(name);
    if (handle?.texture) {
      handle.texture.dispose();
    }
    this.textures.delete(name);
  }

  /** Dispose a specific buffer. */
  disposeBuffer(name: string): void {
    this.buffers.delete(name);
  }

  /** Dispose all resources. */
  disposeAll(): void {
    for (const handle of this.textures.values()) {
      handle.texture?.dispose();
    }
    this.textures.clear();
    this.buffers.clear();
    this.pendingUploads.length = 0;
  }
}

// --- ECS Resource registration -------------------------------------------

export const GPUStoreRes = defineResource<GPUStore>('gpuStore', () => {
  throw new Error(
    'GPUStore resource not initialized. Call `GPUStoreRes.set(world, new GPUStore())` before use.',
  );
});
