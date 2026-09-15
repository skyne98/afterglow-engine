// Bounded field-codec prototype. This does not change the active capture ABI.
export type FieldPrivacy = 'public' | 'sensitive' | 'secret';
export type FieldReferenceKind = 'operation' | 'slice' | 'event' | 'resource' | 'epoch' | 'queue' | 'ticket' | 'snapshot' | 'artifact' | 'track';
export interface FieldSpec {
  readonly name: string;
  readonly type: 'f64' | 'u32' | 'i32' | 'bool' | 'text' | 'enum' | 'ref';
  readonly privacy?: FieldPrivacy;
  readonly maxBytes?: number;
  readonly values?: readonly string[];
  readonly refKind?: FieldReferenceKind;
}
export interface FieldLimits { readonly fields: number; readonly payloadBytes: number; readonly metadataBytes: number }
export interface FieldPolicy { readonly defaultPrivacy: FieldPrivacy; readonly retainSensitive: boolean }
export interface FieldReference {
  readonly kind: FieldReferenceKind;
  readonly session: readonly [number, number, number, number];
  readonly producer: number;
  readonly producerGeneration: number;
  readonly id: bigint;
  readonly generation: number;
}
// null means explicitly redacted. Enum writers use registered numeric indices.
export type FieldValue = number | boolean | string | FieldReference | null;
export enum FieldStatus { Ok = 0, Capacity = -1, InvalidValue = -2, InvalidEncoding = -3 }
interface Slot { readonly type: FieldSpec['type']; readonly width: number; readonly retained: boolean; readonly maxBytes: number; readonly enumCount: number; readonly refKind: FieldReferenceKind | undefined }
const referenceKinds = new Set<FieldReferenceKind>(['operation', 'slice', 'event', 'resource', 'epoch', 'queue', 'ticket', 'snapshot', 'artifact', 'track']);
function privacy(value: unknown): value is FieldPrivacy { return value === 'public' || value === 'sensitive' || value === 'secret'; }
function uint(value: unknown): value is number { return typeof value === 'number' && Number.isInteger(value) && value >= 0 && value <= 0xffffffff; }

// Validate surrogate pairs before output mutation. No temporary encoded string is necessary.
// @hot-no-alloc-begin FieldCodec.utf8
function utf8(value: string, output?: DataView, start = 0, maximum = Number.MAX_SAFE_INTEGER): number {
  let length = 0;
  for (let index = 0; index < value.length; index++) {
    let point = value.charCodeAt(index);
    if (point >= 0xd800 && point <= 0xdbff) {
      const low = value.charCodeAt(++index);
      if (!(low >= 0xdc00 && low <= 0xdfff)) return -1;
      point = 0x10000 + (point - 0xd800) * 1024 + low - 0xdc00;
    } else if (point >= 0xdc00 && point <= 0xdfff) return -1;
    const count = point < 0x80 ? 1 : point < 0x800 ? 2 : point < 0x10000 ? 3 : 4;
    if (output) {
      if (count === 1) output.setUint8(start + length, point);
      else {
        output.setUint8(start + length, (count === 2 ? 0xc0 : count === 3 ? 0xe0 : 0xf0) | (point >> (6 * (count - 1))));
        for (let byte = 1; byte < count; byte++) output.setUint8(start + length + byte, 0x80 | ((point >> (6 * (count - byte - 1))) & 63));
      }
    }
    length += count;
    if (length > maximum) return -1;
  }
  return length;
}
// @hot-no-alloc-end FieldCodec.utf8
function validUtf8(input: DataView, start: number, length: number): boolean {
  const end = start + length;
  while (start < end) {
    const head = input.getUint8(start++);
    if (head < 0x80) continue;
    const count = head >= 0xc2 && head <= 0xdf ? 1 : head >= 0xe0 && head <= 0xef ? 2 : head >= 0xf0 && head <= 0xf4 ? 3 : -1;
    if (count < 0 || start + count > end) return false;
    let point = head & (count === 1 ? 31 : count === 2 ? 15 : 7);
    for (let index = 0; index < count; index++) {
      const next = input.getUint8(start++);
      if ((next & 0xc0) !== 0x80) return false;
      point = (point << 6) | (next & 63);
    }
    if (point < (count === 1 ? 0x80 : count === 2 ? 0x800 : 0x10000) || point > 0x10ffff || (point >= 0xd800 && point <= 0xdfff)) return false;
  }
  return true;
}

export class FieldCodec {
  readonly maxEncodedBytes: number;
  readonly fieldCount: number;
  private readonly slots: readonly Slot[];
  private readonly decoder = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true });
  constructor(fields: readonly FieldSpec[], limits: FieldLimits, policy: FieldPolicy) {
    for (const value of [limits.fields, limits.payloadBytes, limits.metadataBytes]) if (!Number.isSafeInteger(value) || value < 0) throw new RangeError('Invalid field limit');
    if (!privacy(policy.defaultPrivacy) || typeof policy.retainSensitive !== 'boolean') throw new TypeError('Invalid field policy');
    if (fields.length > limits.fields) throw new RangeError('Field capacity exceeded');
    const slots: Slot[] = [], names = new Set<string>();
    let bytes = 0, metadata = 0;
    for (const field of fields) {
      if (typeof field.name !== 'string' || !field.name || names.has(field.name) || (field.privacy !== undefined && !privacy(field.privacy))) throw new TypeError('Invalid field schema');
      const nameBytes = utf8(field.name, undefined, 0, limits.metadataBytes - metadata);
      if (nameBytes < 0) throw new TypeError('Invalid field name');
      names.add(field.name); metadata += nameBytes;
      let width: number, maxBytes = 0, enumCount = 0;
      switch (field.type) {
        case 'f64': width = 8; break;
        case 'u32': case 'i32': width = 4; break;
        case 'bool': width = 1; break;
        case 'text':
          if (!uint(field.maxBytes)) throw new TypeError('Invalid text capacity');
          maxBytes = field.maxBytes; width = 4 + maxBytes; break;
        case 'enum': {
          const values = field.values;
          if (!values || values.length === 0 || values.length > 0xffffffff) throw new TypeError('Invalid enum');
          const unique = new Set<string>();
          for (const value of values) {
            if (typeof value !== 'string' || !value || unique.has(value)) throw new TypeError('Invalid enum value');
            const size = utf8(value, undefined, 0, limits.metadataBytes - metadata);
            if (size < 0) throw new TypeError('Invalid or excessive enum text');
            metadata += size; unique.add(value);
          }
          enumCount = values.length; width = 4; break;
        }
        case 'ref': if (!referenceKinds.has(field.refKind!)) throw new TypeError('Invalid reference kind'); width = 36; break;
        default: throw new TypeError('Invalid field type');
      }
      const classification = field.privacy ?? policy.defaultPrivacy;
      const retained = classification === 'public' || (classification === 'sensitive' && policy.retainSensitive);
      bytes += 1 + (retained ? width : 0);
      if (!Number.isSafeInteger(bytes) || bytes > limits.payloadBytes || !Number.isSafeInteger(metadata) || metadata > limits.metadataBytes) throw new RangeError('Field capacity exceeded');
      slots.push(Object.freeze({ type: field.type, width, retained, maxBytes, enumCount, refKind: field.refKind }));
    }
    this.slots = Object.freeze(slots); this.maxEncodedBytes = bytes; this.fieldCount = slots.length;
    Object.freeze(this);
  }

  // Caller owns and reuses values and DataView. All validation precedes output mutation.
  // @hot-no-alloc-begin FieldCodec.encodeInto
  encodeInto(values: readonly FieldValue[], output: DataView): number {
    if (!(output.buffer instanceof ArrayBuffer)) return FieldStatus.InvalidValue;
    if (values.length !== this.fieldCount) return FieldStatus.InvalidValue;
    let needed = this.fieldCount;
    for (let index = 0; index < this.fieldCount; index++) {
      const slot = this.slots[index]!;
      if (!slot.retained) continue;
      const size = this.valueSize(slot, values[index]!);
      if (size < 0) return FieldStatus.InvalidValue;
      needed += size;
    }
    if (output.byteLength < needed) return FieldStatus.Capacity;
    let cursor = 0;
    for (let index = 0; index < this.fieldCount; index++) {
      const slot = this.slots[index]!, value = values[index]!;
      const present = slot.retained && value !== null;
      output.setUint8(cursor++, present ? 1 : 0);
      if (!present) continue;
      const offset = cursor;
      switch (slot.type) {
        case 'f64': output.setFloat64(offset, value as number, true); cursor += 8; break;
        case 'u32': case 'enum': output.setUint32(offset, value as number, true); cursor += 4; break;
        case 'i32': output.setInt32(offset, value as number, true); cursor += 4; break;
        case 'bool': output.setUint8(offset, value ? 1 : 0); cursor++; break;
        case 'text': {
          const size = utf8(value as string, output, offset + 4);
          output.setUint32(offset, size, true); cursor += 4 + size; break;
        }
        case 'ref': {
          const ref = value as FieldReference;
          for (let index = 0; index < 4; index++) output.setUint32(offset + index * 4, ref.session[index]!, true);
          output.setUint32(offset + 16, ref.producer, true); output.setUint32(offset + 20, ref.producerGeneration, true);
          output.setBigUint64(offset + 24, ref.id, true); output.setUint32(offset + 32, ref.generation, true); cursor += 36; break;
        }
      }
    }
    return needed;
  }

  // @hot-no-alloc-end FieldCodec.encodeInto

  // @hot-no-alloc-begin FieldCodec.valueSize
  private valueSize(slot: Slot, value: FieldValue): number {
    if (value === null) return 0;
    switch (slot.type) {
      case 'f64': return typeof value === 'number' && Number.isFinite(value) ? 8 : -1;
      case 'u32': return uint(value) ? 4 : -1;
      case 'i32': return typeof value === 'number' && Number.isInteger(value) && value >= -2147483648 && value <= 2147483647 ? 4 : -1;
      case 'bool': return typeof value === 'boolean' ? 1 : -1;
      case 'text': { if (typeof value !== 'string') return -1; const size = utf8(value, undefined, 0, slot.maxBytes); return size < 0 ? -1 : 4 + size; }
      case 'enum': return uint(value) && value < slot.enumCount ? 4 : -1;
      case 'ref': {
        if (typeof value !== 'object' || value.kind !== slot.refKind || !Array.isArray(value.session) || value.session.length !== 4) return -1;
        let nonzero = false;
        for (let index = 0; index < 4; index++) { if (!uint(value.session[index])) return -1; nonzero ||= value.session[index] !== 0; }
        return nonzero && uint(value.producer) && uint(value.producerGeneration) && value.producerGeneration > 0 && uint(value.generation) && value.generation > 0 && typeof value.id === 'bigint' && value.id >= 0n && value.id <= 0xffffffffffffffffn ? 36 : -1;
      }
    }
  }

  // @hot-no-alloc-end FieldCodec.valueSize

  // Cold decoder: strings and reference objects allocate only after full validation.
  decodeInto(input: DataView, output: FieldValue[]): FieldStatus {
    if (!(input.buffer instanceof ArrayBuffer) || input.byteLength > this.maxEncodedBytes) return FieldStatus.InvalidEncoding;
    if (output.length < this.fieldCount) return FieldStatus.Capacity;
    let cursor = 0;
    for (let index = 0; index < this.fieldCount; index++) {
      const size = this.slotSize(this.slots[index]!, input, cursor);
      if (size < 0) return FieldStatus.InvalidEncoding;
      cursor += size;
    }
    if (cursor !== input.byteLength) return FieldStatus.InvalidEncoding;
    cursor = 0;
    for (let index = 0; index < this.fieldCount; index++) {
      const slot = this.slots[index]!, offset = cursor + 1;
      const size = this.slotSize(slot, input, cursor);
      const present = input.getUint8(cursor) === 1;
      cursor += size;
      if (!present) { output[index] = null; continue; }
      switch (slot.type) {
        case 'f64': output[index] = input.getFloat64(offset, true); break;
        case 'u32': case 'enum': output[index] = input.getUint32(offset, true); break;
        case 'i32': output[index] = input.getInt32(offset, true); break;
        case 'bool': output[index] = input.getUint8(offset) === 1; break;
        case 'text': output[index] = this.decoder.decode(new Uint8Array(input.buffer, input.byteOffset + offset + 4, input.getUint32(offset, true))); break;
        case 'ref': output[index] = { kind: slot.refKind!, session: [input.getUint32(offset, true), input.getUint32(offset + 4, true), input.getUint32(offset + 8, true), input.getUint32(offset + 12, true)], producer: input.getUint32(offset + 16, true), producerGeneration: input.getUint32(offset + 20, true), id: input.getBigUint64(offset + 24, true), generation: input.getUint32(offset + 32, true) }; break;
      }
    }
    return FieldStatus.Ok;
  }

  private slotSize(slot: Slot, input: DataView, start: number): number {
    if (start >= input.byteLength) return -1;
    const offset = start + 1, state = input.getUint8(start);
    if (state === 0) return 1;
    if (state !== 1 || !slot.retained) return -1;
    if (slot.type === 'text') {
      if (input.byteLength - offset < 4) return -1;
      const length = input.getUint32(offset, true);
      if (length > slot.maxBytes || length > input.byteLength - offset - 4 || !validUtf8(input, offset + 4, length)) return -1;
      return 5 + length;
    }
    if (input.byteLength - offset < slot.width) return -1;
    switch (slot.type) {
      case 'f64': return Number.isFinite(input.getFloat64(offset, true)) ? 9 : -1;
      case 'u32': case 'i32': return 5;
      case 'bool': return input.getUint8(offset) <= 1 ? 2 : -1;
      case 'enum': return input.getUint32(offset, true) < slot.enumCount ? 5 : -1;
      case 'ref': return (input.getUint32(offset, true) !== 0 || input.getUint32(offset + 4, true) !== 0 || input.getUint32(offset + 8, true) !== 0 || input.getUint32(offset + 12, true) !== 0) && input.getUint32(offset + 20, true) !== 0 && input.getUint32(offset + 32, true) !== 0 ? 37 : -1;
    }
  }
}
