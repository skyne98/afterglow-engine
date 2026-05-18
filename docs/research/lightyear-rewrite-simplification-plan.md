# Lightyear Rewrite Simplification Plan

## Goal

Fully remove the previous custom networking/input stack and replace it with
Lightyear, Leafwing Input Manager, `lightyear_inputs_leafwing`, and the custom
Afterglow server rewind layer described in
`server-rewind-component-history-plan.md`.

The desired game-facing result is normal Bevy gameplay code with declarative
network/rewind registration, not hand-written transport/session/packet plumbing.
The detailed step-by-step migration plan lives in
`docs/research/lightyear-migration-plan.md`.

## Code Scan Notes

The requested `opencode run -m opencode-go/deepseek-v4-flash` subagent failed in
this environment with `Session not found` even for a one-word prompt. I retried
with a minimal invocation and with opencode session environment variables unset;
the failure persisted. The estimates below combine a read-only Task subagent scan
with local `rg --files | wc -l` counts.

## Existing LOC

| Area | LOC | Notes |
|---|---:|---|
| `crates/afterglow-engine/src/network` | 10,982 | Custom transport, handshake, session, replication, prediction, interpolation, interest, rollback, Iroh, Steam, tests |
| `crates/afterglow-engine/src/input` | 1,842 | Custom command/binding/evaluation system and tests |
| Networking benches except persistence | 749 | replication, authority, prediction, reconciliation, interpolation, baseline, ggrs |
| `crates/afterglow-engine-macros` | 79 | Old `Replicate` derive/attribute support |
| `crates/mock-rpg-network-tests` | 2,852 | Integration harness built around custom network primitives |
| **Total touched old code** | **16,504** | Includes tests and benches |

## Delete Or Gut

| Path | Action | Replacement |
|---|---|---|
| `src/network/commands` | Delete | Leafwing action state plus Lightyear input messages |
| `src/network/authority` | Delete | Lightyear server input flow plus Afterglow gameplay validation |
| `src/network/session` | Delete | Lightyear client/server/link state plus stable avatar mapping |
| `src/network/handshake` | Delete | Lightyear protocol/config and optional platform admission |
| `src/network/iroh` | Delete | Future Lightyear-compatible IO only if Iroh is still required |
| `src/network/steam` | Delete | Prefer Lightyear Steam support plus separate lobby/auth layer after core rewrite |
| `src/network/replication` | Delete | Lightyear component replication |
| `src/network/prediction` | Delete | Lightyear prediction |
| `src/network/reconciliation` | Delete | Lightyear reconciliation/correction flow |
| `src/network/interpolation` | Delete | Lightyear interpolation |
| `src/network/interest` | Delete | Lightyear replication filtering or later tiny adapter |
| `src/network/baseline` | Delete | Lightyear connect/replication plus Afterglow persistence |
| `src/network/local_server` | Delete | Lightyear host/client/server setup |
| `src/network/rollback` | Replace | New typed `network::rewind`; salvage only concepts/tests worth porting |
| `src/input` | Delete | Leafwing Input Manager plus thin Afterglow action enum |
| `crates/afterglow-engine-macros` | Delete | Use Lightyear registration/derive patterns directly |

## Add

| Path | Estimate | Purpose |
|---|---:|---|
| `src/network/lightyear` | 300-450 LOC | `AfterglowLightyearPlugin`, tick config, protocol registration helpers, schedule integration |
| `src/network/rewind` | 900-1,300 LOC | typed component history, checkpoints, entity lifecycle, replay, correction diffing |
| `src/input_leafwing` or rewritten `src/input` | 200-350 LOC | `AfterglowAction`, Leafwing plugin wrapper, test/script input helpers |
| rewritten mock RPG harness | 1,000-1,500 LOC | current network-boundary tests now; Lightyear client/server scenarios later |
| new benches | 300-500 LOC | `server_rewind`, component history, Lightyear integration pressure |

## Net LOC Estimate

Conservative estimate:

| Bucket | Removed | Added | Net |
|---|---:|---:|---:|
| Engine network/input/macros/benches | 13,652 | 1,700-2,600 | -11,052 to -12,052 |
| Mock RPG harness | 2,852 | 2,000-2,800 | -52 to -852 |
| **Total** | **16,504** | **3,700-5,400** | **-11,104 to -12,804** |

Round this down for unknown glue and compatibility work: expect roughly **10k to
12k net LOC removed** once the old stack is fully deleted.

## Migration Phases

1. Add `lightyear`, `lightyear_inputs_leafwing`, and `leafwing-input-manager` with
   semver-pinned versions compatible with Bevy 0.18.
2. Replace `src/input` with the Leafwing wrapper and port the controller/demo to
   `ActionState<AfterglowAction>` in fixed schedules.
3. Add `AfterglowLightyearPlugin` and register one minimal replicated component,
   one message, and one Leafwing input type.
4. Port the mock RPG smoke path to the current network boundary and delete the
   custom transport/session/handshake modules.
5. Implement `network::rewind` with typed component checkpoints/deltas and entity
   lifetime events.
6. Port the late shield/death/corpse/loot regression test to server rewind, then
   drive it through real Lightyear client/server once link setup lands.
7. Delete custom replication, prediction, reconciliation, interpolation, interest,
   baseline, old rollback, macros, old benches, and stale docs.
8. Add focused benchmarks for Lightyear integration and server rewind component
   history at 1k, 10k, and 100k entities.

## Regression Gate

The rewrite is not complete until this passes on the new stack:

```text
T100: A raises shield.
T108: B arrow appears to kill A; corpse and loot spawn provisionally.
T111: A's late-but-valid shield input arrives.
Replay: shield blocks arrow.
Correction: A lives; corpse, loot, death cue, and stale projectile hit vanish.
```

## Risks

| Risk | Mitigation |
|---|---|
| Lightyear/Bevy version mismatch | Pin Lightyear 0.26.x if staying on Bevy 0.18, or upgrade Bevy deliberately |
| Leafwing entity-scoped input only | Put networked controls on avatar/control entities; keep UI/global input separate |
| Full server rewind complexity | Rewind only authoritative gameplay domains, not the open world or presentation |
| Lost old transport features | Use Lightyear built-in transports first; reintroduce custom transport only after a proven gap |
| Big-bang compile breakage | Migrate in feature-gated phases, but delete legacy paths once replacement tests pass |
