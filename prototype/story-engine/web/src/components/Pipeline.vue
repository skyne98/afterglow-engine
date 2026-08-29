<script setup lang="ts">
import { ref } from "vue";

/* Navigable vertical pipeline: each stage is an expandable card.
   All detail lives here; nothing is hidden behind a separate page. */

interface Stage {
  id: string;
  title: string;
  summary: string;
  details: string[];
  accent?: boolean;
}

const STAGES: Stage[] = [
  {
    id: "input",
    title: "1 · Input / Event",
    summary: "Something happens — a player speech, a game event, a mod event, or a scheduled job starts the loop.",
    details: [
      "Player voice or text chat (hotkeys, voice spells, PrismaUI overlay).",
      "Game events: combat, hit, death, book read, quest stage, activation, crime, dragon soul.",
      "Mod / SKSE events via the event API.",
      "Auto-scheduled: thoughts, diary, memory job, continuous mode, GameMaster.",
    ],
  },
  {
    id: "whowhy",
    title: "2 · Who & Why",
    summary: "Pick who can perceive it, which NPC speaks, and which role runs.",
    details: [
      "Audience = who can perceive: proximity · line-of-sight · whisper radius · telepathy.",
      "Speaker = the responding NPC.",
      "Role: dialogue · action · memory · meta (next-speaker) · GameMaster · thought.",
    ],
  },
  {
    id: "gather",
    title: "3 · Gather context",
    summary: "Decorators read the NPC's live world and private data from game memory.",
    details: [
      "Identity / bio: render_character_profile (name, race, gender, bio, speech style).",
      "Scene: get_scene_context — who's near, dead/unconscious/summoned/reanimated, distance.",
      "Events: get_recent_events — recent history, role/time/location tagged.",
      "Private memory: get_relevant_memories (summary, details, emotion, tags, location, importance).",
      "State: combat (weapon drawn, vitals), weather/time, telepathy, OmniSight (what it sees).",
      "Eligible actions (native-provided, pre-warmed during transcription).",
    ],
  },
  {
    id: "prompt",
    title: "4 · Build the prompt",
    summary: "Render one system + user prompt in Inja from everything gathered.",
    details: [
      "[system] “You are {name}, a {gender} {race} in Skyrim.” + “You are speaking to {target}.”",
      "system_head: instructions · format rules · actor bios · telepathy · scene · OmniSight · speech style.",
      "event_history: NPC messages → assistant, others → user; time/location markers; filters noise.",
      "memory block: relevant private memories (summary/details/emotion/tags/importance).",
      "[user] final instructions: environment · combat · telepathy · response format · embedded actions.",
      "Cue: “Respond in character now.” / “Pick one action.” / “Generate a memory.”",
    ],
  },
  {
    id: "model",
    title: "5 · Call the model",
    summary: "The chosen role's model generates the reply, streamed out as it writes.",
    details: [
      "Roles map to models: dialogue(reply) · action(select) · memory(generate/re-rank) · meta(mood, next-speaker) · GameMaster.",
      "Streamed — TTS sentence-by-sentence while the model keeps generating.",
      "Interrupted if the player speaks again.",
    ],
    accent: true,
  },
  {
    id: "outgame",
    title: "6a · Output → Game",
    summary: "The reply is spoken and/or executed as a real in-game action.",
    details: [
      "Spoken via TTS.",
      "Action executed: trade / follow / wait / rent / give / take.",
      "AI package + Actor.EvaluatePackage → the NPC turns, talks, moves.",
      "The line + event are persisted → history feeds the next turn.",
    ],
  },
  {
    id: "outmem",
    title: "6b · Output → Memory",
    summary: "The event is stored and later becomes a searchable memory — feeding the loop back.",
    details: [
      "Event persisted.",
      "Memory generation: first-person JSON — content, location, emotion, importance 0–1, tags, type (experience/relationship/knowledge/…).",
      "Embed + store per-NPC with importance-weighted decay.",
      "Recall: retrieval rank → next turn's context (loop back to ③).",
    ],
  },
];

const open = ref<Set<string>>(new Set(["whowhy"]));
function toggle(id: string) {
  const s = new Set(open.value);
  if (s.has(id)) s.delete(id);
  else s.add(id);
  open.value = s;
}
</script>

<template>
  <div class="pipeline">
    <header class="hero">
      <h1>SkyrimNet — the full NPC pipeline</h1>
      <p class="sub">
        One closed loop, re-reading persisted state each turn. Click a stage to expand its detail.
      </p>
    </header>

    <div class="spine">
      <div v-for="(s, i) in STAGES" :key="s.id" class="stage-row">
        <div class="rail">
          <div class="bubble" :class="{ accent: s.accent }">{{ i + 1 }}</div>
          <div v-if="i < STAGES.length - 1" class="rail-line"></div>
        </div>

        <div class="card" :class="{ accent: s.accent }">
          <button class="head" @click="toggle(s.id)" :aria-expanded="open.has(s.id)">
            <div class="head-text">
              <span class="title">{{ s.title }}</span>
              <span class="summary">{{ s.summary }}</span>
            </div>
            <span class="chev" :class="{ open: open.has(s.id) }">▸</span>
          </button>
          <ul v-if="open.has(s.id)" class="body">
            <li v-for="d in s.details" :key="d">{{ d }}</li>
          </ul>
        </div>
      </div>

      <!-- feedback loop callout -->
      <div class="stage-row">
        <div class="rail">
          <div class="bubble loop" title="feedback loop"><span class="loop-glyph">🔁</span></div>
        </div>
        <div class="card loop">
          <div class="loop-note">
            <strong>Then the loop closes.</strong>
            Every reply and action is persisted as an event; events become memories; and on the
            next turn those events &amp; memories are <em>re-rendered back into ③ Gather context</em> —
            which is why an NPC “remembers” across sessions even though the model forgets after every call.
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.pipeline {
  max-width: 780px;
  margin: 0 auto;
  padding: 20px 20px 70px;
  line-height: 1.5;
  overflow-y: auto;
  height: 100%;
}
.hero h1 { margin: 0 0 4px; font-size: 1.4em; }
.sub { color: #9aa0b0; margin: 0 0 18px; }

.spine { display: flex; flex-direction: column; }

.stage-row { display: flex; gap: 14px; }
.rail { display: flex; flex-direction: column; align-items: center; width: 30px; flex: 0 0 auto; }
.bubble {
  width: 30px; height: 30px; border-radius: 50%;
  background: #1a1f2a; border: 2px solid #2c3545;
  color: #cfd3de; display: flex; align-items: center; justify-content: center;
  font-size: 13px; font-weight: 700; z-index: 1;
}
.bubble.accent { background: #5b9dff; border-color: #5b9dff; color: #08101c; }
.bubble.loop { background: #14311c; border-color: #7ad08a; }
.rail-line { flex: 1; width: 2px; background: #2c3545; margin: 4px 0; }

.card {
  flex: 1; min-width: 0; margin-bottom: 12px;
  background: #141925; border: 1px solid #2c3545; border-radius: 10px; overflow: hidden;
}
.card.accent { border-color: #5b9dff; background: #101c30; }
.card.loop { border-color: #2f5b3d; background: #0e1713; }

.head { width: 100%; display: flex; align-items: center; gap: 12px; padding: 12px 14px; background: none; border: none; color: inherit; text-align: left; cursor: pointer; }
.head-text { display: flex; flex-direction: column; gap: 2px; flex: 1; }
.title { font-size: 15px; font-weight: 800; color: #fff; }
.summary { font-size: 13px; color: #9aa0b0; }
.chev { color: #5b9dff; transition: transform 0.15s; font-size: 13px; }
.chev.open { transform: rotate(90deg); }

.body { list-style: none; margin: 0; padding: 0 16px 12px 16px; }
.body li { padding: 5px 0 5px 16px; position: relative; color: #c3c7d2; font-size: 13.5px; }
.body li::before { content: "•"; color: #5b9dff; position: absolute; left: 0; }

.loop-note { padding: 12px 14px; color: #bfe6ca; font-size: 13.5px; }
.loop-glyph { font-size: 13px; }
</style>
