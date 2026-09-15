use afterglow_telemetry::fields::*;

const POLICY: Policy = Policy {
    default_privacy: Privacy::Public,
    retain_sensitive: false,
};
const LIMITS: Limits = Limits {
    fields: 16,
    payload_bytes: 256,
    metadata_bytes: 256,
};
const ENUM: &[&str] = &["idle", "ready"];
const FIELDS: &[Field<'static>] = &[
    Field {
        name: "cost",
        kind: FieldKind::F64,
        privacy: None,
    },
    Field {
        name: "count",
        kind: FieldKind::U32,
        privacy: None,
    },
    Field {
        name: "delta",
        kind: FieldKind::I32,
        privacy: None,
    },
    Field {
        name: "ready",
        kind: FieldKind::Bool,
        privacy: None,
    },
    Field {
        name: "label",
        kind: FieldKind::Text { max_bytes: 8 },
        privacy: Some(Privacy::Public),
    },
    Field {
        name: "state",
        kind: FieldKind::Enum(ENUM),
        privacy: None,
    },
    Field {
        name: "target",
        kind: FieldKind::Reference(ReferenceKind::Resource),
        privacy: None,
    },
    Field {
        name: "private",
        kind: FieldKind::Text { max_bytes: 4 },
        privacy: Some(Privacy::Sensitive),
    },
    Field {
        name: "secret",
        kind: FieldKind::U32,
        privacy: Some(Privacy::Secret),
    },
];
fn values() -> [Value<'static>; 9] {
    [
        Value::F64(-1.5),
        Value::U32(u32::MAX),
        Value::I32(i32::MIN),
        Value::Bool(true),
        Value::Text("é😀"),
        Value::Enum(1),
        Value::Reference(Reference {
            kind: ReferenceKind::Resource,
            session: [1, 2, 3, 4],
            producer: 0,
            producer_generation: 5,
            id: u64::MAX,
            generation: 6,
        }),
        Value::Text("excluded"),
        Value::U32(123),
    ]
}

#[test]
fn compact_fields_round_trip_without_excluded_values() {
    let codec = FieldCodec::new(FIELDS, LIMITS, POLICY).unwrap();
    assert_eq!(codec.field_count(), 9);
    assert_eq!(codec.max_encoded_len(), 78);
    let mut bytes = [0x55; 94];
    let mut decoded = [Value::Redacted; 10];
    assert_eq!(codec.encode_into(&values(), &mut bytes).unwrap(), 76);
    assert_eq!(&bytes[76..], &[0x55; 18]);
    assert_eq!(codec.decode_into(&bytes[..76], &mut decoded).unwrap(), 9);
    assert_eq!(&decoded[..7], &values()[..7]);
    assert_eq!(&decoded[7..], &[Value::Redacted; 3]);
    assert_eq!(&bytes[74..76], &[0, 0]);
    let expected = include_str!("fixtures/fields.hex")
        .split_whitespace()
        .map(|byte| u8::from_str_radix(byte, 16).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(&bytes[..76], expected);
}

#[test]
fn failed_encoding_keeps_output_unchanged() {
    let codec = FieldCodec::new(FIELDS, LIMITS, POLICY).unwrap();
    let mut bytes = [0x55; 90];
    for (index, value) in [
        (0, Value::F64(f64::NAN)),
        (0, Value::F64(f64::INFINITY)),
        (1, Value::I32(0)),
        (4, Value::Text("123456789")),
        (5, Value::Enum(2)),
        (
            6,
            Value::Reference(Reference {
                kind: ReferenceKind::Event,
                session: [1, 2, 3, 4],
                producer: 0,
                producer_generation: 1,
                id: 0,
                generation: 1,
            }),
        ),
    ] {
        let mut input = values();
        input[index] = value;
        assert_eq!(
            codec.encode_into(&input, &mut bytes),
            Err(FieldError::InvalidValue)
        );
        assert_eq!(bytes, [0x55; 90]);
    }
    assert_eq!(
        codec.encode_into(&values()[..8], &mut bytes),
        Err(FieldError::InvalidValue)
    );
    assert_eq!(
        codec.encode_into(&values(), &mut bytes[..75]),
        Err(FieldError::Capacity)
    );
    assert_eq!(bytes, [0x55; 90]);
}

#[test]
fn invalid_decoding_keeps_output_unchanged() {
    let codec = FieldCodec::new(FIELDS, LIMITS, POLICY).unwrap();
    let mut bytes = [0; 76];
    codec.encode_into(&values(), &mut bytes).unwrap();
    let mut output = [Value::U32(42); 9];
    for length in 0..76 {
        assert_eq!(
            codec.decode_into(&bytes[..length], &mut output),
            Err(FieldError::InvalidEncoding)
        );
        assert_eq!(output, [Value::U32(42); 9]);
    }
    // State, boolean, UTF-8, length, enum, and excluded field state.
    for (index, value) in [
        (0, 2),
        (20, 2),
        (26, 0xff),
        (22, 9),
        (32, 2),
        (33, 2),
        (74, 1),
        (75, 1),
    ] {
        let mut bad = bytes;
        bad[index] = value;
        let mut output = [Value::U32(42); 9];
        assert_eq!(
            codec.decode_into(&bad, &mut output),
            Err(FieldError::InvalidEncoding),
            "byte {index}"
        );
        assert_eq!(output, [Value::U32(42); 9]);
    }
    let mut bad = bytes;
    bad[70..74].fill(0);
    assert_eq!(
        codec.decode_into(&bad, &mut output),
        Err(FieldError::InvalidEncoding)
    );
    assert_eq!(
        codec.decode_into(&bytes, &mut output[..8]),
        Err(FieldError::Capacity)
    );
}

#[test]
fn compact_lengths_are_bounded_without_padding() {
    let fields = [
        Field {
            name: "text",
            kind: FieldKind::Text { max_bytes: 8 },
            privacy: None,
        },
        Field {
            name: "count",
            kind: FieldKind::U32,
            privacy: None,
        },
        Field {
            name: "secret",
            kind: FieldKind::Text {
                max_bytes: 1_000_000,
            },
            privacy: Some(Privacy::Secret),
        },
    ];
    let codec = FieldCodec::new(
        &fields,
        Limits {
            payload_bytes: 19,
            ..LIMITS
        },
        POLICY,
    )
    .unwrap();
    assert_eq!(codec.max_encoded_len(), 19);
    for text in [
        Value::Redacted,
        Value::Text(""),
        Value::Text("é"),
        Value::Text("12345678"),
    ] {
        for count in [Value::Redacted, Value::U32(0), Value::U32(u32::MAX)] {
            let input = [text, count, Value::F64(f64::NAN)];
            let mut bytes = [0x55; 20];
            let length = codec.encode_into(&input, &mut bytes).unwrap();
            assert!(length <= 19);
            assert!(bytes[length..].iter().all(|byte| *byte == 0x55));
            let mut decoded = [Value::Redacted; 3];
            codec.decode_into(&bytes[..length], &mut decoded).unwrap();
            assert_eq!(decoded, [text, count, Value::Redacted]);
            let mut exact = vec![0; length];
            assert_eq!(codec.encode_into(&input, &mut exact).unwrap(), length);
            assert_eq!(exact, bytes[..length]);
            assert_eq!(
                codec.encode_into(&input, &mut exact[..length - 1]),
                Err(FieldError::Capacity)
            );
            let mut padded = exact.clone();
            padded.push(0);
            let mut unchanged = [Value::U32(42); 3];
            assert_eq!(
                codec.decode_into(&padded, &mut unchanged),
                Err(FieldError::InvalidEncoding)
            );
            assert_eq!(unchanged, [Value::U32(42); 3]);
        }
    }
}

#[test]
fn schema_limits_and_privacy_are_explicit() {
    for limits in [
        Limits {
            fields: 8,
            ..LIMITS
        },
        Limits {
            payload_bytes: 77,
            ..LIMITS
        },
        Limits {
            metadata_bytes: 1,
            ..LIMITS
        },
    ] {
        assert!(matches!(
            FieldCodec::new(FIELDS, limits, POLICY),
            Err(FieldError::Capacity)
        ));
    }
    let duplicate = [FIELDS[0], FIELDS[0]];
    assert!(matches!(
        FieldCodec::new(&duplicate, LIMITS, POLICY),
        Err(FieldError::InvalidSchema)
    ));
    let invalid = [Field {
        name: "enum",
        kind: FieldKind::Enum(&["same", "same"]),
        privacy: None,
    }];
    assert!(matches!(
        FieldCodec::new(&invalid, LIMITS, POLICY),
        Err(FieldError::InvalidSchema)
    ));
    let codec = FieldCodec::new(
        FIELDS,
        LIMITS,
        Policy {
            default_privacy: Privacy::Secret,
            retain_sensitive: true,
        },
    )
    .unwrap();
    let mut input = values();
    input[7] = Value::Text("yes");
    let mut bytes = [0; 90];
    let length = codec.encode_into(&input, &mut bytes).unwrap();
    assert_eq!(length, 26);
    let mut output = [Value::Redacted; 9];
    codec.decode_into(&bytes[..length], &mut output).unwrap();
    assert_eq!(output[0], Value::Redacted);
    assert_eq!(output[4], Value::Text("é😀"));
    assert_eq!(output[7], Value::Text("yes"));
    assert_eq!(output[8], Value::Redacted);
    let empty = FieldCodec::new(
        &[],
        Limits {
            fields: 0,
            payload_bytes: 0,
            metadata_bytes: 0,
        },
        POLICY,
    )
    .unwrap();
    assert_eq!(empty.encode_into(&[], &mut []).unwrap(), 0);
    assert_eq!(empty.decode_into(&[], &mut []).unwrap(), 0);
}
