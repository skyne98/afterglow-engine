import { describe, expect, test } from 'bun:test';
import {
  BULK_RANGE_CAPACITY,
  estimatedBulkResponseBytes,
  parseMultipartByteRanges,
} from './bulk-range.ts';

const encoder = new TextEncoder();

function multipart(boundary: string): Uint8Array {
  return encoder.encode([
    `--${boundary}\r\n`,
    'Content-Type: application/octet-stream\r\n',
    'Content-Range: bytes 2-4/16\r\n\r\n',
    'cde\r\n',
    `--${boundary}\r\n`,
    'Content-Type: application/octet-stream\r\n',
    'Content-Range: bytes 10-13/16\r\n\r\n',
    'klmn\r\n',
    `--${boundary}--\r\n`,
  ].join(''));
}

describe('bulk byte-range transport', () => {
  test('parses ordered binary parts by Content-Range length', () => {
    const body = multipart('test-boundary');
    const parts = parseMultipartByteRanges(
      body,
      'multipart/byteranges; boundary="test-boundary"',
      [{ offset: 2, length: 3 }, { offset: 10, length: 4 }],
    );
    expect(parts.map(part => new TextDecoder().decode(part))).toEqual(['cde', 'klmn']);
  });

  test('rejects response order drift and overlap', () => {
    const body = multipart('test-boundary');
    expect(() => parseMultipartByteRanges(
      body,
      'multipart/byteranges; boundary=test-boundary',
      [{ offset: 10, length: 4 }, { offset: 2, length: 3 }],
    )).toThrow('does not match');
    expect(() => parseMultipartByteRanges(
      body,
      'multipart/byteranges; boundary=test-boundary',
      [{ offset: 2, length: 3 }, { offset: 4, length: 2 }],
    )).toThrow('must not overlap');
  });

  test('accounts for envelope bytes and enforces fixed range capacity', () => {
    expect(estimatedBulkResponseBytes([{ offset: 0, length: 1024 }])).toBeGreaterThan(1024);
    const tooMany = Array.from({ length: BULK_RANGE_CAPACITY + 1 }, (_, index) => ({
      offset: index * 2,
      length: 1,
    }));
    expect(() => parseMultipartByteRanges(new Uint8Array(), '', tooMany)).toThrow('count');
  });
});
