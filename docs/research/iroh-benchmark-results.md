# iroh Network Benchmark — Laptop ↔ Workstation

## Setup

- **Laptop**: Fedora 43, Ryzen 7 6800U, connected via Tailscale
- **Workstation**: NixOS, Ryzen 9 9950X3D, connected via Tailscale
- **Transport**: iroh 0.98.2 over QUIC, presets::N0DisableRelay (direct connections only)
- **Connection**: Tailscale wireguard (100.x.x.x IPs), same LAN subnet (192.168.1.x)

## Results

### Connection Time
| Metric | Value |
|---|---|
| Connection establishment | **3.86 ms** |

### Round-Trip Time (QUIC stream echo)

| Payload Size | Avg | Min | P50 | P99 |
|---|---|---|---|---|
| 16 B | 2.74 ms | 1.85 ms | 2.13 ms | 4.38 ms |
| 64 B | 2.23 ms | 1.84 ms | 2.03 ms | 4.05 ms |
| 256 B | 2.23 ms | 1.77 ms | 2.01 ms | 4.02 ms |
| 1024 B | 2.00 ms | 1.78 ms | 1.99 ms | 2.30 ms |
| 4096 B | 2.00 ms | 1.54 ms | 1.99 ms | 2.58 ms |

### Throughput

| Mode | Payload | Count | Total | Time | Rate |
|---|---|---|---|---|---|
| Bidirectional (echo) | 1 MB | 1 | 1 MB | 0.04 s | **23 MB/s** |
| **Unidirectional (fire-and-forget)** | **1 MB** | **100** | **104.9 MB** | **1.61 s** | **65 MB/s (523 Mbps)** |

The unidirectional test is the realistic max: the sender fire-and-forgets messages without waiting for confirmation. 65 MB/s is close to saturating a 1 Gbps link minus ~300 Mbps for Tailscale wireguard + QUIC overhead.

## Analysis

- **RTT ~2ms** over Tailscale is excellent — ~1ms above raw ping (wireguard encryption).
- **65 MB/s unidirectional** — close to gigabit LAN ceiling with QUIC + wireguard overhead.
- Connection establishment in <4ms confirms 0-RTT QUIC handshake is working.
- No relay was needed — Tailscale provides direct connectivity.
- For game networking (typically <1KB messages at 60Hz), latency is the constraint, not bandwidth.
  At 2ms RTT and ~1KB per update, iroh can handle **~65,000 messages/second** unidirectional.

## References

- iroh: https://github.com/n0-computer/iroh
- Benchmark code: `/tmp/iroh-bench/`
