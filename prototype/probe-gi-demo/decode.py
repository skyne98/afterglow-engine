#!/usr/bin/env python3
"""Decode capture-base64 markers from a demo run log into PNGs.

    python3 prototype/probe-gi-demo/decode.py prototype/probe-gi-demo/last-run.log

Reads every `capture-base64-begin <index> <w> <h> ... capture-base64-end`
block and writes `frame-<index>.png` next to the log.
"""
import base64
import re
import struct
import sys
import zlib
from pathlib import Path

HDR = b'\x89PNG\r\n\x1a\n'


def png_bytes(width, height, rgb):
    def chunk(tag, data):
        return (struct.pack('>I', len(data)) + tag + data +
                struct.pack('>I', zlib.crc32(tag + data) & 0xffffffff))
    rows = bytearray()
    stride = width * 3
    for y in range(height):
        rows += b'\x00' + rgb[y * stride:(y + 1) * stride]
    ihdr = struct.pack('>IIBBBBB', width, height, 8, 2, 0, 0, 0)
    return (HDR + chunk(b'IHDR', ihdr) + chunk(b'IDAT', zlib.compress(bytes(rows), 6)) +
            chunk(b'IEND', b''))


def main(log_path):
    log = Path(log_path)
    text = log.read_text(encoding='utf8', errors='ignore')
    # Chunked capture format: `capture-begin <i> <w> <h>`, then any number of
    # `capture-chunk <i> <payload>` lines (possibly interleaved with other log output),
    # then `capture-end <i>`. Grouping by index means interleaving cannot corrupt payload.
    headers = {int(m.group(1)): (int(m.group(2)), int(m.group(3)))
               for m in re.finditer(r'capture-begin (\d+) (\d+) (\d+)', text)}
    chunks: dict[int, list[str]] = {}
    for m in re.finditer(r'^\[probe-gi\] capture-chunk (\d+) ([A-Za-z0-9+/=]+)$', text, re.M):
        chunks.setdefault(int(m.group(1)), []).append(m.group(2))
    found = 0
    for index in sorted(headers):
        width, height = headers[index]
        b64 = ''.join(chunks.get(index, []))
        b64 += '=' * (-len(b64) % 4)
        raw = base64.b64decode(b64)
        if len(raw) != width * height * 4:
            print(f'view {index}: BAD SIZE {len(raw)} != {width * height * 4}')
            continue
        rgb = bytearray(width * height * 3)
        for i in range(width * height):
            rgb[i * 3:i * 3 + 3] = raw[i * 4:i * 4 + 3]
        out = log.parent / f'frame-{index}.png'
        out.write_bytes(png_bytes(width, height, rgb))
        mean = sum(rgb) / len(rgb)
        print(f'view {index}: {width}x{height} mean luma {mean:.1f} -> {out.name}')
        found += 1
    if not found:
        print(f'no captures found in {log_path}')
    return 0 if found else 1


if __name__ == '__main__':
    sys.exit(main(sys.argv[1] if len(sys.argv) > 1 else 'prototype/probe-gi-demo/last-run.log'))
