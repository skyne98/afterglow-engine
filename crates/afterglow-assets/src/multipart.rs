//! Bounded HTTP multi-range parsing and streaming multipart responses.
//!
//! This is a policy-free serving primitive shared by CEF and the development
//! HTTP server. It never buffers source payload bytes: the multipart envelope
//! is bounded metadata and each body segment delegates to `AssetSource::read_at`.

use crate::source::AssetSource;
use std::io;

/// Maximum number of explicit byte ranges accepted in one bulk request.
pub const MAX_BULK_RANGES: usize = 256;
/// Maximum complete multipart response, including MIME envelope bytes.
pub const MAX_BULK_RESPONSE_BYTES: u64 = 4 * 1024 * 1024;
/// Stable boundary used by authored Afterglow serving backends.
pub const MULTIPART_BOUNDARY: &str = "afterglow_bulk_v1";
/// Response MIME for [`MultipartSource`].
pub const MULTIPART_CONTENT_TYPE: &str = "multipart/byteranges; boundary=afterglow_bulk_v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ByteRange {
    pub start: u64,
    pub end: u64,
}

impl ByteRange {
    pub fn len(self) -> u64 {
        self.end - self.start + 1
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum MultiRangeSpec {
    /// No comma was present; use the ordinary single-range parser.
    NotMultiple,
    /// Two or more valid, explicit byte ranges.
    Ranges(Vec<ByteRange>),
    /// Malformed, out of bounds, overlapping, or over capacity.
    Unsatisfiable,
}

/// Parse an explicit HTTP multi-range request.
///
/// Only `bytes=START-END,...` is accepted. Open and suffix ranges remain part
/// of the single-range API; the authored bulk client always knows exact spans.
pub fn parse_multi_range(header: Option<&str>, total: u64) -> MultiRangeSpec {
    let Some(header) = header else {
        return MultiRangeSpec::NotMultiple;
    };
    if !header.contains(',') {
        return MultiRangeSpec::NotMultiple;
    }
    let Some(body) = header.trim().strip_prefix("bytes=") else {
        return MultiRangeSpec::Unsatisfiable;
    };
    let mut ranges = Vec::with_capacity(MAX_BULK_RANGES.min(body.matches(',').count() + 1));
    for item in body.split(',') {
        if ranges.len() == MAX_BULK_RANGES {
            return MultiRangeSpec::Unsatisfiable;
        }
        let Some((start, end)) = item.trim().split_once('-') else {
            return MultiRangeSpec::Unsatisfiable;
        };
        let (Ok(start), Ok(end)) = (start.trim().parse::<u64>(), end.trim().parse::<u64>()) else {
            return MultiRangeSpec::Unsatisfiable;
        };
        if start > end || end >= total {
            return MultiRangeSpec::Unsatisfiable;
        }
        if ranges
            .iter()
            .any(|existing: &ByteRange| start <= existing.end && end >= existing.start)
        {
            return MultiRangeSpec::Unsatisfiable;
        }
        ranges.push(ByteRange { start, end });
    }
    if ranges.len() < 2 {
        MultiRangeSpec::Unsatisfiable
    } else {
        MultiRangeSpec::Ranges(ranges)
    }
}

#[derive(Debug)]
struct Part {
    response_start: u64,
    header_start: usize,
    header_len: usize,
    data_start: u64,
    source_start: u64,
    data_len: u64,
}

/// Streaming multipart response over one positional source.
///
/// Metadata allocation is bounded by [`MAX_BULK_RANGES`]. Source payload bytes
/// are never assembled into a temporary response buffer.
pub struct MultipartSource {
    source: Box<dyn AssetSource + Send + Sync>,
    headers: Vec<u8>,
    parts: Vec<Part>,
    trailer_start: u64,
    trailer: Vec<u8>,
    len: u64,
}

impl MultipartSource {
    pub fn new(
        source: Box<dyn AssetSource + Send + Sync>,
        ranges: &[ByteRange],
        mime: &str,
    ) -> io::Result<Self> {
        if ranges.len() < 2 || ranges.len() > MAX_BULK_RANGES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "multipart range count is outside capacity",
            ));
        }
        let total = source.len();
        let mut headers = Vec::with_capacity(ranges.len() * 128);
        let mut parts = Vec::with_capacity(ranges.len());
        let mut response_offset = 0u64;
        for range in ranges {
            if range.start > range.end || range.end >= total {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "multipart range exceeds source",
                ));
            }
            let header_start = headers.len();
            headers.extend_from_slice(
                format!(
                    "--{MULTIPART_BOUNDARY}\r\nContent-Type: {mime}\r\nContent-Range: bytes {}-{}/{total}\r\n\r\n",
                    range.start, range.end,
                )
                .as_bytes(),
            );
            let header_len = headers.len() - header_start;
            let data_start = response_offset + header_len as u64;
            let data_len = range.len();
            parts.push(Part {
                response_start: response_offset,
                header_start,
                header_len,
                data_start,
                source_start: range.start,
                data_len,
            });
            response_offset = data_start
                .checked_add(data_len)
                .and_then(|value| value.checked_add(2))
                .ok_or_else(|| io::Error::other("multipart response length overflow"))?;
        }
        let trailer = format!("--{MULTIPART_BOUNDARY}--\r\n").into_bytes();
        let trailer_start = response_offset;
        let len = trailer_start
            .checked_add(trailer.len() as u64)
            .ok_or_else(|| io::Error::other("multipart response length overflow"))?;
        if len > MAX_BULK_RESPONSE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "multipart response exceeds 4 MiB capacity",
            ));
        }
        Ok(Self {
            source,
            headers,
            parts,
            trailer_start,
            trailer,
            len,
        })
    }

    fn copy_metadata(source: &[u8], offset: usize, out: &mut [u8]) -> usize {
        if offset >= source.len() {
            return 0;
        }
        let count = out.len().min(source.len() - offset);
        out[..count].copy_from_slice(&source[offset..offset + count]);
        count
    }
}

impl AssetSource for MultipartSource {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, out: &mut [u8]) -> io::Result<usize> {
        if offset >= self.len || out.is_empty() {
            return Ok(0);
        }
        let mut response_offset = offset;
        let mut written = 0usize;
        while written < out.len() && response_offset < self.len {
            if response_offset >= self.trailer_start {
                let relative =
                    usize::try_from(response_offset - self.trailer_start).unwrap_or(usize::MAX);
                written += Self::copy_metadata(&self.trailer, relative, &mut out[written..]);
                break;
            }
            let Some(part) = self.parts.iter().find(|part| {
                response_offset >= part.response_start
                    && response_offset < part.data_start + part.data_len + 2
            }) else {
                break;
            };
            if response_offset < part.data_start {
                let relative =
                    usize::try_from(response_offset - part.response_start).unwrap_or(usize::MAX);
                let header = &self.headers[part.header_start..part.header_start + part.header_len];
                let count = Self::copy_metadata(header, relative, &mut out[written..]);
                written += count;
                response_offset += count as u64;
                continue;
            }
            let data_end = part.data_start + part.data_len;
            if response_offset < data_end {
                let source_offset = part.source_start + response_offset - part.data_start;
                let want = usize::try_from(data_end - response_offset)
                    .unwrap_or(usize::MAX)
                    .min(out.len() - written);
                let count = self
                    .source
                    .read_at(source_offset, &mut out[written..written + want])?;
                if count == 0 {
                    break;
                }
                written += count;
                response_offset += count as u64;
                continue;
            }
            let suffix = b"\r\n";
            let relative = usize::try_from(response_offset - data_end).unwrap_or(usize::MAX);
            let count = Self::copy_metadata(suffix, relative, &mut out[written..]);
            written += count;
            response_offset += count as u64;
        }
        Ok(written)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BytesSource;

    #[test]
    fn parses_bounded_explicit_ranges() {
        assert_eq!(parse_multi_range(None, 100), MultiRangeSpec::NotMultiple);
        assert_eq!(
            parse_multi_range(Some("bytes=0-3,10-12"), 100),
            MultiRangeSpec::Ranges(vec![
                ByteRange { start: 0, end: 3 },
                ByteRange { start: 10, end: 12 },
            ]),
        );
        assert_eq!(
            parse_multi_range(Some("bytes=0-4,4-8"), 100),
            MultiRangeSpec::Unsatisfiable,
        );
        assert_eq!(
            parse_multi_range(Some("bytes=0-4,100-101"), 100),
            MultiRangeSpec::Unsatisfiable,
        );
    }

    #[test]
    fn streams_multipart_body_without_payload_assembly() {
        let ranges = [
            ByteRange { start: 1, end: 3 },
            ByteRange { start: 7, end: 9 },
        ];
        let source = MultipartSource::new(
            Box::new(BytesSource(b"abcdefghijkl")),
            &ranges,
            "application/octet-stream",
        )
        .unwrap();
        let mut bytes = vec![0u8; source.len() as usize];
        let mut offset = 0usize;
        while offset < bytes.len() {
            let end = (offset + 7).min(bytes.len());
            let count = source
                .read_at(offset as u64, &mut bytes[offset..end])
                .unwrap();
            assert!(count > 0);
            offset += count;
        }
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("Content-Range: bytes 1-3/12\r\n\r\nbcd\r\n"));
        assert!(text.contains("Content-Range: bytes 7-9/12\r\n\r\nhij\r\n"));
        assert!(text.ends_with("--afterglow_bulk_v1--\r\n"));
    }
}
