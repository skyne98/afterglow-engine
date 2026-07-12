//! HTTP `Range` header parsing — single-range only.
//!
//! Both the CEF scheme handler and the web HTTP dev server use this to turn a
//! `Range: bytes=...` header into a `(start, end_inclusive)` spec they can pass
//! to [`crate::AssetSource::read_at`](crate::source::AssetSource::read_at).
//!
//! Single-range only (`bytes=0-499`, `bytes=500-`, `bytes=-500`). A
//! multi-range request (`bytes=0-99,200-299`) or any unparseable value yields
//! [`RangeSpec::Unsatisfiable`], and callers serve a full `200` response (we do
//! not emit `416 Range Not Satisfiable` — single-range video seeks are the only
//! real use case, and falling back to the full file is correct for them).

/// A parsed single range, or a signal to serve the full resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeSpec {
    /// `bytes=START-END` (inclusive). `end` is clamped to `len - 1`.
    Range { start: u64, end: u64 },
    /// Serve the full resource (`200`): no `Range` header, an unparseable
    /// value, or a multi-range request.
    Full,
    /// The range is outside `[0, len)` — caller should serve `416`.
    Unsatisfiable,
}

impl RangeSpec {
    /// The first byte to read. For `Full`, `0`.
    pub fn start(self) -> u64 {
        match self {
            RangeSpec::Range { start, .. } => start,
            RangeSpec::Full => 0,
            RangeSpec::Unsatisfiable => 0,
        }
    }

    /// The number of bytes to serve. For `Full`, `len`.
    pub fn len(self, total: u64) -> u64 {
        match self {
            RangeSpec::Range { start, end } => end - start + 1,
            RangeSpec::Full => total,
            RangeSpec::Unsatisfiable => 0,
        }
    }

    /// `true` if this spec serves no bytes.
    pub fn is_empty(self, total: u64) -> bool {
        self.len(total) == 0
    }
}

/// Parse a `Range` header value against a resource of `len` bytes.
///
/// Returns [`RangeSpec::Full`] when `header` is `None` or empty. Returns
/// [`RangeSpec::Unsatisfiable`] for an out-of-range request or any value that
/// isn't a single `bytes=` range.
pub fn parse_range(header: Option<&str>, len: u64) -> RangeSpec {
    let Some(h) = header.filter(|s| !s.is_empty()) else {
        return RangeSpec::Full;
    };
    let h = h.trim();
    let Some(rest) = h.strip_prefix("bytes=").map(str::trim) else {
        return RangeSpec::Full; // not a byte-range request
    };
    // Multi-range: reject → full (we don't serve multipart/byteranges).
    if rest.contains(',') {
        return RangeSpec::Full;
    }
    let Some((s, e)) = rest.split_once('-') else {
        return RangeSpec::Unsatisfiable;
    };
    let s = s.trim();
    let e = e.trim();
    match (s.is_empty(), e.is_empty()) {
        // `bytes=-N` — last N bytes
        (true, false) => match e.parse::<u64>() {
            Ok(n) => {
                if n == 0 || len == 0 {
                    return RangeSpec::Unsatisfiable;
                }
                let n = n.min(len);
                let start = len - n;
                RangeSpec::Range { start, end: len - 1 }
            }
            Err(_) => RangeSpec::Unsatisfiable,
        },
        // `bytes=N-` — from N to end
        (false, true) => match s.parse::<u64>() {
            Ok(start) => {
                if start >= len || len == 0 {
                    return RangeSpec::Unsatisfiable;
                }
                RangeSpec::Range { start, end: len - 1 }
            }
            Err(_) => RangeSpec::Unsatisfiable,
        },
        // `bytes=START-END`
        (false, false) => match (s.parse::<u64>(), e.parse::<u64>()) {
            (Ok(start), Ok(end)) => {
                if start > end || start >= len || len == 0 {
                    return RangeSpec::Unsatisfiable;
                }
                let end = end.min(len - 1);
                RangeSpec::Range { start, end }
            }
            _ => RangeSpec::Unsatisfiable,
        },
        // `bytes=-` — invalid
        (true, true) => RangeSpec::Unsatisfiable,
    }
}

/// Format a `Content-Range` header value for a `(start, end, total)` range.
pub fn content_range(start: u64, end: u64, total: u64) -> String {
    format!("bytes {start}-{end}/{total}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEN: u64 = 1000;

    #[test]
    fn no_header_is_full() {
        assert_eq!(parse_range(None, LEN), RangeSpec::Full);
        assert_eq!(parse_range(Some(""), LEN), RangeSpec::Full);
    }

    #[test]
    fn non_bytes_header_is_full() {
        // Not a byte-range → caller serves full 200.
        assert_eq!(parse_range(Some("items=0-10"), LEN), RangeSpec::Full);
    }

    #[test]
    fn start_end() {
        assert_eq!(
            parse_range(Some("bytes=0-499"), LEN),
            RangeSpec::Range { start: 0, end: 499 }
        );
        assert_eq!(
            parse_range(Some("bytes=100-199"), LEN),
            RangeSpec::Range { start: 100, end: 199 }
        );
    }

    #[test]
    fn end_clamped_to_len() {
        // `bytes=500-2000` on a 1000-byte resource → clamps to 500-999
        assert_eq!(
            parse_range(Some("bytes=500-2000"), LEN),
            RangeSpec::Range { start: 500, end: 999 }
        );
    }

    #[test]
    fn open_end() {
        assert_eq!(
            parse_range(Some("bytes=500-"), LEN),
            RangeSpec::Range { start: 500, end: 999 }
        );
    }

    #[test]
    fn suffix() {
        assert_eq!(
            parse_range(Some("bytes=-500"), LEN),
            RangeSpec::Range { start: 500, end: 999 }
        );
        // suffix larger than resource → whole thing
        assert_eq!(
            parse_range(Some("bytes=-2000"), LEN),
            RangeSpec::Range { start: 0, end: 999 }
        );
    }

    #[test]
    fn start_at_len_is_unsatisfiable() {
        assert_eq!(parse_range(Some("bytes=1000-"), LEN), RangeSpec::Unsatisfiable);
        assert_eq!(parse_range(Some("bytes=1000-2000"), LEN), RangeSpec::Unsatisfiable);
    }

    #[test]
    fn invalid_values() {
        assert_eq!(parse_range(Some("bytes=-"), LEN), RangeSpec::Unsatisfiable);
        assert_eq!(parse_range(Some("bytes=abc-"), LEN), RangeSpec::Unsatisfiable);
        assert_eq!(parse_range(Some("bytes=10-5"), LEN), RangeSpec::Unsatisfiable);
        assert_eq!(parse_range(Some("bytes=-0"), LEN), RangeSpec::Unsatisfiable);
    }

    #[test]
    fn multi_range_is_full() {
        // Multi-range → caller serves full 200 (we don't do multipart).
        assert_eq!(parse_range(Some("bytes=0-99,200-299"), LEN), RangeSpec::Full);
    }

    #[test]
    fn empty_resource() {
        assert_eq!(parse_range(Some("bytes=0-0"), 0), RangeSpec::Unsatisfiable);
        assert_eq!(parse_range(Some("bytes=-500"), 0), RangeSpec::Unsatisfiable);
        assert_eq!(parse_range(None, 0), RangeSpec::Full);
    }

    #[test]
    fn whitespace_tolerant() {
        assert_eq!(
            parse_range(Some("bytes= 0 - 499 "), LEN),
            RangeSpec::Range { start: 0, end: 499 }
        );
    }

    #[test]
    fn content_range_format() {
        assert_eq!(content_range(0, 499, 1000), "bytes 0-499/1000");
        assert_eq!(content_range(500, 999, 1000), "bytes 500-999/1000");
    }

    #[test]
    fn range_len_and_start() {
        let r = RangeSpec::Range { start: 100, end: 199 };
        assert_eq!(r.start(), 100);
        assert_eq!(r.len(1000), 100);
        assert!(!r.is_empty(1000));
        assert!(RangeSpec::Unsatisfiable.is_empty(1000));
        assert_eq!(RangeSpec::Full.len(1000), 1000);
    }
}
