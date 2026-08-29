// crates/afterglow-web/web/src/engine/assets/bulk-range.ts
var BULK_RANGE_CAPACITY = 256;
var BULK_RESPONSE_MAX_BYTES = 4 * 1024 * 1024;
var BULK_IN_FLIGHT_MAX_BYTES = 8 * 1024 * 1024;
var HEADER_ALLOWANCE_PER_RANGE = 192;
var RESPONSE_FIXED_ALLOWANCE = 64;
var decoder = new TextDecoder("ascii");
function estimatedBulkResponseBytes(ranges) {
  let bytes = RESPONSE_FIXED_ALLOWANCE;
  for (const range of ranges)
    bytes += range.length + HEADER_ALLOWANCE_PER_RANGE;
  return bytes;
}
function validateRanges(ranges) {
  if (ranges.length < 1 || ranges.length > BULK_RANGE_CAPACITY)
    throw new RangeError(`bulk range count must be 1..${BULK_RANGE_CAPACITY}`);
  if (estimatedBulkResponseBytes(ranges) > BULK_RESPONSE_MAX_BYTES)
    throw new RangeError("bulk response exceeds 4 MiB capacity");
  for (let index = 0;index < ranges.length; index++) {
    const range = ranges[index];
    if (!Number.isSafeInteger(range.offset) || range.offset < 0 || !Number.isSafeInteger(range.length) || range.length <= 0)
      throw new RangeError("bulk ranges require positive safe-integer spans");
    const end = range.offset + range.length - 1;
    if (!Number.isSafeInteger(end))
      throw new RangeError("bulk range end is unsafe");
    for (let other = 0;other < index; other++) {
      const prior = ranges[other];
      const priorEnd = prior.offset + prior.length - 1;
      if (range.offset <= priorEnd && end >= prior.offset)
        throw new RangeError("bulk ranges must not overlap");
    }
  }
}
function boundaryFrom(contentType) {
  const match = /(?:^|;)\s*boundary=(?:"([^"]+)"|([^;\s]+))/i.exec(contentType);
  const boundary = match?.[1] ?? match?.[2] ?? "";
  if (!boundary || boundary.length > 128 || /[^\x21-\x7e]/.test(boundary))
    throw new Error("multipart byte-range response has an invalid boundary");
  return boundary;
}
function matches(bytes, offset, pattern) {
  if (offset < 0 || offset + pattern.length > bytes.length)
    return false;
  for (let index = 0;index < pattern.length; index++)
    if (bytes[offset + index] !== pattern[index])
      return false;
  return true;
}
function find(bytes, pattern, start, limit) {
  const end = Math.min(bytes.length - pattern.length, limit);
  for (let offset = start;offset <= end; offset++)
    if (matches(bytes, offset, pattern))
      return offset;
  return -1;
}
function parseMultipartByteRanges(body, contentType, requested) {
  validateRanges(requested);
  if (body.byteLength > BULK_RESPONSE_MAX_BYTES)
    throw new RangeError("bulk response exceeded 4 MiB capacity");
  const boundary = new TextEncoder().encode(`--${boundaryFrom(contentType)}`);
  const headerEndMarker = new Uint8Array([13, 10, 13, 10]);
  const crlf = new Uint8Array([13, 10]);
  const output = new Array(requested.length);
  let cursor = 0;
  for (let index = 0;index < requested.length; index++) {
    if (!matches(body, cursor, boundary))
      throw new Error(`multipart boundary missing at part ${index}`);
    cursor += boundary.length;
    if (!matches(body, cursor, crlf))
      throw new Error("multipart part has no header line break");
    cursor += 2;
    const headerEnd = find(body, headerEndMarker, cursor, cursor + 1024);
    if (headerEnd < 0)
      throw new Error("multipart part headers exceed 1 KiB");
    const headers = decoder.decode(body.subarray(cursor, headerEnd));
    const match = /(?:^|\r\n)Content-Range:\s*bytes\s+(\d+)-(\d+)\/(?:\d+|\*)/i.exec(headers);
    if (!match)
      throw new Error("multipart part has no valid Content-Range");
    const start = Number(match[1]);
    const end = Number(match[2]);
    const expected = requested[index];
    if (start !== expected.offset || end !== expected.offset + expected.length - 1)
      throw new Error(`multipart part ${index} does not match its requested range`);
    const dataStart = headerEnd + 4;
    const dataEnd = dataStart + expected.length;
    if (dataEnd > body.length)
      throw new Error("multipart part payload is truncated");
    output[index] = body.subarray(dataStart, dataEnd);
    cursor = dataEnd;
    if (!matches(body, cursor, crlf))
      throw new Error("multipart part has no trailing line break");
    cursor += 2;
  }
  if (!matches(body, cursor, boundary))
    throw new Error("multipart closing boundary is missing");
  cursor += boundary.length;
  if (body[cursor] !== 45 || body[cursor + 1] !== 45)
    throw new Error("multipart closing boundary is malformed");
  return output;
}
async function fetchByteRanges(url, ranges) {
  validateRanges(ranges);
  const value = ranges.map((range) => `${range.offset}-${range.offset + range.length - 1}`).join(",");
  const response = await fetch(url, { headers: { Range: `bytes=${value}` } });
  if (response.status !== 206)
    throw new Error(`bulk asset range expected 206, got ${response.status}: ${url}`);
  const body = new Uint8Array(await response.arrayBuffer());
  if (ranges.length === 1) {
    if (body.byteLength !== ranges[0].length)
      throw new Error(`asset range returned ${body.byteLength} bytes; expected ${ranges[0].length}: ${url}`);
    return [body];
  }
  return parseMultipartByteRanges(body, response.headers.get("content-type") ?? "", ranges);
}

// crates/afterglow-web/web/src/engine/assets/big-format.ts
function decodeVarint(bytes, off) {
  let r = 0;
  for (let shift = 0;shift < 56; shift += 7) {
    if (off >= bytes.length)
      throw new Error("postcard varint truncated");
    const b = bytes[off++];
    r += (b & 127) * 2 ** shift;
    if (!(b & 128))
      return [r, off];
  }
  throw new Error("postcard varint overflows");
}
function decodeU32(bytes, off) {
  return decodeVarint(bytes, off);
}
function decodeU64(bytes, off) {
  let result = 0n;
  for (let shift = 0n;shift < 70n; shift += 7n) {
    if (off >= bytes.length)
      throw new Error("postcard u64 varint truncated");
    const byte = bytes[off++];
    result |= BigInt(byte & 127) << shift;
    if (!(byte & 128)) {
      if (result > 0xffff_ffff_ffff_ffffn)
        throw new Error("postcard u64 varint overflows");
      return [result, off];
    }
  }
  throw new Error("postcard u64 varint overflows");
}
function decodeString(bytes, off) {
  const [len, o] = decodeVarint(bytes, off);
  const str = new TextDecoder().decode(bytes.subarray(o, o + len));
  return [str, o + len];
}
function decodeVec(bytes, off, decodeFn) {
  const [len, o] = decodeVarint(bytes, off);
  const result = [];
  let pos = o;
  for (let i = 0;i < len; i++) {
    const [item, newOff] = decodeFn(bytes, pos);
    result.push(item);
    pos = newOff;
  }
  return [result, pos];
}
function decodeBool(bytes, off) {
  return [bytes[off] !== 0, off + 1];
}
function decodeU8(bytes, off) {
  return [bytes[off], off + 1];
}
function decodeAssetType(bytes, off) {
  const [variant, o] = decodeU32(bytes, off);
  switch (variant) {
    case 0:
      return ["Texture", o];
    case 1:
      return ["Mesh", o];
    case 2:
      return ["VirtualTexture", o];
    default:
      throw new Error(`unknown AssetType variant: ${variant}`);
  }
}
function decodeCompression(bytes, off) {
  const [variant, o] = decodeU32(bytes, off);
  switch (variant) {
    case 0:
      return ["Meshopt", o];
    case 1:
      return ["None", o];
    default:
      throw new Error(`unknown Compression variant: ${variant}`);
  }
}
function decodeTextureEncoding(bytes, off) {
  const [variant, next] = decodeU32(bytes, off);
  if (variant === 0)
    return ["RawRgba8", next];
  if (variant === 1)
    return ["Basis", next];
  throw new Error(`unknown TextureEncoding variant: ${variant}`);
}
function decodeTextureFormat(bytes, off) {
  const [variant, next] = decodeU32(bytes, off);
  if (variant === 0)
    return ["Rgba8", next];
  if (variant === 1)
    return ["R8", next];
  throw new Error(`unknown TextureFormat variant: ${variant}`);
}
function decodeChunkMeta(bytes, off) {
  const [variant, o] = decodeU32(bytes, off);
  switch (variant) {
    case 0: {
      const [w, o2] = decodeU32(bytes, o);
      const [h, o3] = decodeU32(bytes, o2);
      const [format, o4] = decodeTextureFormat(bytes, o3);
      return [{ type: "Texture", width: w, height: h, format }, o4];
    }
    case 1: {
      const [ic, o2] = decodeU32(bytes, o);
      const [vc, o3] = decodeU32(bytes, o2);
      const [ps, o4] = decodeU32(bytes, o3);
      const [us, o5] = decodeU32(bytes, o4);
      return [{ type: "Mesh", indexCount: ic, vertexCount: vc, positionStride: ps, uvStride: us }, o5];
    }
    case 2:
      return [{ type: "Raw" }, o];
    default:
      throw new Error(`unknown ChunkMeta variant: ${variant}`);
  }
}
function decodeChunkInfo(bytes, off) {
  const [offset, o1] = decodeU64(bytes, off);
  const [compressedSize, o2] = decodeU64(bytes, o1);
  const [uncompressedSize, o3] = decodeU64(bytes, o2);
  const [lodLevel, o4] = decodeU8(bytes, o3);
  const [mipLevel, o5] = decodeU8(bytes, o4);
  const [compression, o6] = decodeCompression(bytes, o5);
  const [meta, o7] = decodeChunkMeta(bytes, o6);
  return [{
    offset,
    compressedSize,
    uncompressedSize,
    lodLevel,
    mipLevel,
    compression,
    meta
  }, o7];
}
function decodeVTMipDirectory(bytes, off) {
  const [mip, o1] = decodeU8(bytes, off);
  const [pagesX, o2] = decodeU32(bytes, o1);
  const [pagesY, o3] = decodeU32(bytes, o2);
  const [offset, o4] = decodeU64(bytes, o3);
  const [pageSizes, o5] = decodeVec(bytes, o4, decodeU32);
  return [{ mip, pagesX, pagesY, offset, pageSizes }, o5];
}
function decodeVTTailDirectory(bytes, off) {
  const [firstMip, o1] = decodeU8(bytes, off);
  const [offset, o2] = decodeU64(bytes, o1);
  const [size, o3] = decodeU32(bytes, o2);
  return [{ firstMip, offset, size }, o3];
}
function decodeVTDirectory(bytes, off) {
  const [width, o1] = decodeU32(bytes, off);
  const [height, o2] = decodeU32(bytes, o1);
  const [encoding, o3] = decodeTextureEncoding(bytes, o2);
  const [mips, o4] = decodeVec(bytes, o3, decodeVTMipDirectory);
  const [hasTail, o5] = decodeBool(bytes, o4);
  if (!hasTail)
    return [{ width, height, encoding, mips, tail: null }, o5];
  const [tail, o6] = decodeVTTailDirectory(bytes, o5);
  return [{ width, height, encoding, mips, tail }, o6];
}
function decodeAssetEntry(bytes, off) {
  const [name, o1] = decodeString(bytes, off);
  const [assetType, o2] = decodeAssetType(bytes, o1);
  const [chunks, o3] = decodeVec(bytes, o2, decodeChunkInfo);
  const [hasVirtualTexture, o4] = decodeBool(bytes, o3);
  if (!hasVirtualTexture)
    return [{ name, assetType, chunks, virtualTexture: null }, o4];
  const [virtualTexture, o5] = decodeVTDirectory(bytes, o4);
  return [{ name, assetType, chunks, virtualTexture }, o5];
}
var BIG_MAGIC = 826755394;
var BIG_VERSION = 6;
var BIG_MIN_READABLE_VERSION = 5;
function parseBigHeader(data) {
  if (data.length < 16)
    throw new Error(".big: file too small");
  const magic = new DataView(data.buffer, data.byteOffset, 4).getUint32(0, true);
  if (magic !== BIG_MAGIC)
    throw new Error(".big: bad magic");
  const version = new DataView(data.buffer, data.byteOffset + 4, 4).getUint32(0, true);
  if (version < BIG_MIN_READABLE_VERSION || version > BIG_VERSION) {
    throw new Error(`.big: version ${version} not in [${BIG_MIN_READABLE_VERSION},${BIG_VERSION}]`);
  }
  const dataOffset = Number(new DataView(data.buffer, data.byteOffset + 8, 8).getBigUint64(0, true));
  const headerBytes = data.subarray(16, dataOffset);
  let off = 0;
  const [hdrVersion, o1] = decodeU32(headerBytes, off);
  off = o1;
  const [hdrDataOffset, o2] = decodeU64(headerBytes, off);
  off = o2;
  const [assets, o3] = decodeVec(headerBytes, off, decodeAssetEntry);
  off = o3;
  return {
    header: { version: hdrVersion, dataOffset: hdrDataOffset, assets },
    dataOffset
  };
}
function findVTPageChunk(header, assetName, mip, pageX, pageY) {
  const directory = header.assets.find((asset) => asset.name === assetName)?.virtualTexture;
  const mipDirectory = directory?.mips.find((candidate) => candidate.mip === mip);
  if (!directory || !mipDirectory || pageX < 0 || pageY < 0 || pageX >= mipDirectory.pagesX || pageY >= mipDirectory.pagesY)
    return null;
  const page = pageY * mipDirectory.pagesX + pageX;
  let offset = mipDirectory.offset;
  for (let index = 0;index < page; index++)
    offset += BigInt(mipDirectory.pageSizes[index]);
  const size = mipDirectory.pageSizes[page];
  return {
    offset,
    compressedSize: BigInt(size),
    uncompressedSize: BigInt(size),
    lodLevel: 0,
    mipLevel: mip,
    compression: "None",
    meta: { type: "VirtualTexturePage", mip, pageX, pageY, encoding: directory.encoding }
  };
}

// crates/afterglow-web/web/src/engine/assets/asset-range.ts
async function readBigHeader(source, path, maxHeaderBytes) {
  if (!Number.isSafeInteger(maxHeaderBytes) || maxHeaderBytes < 16)
    throw new RangeError("BIG maxHeaderBytes must be at least 16");
  const prefix = await source.read(path, 0, 16);
  if (prefix.byteLength !== 16)
    throw new Error("BIG container prefix is truncated");
  const view = new DataView(prefix.buffer, prefix.byteOffset, prefix.byteLength);
  if (view.getUint32(0, true) !== BIG_MAGIC)
    throw new Error("BIG container has invalid magic");
  const version = view.getUint32(4, true);
  if (version < BIG_MIN_READABLE_VERSION || version > BIG_VERSION) {
    throw new Error(`BIG container version ${version} is unsupported`);
  }
  const dataOffset = Number(view.getBigUint64(8, true));
  if (!Number.isSafeInteger(dataOffset) || dataOffset < 16 || dataOffset > maxHeaderBytes)
    throw new RangeError(`BIG header size ${dataOffset} exceeds configured capacity ${maxHeaderBytes}`);
  const bytes = await source.read(path, 0, dataOffset);
  if (bytes.byteLength !== dataOffset)
    throw new Error("BIG container header is truncated");
  return parseBigHeader(bytes).header;
}
function createFetchRangeLoader(baseUrl = "") {
  const url = (path) => baseUrl + path;
  const identity = async (path) => {
    const response = await fetch(url(path), { headers: { Range: "bytes=0-0" } });
    if (response.status !== 206)
      throw new Error(`asset identity range expected 206, got ${response.status}: ${path}`);
    const contentRange = response.headers.get("content-range") ?? "";
    const separator = contentRange.lastIndexOf("/");
    const size = Number(separator < 0 ? "" : contentRange.slice(separator + 1));
    if (!Number.isSafeInteger(size) || size < 1)
      throw new Error(`asset identity has invalid content-range: ${path}`);
    return {
      size,
      etag: response.headers.get("etag"),
      lastModified: response.headers.get("last-modified")
    };
  };
  return {
    async load(path) {
      const response = await fetch(url(path));
      if (!response.ok)
        throw new Error(`asset fetch ${response.status}: ${path}`);
      return new Uint8Array(await response.arrayBuffer());
    },
    async size(path) {
      return (await identity(path)).size;
    },
    identity,
    async read(path, offset, len) {
      if (!Number.isSafeInteger(offset) || offset < 0 || !Number.isSafeInteger(len) || len < 0)
        throw new RangeError("asset range must use non-negative safe integers");
      if (len === 0)
        return new Uint8Array(0);
      return (await fetchByteRanges(url(path), [{ offset, length: len }]))[0];
    },
    async readBulk(path, ranges) {
      return fetchByteRanges(url(path), ranges);
    }
  };
}

// crates/afterglow-web/web/src/engine/assets/vt-page-directory.ts
class VtPageDirectory {
  textures = new Map;
  constructor(header) {
    for (const asset of header.assets) {
      const source = asset.virtualTexture;
      if (!source)
        continue;
      let maxMip = 0;
      for (const mip of source.mips)
        maxMip = Math.max(maxMip, mip.mip);
      const mips = new Array(maxMip + 1).fill(null);
      for (const mip of source.mips) {
        const sizes = Uint32Array.from(mip.pageSizes);
        const offsets = new Float64Array(sizes.length);
        let offset = Number(mip.offset);
        for (let page = 0;page < sizes.length; page++) {
          offsets[page] = offset;
          offset += sizes[page] ?? 0;
        }
        mips[mip.mip] = {
          pagesX: mip.pagesX,
          pagesY: mip.pagesY,
          offsets,
          sizes
        };
      }
      this.textures.set(asset.name, {
        encoding: source.encoding,
        mips,
        tailOffset: source.tail ? Number(source.tail.offset) : 0,
        tailSize: source.tail?.size ?? 0
      });
    }
  }
  resolve(address) {
    const texture = this.textures.get(address.path);
    if (!texture)
      throw new Error(`VT directory not found: ${address.path}`);
    let offset = 0;
    let length = 0;
    if (address.tail) {
      offset = texture.tailOffset;
      length = texture.tailSize;
    } else {
      const mip = texture.mips[address.mip];
      if (!mip || address.x < 0 || address.y < 0 || address.x >= mip.pagesX || address.y >= mip.pagesY) {
        throw new Error(`VT page out of range: ${address.path} mip=${address.mip} (${address.x},${address.y})`);
      }
      const page = address.y * mip.pagesX + address.x;
      offset = mip.offsets[page] ?? 0;
      length = mip.sizes[page] ?? 0;
    }
    if (length === 0) {
      throw new Error(`VT page not found: ${address.path} mip=${address.mip} (${address.x},${address.y})`);
    }
    return {
      path: address.path,
      mip: address.mip,
      x: address.x,
      y: address.y,
      tail: address.tail === true,
      offset,
      length,
      encoding: texture.encoding
    };
  }
}

// crates/afterglow-web/web/src/engine/assets/source-sorted-page-reader.ts
function createSourceSortedPageReader(loader, header, readConcurrency = 16) {
  if (!Number.isInteger(readConcurrency) || readConcurrency < 1)
    throw new RangeError("source-sorted page reader concurrency must be positive");
  const directory = new VtPageDirectory(header);
  let reads = 0, totalReadMs = 0, maxReadMs = 0;
  let batches = 0, pagesRequested = 0, pagesCoalesced = 0, runs = 0;
  const stats = {
    reads: 0,
    averageReadMs: 0,
    maxReadMs: 0,
    batches: 0,
    pagesRequested: 0,
    pagesCoalesced: 0,
    runs: 0
  };
  const resolve = (request, index) => ({
    ...directory.resolve(request),
    index
  });
  const coalesce = (group) => {
    const out = [];
    let runStart = 0;
    for (let i = 1;i <= group.length; i++) {
      const prev = group[i - 1];
      const cur = i < group.length ? group[i] : null;
      const contiguous = cur !== null && cur.x === prev.x + 1 && cur.length === prev.length && cur.offset === prev.offset + prev.length;
      if (!contiguous) {
        out.push(group.slice(runStart, i));
        runStart = i;
      }
    }
    return out;
  };
  const readBatch = async (requests, signal) => {
    if (signal?.aborted)
      throw new Error("batch read canceled");
    batches++;
    pagesRequested += requests.length;
    const results = new Array(requests.length);
    const resolved = requests.map(resolve);
    if (loader.readBulk) {
      const ordered = resolved.slice().sort((left, right) => left.offset - right.offset);
      const groups2 = [];
      let group = [];
      let ranges = [];
      for (const page of ordered) {
        const candidate = { offset: page.offset, length: page.length };
        ranges.push(candidate);
        if (ranges.length > BULK_RANGE_CAPACITY || estimatedBulkResponseBytes(ranges) > BULK_RESPONSE_MAX_BYTES) {
          ranges.pop();
          if (group.length === 0)
            throw new RangeError("one page exceeds bulk response capacity");
          groups2.push(group);
          group = [];
          ranges = [candidate];
        }
        group.push(page);
      }
      if (group.length !== 0)
        groups2.push(group);
      const readGroup = async (pages) => {
        if (signal?.aborted)
          throw new Error("batch read canceled");
        const spans = pages.map((page) => ({ offset: page.offset, length: page.length }));
        const readStartedAt = performance.now();
        const parts = await loader.readBulk(spans);
        const readMs = performance.now() - readStartedAt;
        if (parts.length !== pages.length)
          throw new Error("bulk page response count mismatch");
        reads++;
        totalReadMs += readMs;
        maxReadMs = Math.max(maxReadMs, readMs);
        runs++;
        if (pages.length > 1)
          pagesCoalesced += pages.length;
        for (let index = 0;index < pages.length; index++) {
          if (parts[index].byteLength !== pages[index].length)
            throw new Error("bulk page response length mismatch");
          results[pages[index].index] = parts[index];
        }
      };
      let nextGroup = 0;
      const concurrency = Math.min(2, readConcurrency);
      await Promise.all(Array.from({ length: concurrency }, async () => {
        while (true) {
          const index = nextGroup++;
          if (index >= groups2.length)
            return;
          await readGroup(groups2[index]);
        }
      }));
      return results;
    }
    const groups = new Map;
    for (const r of resolved) {
      const key = r.tail ? `${r.path}:tail:${r.mip}` : `${r.path}:${r.mip}:${r.y}`;
      let g = groups.get(key);
      if (!g) {
        g = [];
        groups.set(key, g);
      }
      g.push(r);
    }
    const allRuns = [];
    for (const group of groups.values()) {
      if (group[0].tail) {
        for (const r of group)
          allRuns.push([r]);
      } else {
        group.sort((a, b) => a.x - b.x);
        for (const run of coalesce(group))
          allRuns.push(run);
      }
    }
    const readRun = async (run) => {
      const runOffset = run[0].offset;
      let runSize = 0;
      for (const p of run)
        runSize += p.length;
      const readStartedAt = performance.now();
      const batchData = await loader.read(runOffset, runSize);
      const readMs = performance.now() - readStartedAt;
      reads++;
      totalReadMs += readMs;
      maxReadMs = Math.max(maxReadMs, readMs);
      runs++;
      if (run.length > 1)
        pagesCoalesced += run.length;
      let rel = 0;
      for (const p of run) {
        results[p.index] = batchData.subarray(rel, rel + p.length);
        rel += p.length;
      }
    };
    let next = 0;
    await Promise.all(Array.from({ length: readConcurrency }, async () => {
      while (true) {
        const i = next++;
        if (i >= allRuns.length)
          return;
        await readRun(allRuns[i]);
      }
    }));
    return results;
  };
  return {
    readBatch,
    getStats() {
      stats.reads = reads;
      stats.averageReadMs = reads === 0 ? 0 : totalReadMs / reads;
      stats.maxReadMs = maxReadMs;
      stats.batches = batches;
      stats.pagesRequested = pagesRequested;
      stats.pagesCoalesced = pagesCoalesced;
      stats.runs = runs;
      return stats;
    }
  };
}

// crates/afterglow-web/web/src/demos/range-bench/main.ts
var CONTAINER = "dungeon.big";
var ASSET = "Rock064_Color.png";
var DEFAULT_CONCURRENCY = 16;
function readConcurrency() {
  const value = Number(new URLSearchParams(location.search).get("concurrency"));
  return Number.isSafeInteger(value) && value > 0 && value <= 32 ? value : DEFAULT_CONCURRENCY;
}
function coalesceBytes() {
  const value = Number(new URLSearchParams(location.search).get("coalesceMiB"));
  return Number.isSafeInteger(value) && value >= 1 && value <= 16 ? value * 1024 * 1024 : 0;
}
function shuffle(requests) {
  let state = 2654435769;
  for (let index = requests.length - 1;index > 0; index--) {
    state ^= state << 13;
    state ^= state >>> 17;
    state ^= state << 5;
    const other = (state >>> 0) % (index + 1);
    const current = requests[index];
    requests[index] = requests[other];
    requests[other] = current;
  }
}
function protocols() {
  const values = new Set;
  for (const entry of performance.getEntriesByType("resource")) {
    const resource = entry;
    if (new URL(resource.name).pathname.endsWith(`/${CONTAINER}`))
      values.add(resource.nextHopProtocol || "unknown");
  }
  return [...values].sort();
}
function print(result) {
  const output = document.getElementById("out");
  if (!output)
    return;
  const aggregation = result.coalesceMiB === 0 ? "rows" : `${result.coalesceMiB} MiB`;
  output.textContent = [
    `asset=${ASSET} pages=${result.pages} source=${(result.bytes / 1048576).toFixed(1)} MiB`,
    `concurrency=${result.readConcurrency} reads=${result.rangeReads} coalesce=${aggregation} protocols=${result.protocols.join(",")}`,
    `${result.mibPerSecond.toFixed(1)} MiB/s in ${result.elapsedMs.toFixed(1)} ms`
  ].join(`
`);
}
async function run() {
  const source = createFetchRangeLoader();
  const header = await readBigHeader(source, CONTAINER, 2 * 1024 * 1024);
  const asset = header.assets.find((candidate) => candidate.name === ASSET)?.virtualTexture;
  if (!asset)
    throw new Error(`virtual texture not found: ${ASSET}`);
  const requests = [];
  let maxMip = 0;
  for (const mip of asset.mips) {
    maxMip = Math.max(maxMip, mip.mip);
    for (let y = 0;y < mip.pagesY; y++)
      for (let x = 0;x < mip.pagesX; x++)
        requests.push({ path: ASSET, mip: mip.mip, x, y });
  }
  const tail = findVTPageChunk(header, ASSET, maxMip + 1, 0, 0);
  if (tail)
    requests.push({ path: ASSET, mip: maxMip + 1, x: 0, y: 0, tail: true });
  shuffle(requests);
  let bytes = 0;
  for (const request of requests) {
    const chunk = request.tail ? tail : findVTPageChunk(header, ASSET, request.mip, request.x, request.y);
    if (!chunk)
      throw new Error(`virtual texture page not found: ${request.mip}:${request.x}:${request.y}`);
    bytes += Number(chunk.compressedSize);
  }
  const concurrency = readConcurrency();
  const maxCoalesceBytes = coalesceBytes();
  performance.setResourceTimingBufferSize(512);
  performance.clearResourceTimings();
  const startedAt = performance.now();
  let rangeReads = 0;
  if (maxCoalesceBytes === 0) {
    const reader = createSourceSortedPageReader({
      read: (offset, length) => source.read(CONTAINER, offset, length),
      readBulk: (ranges) => source.readBulk(CONTAINER, ranges)
    }, header, concurrency);
    const pages = await reader.readBatch(requests);
    for (const page of pages)
      if (page.byteLength === 0)
        throw new Error("empty range response");
    rangeReads = reader.getStats().reads;
  } else {
    const ranges = [];
    for (const mip of asset.mips) {
      let offset = Number(mip.offset), runOffset = offset, runLength = 0;
      for (const length of mip.pageSizes) {
        if (runLength !== 0 && runLength + length > maxCoalesceBytes) {
          ranges.push({ offset: runOffset, length: runLength });
          runOffset = offset;
          runLength = 0;
        }
        runLength += length;
        offset += length;
      }
      if (runLength !== 0)
        ranges.push({ offset: runOffset, length: runLength });
    }
    if (tail)
      ranges.push({ offset: Number(tail.offset), length: Number(tail.compressedSize) });
    let next = 0;
    await Promise.all(Array.from({ length: concurrency }, async () => {
      while (true) {
        const index = next++;
        if (index >= ranges.length)
          return;
        const range = ranges[index];
        const data = await source.read(CONTAINER, range.offset, range.length);
        if (data.byteLength !== range.length)
          throw new Error("short coalesced range response");
      }
    }));
    rangeReads = ranges.length;
  }
  const elapsedMs = performance.now() - startedAt;
  const result = {
    bytes,
    elapsedMs,
    mibPerSecond: bytes / 1048576 / (elapsedMs / 1000),
    pages: requests.length,
    rangeReads,
    coalesceMiB: maxCoalesceBytes / 1048576,
    protocols: protocols(),
    readConcurrency: concurrency
  };
  print(result);
  return result;
}
window.runAfterglowRangeBench = run;
