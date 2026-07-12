// engine/asset-store.ts
const THREE = window.THREE;

// engine/resource.ts
var RESOURCES = Symbol.for("afterglow-resources");
function ensureStore(world) {
  const w = world;
  if (!w[RESOURCES])
    w[RESOURCES] = {};
  return w[RESOURCES];
}

class Resource {
  name;
  factory;
  constructor(name, factory) {
    this.name = name;
    this.factory = factory;
  }
  get(world) {
    const store = ensureStore(world);
    if (!(this.name in store))
      store[this.name] = this.factory();
    return store[this.name];
  }
  set(world, value) {
    ensureStore(world)[this.name] = value;
  }
  has(world) {
    return this.name in ensureStore(world);
  }
  remove(world) {
    delete ensureStore(world)[this.name];
  }
}
function defineResource(name, factory) {
  return new Resource(name, factory);
}

// engine/asset-handle.ts
class AssetHandle {
  asset;
  generation = 0;
  state = "loading";
  path;
  constructor(path, fallback) {
    this.path = path;
    this.asset = fallback;
  }
  get isReady() {
    return this.state === "ready";
  }
  get isError() {
    return this.state === "error";
  }
  lod = -1;
}

// engine/fallback.ts
var _fallbackTexture = null;
var _fallbackGeometry = null;
var _fallbackMaterial = null;
var _fallbackGroup = null;
function createCheckerboardTexture() {
  const size = 64;
  const sq = 8;
  const canvas = document.createElement("canvas");
  canvas.width = size;
  canvas.height = size;
  const ctx = canvas.getContext("2d");
  for (let y = 0;y < size; y += sq) {
    for (let x = 0;x < size; x += sq) {
      const isBlack = (x / sq + y / sq) % 2 === 0;
      ctx.fillStyle = isBlack ? "#000000" : "#9b00ff";
      ctx.fillRect(x, y, sq, sq);
    }
  }
  const texture = new THREE.CanvasTexture(canvas);
  texture.wrapS = THREE.RepeatWrapping;
  texture.wrapT = THREE.RepeatWrapping;
  texture.repeat.set(4, 4);
  texture.colorSpace = THREE.SRGBColorSpace;
  return texture;
}
function fallbackTexture() {
  if (!_fallbackTexture)
    _fallbackTexture = createCheckerboardTexture();
  return _fallbackTexture;
}
function fallbackGeometry() {
  if (!_fallbackGeometry) {
    _fallbackGeometry = new THREE.BoxGeometry(2, 2, 2);
  }
  return _fallbackGeometry;
}
function fallbackMaterial() {
  if (!_fallbackMaterial) {
    _fallbackMaterial = new THREE.MeshBasicMaterial({ color: 16711935 });
  }
  return _fallbackMaterial;
}
function fallbackGroup() {
  if (!_fallbackGroup) {
    _fallbackGroup = new THREE.Group;
    const mesh = new THREE.Mesh(fallbackGeometry(), fallbackMaterial());
    _fallbackGroup.add(mesh);
    _fallbackGroup.name = "__afterglow_fallback__";
  }
  return _fallbackGroup.clone(true);
}

// engine/asset-store.ts
var FORMAT_BC7 = 0;
var FORMAT_ASTC = 1;
var FORMAT_RGBA = 4;
var _bestFormat = null;
async function detectBestTextureFormat() {
  _bestFormat = FORMAT_RGBA;
  return FORMAT_RGBA;
}
var MAX_SINGLE_LOAD = 1 << 20;
var CHUNK_SIZE = 512 * 1024;
var MAX_MIP_UPLOADS_PER_FRAME = 2;
var DEFAULT_LOD_RATIOS = [1, 0.5, 0.25, 0.1];
var DEFAULT_TARGET_ERROR = 0.02;
async function parseTexture(bytes) {
  const bitmap = await createImageBitmap(new Blob([bytes]));
  const tex = new THREE.Texture(bitmap);
  tex.needsUpdate = true;
  tex.colorSpace = THREE.SRGBColorSpace;
  return tex;
}
async function parseGLTF(bytes, loader) {
  const buf = new ArrayBuffer(bytes.byteLength);
  new Uint8Array(buf).set(bytes);
  if (loader)
    return new Promise((res, rej) => loader.parse(buf, "", (r) => res(r.scene), rej));
  const GLTFLoader2 = THREE.GLTFLoader;
  if (!GLTFLoader2)
    throw new Error("GLTFLoader not available");
  const gl = new GLTFLoader2;
  return new Promise((res, rej) => gl.parse(buf, "", (r) => res(r.scene), rej));
}
function parseJSON(bytes) {
  return JSON.parse(new TextDecoder().decode(bytes));
}
function nearestUpscale(src, srcW, srcH, dstW, dstH) {
  if (srcW === dstW && srcH === dstH)
    return src;
  const dst = new Uint8Array(dstW * dstH * 4);
  for (let y = 0;y < dstH; y++) {
    const sy = Math.floor(y * srcH / dstH);
    for (let x = 0;x < dstW; x++) {
      const sx = Math.floor(x * srcW / dstW);
      const si = (sy * srcW + sx) * 4;
      const di = (y * dstW + x) * 4;
      dst[di] = src[si];
      dst[di + 1] = src[si + 1];
      dst[di + 2] = src[si + 2];
      dst[di + 3] = src[si + 3];
    }
  }
  return dst;
}

class AssetStore {
  cache = new Map;
  pending = new Map;
  streaming = new Map;
  meshopt;
  texture;
  loader;
  constructor(loader, meshopt, texture) {
    this.loader = loader;
    this.meshopt = meshopt;
    this.texture = texture;
  }
  get assetLoader() {
    return this.loader;
  }
  poll() {
    this.loader.poll();
    this.meshopt?.poll();
    this.texture?.poll();
    this.processPendingLoads();
    this.processStreaming();
  }
  processPendingLoads() {
    for (const [path, pending] of this.pending) {
      Promise.resolve(pending.promise).then((bytes) => {
        Promise.resolve(pending.parser(bytes)).then((asset) => {
          pending.handle.asset = asset;
          pending.handle.generation++;
          pending.handle.state = "ready";
          pending.handle.lod = 0;
          this.cache.set(path, pending.handle);
          this.pending.delete(path);
        }, (err) => {
          pending.handle.state = "error";
          console.error(`[afterglow] parse failed: ${path}`, err);
          this.pending.delete(path);
        });
      }, (err) => {
        pending.handle.state = "error";
        console.error(`[afterglow] load failed: ${path}`, err);
        this.pending.delete(path);
      });
    }
  }
  processStreaming() {
    let processed = 0;
    for (const [, stream] of this.streaming) {
      while (processed < MAX_MIP_UPLOADS_PER_FRAME && stream.pendingMips.length > 0) {
        const mip = stream.pendingMips.shift();
        const upscaled = nearestUpscale(mip.data, mip.width, mip.height, stream.fullW, stream.fullH);
        stream.texture.image = { data: upscaled, width: stream.fullW, height: stream.fullH };
        stream.texture.needsUpdate = true;
        stream.mipsUploaded++;
        stream.handle.generation++;
        processed++;
      }
      if (stream.pendingMips.length === 0) {
        stream.handle.state = "ready";
        this.streaming.delete(stream.handle.path);
      }
    }
  }
  load(path, parser, fallback) {
    const cached = this.cache.get(path);
    if (cached)
      return cached;
    const inflight = this.pending.get(path);
    if (inflight)
      return inflight.handle;
    const handle = new AssetHandle(path, fallback);
    const promise = this.startLoad(path);
    this.pending.set(path, { handle, promise, parser });
    return handle;
  }
  getHandle(path) {
    return this.cache.get(path);
  }
  has(path) {
    return this.cache.has(path);
  }
  isLoading(path) {
    return this.pending.has(path);
  }
  loadModel(path) {
    const cached = this.cache.get(path);
    if (cached)
      return cached;
    const handle = new AssetHandle(path, undefined);
    const promise = this.startLoad(path);
    promise.then(async (bytes) => {
      try {
        const asset = await this.processModel(bytes, path);
        handle.asset = asset;
        handle.generation++;
        handle.state = "ready";
        this.cache.set(path, handle);
      } catch (err) {
        handle.state = "error";
        console.error(`[afterglow] loadModel failed: ${path}`, err);
      }
    });
    return handle;
  }
  async processModel(bytes, path) {
    let meshes = [];
    let textures = new Map;
    try {
      const scene = await parseGLTF(bytes);
      scene.traverse((obj) => {
        if (obj.isMesh && obj.geometry?.index) {
          const geo = obj.geometry;
          meshes.push({
            indices: new Uint32Array(geo.index.array),
            positions: new Float32Array(geo.attributes.position.array),
            uvs: geo.attributes.uv ? new Float32Array(geo.attributes.uv.array) : new Float32Array(0)
          });
        }
        if (obj.isMesh && obj.material?.map)
          textures.set("diffuse", obj.material.map);
      });
    } catch {
      meshes = this.parseMinimalGLB(bytes);
    }
    const stats = [];
    const meshLods = [];
    for (const mesh of meshes) {
      const { lods, stat } = await this.optimizeMesh(mesh.indices, mesh.positions, mesh.uvs);
      meshLods.push(lods);
      if (stat)
        stats.push(stat);
    }
    return { meshes: meshLods, textures, stats };
  }
  parseMinimalGLB(bytes) {
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    if (view.getUint32(0, true) !== 1179937895)
      throw new Error("not a GLB");
    const totalLen = view.getUint32(8, true);
    let off = 12;
    let json = null;
    let bin = null;
    while (off < totalLen) {
      const len = view.getUint32(off, true);
      off += 4;
      const type = view.getUint32(off, true);
      off += 4;
      if (type === 1313821514)
        json = JSON.parse(new TextDecoder().decode(bytes.subarray(off, off + len)));
      else if (type === 5130562)
        bin = bytes.subarray(off, off + len);
      off += len;
    }
    if (!json || !bin)
      throw new Error("GLB missing JSON or BIN");
    const accs = json.accessors || [];
    const bvs = json.bufferViews || [];
    const prims = json.meshes?.[0]?.primitives || [];
    const meshes = [];
    for (const prim of prims) {
      const read = (aIdx, T) => {
        if (aIdx === undefined)
          return new T(0);
        const a = accs[aIdx];
        const bv = bvs[a.bufferView];
        const o = (bv.byteOffset || 0) + (a.byteOffset || 0);
        const comps = a.type === "VEC3" ? 3 : a.type === "VEC2" ? 2 : 1;
        return new T(bin.buffer, bin.byteOffset + o, a.count * comps).slice();
      };
      meshes.push({
        indices: read(prim.indices, Uint32Array),
        positions: read(prim.attributes.POSITION, Float32Array),
        uvs: prim.attributes.TEXCOORD_0 !== undefined ? read(prim.attributes.TEXCOORD_0, Float32Array) : new Float32Array(0)
      });
    }
    return meshes;
  }
  async optimizeMesh(indices, positions, uvs) {
    const vertexCount = positions.length / 3;
    const originalTriangles = indices.length / 3;
    const stride = 12;
    const uvStride = 8;
    if (!this.meshopt) {
      return {
        lods: [{
          indices,
          positions,
          uvs,
          triangleCount: originalTriangles
        }]
      };
    }
    const origStats = await this.meshopt.analyzeVertexCache(indices, vertexCount);
    const originalAcmr = origStats[0];
    let optimized = await this.meshopt.optimizeVertexCache(indices, vertexCount);
    optimized = await this.meshopt.optimizeOverdraw(optimized, positions, stride, 1.05);
    const optStats = await this.meshopt.analyzeVertexCache(optimized, vertexCount);
    const optimizedAcmr = optStats[0];
    const compressed = await this.meshopt.encodeIndexBuffer(optimized, vertexCount);
    const lods = [];
    for (const ratio of DEFAULT_LOD_RATIOS) {
      if (ratio >= 1) {
        lods.push({
          indices: optimized,
          positions,
          uvs,
          triangleCount: optimized.length / 3,
          stats: ratio === 1 ? {
            originalTriangles,
            originalAcmr,
            optimizedAcmr,
            compressedIndexBytes: compressed.length,
            uncompressedIndexBytes: optimized.length * 4
          } : undefined
        });
      } else {
        const targetTris = Math.max(4, Math.floor(originalTriangles * ratio));
        const targetIndexCount = targetTris * 3;
        const simplified = await this.meshopt.simplifyWithUvs(optimized, positions, stride, uvs, uvStride, 0.5, targetIndexCount, DEFAULT_TARGET_ERROR);
        lods.push({
          indices: simplified,
          positions,
          uvs,
          triangleCount: simplified.length / 3
        });
      }
    }
    return {
      lods,
      stat: {
        originalTriangles,
        originalAcmr,
        optimizedAcmr,
        compressedIndexBytes: compressed.length,
        uncompressedIndexBytes: optimized.length * 4
      }
    };
  }
  loadTexture(path) {
    const lower = path.toLowerCase();
    if (this.texture && (lower.endsWith(".basis") || lower.endsWith(".ktx2"))) {
      return this.loadStreamingBasisTexture(path);
    }
    return this.load(path, parseTexture, fallbackTexture());
  }
  loadStreamingBasisTexture(path) {
    const cached = this.cache.get(path);
    if (cached)
      return cached;
    const handle = new AssetHandle(path, fallbackTexture());
    const promise = this.startLoad(path);
    promise.then(async (bytes) => {
      const format = await detectBestTextureFormat();
      const transcoded = await this.texture.transcode(bytes, format);
      const mips = this.parseSerializedMips(transcoded);
      if (mips.length === 0) {
        handle.state = "error";
        console.error(`[afterglow] basis texture: no mips in ${path}`);
        return;
      }
      const fullW = mips[0].width;
      const fullH = mips[0].height;
      const smallest = mips[mips.length - 1];
      const initialData = nearestUpscale(smallest.data, smallest.width, smallest.height, fullW, fullH);
      const texture = new THREE.DataTexture(initialData, fullW, fullH, THREE.RGBAFormat);
      texture.generateMipmaps = true;
      texture.minFilter = THREE.LinearMipmapLinearFilter;
      texture.magFilter = THREE.LinearFilter;
      texture.wrapS = THREE.RepeatWrapping;
      texture.wrapT = THREE.RepeatWrapping;
      texture.colorSpace = THREE.SRGBColorSpace;
      texture.needsUpdate = true;
      handle.asset = texture;
      handle.generation++;
      handle.state = "loading";
      const queue = [];
      for (let i = mips.length - 2;i >= 0; i--) {
        queue.push(mips[i]);
      }
      this.streaming.set(path, {
        handle,
        texture,
        pendingMips: queue.map((m) => ({
          data: m.data,
          width: m.width,
          height: m.height,
          level: 0
        })),
        mipsUploaded: 1,
        totalMips: mips.length,
        fullW,
        fullH
      });
    }).catch((err) => {
      handle.state = "error";
      console.error(`[afterglow] basis texture failed: ${path}`, err);
    });
    return handle;
  }
  parseSerializedMips(data) {
    if (data.length < 4)
      return [];
    const view = new DataView(data.buffer, data.byteOffset, data.byteLength);
    const count = view.getUint32(0, true);
    if (count === 0 || count > 20)
      return [];
    let offset = 4;
    const mips = [];
    for (let i = 0;i < count; i++) {
      if (offset + 12 > data.length)
        break;
      const w = view.getUint32(offset, true);
      offset += 4;
      const h = view.getUint32(offset, true);
      offset += 4;
      const len = view.getUint32(offset, true);
      offset += 4;
      if (offset + len > data.length)
        break;
      mips.push({ data: data.slice(offset, offset + len), width: w, height: h });
      offset += len;
    }
    return mips;
  }
  queueMipUpload(path, handle, texture, level, data, width, height, mipNumber, totalMips) {
    const stream = this.streaming.get(path);
    if (!stream)
      return;
    stream.pendingMips.push({ data, width, height, level });
    stream.totalMips = totalMips;
    if (mipNumber === 1) {
      handle.generation++;
      handle.state = "loading";
    }
  }
  loadBasisTexture(path) {
    if (!this.texture)
      throw new Error("No texture worker");
    return this.load(path, async (bytes) => {
      const format = await detectBestTextureFormat();
      return this.texture.transcode(bytes, format);
    }, new Uint8Array);
  }
  loadGLTF(path, loader) {
    return this.load(path, (bytes) => parseGLTF(bytes, loader), fallbackGroup());
  }
  loadJSON(path) {
    return this.load(path, parseJSON, null);
  }
  async startLoad(path) {
    const total = await this.loader.size(path);
    if (total > MAX_SINGLE_LOAD)
      return this.loadChunked(path, total);
    return this.loader.load(path);
  }
  async loadChunked(path, total) {
    const chunks = [];
    let offset = 0;
    while (offset < total) {
      const len = Math.min(CHUNK_SIZE, total - offset);
      chunks.push(await this.loader.read(path, offset, len));
      offset += chunks[chunks.length - 1].byteLength;
    }
    const bytes = new Uint8Array(total);
    let pos = 0;
    for (const c of chunks) {
      bytes.set(c, pos);
      pos += c.byteLength;
    }
    return bytes;
  }
  evict(path) {
    this.cache.delete(path);
    this.streaming.delete(path);
  }
  get size() {
    return this.cache.size;
  }
  get cachedPaths() {
    return [...this.cache.keys()];
  }
  dispose() {
    for (const h of this.cache.values())
      h.asset?.dispose?.();
    this.cache.clear();
    this.pending.clear();
    this.streaming.clear();
  }
}
var AssetStoreRes = defineResource("assetStore", () => {
  throw new Error("AssetStore not initialized. Call AssetStoreRes.set(world, new AssetStore(loader, meshopt, texture)).");
});
export {
  parseTexture,
  parseJSON,
  parseGLTF,
  detectBestTextureFormat,
  FORMAT_RGBA,
  FORMAT_BC7,
  FORMAT_ASTC,
  AssetStoreRes,
  AssetStore
};
