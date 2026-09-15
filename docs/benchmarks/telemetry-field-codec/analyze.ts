import { readFileSync } from 'node:fs';

interface Sample {
  case: string;
  arch: string;
  os: string;
  iterations_per_sample: number;
  payload_bytes: number;
  encode_ns_samples: number[];
  decode_ns_samples: number[];
}
function read(name: string): Sample[] {
  return readFileSync(new URL(name, import.meta.url), 'utf8').trim().split('\n').map(line => JSON.parse(line));
}
function median(values: number[]): number {
  if (values.length !== 7 || values.some(value => !Number.isFinite(value) || value < 0)) throw new Error('Invalid benchmark samples');
  return [...values].sort((a, b) => a - b)[3]!;
}
const fixed = read('native.jsonl'), compact = read('compact.jsonl');
if (fixed.length !== compact.length) throw new Error('Different benchmark cases');
console.log(JSON.stringify(fixed.map((before, index) => {
  const after = compact[index]!;
  if (before.case !== after.case || before.arch !== after.arch || before.os !== after.os || before.iterations_per_sample !== after.iterations_per_sample) throw new Error('Different benchmark conditions');
  return {
    case: before.case,
    fixed_bytes: before.payload_bytes, compact_bytes: after.payload_bytes,
    fixed_encode_ns: median(before.encode_ns_samples), compact_encode_ns: median(after.encode_ns_samples),
    fixed_decode_ns: median(before.decode_ns_samples), compact_decode_ns: median(after.decode_ns_samples),
  };
}), null, 2));
