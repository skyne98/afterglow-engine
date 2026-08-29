# Skyrim AI-NPC frameworks: CHIM, MinAI, and SkyrimNet — source audit

> Research for the Afterglow **story-engine** prototype. This audit reads the
> available mod source and documents, in detail, how each framework makes an
> NPC "work": character definition, per-turn prompt reconstruction, memory,
> world/game-state awareness, autonomous behaviour, and the action/reaction
> loop.
>
> **Checked-out source (this dataset):**
> - `abeiro/HerikaServer` @ `30d3cb3` — CHIM server (PHP bridge; the open half).
>   CHIM's SKSE plugin is distributed compiled and is **not** open-source; the
>   server source is authoritative for the AI.
> - `MinLL/MinAI` @ `ef5a7c1` (2026-02-22, build `2.1.3-dev7g`) — CHIM addon
>   (now deprecated by its author in favour of SkyrimNet)
> - `MinLL/SkyrimNet-GamePlugin` @ `4e23528` — SkyrimNet **game-plugin asset
>   repo** (Papyrus + ESP Spriggit JSON + prompts + docs). The native C++/DLL
>   source is **not** tracked (`.gitignore` excludes `*.dll`, `CppAPI/PublicAPI.h`,
>   `Scripts/`, `SkyrimNet.esp`, runtime config/data). Native internals are
>   documented here only to the depth the tracked Papyrus/prompts/docs prove.
>
> **Confidence note.** The CHIM server and MinAI are fully readable and are
> audited to code level. SkyrimNet's *contracts* (Papyrus API, ESP, Inja prompt
> corpus, decorator tests, docs) are audited; its *native DLL implementation*
> is closed and is flagged as such throughout rather than guessed at.

---

## 0. Landscape and lineage (what these are)

- **CHIM** — a large AI framework for Skyrim by **Dwemer Dynamics** (`abeiro`).
  It turns NPC interaction into a broader AI gameplay layer: dynamic dialogue,
  world awareness, lore-aware roleplay, narration, and in-game actions. It is a
  **two-process** design: an SKSE C++ plugin inside Skyrim + a PHP server
  (`HerikaServer`) the plugin talks to over HTTP, plus a Plugin Manager for
  addons. Most AI "thinking" (prompting, memory, personality, actions) lives in
  the server; the game plugin feeds telemetry and event history and executes
  returned actions.
- **MinAI** (`MinLL`) — a Papyrus+PHP **extension of CHIM** that replaces
  CHIM's assembled system prompt with a modular, perspective-aware prompt,
  exports live game/mod state into context, admits nearby ambient NPCs
  ("Sapience"), and drives autonomous NPC↔NPC conversation ("Radiance"). Its
  author now describes it as deprecated in favour of SkyrimNet.
- **SkyrimNet** (`MinLL`) — the next-generation architecture: a **single native
  Windows SKSE DLL loaded inside Skyrim** (no external server, no WSL/Python).
  It reads game state directly from memory, renders **Inja** prompts with 100+
  "decorators", keeps per-NPC vector memory, drives real Skyrim AI packages and
  actions on the game thread, and exposes an in-game PrismaUI overlay plus a
  process-local dashboard (`localhost:8080`) and MCP server. The tracked repo
  is the Papyrus/ESP/prompt contract; the DLL is closed.

Together they form the clearest real-world references for "LLM-driven NPCs in
a game": one external-server style, one in-process-native style, and an addon
that shows how prompt/context engineering is done well. All three are directly
relevant to the story-engine.

---

## 1. CHIM — the character model (what an NPC "is")

### 1.1 Canonical NPC row and identity strata

The server keeps one canonical runtime row per NPC in `core_npc_master`
(`lib/core/database_schema/core_npc_master.sql`). The identity-bearing fields:

| Field | Role |
|---|---|
| `npc_name`, `md5`, `refid`, `base` | Name, profile-selection identity, game reference, base identity |
| `core` | Compact character premise ("really quick summary") → part of `HERIKA_PERS` |
| `npc_static_bio`, `personality`, `appearance`, `occupation`, `skills`, `speechstyle`, `goals` | The detailed biography, rendered as separate prompt sections |
| `gender`, `race` | Explicit identity; visible in scene/nearby descriptions |
| `oghma_knowledge_tags` | Per-NPC lore-access tags (asymmetric knowledge) |
| `relationships` / `extended_data.relationships` | Initial prose/JSON and the evolving structured affinity map |
| `voiceid`, `emote_moods` | Presentation identity: TTS voice + emotion/mood set |
| `profile_id`, `prompt_head`, `metadata`, `extended_data`, `dynamic_profile` | Shared model/provider policy + per-NPC overrides |

**Key invariant:** there is **no persistent loaded persona**. Every request
reloads the row into request globals and reconstructs a fresh character prompt.
`NpcMaster::setOldGlobalsFromCurrentNpcData()` resets name, prompt head,
biography, personality, speech style, goals, and builds the base persona:

```php
$GLOBALS['HERIKA_PERS'] = "Roleplay as {$GLOBALS['HERIKA_NAME']}.\n{$currentNpcData['core']}";
```
(`lib/core/npc_master.class.php`)

This explicit reset prevents one NPC's identity leaking into the next
(`lib/core/npc_master.class.php` comment; also nulls leftover personality).

### 1.2 Definition sources

1. **Seeded curated templates** — `data/bio_templates_*.sql` rows supply `core`,
   background, appearance, personality, relationships, occupation, skills,
   speech, goals, and `oghma_knowledge_tags` in natural language.
2. **User templates / CSV import** — `processor/biography_import.php` upserts a
   CSV into `bio_templates_custom` (requires `npc_name` + `core`; accepts
   lore tags, voice, gender/race, refid, bio fields, relationship JSON).
3. **The live row** — at encounter/profile creation `NpcMaster::createProfile()`
   converts template material into an NPC-specific `core_npc_master` row
   (fallback template is only "Roleplay as <codename>").

**Likes/dislikes** are not first-class columns. They are (a) authored prose in
personality/biography/goals, and (b) relationship-map targets whose subject may
be a person, faction, group, or concept. The relationship manager renders
scores into phrases from "strongly values and supports" to "despises and would
act against". Relationship entries are shaped as JSON, e.g.
`{"Greta":{"aff":80,"type":"familial","relation":"wife","note":"..."}}`.

### 1.3 Profiles (policy) vs NPC identity

`core_profiles` is reusable technical/behavioural policy (TTS/ITT/diary
connectors, LLM connector + fallback, profile-level `prompt`), referenced by
`core_npc_master.profile_id`. It is **not** the biography. Load order per
request: connector settings → profile settings → NPC row last (so NPC
metadata/extended-data overrides win). `lock_profile` protects characterization
from timeline restore, while relationship state is treated as timeline data
(rolled back on save restore even for locked profiles).

### 1.4 The player is not an NPC

`core_player` is a simple key/value store (`player_name`, `appearance`,
`speech_style`, `bio`, `bio_known_by_all`, diaries, stat JSON). Player bio is
**not** injected automatically: ordinary NPCs only see it via the
`bio_known_by_all` policy (or the Narrator, who is privileged). `gamedata.php`
writes player equipment/inventory/skills/stats into `core_player` JSON keys.

---

## 2. CHIM — per-turn prompt construction

### 2.1 The character block

`buildDynamicBiography()` pulls the active NPC's live row and adds:
authored bio fields, activity status, numeric skills → Novice–Master, current
equipment/inventory with descriptions, current health/magicka/stamina condition
(qualitative: "Near full health", "Wounded", "Badly wounded", "On the brink of
collapse"), spells, faction membership, reanimation, and quest state.

### 2.2 Final XML system message

`main.php` assembles one structured system message:

```
<roleplay_instructions>    PROMPT_HEAD
<world> ...                location/hold/weather/date/time
<character>                HERIKA_PERS + dynamicBiography + <profile group> + <middle_term_memory>
<knowledge>                OGHMA_HINT
<general_instructions>     COMMAND_PROMPT
+ <available_actions_list> + <nearby_actors> + rumours
```
(`main.php` ~2413-2447)

The message list for a normal dialogue turn is:
`[ system head ] + [ world/nearby/historic contextDataFull ] + [ optional
memory user-message ] + [ final user cue/request ]`. Crucially, long-term
memory is injected as a **user-role message**, not spliced into the system
prompt:
```php
['role' => 'user', 'content' => "<memory> {$GLOBALS['HERIKA_NAME']} remembers this: [$memoryInjection] </memory>"]
```
(`main.php` 1975-1993)

### 2.3 The request "cue" (why the NPC speaks this turn)

`prompts/prompts.php` is an event→cue table. For ordinary `inputtext` the cue is:
```
$TEMPLATE_ACTION . TEMPLATE_DIALOG . MAXIMUM_WORDS
```
`processor/request.php` then picks a random cue (or a configured
`player_request`), appends `(Talking to <HERIKA_NAME>)` to the player data, and
for memory/diary/book/quest/narrator loads DB-managed prompt text. The verbatim
player line is already in the event history; the **cue** tells the model to
*produce the next line/action*.

### 2.4 Actions become schema, not (necessarily) native tools

`functions/functions.php` builds an action catalogue; `functions/json_response.php`
turns the enabled subset into: an `<available_actions_list>` text, a descriptive
JSON template (`character`, `listener`, `message`, `mood`, `action`, `target`,
`item`, optional `amount`, `lang`, `emotion`), an optional strict JSON-schema
`response_format`, and a GBNF grammar for Kobold-style providers. Modern
connectors therefore ask for a JSON object and translate its `action` rather
than using native OpenAI `tools`.

---

## 3. CHIM — context that is gathered

### 3.1 World and scene

`buildWorldPrompt()` renders `<world>` with location, hold, weather, converted
date, and time-of-day. Location/weather/time/people-present come chiefly from
the `eventlog` (via `infoloc`/`location`/`infonpc` events), **not** from
`gamedata.php` (which is only a telemetry persistence API with `type` +
`actor_name`/`actor_type`). `DataLastInfoFor('', -2, true)` builds
`<nearby_actors>` from `DataBeingsInCloseRange()`, including for each profiled
NPC a compact core summary, gender/race, appearance, equipment, activity,
faction/custom state, power assessment, and a first-contact framing when the
speaker has never directly spoken to them.

### 3.2 Conversation history

`main.php` loads `CONTEXT_HISTORY`, retrieves compacted, role-re-mapped history
with `DataLastDataExpandedFor(HERIKA_NAME, -N, sqlfilter)`, a three-stage
pipeline: `buildHistoricContext` → `compactHistoricContext` (with self-rechat
guard) → `replaceRoles`. Data is scoped by the event's `people`/`location`/
`party` (whisper/stealth restrict who sees it).

### 3.3 Live NPC state

`gamedata.php` stores equipment per slot, inventory, skills, stats
(`level`, current/max health/magicka/stamina, `scale`), known spells,
furniture/activity, transformation, and a rolling snapshot of **low-process
actors**. `buildDynamicBiography()` reads the active speaker's own state into
the prompt, so an NPC knows its own condition and loadout.

---

## 4. CHIM — memory, lore, and relationships (continuity)

### 4.1 Memory storage → retrieval

- `logMemory()` writes `(speaker, listener, message, gamets, event, ts)` to the
  `memory` table.
- `PackIntoSummary()` groups raw memory into `memory_summary` by **time bucket
  and location change**, with classifier `dialogue`, a pipe-separated
  `companions` association, and default `scope='global'`.
- Retrieval is **character-scoped**: `dataGetMemoryScopeConditionSql()` uses
  `scope='<npc>'` only when `extended_data.individual_memory_enabled` is set,
  else `scope IS NULL OR scope='global'`; search also requires the speaker to be
  in the summary's `companions` list. Full-text search ranks `native_vec`
  against generated tags; vector mode filters by scope, companions, age
  threshold, embedding distance, and FTS rank. The Narrator changes `$npc` to
  empty ("Narrator knows all").
- At turn-time, `offerMemory()` selects an **old, relevant** memory (not a
  duplicate of the recent transcript), rejects too-recent/scores-below-threshold
  memories, strips `#Tags`, prefixes age/date, and returns nothing when
  irrelevant. Only one injected memory appears as a user message.
- **Middle-term memory:** a worker selects summaries since the last timestamp,
  asks an LLM for a continuity narrative, stores it as
  `extended_data.middle_term_memory[$lastgamets]`; only the latest is appended
  to the character block (Narrator excluded).

### 4.2 Oghma — asymmetric lore knowledge

Each NPC carries `oghma_knowledge_tags`. `processor/oghma.php` (run only when
MiniMe OR OGHMA_CUSTOM enabled, **and** OGHMA_INFINIUM enabled) extracts a
current topic, builds the NPC's tag array (plus the NPC's own name as a tag),
and ranks candidate `oghma` entries (full-text topic 10x, previous/current
Oghma topic 5x, location 2x, recent keywords 1x, topic alias boost 20x). Then
the **asymmetry gate** decides access level:

- Advanced access allowed if `knowledge_class` is empty, or a positive class
  intersects the NPC tags, **unless** any `!negative-class` matches
  (`knowall` overrides to advanced).
- Otherwise the same test is applied to `knowledge_class_basic`, emitting the
  basic description or explicit "You do not know ANYTHING about <topic>".

`processor/dynamicoghma.php` lets quest progression insert/update the same gated
corpus (so per-NPC tags still decide advanced/basic/unknown). This is the
concrete mechanism by which a scholar, faction member, local, or named NPC
knows more/less than another.

### 4.3 Relationships

Scores are clamped to `[-100,100]` and mapped to
Hostile/Hateful/Resentful/Cold/Wary/Neutral/Acquaintance/Friendly/Fond/Devoted/
Bonded. The Player's persistence key is canonical `Player` (Dragonborn/name
aliases normalize to it). Relationship context is injected into `HERIKA_PERS`
via `ext/relationship_system/context_pre.php` (always includes Player; nearby
NPCs only when an entry exists). Mutation happens two ways:

- **No dedicated relationship LLM:** the dialogue model emits `#REL:Target=+/-N#`
  / `#TYPE:Target=Type#`; `RelationshipManager::parseChanges()` clamps and
  stores them, then removes the command text.
- **Dedicated `RELLLM_CONNECTOR`:** after a response, a worker independently
  re-derives relationships (speaker/listener, recent dialogue, player action,
  nearby NPCs) without blocking the reply.

A `relationships_locked` flag makes manual edits authoritative.

---

## 5. CHIM — the dialogue turn pipeline (end to end)

### 5.1 Entry and routing

The game plugin sends a base64 pipe payload (`inputtext|ts|gamets|Player: text`
+ profile hash) to `main.php` (which reads the raw query after `DATA=`, not
`$_GET['DATA']`, to avoid base64 `+` corruption). `index.php` is NOT the game
API (it only redirects to `ui/`); `stream.php` just requires `main.php`.

Flow:
1. Player-input requests write a lightweight `eventlog` row of type
   `user_input`; the long-running stream checks it to abandon old generation as
   soon as newer input exists. Non-fast requests acquire the logical `MAIN`
   semaphore (telemetry bypasses it).
2. `processor/chim_modes.php` applies modes (whisper/narrator/director/cheat/
   autochat/injection) before prompt work.
3. `processor/comm.php` dispatches high-volume non-LLM events early
   (`request`, `infonpc`, `infonpc_close`).

### 5.2 Speaker / profile / connector / voice selection

For a `profile` hash, `main.php` retrieves the NPC, resolves `profile_id`,
picks a connector slot via `LLMRandomizer`, loads connector/profile/NPC globals
in order (so NPC overrides win), and applies the NPC's TTS voice override.
Narrator requests instead use Narrator data/profile and call
`loadCharacterIntoGlobals()`.

### 5.3 Context assembly → LLM → streaming

`call_llm()` → `call_llm_internal()` opens the selected connector, streams via
`process()`, chunks on safe sentence boundaries (~20 then 75 chars) and calls
`returnLines()` immediately, stops on newer `user_input`, then on completion
asks `processActions()`. Failure triggers profile `llm_fallback_id` retry, else
the configured "didn't hear you" line.

The modern `openaijson` driver appends a final user instruction ("Use only this
JSON object…") plus the JSON template, POSTs `chat/completions` with
`response_format: {type:json_object}`, parses SSE, hides chain-of-thought,
accumulates the full JSON, and emits only the incrementally new `message`.
`processActions()` lazy-decodes the completed object and converts `action` into
a game command.

### 5.4 Mood, speech synthesis, and the game reply protocol

- **Mood is LLM-selected** (schema/prompt-led), not a separate `choose_mood`
  function. The JSON connector normalizes the parsed `mood` to the first valid
  emotemood and sets `SCRIPTLINE_ANIMATION`/`SCRIPTLINE_EXPRESSION` via
  `GetAnimationHex`/`GetExpression`. `returnLines()` picks mood in priority:
  speech-marker/forced mood → `LAST_LLM_RESPONSE['mood']` → legacy parsed mood →
  default → static/`FORCE_MOOD` → `extractFirstEmoteMood`.
- `returnLines()` cleans text (`cleanResponse` strips XML/braces/typography),
  translates if configured, then calls TTS per spoken sentence via
  `callNpcTtsWithFallback(text, mood, subtitleHash)` (retry with the NPC's
  fallback voice). TTS drivers live in `tts/` (Azure, OpenAI, ElevenLabs,
  Cartesia, Deepgram, XTTS, Piper, Kokoro…).
- The reply goes to the game as a deterministic line:
  ```
  <Actor>|ScriptQueue|<subtitle>/<expression>/<atomic-listener>/<animation>/<phonetic-text>/1.0/<rechat-target>/<utterance-id>
  ```
  It is flushed immediately, then persisted as `prechat` + `chat` (becoming the
  next turn's history). Actions are separately echoed as
  `|command|Action@param` lines.

### 5.5 Post-processing

After the response closes, `processor/postrequest.php` can do second-pass
memory search/logging and Minime scene-genre classification (updates
`scene_status`, infers a task). `processor/funcret.php` later turns a game
action result back into an assistant tool-call + tool-result + after-function
cue for a follow-up LLM turn.

### 5.6 STT / player TTS (side paths)

`stt.php` saves the uploaded game WAV and dispatches a configured STT driver
(Azure/Whisper/local-whisper/Deepgram/Gemini), returning the transcript, which
enters the main route. `processor/player_tts.php` can voice the player's line
with the player's assigned TTS connector.

---

## 6. CHIM — autonomy and background life

- **Background Life** is opt-in per NPC (`extended_data.background_life_enabled`),
  with optional `background_life_commands` distinguishing passive thought/diary
  vs active game commands. The middle-term worker tracks positions, avoids
  NPCs recently near the player, observes an in-game-hour trigger, and runs at
  most one passive/active update per iteration. Active mode can generate an
  inner thought/letter, a background-life diary memory, and select supported
  commands.
- `processor/background_event.php` maps a finished package to a `BackgroundCmd`
  that asks BGL for the next action; `npc_reanimated` appends "…reanimated from
  death as a zombie" to the core identity.
- CHIM modes mostly control routing/semantics (whisper radius, narrator,
  director/scene coordination), not free-running goal systems.

**Caveat:** autonomy is bounded and opt-in. This is not a continuously running
mind for every NPC; it is a scheduled background-event system.

---

## 7. MinAI — context/prompt engineering layer over CHIM

MinAI is a **Papyrus + PHP extension**, not a separate LLM server. It makes
CHIM's request speaker-specific, target-aware, stateful, and actionable.

### 7.1 The state bus

- `minai_AIFF.psc:Maintenance()` hooks `CHIM_CommandReceived`, `CHIM_TextReceived`,
  and `CHIM_NPC`.
- `SetActorVariable()` serializes an actor value as `_minai_<actor>//<key>@<value>`
  through `AIAgentFunctions.logMessage(..., "setconf")` into CHIM's `conf_opts`;
  PHP `util.php` reads/caches it. This is the common state bus.
- `minai_ContextEffect.psc` is a persistent context spell on actors that
  periodically samples inventory and calls `aiff.SetContext(actor)`.
- **Sapience** (`MinAI_DISTR.ini` SPID rule → `minai_SapienceEffect.psc`) puts
  the context spell on `ActorTypeNPC`, so nearby ordinary NPCs are dynamically
  admitted as AI agents (and removed when out of range, with a 60s dialogue
  grace period).

### 7.2 The prompt overhaul

`minai_plugin/context.php` unconditionally runs `UpdateSystemPrompt()` which
replaces `head[0]` with `BuildSystemPrompt()` — MinAI's modular generator, not
CHIM's loose concatenation. It builds, per request:

```
<background_context>
<who_are_you> You are <speaker>, you live in Skyrim…
<description_of_your_character> (speaker core, current state, status, interaction)
<description_of_interlocutor_character> (target — what the speaker perceives)
<environmental_context_description> + <additional_information>
</background_context>
```

A `ContextBuilderRegistry` orders builders by priority with SFW/NSFW markers
(`MINAI_CONTEXT_ENABLE_<id>` overrides; per-builder booleans in
`config.base.php`). This solves two CHIM problems: it separates what the
speaker *privately is/feels/has* from what they can *perceive about the
interlocutor*, and it makes section labels legible to the model.

`core_context.php:BuildPersonalityContext()` combines CHIM profile fields
(HERIKA_PERS, background, traits, speech, appearance, skills, occupation,
goals, relationships) and injects `<middle_term_memory>` verbatim.

### 7.3 Dynamic state injected

Papyrus samples game/mod state at tiered cadence (`high` 5s, `medium` 15s,
`low` 30s; context spell 30s) and PHP renders it into prompt sections:

- **Vitals:** `Actor.IsWeaponDrawn()` + health/magicka/stamina → graded
  natural-language states and "weapons at the ready / sheathed".
- **Combat:** current allies/enemies/hostility, victory/defeat/bleedout event
  cues; combat also changes allowed actions and speech style.
- **Inventory:** bounded (default 5 items shown) as `Name (count)`; `GiveItem`
  / `TakeItem` execute through Papyrus with validation and outcome narration.
- **Equipment/nudity:** visible gear, nakedness, restraints; hidden garments
  only shown to narrator/player perspective.
- **Dirt & blood / scent:** wrappers around Dirt and Blood/More Soaps → Cleanliness
  section and "bathing" action.
- **Following/party:** temporary `FollowingPlayerFaction` vs real follower
  membership; NFF looting orders.
- **Environment/relationships:** time, weather, moon, location, Frostfall
  temperature, nearby actors, "smug after a bribe" / "anxious after
  intimidation".

### 7.4 Request-cue rewrites

`prompts/chim_prompts.php` replaces CHIM's "replies to player" cues with
speaker-turn cues ("(<HERIKA>'s turn to speak about the current topic)"), combat
variants, varied rechat turns, and a large randomized radiant-topic set. This
separates **enduring identity/world (system prompt)** from **why the NPC speaks
now (user cue)**.

### 7.5 Actions and gating

PHP exposes only plausible actions per request (follower status, opt-out
factions, mod availability, combat/scene state, target), and Papyrus actually
performs them and logs a success/failure event into the next context. Includes
bathing, service actions (innkeeper meals, rent, carriage travel, merchant
trade, training), crime (bounty/arrest), following/looting, and heavily gated
optional adult content behind `disable_nsfw` + faction opt-outs.

### 7.6 Radiance — autonomous NPC↔NPC conversation

`minai_SapienceController.psc` runs a scheduler (default every 30s, 50% chance,
3–5 rechats) that picks two eligible nearby AI agents (not in dialogue, no
dialogue target, animatable), classifies the pair by combat/hostility, and
requests `radiant` / `radiantsearchingfriend` / `radiantsearchinghostile` /
`radiantcombatfriend` / `radiantcombathostile` events. PHP's
`customintegrations.php` guards against player-input races (15s delay), stores
the endpoints, and the prompt builder constructs a speaker+other-NPC prompt.
`CheckForRechat` picks the next speaker (not the prior) up to `targetRechatCount`.

### 7.7 Memory / diary

MinAI does **not** implement its own vector store; it uses CHIM event history,
profiles, middle-term memory, diary, and the CHIM `SearchMemory` action. It
triggers diary/profile synthesis (`UpdateDiary` → `minai_diary` → CHIM `diary`),
and reads CHIM's `diarylog`. Third-party temp facts use `custom_context`
upserts with TTL (`MinAI_SetContext(_NPC)`), including a "everyone" scope.

### 7.8 Public modder API

Seven ModEvents: `MinAI_RegisterEvent`, `MinAI_RequestResponse`,
`MinAI_RequestResponseDialogue`, `MinAI_SetContext`, `MinAI_SetContextNPC`,
`MinAI_RegisterAction`, `MinAI_RegisterActionNPC`. Receivers listen to
`CHIM_CommandReceived` and run `ExtCmd<action>`. `NoActionsFaction` /
`NoSexActionsFaction` / `NoNSFWActionsFaction` allow opt-out without hard
dependency.

---

## 8. SkyrimNet — the in-process native design

> The following is verified to the depth the **tracked Papyrus/ESP/prompt/docs**
> allow. The native DLL internals (memory reads, vector DB, event monitor,
> HTTP, C++ ABI, full decorator registry) are closed; they are flagged inline.

### 8.1 Architecture

A single native Windows SKSE plugin loads inside Skyrim. It owns: native SKSE
bindings, live-state collection (reads game memory directly, guarded
reference-counted wrappers, lock ordering), Inja prompt rendering, LLM/STT/TTS
orchestration, event + memory persistence, AI-package bookkeeping, PrismaUI
bindings, the HTTP dashboard/MCP server, and worker scheduling. Papyrus
(`SkyrimNetApi` globals) and the Inja decorators are the boundary. The tracked
ESP provides the quest/aliases/spells/packages/factions/keywords that make
decisions visible on actors.

End-to-end:
```
player voice/text/game hook/mod event
  -> native hotkey / magic-effect Papyrus / PrismaUI chat / trigger monitor
  -> SkyrimNetApi (Papyrus Native) / native event monitor
  -> DLL records event, determines audience/target/speaker
  -> reads loaded-game state, renders Inja (bio + live scene + history + memories)
  -> chosen LLM role (dialogue/target/memory/action/GM)
  -> action selection + eligibility
  -> streaming TTS / game-thread actor/package/UI work
  -> history, short-lived context, eventual memories
  -> next NPC sees only the context/audience it is allowed to perceive
```

(README: "No WSL. No Python launcher. No background server window to keep
alive." The dashboard/MCP are process-local services of the same DLL.)

### 8.2 The Papyrus/ESP glue (tracked)

- `skynet_MainController`/`skynet_PlayerAlias` bind to quest `000802:SkyrimNet.esp`
  (StartGameEnabled), init the library, grant the wheel spell.
- **Actor control:** `SetActorDialogueTarget()` sets a PO3 linked ref under
  `SkyrimNet_DialogueTargetKeyword`, calls `SkyrimNetApi.RegisterPackage(actor,
  "TalkToPlayer"|"TalkToNPC", ...)`, and `akActor.EvaluatePackage()`. The ESP
  `SkyrimNet_NPCDialoguePackage` (±256 travel, sandbox) and
  `SkyrimNet_TalkToPlayerPackage` (follow package, suppress ambient) turn a
  generated reply into an NPC turning/approaching.
- **Player surfaces:** native/Papyrus hotkeys, the wheel power, voice magic
  effects (normal/thought/transform/direct), and the current text chat as an
  in-game **PrismaUI JS overlay** (`SkyrimNetInternal.GetPlayerInput()` logs
  "Papyrus text input removed, use PrismaUI"). `input.js` recognizes `~`/think,
  `!`/narrate, `/silent`, `/transform`, `@` direct. The ESP also defines
  telepathy perks (eavesdrop + canonical).

### 8.3 Inja prompt templating and decorators

Prompts use **Inja**. The dialogue response entry:
```inja
[ system ]
You are {{ decnpc(npc.UUID).name }}, a {{ decnpc(npc.UUID).gender }} {{ decnpc(npc.UUID).race }}…
{{ render_subcomponent("system_head", "full") }}
[ end system ]
{{ render_template("components\\event_history") }}
[ user ]
{{ render_subcomponent("user_final_instructions", "full") }}
[ end user ]
```
`get_scene_context(npc.UUID, target.UUID, "full")` is the live scene boundary:
nearby NPCs with profile/race/gender/distance, distinct dead/unconscious/
summoned/reanimated/virtual actors, and only recent short-lived events when
nonzero.

**Decorators.** The README claims 100+. The only shipped static catalogue
(`SKSE/Plugins/SkyrimNet/docs/decorators.html`, dated 2025-07-07) lists **40
decorators + 18 variables + 13 categories**; the current prompt corpus's
`test_decorators/*.prompt` files exercise 100+ runtime-shaped names. Neither is
an authoritative registry for this revision (the DLL source is absent), so this
is the honest recoverable surface:

- **Actor:** `decnpc(uuid)->object` (name/race/level/factions/pronouns/combat…),
  `isValidActor`.
- **Equipment:** `get_worn_equipment`, `worn_has_keyword`, `get_inventory`.
- **Faction:** `is_in_faction`, `get_faction_rank`.
- **Quest/forms:** `get_all_active_quests`, `get_quest_stage`, `is_quest_active`,
  `get_form_name`, `get_civil_war_side`, `prompt_file_exists`, `is_plugin_loaded`.
- **Item customization:** `is_item_enabled`, `get_item_name/description`,
  `get_item_customization`.
- **Mood:** `moodsList`, `moodDescriptions`, `mood_description`.
- **Scene/events:** `get_short_lived_events_count`, `get_nearby_npc_list`,
  `get_active_short_lived_events`, `get_short_lived_events_by_type/by_entity`.
- **Location/time/world:** `currentWeather`, `currentTime`, `gameTime`,
  `location`, `gameTimeJson`, `short_time`, `units_to_meters`.
- **String utilities:** `join`, `length`, `lower`, `contains`, `to_number`,
  `is_array`.

Plus (from the current prompt tests) actor motion/state (`distance_between`,
`get_actor_value`, `is_sneaking`, `is_reanimated`, `is_in_package`, …), inventory
inventory (math), combat/magic (`is_in_combat`, `has_weapon_drawn`, `get_combat_targets`,
`has_spell`, `has_perk`, `has_magic_archetype`), crime/relationship
(`get_relationship_rank`, `get_crime_gold`), OmniSight + persistent entity state
(`capture_current_scene`, `get_omnisight_description`, `get_entity_state`,
`set_entity_state`, `track_entity_state`), and memory/history retrieval
(`get_relevant_memories`, `get_diary_entries`, `get_world_knowledge`,
`get_recent_events`). Custom decorators register through Papyrus:
`SkyrimNetApi.RegisterDecorator("name", "Mod_Script", "GlobalFn")`, usable as
`{{ name(npc.UUID).field }}`.

> The project's own modding workflow says to query the running game's
> `get_decorators` MCP tool as the source of truth; treat the tables above as
> evidence of intended surface, not a frozen API.

### 8.4 Memory and shared knowledge

Per-NPC private **semantic/vector memory**: `memory/memory_access.prompt` calls
`get_relevant_memories(npc.UUID)` (summary, details, emotion, tags, location,
0–10 importance). `memory/generate_memory.prompt` converts a batch of
history/live profile into a compact first-person present-tense JSON memory with
`content`, `location`, a defined `emotion`, `importance_score` 0–1, 5–10
`tags`, and a `type` (`EXPERIENCE`, `RELATIONSHIP`, `KNOWLEDGE`, `LOCATION`,
`SKILL`, `TRAUMA`, `JOY`). `memory/memory_ranker.prompt` ranks candidates by
location, mood, relationship, recent events, importance, recency, type, nearby
actors, ongoing quests. The README additionally claims a GPU-accelerated local
embedding model, importance-weighted decay, and diary entries becoming
searchable memories (all native, untracked).

**World Knowledge** is the non-private counterpart (`AddWorldKnowledge(content,
conditionExpr, alwaysInject, importance)`): content is embedded; the Inja
condition gates which NPCs receive it; `alwaysInject=true` means deterministic
inclusion, else semantic retrieval.

**Thoughts/diary:** `GenerateNPCThought` creates a private `EVENT_NPC_THOUGHTS`
history event (audience = thinker, unvoiced, persisted, influences later
dialogue/actions, skips dead/unconscious/sleeping, cooldowns).
`GenerateDiaryEntry` / `UpdateActorDynamicBio` do asynchronous diary/profile
work; dynamic profiles use recent events + important memories.

### 8.5 Events and reactions

Public event levels:
- `RegisterShortLivedEvent(id,type,description,data,ttl,source,target)` —
  deduplicable ephemeral scene context.
- `RegisterEvent` / `RegisterPersistentEvent` — persist history (NPCs know it
  but do NOT immediately reply).
- `DirectNarration(content, originator, target)` — establishes a fact and
  **forces** a reaction (selects/lets a speaker talk, targets an actor or
  broadcasts nearby).
- `RegisterDialogue[ToListener]`, `TransformDialogue` (freeform player text
  through a dialogue model → nearby NPCs react), UUID variants for
  virtual/non-body entities, and `RegisterEventSchema` (structured data, raw/
  compact/verbose formats, short-lived-context + interrupt flags).

The perception-to-reaction flow: an observed/registered event gets origin,
target, type, timing/location, and audience; qualifying nearby/visible/hearing
NPC context is rendered; the selected actor's bio + private memories + event
history + live scene + instructions are prompted; the response may be spoken,
an action selected, or remain silent/persistent. Whisper directly alters
audience; private thoughts don't leak unless telepathy broadens the audience.

### 8.6 Actions (Papyrus, tracked)

`skynet_Library.RegisterActions` registers a `follower` tag and nine `PAPYRUS`
actions at every maintenance: `OpenTrade`, `AccompanyTarget`, `StopAccompanying`,
`WaitHere`, `RentRoom`, `CompanionFollow`, `CompanionWait`, `CompanionInventory`,
`CompanionGiveTask` — each with specific eligibility (excludes merchant rank -2
for trade; non-vanilla-follower for follow/wait; rent-room faction + gold for
rent; etc.). Third parties add via `RegisterAction`/`RegisterSubCategory`/
`RegisterTag`/`UnregisterAction`/`ExecuteAction[ByUUID]` with eligibility,
schema, event types, priority, and cooldowns. YAML actions map quest editor id
→ script/function with static/speaker/dynamic params. The LLM receives only
`eligible_actions` (cached/pre-warmed during transcription per README).

### 8.7 Integrations and model roles

- **MinAI bridge:** always-registered ModEvents map TTL context, silent
  persistent events, and dialogue/narration into SkyrimNet's event model.
- **VRIK** (VR gestures), **DBVO** (voices player lines / pre-generates silent
  NPC audio), **PrismaUI** chat bridge (`window.updateMessages`, `chatSubmit`…).
- **Model presets** (`model-presets/`) separate roles: dialogue, `AgentDefault`,
  profile, diary, translator, combat, `action_evaluation`, GameMaster, `memory`,
  `meta`, vision — not one model for everything.

---

## 9. Comparative synthesis (for the story-engine)

| Aspect | CHIM (server style) | MinAI (CHIM addon) | SkyrimNet (in-process style) |
|---|---|---|---|
| Process model | SKSE plugin + PHP HTTP server | Papyrus + PHP extension of CHIM | Single native DLL in Skyrim |
| Character identity | `core_npc_master` row; prompt rebuilt per turn | CHIM row + modular prompt | Per-UUID profile + Inja bio |
| Prompt engine | PHP string/XML + cue table + JSON template | Modular context builders | Inja templates + decorators |
| Live game state | `gamedata.php` telemetry + eventlog | Papyrus-tiered sampling → `conf_opts` | Direct memory reads via native decorators |
| Memory | eventlog → summary, participant-scoped, FTS/vector | Uses CHIM memory + middle-term + diary | Per-NPC vector memory + World Knowledge |
| Knowledge asymmetry | Oghma (advanced/basic/ignorant by tag) | Uses Oghma | World Knowledge gating + memory |
| Relationships | JSON affinity map + `#REL`/dedicated rellm | Priority via profile/bio | (native) |
| NPC autonomy | Background Life (opt-in worker) + modes | Sapience + Radiance (NPC↔NPC) | Event monitor + short-lived/persistent events |
| Actions | JSON `action` → ScriptQueue / command | Dynamic action schema → Papyrus | `eligible_actions` → Papyrus, package control |
| Speech | mood → TTS WAV → ScriptQueue protocol | same via CHIM | in-DLL TTS streaming (native) |
| Player-facing UI | MCM + PrismaUI / web | MCM/hotkeys | PrismaUI overlay + localhost:8080 dashboard + MCP |

**Patterns common to all three (design lessons):**
1. **Identity is data, reconstructed per turn,** not a resident "agent" object.
2. **A single system/context message holds personality + world + scene + state;
   a separate user "cue" drives why to speak now.**
3. **Asymmetric knowledge is explicit** (Oghma tags; World Knowledge
   conditions) — NPCs should not be omniscient.
4. **Memory is selective, participant-scoped, and old-by-design**, injected as a
   user message rather than a dump.
5. **Live state is sampled and graded into prose** (vitals/equipment/nearby),
   not passed as raw JSON to the model.
6. **Actions/affordances are gated** to what's plausible, executable in the
   game, and sent with consequences back into context.
7. **Mood/emotion is LLM-chosen and mapped to game animation/TTS.**
8. **Autonomy is bounded and opt-in** (background workers, cooldowns,
   proximity gates).

**Direct implications for the story-engine (current prototype):**
- Our sandbox `StoryWorld` already mirrors the two halves: a reactive world
  (entities/player/NPCs) + the server `NpcEngine` (conversation + memory).
- CHIM's per-turn reconstructed prompt + `HERIKA_PERS` (roleplay core +
  biography) match our `characterPrompt(npc)`; we can adopt CHIM's structured
  XML sections (`<world>/<character>/<knowledge>/<general_instructions>`) and
  a separate cue.
- Oghma-style **knowledge tags** and **relationship maps** are the clearest
  gap we can add for asymmetric behavior.
- CHIM's **participant-scoped memory summary + selective recall** and
  SkyrimNet's **memory type taxonomy + importance + ranker** are the strongest
  memory designs; both are better than our current fixed-context DB.
- SkyrimNet's **decorators / live-state-as-decorators** and **event →
  audience → reaction** pipeline are the model for giving our sandbox NPCs
  perception (see nearby, react to events).

---

## 10. Source-map and fidelity caveats

- **CHIM server** (`HerikaServer`) is fully read; line references are against
  the `30d3cb3` checkout. CHIM's SKSE plugin is closed-source (distributed
  compiled); game-side telemetry/event/action execution is inferred from the
  server's inbound contract.
- **MinAI** is fully read at `ef5a7c1`; it declares itself deprecated in favour
  of SkyrimNet.
- **SkyrimNet** `SkyrimNet-GamePlugin@4e23528` verifies contracts only. Native
  DLL internals (memory reads, vector DB, event monitor geometry, HTTP/MCP,
  C++ ABI, the full current 100+ decorator registry) are **absent from this
  repo** and documented as claimed behaviours, not audited code. To audit them,
  obtain the matching native source / compiled release plus
  `CppAPI/PublicAPI.h`, or query the running game's `get_decorators` MCP tool.

---

*Dataset: `~/Project/research-mods/{HerikaServer,MinAI,SkyrimNet-GamePlugin}`;
analysis notes in `~/Project/research-mods/notes/01..04*.md`. This is Afterglow
research documented in-repo per `AGENTS.md`.*
