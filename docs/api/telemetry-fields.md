# Bounded telemetry fields

## Status

This codec is a prototype for registered record fields.
It does not change the active 40-byte records, DGTB batches, DGTL files, or CLI output.
The current capture path does not use it.
Schema registration and record integration remain open.

The implementations are:

- Rust: `crates/afterglow-telemetry/src/fields.rs`.
- TypeScript: `crates/afterglow-telemetry/web/src/fields.ts`.

## Layout and limits

`FieldCodec` registers an ordered field layout during bootstrap.
It calculates the maximum encoded size before record encoding.
The caller supplies field-count, payload-byte, and metadata-byte limits.
Metadata bytes include UTF-8 field names and enum values.
Field-count and metadata limits also bound the layout size and enum catalog.
Registration rejects duplicate names, duplicate enum values, empty names, and empty enums.

Each field starts with one state byte: `0` means redacted and `1` means present.
A redacted field has no payload bytes.
A present field has the following payload:

| Field type | Payload bytes | Validation |
| --- | ---: | --- |
| `f64` | 8 | Finite values only |
| `u32` | 4 | Unsigned 32-bit integer |
| `i32` | 4 | Signed 32-bit integer |
| `bool` | 1 | `0` or `1` |
| `text` | `4 + actual UTF-8 bytes` | `u32` byte length and text, bounded by `maxBytes` |
| `enum` | 4 | Index into the registered enum catalog |
| `ref` | 36 | Session, producer, producer generation, ID, generation |

Fields have no padding.
The maximum record size includes one state byte per field and the maximum payload of each retained field.
The actual record can be smaller.
All numbers use little-endian byte order.
The reference session has four `u32` values and must not be all zero.
The producer and generations use `u32` values.
The ID uses `u64` in Rust and `bigint` in TypeScript.
Both generations must be nonzero.
Producer zero and ID zero are permitted.
The layout specifies the reference kind, which is not repeated in the payload.

## Field exclusion

The caller supplies the default field classification and the `retainSensitive` policy during bootstrap.
Public fields are retained.
Sensitive fields are retained only when that policy is true.
Secret fields are never retained.
This policy controls recorded data, not connection permissions.

Each excluded field contains one zero state byte.
Their input values are not examined.
Explicit redaction uses `Value::Redacted` in Rust and `null` in TypeScript.
Text never truncates silently.

## Ownership and failure

Rust uses `FieldCodec::new`, `max_encoded_len`, `field_count`, `encode_into`, and `decode_into`.
TypeScript uses the constructor, `maxEncodedBytes`, `fieldCount`, `encodeInto`, and `decodeInto`.
Rust encoding returns the actual byte count through `Result<usize, FieldError>`.
TypeScript encoding returns the actual byte count on success or a negative `FieldStatus` on failure.
Its decoder returns `FieldStatus.Ok` on success.
Its constructor throws when the layout is invalid or exceeds its limits.
The output capacity must be sufficient for the actual record, not necessarily the maximum.

The caller owns and reuses the value array and output buffer.
TypeScript callers must supply stable ordinary values, not getters with side effects.
The encoder validates the full record before output mutation.
It changes only the necessary output prefix.
The decoder accepts only the exact encoded byte length.
It validates all slots before it changes output values.
Invalid states, lengths, UTF-8, enum indices, reference generations, and excluded-field payloads cause rejection.
Trailing bytes also cause rejection.
Use only the returned encoded prefix for transfer and decoding.

Rust encoding and decoding have no runtime allocation after bootstrap.
Decoded Rust strings borrow the input.
TypeScript encoding uses caller-owned arrays and an existing `DataView`.
TypeScript decoding is a cold operation: strings and reference objects allocate.
TypeScript rejects `SharedArrayBuffer` input and output to prevent concurrent payload mutation.
No sealed-runtime allocation claim applies to the full capture path.

## Checks

`tests/fixtures/fields.hex` is a shared 76-byte Rust/TypeScript fixture.
It includes UTF-8, integer limits, a full `u64` reference ID, and excluded fields.
Tests include exact-size output, short records, redaction, trailing bytes, malformed input, and the maximum-size limit.
The compact-layout checks passed: 41 Rust tests and 35 TypeScript tests with 44,944 assertions.
The Rust allocator check completed 100,003 encodes and 100,003 decodes without allocation.
Strict scoped Clippy, TypeScript checking, allocation lint, and the diff check also passed.
Empty 1 KiB text now encodes in five bytes, and redacted text encodes in one byte.
Their native median decode times were 13.021 ns and 6.874 ns in this microbenchmark.
The [codec benchmark](../benchmarks/telemetry-field-codec/README.md) records why the prototype no longer pads fields.
These measurements do not establish capture overhead or sealed-runtime acceptance.
