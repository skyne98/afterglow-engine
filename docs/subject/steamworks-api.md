# Steamworks API Reference — ISteamMatchmaking & steamworks-rs

**Status:** Authoritative reference for Steam lobby/matchmaking APIs that an
external launcher/platform layer may use before handing connection params to the
engine.

**Sources:**
- C++ SDK: https://partner.steamgames.com/doc/api/ISteamMatchmaking
- Rust crate: https://docs.rs/steamworks/latest/steamworks/struct.Matchmaking.html

## Crate

`steamworks` (https://crates.io/crates/steamworks) wraps the Steamworks SDK.
Feature `serde` for serializable types. Requires the Steam SDK native lib
(`steam_api64.so` / `.dll`) at link time; the crate bundles it or you provide
`STEAM_SDK_LOCATION`.

## Identity

| Type | Steam SDK | steamworks-rs | Notes |
|---|---|---|---|
| `CSteamID` / `SteamId` | u64 `CSteamID` | `steamworks::SteamId` (wraps u64) | Stable, unique, authenticated per-player. **This IS the netcode `client_id`.** |
| Local player | `SteamUser()->GetSteamID()` | `Client::user().steam_id()` | Returns the local user's SteamId. |

No separate identity proofing needed — Steam authenticates the player.
For NonSteam dev, we allocate a random/hashed u64; no Ed25519 keypairs.

## Lobby (= Session)

| Concept | Steam SDK | steamworks-rs | Notes |
|---|---|---|---|
| Lobby ID | `CSteamID` (lobby SteamID) | `steamworks::LobbyId` (newtype u64) | Opaque session token. **Maps to `SessionId`.** |
| Owner | `GetLobbyOwner(lobby)` → `CSteamID` | `Matchmaking::lobby_owner(lobby)` → `SteamId` | Transfers automatically when owner leaves. |
| Member limit | `GetLobbyMemberLimit(lobby)` → `int` | `Matchmaking::lobby_member_limit(lobby)` → `Option<usize>` | Max 250. Set at creation, changeable by owner. |
| Member count | `GetNumLobbyMembers(lobby)` → `int` | `Matchmaking::lobby_member_count(lobby)` → `usize` | |
| Members | `GetLobbyMemberByIndex` loop | `Matchmaking::lobby_members(lobby)` → `Vec<SteamId>` | |
| Lobby type | `ELobbyType` enum | `steamworks::LobbyType` enum | Private=0, FriendsOnly=1, Public=2, Invisible=3. |
| Joinable | `SetLobbyJoinable(lobby, bool)` | `Matchmaking::set_lobby_joinable(lobby, bool)` → `bool` | Owner-only. Default true. |

## Lobby Metadata (key/value string pairs)

| Operation | Steam SDK | steamworks-rs | Notes |
|---|---|---|---|
| Set lobby data | `SetLobbyData(lobby, key, value)` → `bool` | `Matchmaking::set_lobby_data(lobby, key, value)` → `bool` | **Owner-only.** Max key len 255, max value len 4096. Batched. |
| Get lobby data | `GetLobbyData(lobby, key)` → `const char*` | `Matchmaking::lobby_data(lobby, key)` → `Option<String>` | |
| Delete lobby data | `DeleteLobbyData(lobby, key)` → `bool` | `Matchmaking::delete_lobby_data(lobby, key)` → `bool` | Owner-only. |
| Data count | `GetLobbyDataCount(lobby)` → `int` | `Matchmaking::lobby_data_count(lobby)` → `u32` | |
| Data by index | `GetLobbyDataByIndex(lobby, i, key, val)` → `bool` | `Matchmaking::lobby_data_by_index(lobby, idx)` → `Option<(String, String)>` | |

## Per-Member Metadata

| Operation | Steam SDK | steamworks-rs | Notes |
|---|---|---|---|
| Set member data | `SetLobbyMemberData(lobby, key, value)` | `Matchmaking::set_lobby_member_data(lobby, key, value)` | Each member sets their own. |
| Get member data | `GetLobbyMemberData(lobby, user, key)` → `const char*` | `Matchmaking::get_lobby_member_data(lobby, user, key)` → `Option<String>` | |

## Lobby Lifecycle Operations

| Operation | Steam SDK | steamworks-rs | Notes |
|---|---|---|---|
| Create | `CreateLobby(ELobbyType, max_members)` → `SteamAPICall_t` | `Matchmaking::create_lobby(ty, max, cb)` | Async callback. Creator joins automatically. |
| Join | `JoinLobby(lobby)` → `SteamAPICall_t` | `Matchmaking::join_lobby(lobby, cb)` | Async callback. |
| Leave | `LeaveLobby(lobby)` | `Matchmaking::leave_lobby(lobby)` | Immediate on client side. |
| Invite | `InviteUserToLobby(lobby, invitee)` → `bool` | *(not in crate yet)* | Sends invite via Steam UI. |

## Lobby Search

| Operation | Steam SDK | steamworks-rs | Notes |
|---|---|---|---|
| Request list | `RequestLobbyList()` → `SteamAPICall_t` | `Matchmaking::request_lobby_list(cb)` | Async. Only one active search at a time. |
| Get by index | `GetLobbyByIndex(i)` → `CSteamID` | *(handled in callback result)* | |
| String filter | `AddRequestLobbyListStringFilter(key, value, cmp)` | `Matchmaking::add_request_lobby_list_string_filter(StringFilter)` | |
| Numerical filter | `AddRequestLobbyListNumericalFilter(key, value, cmp)` | `Matchmaking::add_request_lobby_list_numerical_filter(NumberFilter)` | |
| Near value filter | `AddRequestLobbyListNearValueFilter(key, value)` | `Matchmaking::add_request_lobby_list_near_value_filter(NearFilter)` | Sorts by closeness, doesn't filter. |
| Slots filter | `AddRequestLobbyListFilterSlotsAvailable(n)` | `Matchmaking::set_request_lobby_list_slots_available_filter(slots)` | |
| Distance filter | `AddRequestLobbyListDistanceFilter(ELobbyDistanceFilter)` | `Matchmaking::set_request_lobby_list_distance_filter(distance)` | Close=0, Default=1, Far=2, Worldwide=3. |
| Result count | `AddRequestLobbyListResultCountFilter(n)` | `Matchmaking::set_request_lobby_list_result_count_filter(count)` | |
| Combined filter | — | `Matchmaking::set_lobby_list_filter(LobbyListFilter)` | Struct combining all filter types. |

## Game Server Handoff (lobby → dedicated server)

| Operation | Steam SDK | steamworks-rs | Notes |
|---|---|---|---|
| Set game server | `SetLobbyGameServer(lobby, ip, port, steam_id)` | `Matchmaking::set_lobby_game_server(lobby, SocketAddrV4, Option<SteamId>)` | **Owner-only.** Triggers `LobbyGameCreated_t` to all members. |
| Get game server | `GetLobbyGameServer(lobby, &ip, &port, &steam_id)` → `bool` | `Matchmaking::get_lobby_game_server(lobby)` → `Option<(SocketAddrV4, Option<SteamId>)>` | |

**This is the connection-params delivery mechanism.** The lobby owner sets the
game server address; all members receive `LobbyGameCreated_t` and connect.
For listen-server (host = lobby owner), the owner sets their own address.

## Lobby Chat (binary messages, up to 4KB)

| Operation | Steam SDK | steamworks-rs | Notes |
|---|---|---|---|
| Send | `SendLobbyChatMsg(lobby, data, len)` → `bool` | `Matchmaking::send_lobby_chat_message(lobby, msg)` → `Result<(), SteamError>` | Broadcasts to all members. Low bandwidth. |
| Get entry | `GetLobbyChatEntry(lobby, chat_id, ...)` → `int` | `Matchmaking::get_lobby_chat_entry(lobby, chat_id, buf)` → `&[u8]` | After `LobbyChatMsg_t` callback. |

## Callbacks (as Bevy events in steamworks-rs)

| steamworks-rs event | Steam callback | When | Fields |
|---|---|---|---|
| `LobbyCreated` | `LobbyCreated_t` | Create succeeded | `result: SResult`, `lobby_id: LobbyId` |
| `LobbyEnter` | `LobbyEnter_t` | Joined a lobby | `lobby_id: LobbyId`, `permissions: u32`, `locked: bool`, `response: u32` |
| `LobbyChatUpdate` | `LobbyChatUpdate_t` | Member joined/left/kicked | `lobby_id`, `user_changed: SteamId`, `making_change: SteamId`, `state_change: ChatMemberStateChange` |
| `LobbyDataUpdate` | `LobbyDataUpdate_t` | Lobby/member metadata changed | `lobby_id`, `member: SteamId`, `success: bool` |
| `LobbyChatMsg` | `LobbyChatMsg_t` | Chat message received | `lobby_id`, `user: SteamId`, `entry_type`, `chat_id: i32` |
| `GameLobbyJoinRequested` | `GameLobbyJoinRequested_t` | Invite accepted (in-game) | `lobby_id: LobbyId`, `friend: SteamId` |

### `ChatMemberStateChange` flags (bitfield)
| Value | Meaning |
|---|---|
| `k_EChatMemberStateChangeEntered` (0x0001) | Joined |
| `k_EChatMemberStateChangeLeft` (0x0002) | Left |
| `k_EChatMemberStateChangeDisconnected` (0x0004) | Disconnected |
| `k_EChatMemberStateChangeKicked` (0x0008) | Kicked |
| `k_EChatMemberStateChangeBanned` (0x0010) | Banned |

## Constants

| Constant | Value | Notes |
|---|---|---|
| `k_nMaxLobbyKeyLength` | 255 | Max metadata key length. |
| `k_cubChatMetadataMax` | 4096 | Max metadata value length. |
| Max lobby members | 250 | Hard limit. |

## Integration with Bevy

The `steamworks` crate provides a `Client` that must be polled
(`Client::run_callbacks()` or via the `SteamManager` Bevy plugin). The
crate's callbacks fire as Bevy events if you use
`steamworks::bevy::SteamworksPlugin`.

Typical Bevy integration:
```rust
app.add_plugins(SteamworksPlugin); // polls Steam callbacks, emits events
app.add_event::<LobbyEnter>();
app.add_event::<LobbyChatUpdate>();
// ...
```

## Mapping to Afterglow

Afterglow no longer owns a `SessionProvider` trait. External Steam tooling should
map `SteamId` to `PlayerId` and pass resolved connection parameters into
`AfterglowConnectionPlugin`.
