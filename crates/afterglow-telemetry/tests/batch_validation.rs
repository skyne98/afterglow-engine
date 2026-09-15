use afterglow_telemetry::{
    BATCH_HEADER_BYTES, BatchError, BatchHeader, TraceRecord, decode_batch_into, encode_batch_into,
    encoded_batch_len,
};

fn header() -> BatchHeader {
    BatchHeader {
        record_count: 2,
        next_sequence: 2,
        session: [1, 2, 3, 4],
        producer_generation: 1,
        clock_generation: 1,
        ticks_per_second: 1_000_000_000,
        ..BatchHeader::default()
    }
}

fn records() -> [TraceRecord; 2] {
    [
        TraceRecord {
            timestamp: 10,
            phase: 1,
            ..TraceRecord::default()
        },
        TraceRecord {
            timestamp: 20,
            phase: 1,
            ..TraceRecord::default()
        },
    ]
}

#[test]
fn malformed_input_never_changes_decoder_output() {
    let mut valid = vec![0; encoded_batch_len(2).unwrap()];
    encode_batch_into(header(), &records(), &mut valid).unwrap();
    let sentinel = [TraceRecord {
        timestamp: 99,
        ..TraceRecord::default()
    }; 2];
    let second = BATCH_HEADER_BYTES + 40;
    for (offset, value) in [
        (0, 0),
        (4, 2),
        (6, 0),
        (20, 2),
        (28, 1),
        (56, 0),
        (60, 0),
        (64, 1),
        (72, 3),
        (88, 1),
        (second + 36, 0),
        (second + 36, 9),
        (second + 37, 1),
        (second + 38, 1),
        (second, 1),
    ] {
        let mut input = valid.clone();
        input[offset] = value;
        let mut output = sentinel;
        assert!(
            decode_batch_into(&input, &mut output).is_err(),
            "offset {offset}"
        );
        assert_eq!(output, sentinel);
    }
    let mut zero_rate = valid.clone();
    zero_rate[32..40].fill(0);
    let mut output = sentinel;
    assert_eq!(
        decode_batch_into(&zero_rate, &mut output),
        Err(BatchError::InvalidTickRate)
    );
    assert_eq!(output, sentinel);
    for length in 0..valid.len() {
        let mut output = sentinel;
        assert!(decode_batch_into(&valid[..length], &mut output).is_err());
        assert_eq!(output, sentinel);
    }
    valid.push(0);
    assert!(decode_batch_into(&valid, &mut output).is_err());
    assert_eq!(output, sentinel);
}

#[test]
fn invalid_records_and_headers_never_change_encoder_output() {
    let mut output = vec![0xa5; encoded_batch_len(2).unwrap()];
    for mode in 0..7 {
        let mut header = header();
        let mut records = records();
        match mode {
            0 => header.flags = 2,
            1 => header.ticks_per_second = 0,
            2 => header.record_count = 1,
            3 => records[1].phase = 0,
            4 => records[1].flags = 1,
            5 => records[1].reserved = 1,
            _ => records[1].timestamp = 9,
        }
        assert!(encode_batch_into(header, &records, &mut output).is_err());
        assert!(output.iter().all(|byte| *byte == 0xa5));
    }
    let mut short = [0xa5; 7];
    assert!(encode_batch_into(header(), &records(), &mut short).is_err());
    assert_eq!(short, [0xa5; 7]);
}
