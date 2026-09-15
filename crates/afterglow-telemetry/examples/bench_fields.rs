//! Codec microbenchmark, not an application latency or capture overhead measurement.

use std::hint::black_box;
use std::time::Instant;

use afterglow_telemetry::fields::*;

const ITERATIONS: usize = 200_000;
const SAMPLES: usize = 7;

fn run(name: &str, fields: &[Field<'_>], values: &[Value<'_>]) {
    let codec = FieldCodec::new(
        fields,
        Limits {
            fields: 16,
            payload_bytes: 65_536,
            metadata_bytes: 2048,
        },
        Policy {
            default_privacy: Privacy::Public,
            retain_sensitive: false,
        },
    )
    .unwrap();
    let mut bytes = vec![0; codec.max_encoded_len()];
    let encoded = codec.encode_into(values, &mut bytes).unwrap();
    let mut encode_ns = [0.0; SAMPLES];
    let mut decode_ns = [0.0; SAMPLES];
    for _ in 0..10_000 {
        black_box(
            codec
                .encode_into(black_box(values), black_box(&mut bytes))
                .unwrap(),
        );
    }
    for sample in 0..SAMPLES {
        let start = Instant::now();
        for _ in 0..ITERATIONS {
            black_box(
                black_box(&codec)
                    .encode_into(black_box(values), black_box(&mut bytes))
                    .unwrap(),
            );
        }
        encode_ns[sample] = start.elapsed().as_secs_f64() * 1e9 / ITERATIONS as f64;
        let mut decoded = [Value::Redacted; 16];
        let start = Instant::now();
        for _ in 0..ITERATIONS {
            black_box(
                black_box(&codec)
                    .decode_into(black_box(&bytes[..encoded]), black_box(&mut decoded))
                    .unwrap(),
            );
        }
        decode_ns[sample] = start.elapsed().as_secs_f64() * 1e9 / ITERATIONS as f64;
        assert_eq!(&decoded[..fields.len()], values);
    }
    println!(
        "{{\"case\":\"{name}\",\"arch\":\"{}\",\"os\":\"{}\",\"iterations_per_sample\":{ITERATIONS},\"payload_bytes\":{},\"encode_ns_samples\":{encode_ns:?},\"decode_ns_samples\":{decode_ns:?}}}",
        std::env::consts::ARCH,
        std::env::consts::OS,
        encoded
    );
}

fn main() {
    let scalar = [
        Field {
            name: "count",
            kind: FieldKind::U32,
            privacy: None,
        },
        Field {
            name: "bytes",
            kind: FieldKind::U32,
            privacy: None,
        },
    ];
    run("two_u32", &scalar, &[Value::U32(4096), Value::U32(65536)]);
    let reference = [Field {
        name: "resource",
        kind: FieldKind::Reference(ReferenceKind::Resource),
        privacy: None,
    }];
    run(
        "reference",
        &reference,
        &[Value::Reference(Reference {
            kind: ReferenceKind::Resource,
            session: [1, 2, 3, 4],
            producer: 0,
            producer_generation: 1,
            id: u64::MAX,
            generation: 1,
        })],
    );
    let text = [Field {
        name: "label",
        kind: FieldKind::Text { max_bytes: 64 },
        privacy: None,
    }];
    run("text_64_empty", &text, &[Value::Text("")]);
    run("text_64_short", &text, &[Value::Text("example")]);
    run(
        "text_64_full",
        &text,
        &[Value::Text(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )],
    );
    let large_text = [Field {
        name: "label",
        kind: FieldKind::Text { max_bytes: 1024 },
        privacy: None,
    }];
    run("text_1024_empty", &large_text, &[Value::Text("")]);
    let secret = [Field {
        name: "secret",
        kind: FieldKind::Text { max_bytes: 1024 },
        privacy: Some(Privacy::Secret),
    }];
    // Explicit redaction has the same payload as an excluded value.
    run("text_1024_redacted", &secret, &[Value::Redacted]);
}
