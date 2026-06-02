# World API

## Status

Afterglow does not currently expose a `world` module or an
`AfterglowWorldPlugin`. World streaming, cell manifests, chunk lifecycle, and
chunk persistence are planned systems, not current engine API.

Current world-adjacent public API lives in `core`:

| Item | Purpose |
|---|---|
| `StableEntityId` | Durable entity identity for persistence, replication, and cross-peer references. |
| `StableIdAllocator` | Allocates stable IDs while avoiding authored/reserved IDs. |
| `RuntimeOnly` | Marker for entities excluded from automatic stable ID assignment. |

## Planned Surface

The planned world layer will add cell manifests, load requests, chunk lifecycle
state, persistence deltas, and chunk-interest integration. These names are not
public runtime API yet and should not be imported from engine code until the
modules exist.

Chunk membership remains the data source future network interest filtering should
use. The old peer-interest API has been removed; no replacement chunk-interest
network API is currently exposed.
