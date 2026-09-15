import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { FieldCodec, FieldStatus, type FieldLimits, type FieldPolicy, type FieldSpec, type FieldValue } from './fields.ts';

const policy: FieldPolicy = { defaultPrivacy: 'public', retainSensitive: false };
const limits: FieldLimits = { fields: 16, payloadBytes: 256, metadataBytes: 256 };
const fields: FieldSpec[] = [
  { name: 'cost', type: 'f64' }, { name: 'count', type: 'u32' },
  { name: 'delta', type: 'i32' }, { name: 'ready', type: 'bool' },
  { name: 'label', type: 'text', maxBytes: 8, privacy: 'public' },
  { name: 'state', type: 'enum', values: ['idle', 'ready'] },
  { name: 'target', type: 'ref', refKind: 'resource' },
  { name: 'private', type: 'text', maxBytes: 4, privacy: 'sensitive' },
  { name: 'secret', type: 'u32', privacy: 'secret' },
];
function values(): FieldValue[] {
  return [-1.5, 0xffffffff, -2147483648, true, 'é😀', 1,
    { kind: 'resource', session: [1, 2, 3, 4], producer: 0, producerGeneration: 5, id: 0xffffffffffffffffn, generation: 6 },
    'excluded', 123];
}
const fixture = Uint8Array.from(readFileSync(new URL('../../tests/fixtures/fields.hex', import.meta.url), 'utf8').trim().split(/\s+/), byte => parseInt(byte, 16));

describe('bounded fields', () => {
  test('Rust and TypeScript use the same bytes and preserve output boundaries', () => {
    const codec = new FieldCodec(fields, limits, policy);
    const bytes = new Uint8Array(98).fill(0x55), output = new Array<FieldValue>(10).fill(null);
    const view = new DataView(bytes.buffer, 4, 94);
    expect(codec.fieldCount).toBe(9); expect(codec.maxEncodedBytes).toBe(78);
    expect(codec.encodeInto(values(), view)).toBe(76);
    expect(bytes.subarray(4, 80)).toEqual(fixture);
    expect(bytes.subarray(0, 4)).toEqual(new Uint8Array(4).fill(0x55));
    expect(bytes.subarray(80)).toEqual(new Uint8Array(18).fill(0x55));
    expect(codec.decodeInto(new DataView(bytes.buffer, 4, 76), output)).toBe(FieldStatus.Ok);
    expect(output.slice(0, 7)).toEqual(values().slice(0, 7));
    expect(output.slice(7)).toEqual([null, null, null]);
  });

  test('invalid values and short buffers do not change output', () => {
    const codec = new FieldCodec(fields, limits, policy);
    const bytes = new Uint8Array(90).fill(0x55), view = new DataView(bytes.buffer);
    const invalid: [number, FieldValue][] = [
      [0, NaN], [0, Infinity], [1, -1], [1, 0x100000000], [1, 1.1],
      [2, 2147483648], [2, -2147483649], [3, 1], [4, '123456789'],
      [4, '\ud800'], [4, '\udc00'], [4, '\ud800x'], [5, 2], [5, -1],
      [6, { kind: 'event', session: [1, 2, 3, 4], producer: 0, producerGeneration: 1, id: 0n, generation: 1 }],
      [6, { kind: 'resource', session: [0, 0, 0, 0], producer: 0, producerGeneration: 1, id: 0n, generation: 1 }],
      [6, { kind: 'resource', session: [1, 0, 0, 0], producer: 0, producerGeneration: 0, id: 0n, generation: 1 }],
      [6, { kind: 'resource', session: [1, 0, 0, 0], producer: 0, producerGeneration: 1, id: -1n, generation: 1 }],
      [6, { kind: 'resource', session: [1, 0, 0, 0], producer: 0, producerGeneration: 1, id: 0x10000000000000000n, generation: 1 }],
    ];
    for (const [index, value] of invalid) {
      const input = values(); input[index] = value;
      expect(codec.encodeInto(input, view)).toBe(FieldStatus.InvalidValue);
      expect(bytes).toEqual(new Uint8Array(90).fill(0x55));
    }
    expect(codec.encodeInto(values().slice(1), view)).toBe(FieldStatus.InvalidValue);
    expect(codec.encodeInto(values(), new DataView(bytes.buffer, 0, 75))).toBe(FieldStatus.Capacity);
    expect(codec.encodeInto(values(), new DataView(new SharedArrayBuffer(90)))).toBe(FieldStatus.InvalidValue);
    expect(bytes).toEqual(new Uint8Array(90).fill(0x55));
  });

  test('malformed input never changes decoded values', () => {
    const codec = new FieldCodec(fields, limits, policy), output: FieldValue[] = new Array(9).fill(42);
    for (let length = 0; length < fixture.length; length++) {
      expect(codec.decodeInto(new DataView(fixture.buffer, fixture.byteOffset, length), output)).toBe(FieldStatus.InvalidEncoding);
      expect(output).toEqual(new Array(9).fill(42));
    }
    for (const [offset, value] of [[0, 2], [20, 2], [26, 255], [22, 9], [32, 2], [33, 2], [74, 1], [75, 1], [70, 0], [58, 0]]) {
      const bad = fixture.slice(); bad[offset!] = value!;
      expect(codec.decodeInto(new DataView(bad.buffer), output)).toBe(FieldStatus.InvalidEncoding);
      expect(output).toEqual(new Array(9).fill(42));
    }
    const infinite = fixture.slice(); new DataView(infinite.buffer).setFloat64(1, Infinity, true);
    expect(codec.decodeInto(new DataView(infinite.buffer), output)).toBe(FieldStatus.InvalidEncoding);
    expect(codec.decodeInto(new DataView(new SharedArrayBuffer(90)), output)).toBe(FieldStatus.InvalidEncoding);
    expect(codec.decodeInto(new DataView(fixture.buffer), output.slice(1))).toBe(FieldStatus.Capacity);
    expect(codec.decodeInto(new DataView(new ArrayBuffer(91)), output)).toBe(FieldStatus.InvalidEncoding);
    expect(output).toEqual(new Array(9).fill(42));
  });

  test('text preserves UTF-8 and rejects non-canonical encodings', () => {
    const codec = new FieldCodec([{ name: 'text', type: 'text', maxBytes: 32 }], limits, policy);
    const bytes = new Uint8Array(codec.maxEncodedBytes), view = new DataView(bytes.buffer), output: FieldValue[] = [null];
    for (const text of ['', '\0', '\ufeff', 'Aé€😀\udbff\udfff', 'x'.repeat(32)]) {
      const length = codec.encodeInto([text], view);
      expect(length).toBe(5 + new TextEncoder().encode(text).length);
      expect(codec.decodeInto(new DataView(bytes.buffer, 0, length), output)).toBe(FieldStatus.Ok); expect(output[0]).toBe(text);
    }
    for (const invalid of [[0xc0, 0x80], [0xe0, 0x80, 0x80], [0xed, 0xa0, 0x80], [0xf0, 0x80, 0x80, 0x80], [0xf4, 0x90, 0x80, 0x80], [0xf5, 0x80, 0x80, 0x80], [0xc2], [0x80], [0xe2, 0x28, 0xa1]]) {
      bytes.fill(0); bytes[0] = 1; view.setUint32(1, invalid.length, true); bytes.set(invalid, 5);
      output[0] = 42;
      expect(codec.decodeInto(new DataView(bytes.buffer, 0, 5 + invalid.length), output)).toBe(FieldStatus.InvalidEncoding); expect(output[0]).toBe(42);
    }
  });

  test('UTF-8 output agrees with the platform encoder across the Unicode range', () => {
    const codec = new FieldCodec([{ name: 'text', type: 'text', maxBytes: 4 }], limits, policy);
    const bytes = new Uint8Array(codec.maxEncodedBytes), view = new DataView(bytes.buffer), encoder = new TextEncoder();
    const input: FieldValue[] = [''], output: FieldValue[] = [null];
    for (let point = 0; point <= 0x10ffff; point += 137) {
      if (point >= 0xd800 && point <= 0xdfff) continue;
      input[0] = String.fromCodePoint(point);
      const length = codec.encodeInto(input, view);
      expect(length).toBe(5 + encoder.encode(input[0]).length);
      expect(bytes.subarray(5, length)).toEqual(encoder.encode(input[0]));
      expect(codec.decodeInto(new DataView(bytes.buffer, 0, length), output)).toBe(FieldStatus.Ok);
      expect(output[0]).toBe(input[0]);
    }
  });

  test('short and redacted values need no padding or maximum-size output', () => {
    const codec = new FieldCodec([
      { name: 'text', type: 'text', maxBytes: 8 },
      { name: 'count', type: 'u32' },
      { name: 'secret', type: 'text', maxBytes: 1_000_000, privacy: 'secret' },
    ], { ...limits, payloadBytes: 19 }, policy);
    expect(codec.maxEncodedBytes).toBe(19);
    for (const text of [null, '', 'é', '12345678']) for (const count of [null, 0, 0xffffffff]) {
      const input: FieldValue[] = [text, count, NaN];
      const bytes = new Uint8Array(20).fill(0x55);
      const length = codec.encodeInto(input, new DataView(bytes.buffer));
      expect(length).toBeGreaterThanOrEqual(3); expect(length).toBeLessThanOrEqual(19);
      expect(bytes.subarray(length)).toEqual(new Uint8Array(20 - length).fill(0x55));
      const output: FieldValue[] = [42, 42, 42];
      expect(codec.decodeInto(new DataView(bytes.buffer, 0, length), output)).toBe(FieldStatus.Ok);
      expect(output).toEqual([text, count, null]);
      const exact = new Uint8Array(length);
      expect(codec.encodeInto(input, new DataView(exact.buffer))).toBe(length);
      expect(exact).toEqual(bytes.subarray(0, length));
      expect(codec.encodeInto(input, new DataView(exact.buffer, 0, length - 1))).toBe(FieldStatus.Capacity);
      const padded = new Uint8Array(length + 1); padded.set(exact);
      output.fill(42);
      expect(codec.decodeInto(new DataView(padded.buffer), output)).toBe(FieldStatus.InvalidEncoding);
      expect(output).toEqual([42, 42, 42]);
    }
  });

  test('registration limits and field privacy remain explicit', () => {
    for (const constrained of [{ ...limits, fields: 8 }, { ...limits, payloadBytes: 77 }, { ...limits, metadataBytes: 1 }]) expect(() => new FieldCodec(fields, constrained, policy)).toThrow();
    for (const invalid of [
      [{ name: 'x', type: 'f64' }, { name: 'x', type: 'u32' }],
      [{ name: '', type: 'f64' }], [{ name: '\ud800', type: 'f64' }],
      [{ name: 'x', type: 'enum', values: [] }], [{ name: 'x', type: 'enum', values: ['x', 'x'] }],
      [{ name: 'x', type: 'enum', values: ['\ud800'] }], [{ name: 'x', type: 'text', maxBytes: -1 }],
      [{ name: 'x', type: 'ref', refKind: 'bad' }], [{ name: 'x', type: 'bad' }],
    ]) expect(() => new FieldCodec(invalid as FieldSpec[], limits, policy)).toThrow();
    const codec = new FieldCodec(fields, limits, { defaultPrivacy: 'secret', retainSensitive: true });
    const input = values(), view = new DataView(new ArrayBuffer(90)), output: FieldValue[] = new Array(9).fill(null);
    input[7] = 'yes';
    expect(codec.encodeInto(input, view)).toBe(26); expect(codec.decodeInto(new DataView(view.buffer, 0, 26), output)).toBe(FieldStatus.Ok);
    expect(output).toEqual([null, null, null, null, 'é😀', null, null, 'yes', null]);
    const empty = new FieldCodec([], { fields: 0, payloadBytes: 0, metadataBytes: 0 }, policy);
    expect(empty.encodeInto([], new DataView(new ArrayBuffer(0)))).toBe(FieldStatus.Ok);
    expect(empty.decodeInto(new DataView(new ArrayBuffer(0)), [])).toBe(FieldStatus.Ok);
  });

  test('registration captures immutable layout and every accepted byte change is canonical', () => {
    const spec: FieldSpec[] = [{ name: 'x', type: 'enum', values: ['a', 'b'] }];
    const codec = new FieldCodec(spec, limits, policy), view = new DataView(new ArrayBuffer(5));
    (spec[0]!.values as string[]).push('c'); spec.length = 0;
    expect(codec.encodeInto([2], view)).toBe(FieldStatus.InvalidValue);
    expect(codec.encodeInto([1], view)).toBe(5);
    const full = new FieldCodec(fields, limits, policy);
    for (let offset = 0; offset < fixture.length; offset++) {
      const bad = fixture.slice(); bad[offset] ^= 0xff;
      const output: FieldValue[] = new Array(9).fill(42), encoded = new Uint8Array(fixture.length);
      const status = full.decodeInto(new DataView(bad.buffer), output);
      if (status === FieldStatus.Ok) {
        expect(full.encodeInto(output, new DataView(encoded.buffer))).toBe(fixture.length); expect(encoded).toEqual(bad);
      } else expect(output).toEqual(new Array(9).fill(42));
    }
  });
});
