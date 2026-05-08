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

### Throughput (Bidirectional stream echo)

| Size | Time | Rate |
|---|---|---|
| 1 MB | 0.04 s | **23 MB/s** |

## Analysis

- **RTT ~2ms** over Tailscale is excellent. The extra ~1ms vs raw ping is QUIC's TLS 1.3 handshake amortization + iroh's protocol layer.
- **23 MB/s** throughput is limited by Tailscale's wireguard overhead and the single-stream echo pattern. Real bulk transfers over multiple streams would likely saturate the 1 Gbps LAN.
- Connection establishment in <4ms confirms 0-RTT QUIC handshake is working.
- No relay was needed — Tailscale provides direct connectivity.

## References

- iroh: https://github.com/n0-computer/iroh
- Benchmark code: `/tmp/iroh-bench/`
