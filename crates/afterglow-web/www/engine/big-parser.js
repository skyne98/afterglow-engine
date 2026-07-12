// crates/afterglow-web/www/engine/big-parser.ts
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
  const [val, newOff] = decodeVarint(bytes, off);
  return [BigInt(val), newOff];
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
function decodeChunkMeta(bytes, off) {
  const [variant, o] = decodeU32(bytes, off);
  switch (variant) {
    case 0: {
      const [w, o2] = decodeU32(bytes, o);
      const [h, o3] = decodeU32(bytes, o2);
      return [{ type: "Texture", width: w, height: h }, o3];
    }
    case 1: {
      const [ic, o2] = decodeU32(bytes, o);
      const [vc, o3] = decodeU32(bytes, o2);
      const [ps, o4] = decodeU32(bytes, o3);
      const [us, o5] = decodeU32(bytes, o4);
      return [{ type: "Mesh", indexCount: ic, vertexCount: vc, positionStride: ps, uvStride: us }, o5];
    }
    case 2: {
      const [mip, o2] = decodeU8(bytes, o);
      const [px, o3] = decodeU32(bytes, o2);
      const [py, o4] = decodeU32(bytes, o3);
      const [encoding, o5] = decodeTextureEncoding(bytes, o4);
      return [{ type: "VirtualTexturePage", mip, pageX: px, pageY: py, encoding }, o5];
    }
    case 3: {
      return [{ type: "Raw" }, o];
    }
    case 4: {
      const [firstMip, o2] = decodeU8(bytes, o);
      const [encoding, o3] = decodeTextureEncoding(bytes, o2);
      return [{ type: "VirtualTextureMipTail", mip: firstMip, encoding }, o3];
    }
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
function decodeAssetEntry(bytes, off) {
  const [name, o1] = decodeString(bytes, off);
  const [assetType, o2] = decodeAssetType(bytes, o1);
  const [chunks, o3] = decodeVec(bytes, o2, decodeChunkInfo);
  return [{ name, assetType, chunks }, o3];
}
var BIG_MAGIC = 826755394;
var BIG_VERSION = 3;
function parseBigHeader(data) {
  if (data.length < 16)
    throw new Error(".big: file too small");
  const magic = new DataView(data.buffer, data.byteOffset, 4).getUint32(0, true);
  if (magic !== BIG_MAGIC)
    throw new Error(".big: bad magic");
  const version = new DataView(data.buffer, data.byteOffset + 4, 4).getUint32(0, true);
  if (version !== BIG_VERSION)
    throw new Error(`.big: version ${version} != ${BIG_VERSION}`);
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
function findVTMipTailChunk(header, assetName) {
  const asset = header.assets.find((a) => a.name === assetName);
  return asset?.chunks.find((c) => c.meta.type === "VirtualTextureMipTail") ?? null;
}
function findVTPageChunk(header, assetName, mip, pageX, pageY) {
  const asset = header.assets.find((a) => a.name === assetName);
  if (!asset)
    return null;
  return asset.chunks.find((c) => c.meta.type === "VirtualTexturePage" && c.meta.mip === mip && c.meta.pageX === pageX && c.meta.pageY === pageY) ?? null;
}
function createPageDataProvider(loader, header, textureWorker, format) {
  let bigFileData = null;
  return async (path, req) => {
    const chunk = req.tail ? findVTMipTailChunk(header, path) : findVTPageChunk(header, path, req.mip, req.x, req.y);
    if (!chunk) {
      throw new Error(`VT page not found: ${path} mip=${req.mip} (${req.x},${req.y})`);
    }
    const pageData = await loader.read(path + ".big", Number(chunk.offset), Number(chunk.compressedSize));
    if (chunk.meta.encoding === "RawRgba8") {
      if (format !== 4) {
        throw new Error(`VT page ${path} is raw RGBA8 but GPU format ${format} requires Basis encoding`);
      }
      return pageData;
    }
    const transcoded = await textureWorker.transcode(pageData, format);
    if (transcoded.byteLength < 16)
      throw new Error("truncated transcoded VT page");
    const view = new DataView(transcoded.buffer, transcoded.byteOffset, transcoded.byteLength);
    const count = view.getUint32(0, true);
    const width = view.getUint32(4, true);
    const height = view.getUint32(8, true);
    const length = view.getUint32(12, true);
    if (count < 1 || width !== 136 || height !== 136 || 16 + length > transcoded.byteLength)
      throw new Error(`invalid transcoded VT page header: count=${count}, size=${width}x${height}, bytes=${length}`);
    return transcoded.slice(16, 16 + length);
  };
}
export {
  parseBigHeader,
  findVTPageChunk,
  findVTMipTailChunk,
  createPageDataProvider,
  BIG_VERSION,
  BIG_MAGIC
};
