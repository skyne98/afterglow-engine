# Source Engine Leaked Source Code References

## Official SDK (always available)

```
/tmp/source-sdk-2013/
```

Clone: `git clone --depth 1 https://github.com/ValveSoftware/source-sdk-2013`

Only the game-layer DLL code — no engine layer. Key files:

| File | Purpose |
|------|---------|
| `src/game/shared/gamemovement.cpp` | `CGameMovement` — shared player motor (client + server) |
| `src/game/server/player.cpp` | `CBasePlayer` — server entity |
| `src/game/client/prediction.cpp` | `CPrediction` — client prediction stub |
| `src/game/client/in_main.cpp` | Input → usercmd |
| `src/game/server/client.cpp` | `CClient` — connection management |

## CS:GO 2015 Leak (2018)

```
/tmp/cstrike15_src/
```

Clone: `git clone --depth 1 https://github.com/perilouswithadollarsign/cstrike15_src`

Full engine layer. Origin: CS:GO engine depot leaked 2018, reposted 2020. Contains the core networking architecture missing from the official SDK.

| File | Purpose |
|------|---------|
| `engine/net_chan.cpp` | `CNetChan` — packet framing, send/receive, split packets, encryption, channel state machine |
| `engine/cl_main.cpp` | Client frame loop: `CL_Move`, `CL_SendMove`, input prediction dispatch |
| `engine/cl_pred.cpp` | `CPrediction` — full client prediction implementation |
| `engine/sv_main.cpp` | Server frame loop: `SV_Frame`, `SV_Physics`, `SV_ProcessClientCommands` |
| `engine/sv_client.cpp` | Per-client command handling, `SV_WriteSendTable` |
| `engine/net_ws.cpp` | Winsock UDP socket layer: `NET_SendPacket`, `NET_ReceiveDatagram` |
| `engine/sv_ents_write.cpp` | Entity delta encoding: `SV_WriteEntitiesToClient` |
| `engine/sv_framesnapshot.cpp` | Snapshot creation: `SV_SendClientSnapshot` |
| `engine/cl_ents_parse.cpp` | Client-side entity delta decode |
| `engine/cl_broadcast.cpp` | Broadcast receiver (TV/demo) |
| `engine/baseclient.cpp` | `CBaseClient` — shared client interface |
| `engine/baseserver.cpp` | `CBaseServer` — shared server interface |
| `engine/baseclientstate.cpp` | `CBaseClientState` — shared connection state |
| `common/netmessages.h` | Network message definitions (`NET_TICK`, `SVC_PacketEntities`, etc.) |
| `public/networkstringtabledef.h` | String table system (used for model/sound/SV names) |
| `game/shared/gamemovement.cpp` | Same shared movement motor as the SDK |
| `game/shared/prediction.cpp` | Prediction copy/unpack helpers |

## 2007 Source Engine Leak (DMCA'd — no longer accessible)

Was at `github.com/VSES/SourceEngine2007`. Removed by DMCA Jan 2025. Contained the classic Orange-box-era engine with standalone `engine/cl_move.cpp` and `engine/sv_send.cpp`. The CS:GO leak above covers the same code but reorganized (cl_move logic folded into cl_main.cpp, sv_send into sv_main.cpp).

## Earlier Leaks

- **2003** — Half-Life 2 / Source Engine leaked by Axel "Ago" Gembe before HL2 shipped
- **2007** — Full 2007 branch leaked from a licensed studio (the most complete early engine leak)
- **2018** — CS:GO 2017 engine depot leaked (origin of the cstrike15_src repo)
- **2020** — TF2 2018 branch leaked

## Key Architectural Patterns Visible in the Leaked Code

These confirmed Source Engine architecture details are all visible in `/tmp/cstrike15_src/`:

1. **Client frame (`CL_Move` in cl_main.cpp):** reads input → fills `CUserCmd` → runs `CPrediction::RunPrediction` → sends cmd via `CL_SendMove` → receives snapshot → `CL_PostClientUpdate` → prediction reconciliation
2. **Server frame (`SV_Frame` in sv_main.cpp):** processes incoming cmds → `SV_ProcessClientCommands` → `SV_Physics` (runs `CGameMovement`) → `SV_SendClientSnapshot` → delta-compress entities → write to net channel
3. **Net channel (`CNetChan` in net_chan.cpp):** reliable/unreliable streams, split packet reassembly, encryption, rate limiting, channel state (DISCONNECTED->CHALLENGE->CONNECTED)
4. **Delta compression (`SV_WriteEntitiesToClient` + `CL_ParseEntity`):** per-entity `SendTable`/`RecvTable`, baseline + delta encoding, PVS culling
5. **Lag compensation:** server backtracks hitboxes in `sv_main.cpp`/game code using stored history
