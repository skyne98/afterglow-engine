# Iroh Networking Library — Deep Dive

## Overview

Status: historical research. The current multiplayer target is Lightyear +
Leafwing + Afterglow server rewind. Do not revive the old custom Iroh
`NetworkTransport` adapter as production architecture. If Iroh is still desired
later, integrate it as a Lightyear-compatible transport/link layer or separate
platform admission path.

Iroh is a modular peer-to-peer networking stack for Rust, developed by [n0, inc.](https://n0.computer).
Instead of connecting to IP addresses, you connect to peers by their **public key** (`EndpointId`).

For Afterglow, Iroh is now only future transport research. The active direction
is documented in `docs/research/network-backend-abstraction.md`.

- **GitHub**: https://github.com/n0-computer/iroh (8.5k ⭐)
- **Docs**: https://docs.rs/iroh/latest/iroh/
- **Current**: v0.98.2 (v1.0.0-rc.0 tagged May 2026)
- **License**: MIT OR Apache-2.0

## Architecture

```
  Protocol Layer (iroh-blobs, iroh-gossip, iroh-docs, custom)
  ─────────────────────────────────────────────────────────
  Connection Layer (iroh core): Endpoint → QUIC Connection → Streams/Datagrams
  ─────────────────────────────────────────────────────────
  Transport Layer: QUIC (noq) + UDP + NAT Traversal + Relays
```

### Connection Establishment

1. Each peer creates an `Endpoint` with an ed25519 `SecretKey`/`PublicKey`
2. Peer connects to its **home relay** server
3. To dial another peer, you only need their `EndpointId` (their public key)
4. Initial connection goes through the relay (forwarding encrypted QUIC packets — **relay cannot decrypt**)
5. Both sides attempt **NAT hole punching** in the background
6. If hole punching succeeds, the connection **migrates** to the direct path
7. If hole punching fails, the connection continues over the relay

### Key Protocols

- **QUIC** (via `noq` crate): Multiplexed streams, TLS 1.3, 0-RTT, connection migration
- **TLS 1.3** (via `rustls`): No CA — each `PublicKey` IS the identity
- **STUN-like**: Relays provide address discovery for hole punching
- **Optional**: mDNS (LAN), DNS (global), Mainline DHT (pkarr)

## Core Types

| Type | Purpose |
|---|---|
| `Endpoint` | Main API handle — manages connections, relays, addressing |
| `EndpointId` | 32-byte peer identifier (alias for `PublicKey`) |
| `SecretKey` / `PublicKey` | ed25519 keypair for identity |
| `Connection<State>` | QUIC connection with typed state |
| `SendStream` / `RecvStream` | QUIC stream halves |
| `Router` | ALPN-based protocol router (like HTTP mux) |
| `ProtocolHandler` | Trait for custom protocol handlers |
| `RelayUrl` | Relay server URL |
| `RelayMode` | `Default` (n0 relays), `Custom(RelayMap)`, `Disabled` |
| `AddressLookup` | Resolves `EndpointId` → `EndpointAddr` |

## Connection & Stream Model

- **Bidirectional streams** (`open_bi`/`accept_bi`): Two-way, reliable, ordered
- **Unidirectional streams** (`open_uni`/`accept_uni`): One-way, reliable, ordered
- **Datagrams** (`send_datagram`/`read_datagram`): Unreliable, unordered, ~1KB max

Streams are **lazily created** — the peer isn't notified until data is written. They have no head-of-line blocking (QUIC property).

## Addressing & Discovery

Multiple discovery mechanisms can be combined:

| Service | Scope | Mechanism |
|---|---|---|
| `DnsAddressLookup` | Global | DNS at dns.iroh.link |
| `PkarrPublisher/Resolver` | Global | HTTP pkarr relay servers |
| `DhtAddressLookup` | Global | Mainline DHT |
| `MdnsAddressLookup` | LAN | Multicast DNS |
| `MemoryLookup` | In-process | Manual (testing) |

**Tickets**: `EndpointTicket` wraps an address into a compact serializable string for out-of-band sharing.

## Performance & Suitability for Games

### Strengths
- **NAT traversal is automatic** — players never need port forwarding
- **Built-in encryption** — free TLS 1.3 on all traffic
- **Stream multiplexing** — no head-of-line blocking
- **Datagram support** — unreliable messages within the same encrypted connection
- **Connection migration** — survives network switches (WiFi → cellular)
- **Composable protocols** — gossip for chat/lobby, streams for game state

### Challenges
- **QUIC overhead** — more CPU than raw UDP (negligible for most games)
- **Relay dependency** — worst-case path adds latency (deploy your own relays)
- **QUIC-only** — no TCP/WebSocket fallback, no browser support
- **Pre-1.0** — API churn possible
- **Tokio bound** — iroh itself requires tokio (underlying `noq` supports smol)

### Recommended Bevy Integration

```
Single `Endpoint` stored as a Bevy Resource
└─ Async accept loop via bevy_tasks / tokio::spawn
   └─ Per-connection channels → Bevy systems read events each frame
      ├─ Bidirectional streams for reliable game state
      └─ Datagrams for high-frequency position/input updates
```

The adapter should map Iroh public keys/connections to engine `PeerId`s and use
`PlatformIdentity::Iroh` for authenticated external identity. It should not own
`NetworkPlayerId`, replicated components/resources, command validation,
rollback, prediction, interpolation, chunk interest, or reconnect baselines.

## Legacy Afterglow Implementation

The old backend lives in `crates/afterglow-engine/src/network/iroh.rs` behind the
optional native `iroh` feature. This implementation is slated for deletion during
the Lightyear rewrite.

- `IrohTransport` implements the existing synchronous `NetworkTransport` trait.
- A background Tokio worker owns the async Iroh `Endpoint` and `Connection`
  objects.
- `IrohTransport::bind()` creates an endpoint and exposes its `EndpointAddr`
  for invite codes, dev connects, or future lobby metadata.
- `IrohTransport::connect(peer, addr)` dials another endpoint and maps that
  connection to an engine `PeerId`.
- Inbound connections allocate session-local `PeerId`s starting at
  `IrohTransportConfig::next_inbound_peer`.
- Reliable engine packets are sent over one-shot QUIC unidirectional streams.
- Unreliable and unreliable-sequenced engine packets are sent over QUIC
  datagrams; stale sequenced rejection uses the same engine filter as the
  memory transport.
- Local tests cover reliable packet delivery, reliable ordering, unreliable
  packet delivery, stale unreliable-sequenced rejection, remote disconnect
  reporting, disconnected-peer packet rejection, reconnect sequence-state
  reset, and the shared `service_control_handshake()` path over real local Iroh
  endpoints.
- Regression tests also run gameplay-gating cases over Iroh: unauthorized
  packets are dropped before handshake acceptance, protocol mismatches reject
  peers without session entries, bad post-handshake packet headers evict the
  peer, disconnects remove session state, two clients can concurrently feed
  command streams to one server, and snapshot packets can drive the same client
  reconciliation replay path used by memory transport tests.

This is intentionally not a lobby, account, player, rollback, or replication
system. In the new plan it should be replaced by Lightyear transport support or
future Lightyear-compatible Iroh work.

## Async Runtime

- `iroh` requires **tokio** (v1.44+)
- `noq` (the QUIC impl) supports both tokio and smol via the `Runtime` trait
- WASM support exists but with limited features

## References

- Docs: https://docs.rs/iroh/latest/iroh/
- GitHub: https://github.com/n0-computer/iroh
- Website: https://iroh.computer/
