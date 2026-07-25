export const BULK_RANGE_CAPACITY = 256;
export const BULK_RESPONSE_MAX_BYTES = 4 * 1024 * 1024;
export const BULK_IN_FLIGHT_MAX_BYTES = 8 * 1024 * 1024;

export interface AssetByteRange {
  offset: number;
  length: number;
}

const HEADER_ALLOWANCE_PER_RANGE = 192;
const RESPONSE_FIXED_ALLOWANCE = 64;
const decoder = new TextDecoder('ascii');

export function estimatedBulkResponseBytes(ranges: readonly AssetByteRange[]): number {
  let bytes = RESPONSE_FIXED_ALLOWANCE;
  for (const range of ranges) bytes += range.length + HEADER_ALLOWANCE_PER_RANGE;
  return bytes;
}

function validateRanges(ranges: readonly AssetByteRange[]): void {
  if (ranges.length < 1 || ranges.length > BULK_RANGE_CAPACITY)
    throw new RangeError(`bulk range count must be 1..${BULK_RANGE_CAPACITY}`);
  if (estimatedBulkResponseBytes(ranges) > BULK_RESPONSE_MAX_BYTES)
    throw new RangeError('bulk response exceeds 4 MiB capacity');
  for (let index = 0; index < ranges.length; index++) {
    const range = ranges[index];
    if (!Number.isSafeInteger(range.offset) || range.offset < 0 ||
        !Number.isSafeInteger(range.length) || range.length <= 0)
      throw new RangeError('bulk ranges require positive safe-integer spans');
    const end = range.offset + range.length - 1;
    if (!Number.isSafeInteger(end)) throw new RangeError('bulk range end is unsafe');
    for (let other = 0; other < index; other++) {
      const prior = ranges[other];
      const priorEnd = prior.offset + prior.length - 1;
      if (range.offset <= priorEnd && end >= prior.offset)
        throw new RangeError('bulk ranges must not overlap');
    }
  }
}

function boundaryFrom(contentType: string): string {
  const match = /(?:^|;)\s*boundary=(?:"([^"]+)"|([^;\s]+))/i.exec(contentType);
  const boundary = match?.[1] ?? match?.[2] ?? '';
  if (!boundary || boundary.length > 128 || /[^\x21-\x7e]/.test(boundary))
    throw new Error('multipart byte-range response has an invalid boundary');
  return boundary;
}

function matches(bytes: Uint8Array, offset: number, pattern: Uint8Array): boolean {
  if (offset < 0 || offset + pattern.length > bytes.length) return false;
  for (let index = 0; index < pattern.length; index++)
    if (bytes[offset + index] !== pattern[index]) return false;
  return true;
}

function find(bytes: Uint8Array, pattern: Uint8Array, start: number, limit: number): number {
  const end = Math.min(bytes.length - pattern.length, limit);
  for (let offset = start; offset <= end; offset++)
    if (matches(bytes, offset, pattern)) return offset;
  return -1;
}

/** Parse one bounded `multipart/byteranges` body without scanning payload bytes
 * for boundaries. Each `Content-Range` gives the exact payload length. */
export function parseMultipartByteRanges(
  body: Uint8Array,
  contentType: string,
  requested: readonly AssetByteRange[],
): Uint8Array[] {
  validateRanges(requested);
  if (body.byteLength > BULK_RESPONSE_MAX_BYTES)
    throw new RangeError('bulk response exceeded 4 MiB capacity');
  const boundary = new TextEncoder().encode(`--${boundaryFrom(contentType)}`);
  const headerEndMarker = new Uint8Array([13, 10, 13, 10]);
  const crlf = new Uint8Array([13, 10]);
  const output = new Array<Uint8Array>(requested.length);
  let cursor = 0;
  for (let index = 0; index < requested.length; index++) {
    if (!matches(body, cursor, boundary))
      throw new Error(`multipart boundary missing at part ${index}`);
    cursor += boundary.length;
    if (!matches(body, cursor, crlf)) throw new Error('multipart part has no header line break');
    cursor += 2;
    const headerEnd = find(body, headerEndMarker, cursor, cursor + 1024);
    if (headerEnd < 0) throw new Error('multipart part headers exceed 1 KiB');
    const headers = decoder.decode(body.subarray(cursor, headerEnd));
    const match = /(?:^|\r\n)Content-Range:\s*bytes\s+(\d+)-(\d+)\/(?:\d+|\*)/i.exec(headers);
    if (!match) throw new Error('multipart part has no valid Content-Range');
    const start = Number(match[1]);
    const end = Number(match[2]);
    const expected = requested[index];
    if (start !== expected.offset || end !== expected.offset + expected.length - 1)
      throw new Error(`multipart part ${index} does not match its requested range`);
    const dataStart = headerEnd + 4;
    const dataEnd = dataStart + expected.length;
    if (dataEnd > body.length) throw new Error('multipart part payload is truncated');
    output[index] = body.subarray(dataStart, dataEnd);
    cursor = dataEnd;
    if (!matches(body, cursor, crlf)) throw new Error('multipart part has no trailing line break');
    cursor += 2;
  }
  if (!matches(body, cursor, boundary)) throw new Error('multipart closing boundary is missing');
  cursor += boundary.length;
  if (body[cursor] !== 45 || body[cursor + 1] !== 45)
    throw new Error('multipart closing boundary is malformed');
  return output;
}

/** One same-origin HTTP request for one or more non-overlapping byte ranges. */
export async function fetchByteRanges(
  url: string,
  ranges: readonly AssetByteRange[],
): Promise<Uint8Array[]> {
  validateRanges(ranges);
  const value = ranges
    .map(range => `${range.offset}-${range.offset + range.length - 1}`)
    .join(',');
  const response = await fetch(url, { headers: { Range: `bytes=${value}` } });
  if (response.status !== 206)
    throw new Error(`bulk asset range expected 206, got ${response.status}: ${url}`);
  const body = new Uint8Array(await response.arrayBuffer());
  if (ranges.length === 1) {
    if (body.byteLength !== ranges[0].length)
      throw new Error(`asset range returned ${body.byteLength} bytes; expected ${ranges[0].length}: ${url}`);
    return [body];
  }
  return parseMultipartByteRanges(body, response.headers.get('content-type') ?? '', ranges);
}
