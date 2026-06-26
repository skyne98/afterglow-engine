# Rules

- Use semver for crate versions
- Use semantic commits (feat, fix, chore, refactor, docs, test, etc.)
- Agent must always maintain a docs/api/ directory with notes describing the fully up-to-date engine API surface per system
- Keep docs/research/ for design notes, benchmarks, architectural investigations, and trade-off analyses
- Keep docs/subject/ for stable, comprehensive reference notes on key dependencies and subjects: feature catalogs, module maps, full API reference tables, test-backed usage, and gotchas. These are authoritative references, not design investigations.
  Example subject notes: Lightyear API reference, Avian3d API reference, lightyear_avian3d API reference.
- Keep docs/ROADMAP.md up to date with the current vision
- Write extensive unit and regression tests; do not rely on memory, write tests for everything
- For bug fixes, issue fixes, and review findings, use the regression loop by default: first write or extend a focused test that reproduces the issue, run it and confirm it fails for the expected reason, apply the smallest correct fix, then rerun the test and confirm it passes. If the issue cannot be reproduced with an automated test, document why and use the closest practical verification.
- Before writing tests for an algorithm or system, explicitly identify its edge-case envelope and build tests that box it in: cover normal behavior, boundaries, invalid inputs, ordering/reordering, duplication, deletion/removal, stale state, empty/singleton/maximal cases, and adversarial/security cases where relevant. Do not only test the average path; aim to cover 100% of the edge cases implied by the algorithm.
- Write and extend benchmarks for performance-critical systems, especially rendering, networking, replication, streaming, physics, culling, and persistence; keep benchmark commands documented in docs/api/
- Prototypes live in crates/prototypes/ with a `prototype-` prefix (e.g. `prototype-physics-tumble`), each as a standalone Cargo crate
- Keep crates/engine-rpg-harness as the living integration harness: whenever networking, input, identity, simulation, world streaming, persistence, rollback, or gameplay authority changes, update the RPG scenarios so they track the modern engine instead of becoming legacy examples
- Legacy code is bad; delete legacy code, embrace new code and systems
- From time to time, spawn a subagent to look at the code and suggest cleanups — you might have left a mess
- For research-heavy tasks or parallel synthesis, prefer `opencode` subagents with `opencode-go/deepseek-v4-flash`
- When using `opencode` research agents, keep them read-only by default, give them narrow prompts, ask for primary-source links, and merge the findings back into `docs/research/` yourself
- Preferred `opencode` pattern for research agents: `opencode run -m opencode-go/deepseek-v4-flash --pure --dir /path/to/repo "...prompt..."`
- Always clean up temporary files
- KISS and YAGNI
- No files above 500 LOC
- Build system lives in build-system/ — use `bun run <command>` (e.g. `bun run native`, `bun run wasm`, `bun run check`)

## Lightyear Correctness Rules

- Install `ClientPlugins` / `ServerPlugins` before registering protocol channels, messages, and replicated components; register protocol before spawning Lightyear link/server/client entities.
- For interpolated replicated components, use `.add_interpolation_with(...)` or `.add_linear_interpolation()` on the component registration. Do not rely on `InterpolationRegistry::set_interpolation()` alone; it stores a lerp function but does not add interpolation systems.
- Use Lightyear's native Leafwing input plugin for player input. Do not register `ActionState<AfterglowAction>` as a normal replicated component for movement/combat commands, and do not manually register `lightyear::input::InputChannel`; the input plugin owns the channel and input-message protocol.
- Keep `ActionState` pure input-device state only. Do not send a parallel gameplay intent for the same player action. World targets, selected entities, command ids, rope ids, hit results, etc. are derived by server/predicted fixed simulation from input + world state, then represented in replicated gameplay components using `StableEntityId`. Server authority systems must process received input for all authoritative controlled players, not only the host/local player.
- Write custom client input in `FixedPreUpdate` in `InputSystems::WriteClientInputs`; read gameplay input in fixed simulation after Lightyear has restored/buffered the relevant tick. Guard input-writing systems during rollback when they would overwrite restored history.
- Avoid targeting the same replicated entity to the same client as both `Predicted` and `Interpolated` unless the lifecycle is explicitly proven safe. Confirmed roots are authoritative anchors, not player-facing presentation. For the multiplayer boxes demo, player bodies are predicted to all clients and rendered from `Predicted` copies; cubes are also predicted to all clients for contact responsiveness.
- If a predicted or remote entity needs between-fixed-frame smoothing, add `FrameInterpolate<Transform>` to that presentation/simulation entity in addition to installing `FrameInterpolationPlugin::<Transform>`.
- Spawn `PreSpawned` predicted entities in the fixed simulation schedule (`FixedUpdate` / Lightyear FixedMain), not `PreUpdate`/`Update`, so their spawn tick and prediction history match server confirmation. For input-derived predicted entities, derive deterministic ids/hashes from data both server and predicted client know identically despite input delay (for the boxes rope: owner + selected target, not local processing tick), not from a client-sent command id.
- Do not directly despawn Lightyear-tracked predicted/confirmed entities for local feedback. Use Lightyear's prediction despawn command for predicted despawns, and let authoritative despawn or `PreSpawned` timeout reconcile lifecycle.
- For netcode/UDP clients, do not preinsert `LocalId`, `RemoteId`, or `Connected`; `NetcodeClientPlugin` inserts them after handshake. Explicit ids are only for manual/in-process links such as Crossbeam test transports with no handshake.
- Server-side `ClientOf` links are only safe as replication/control owners after they have a `ReplicationSender`; gate `ControlledBy` binding on sender readiness.
- Replicate logical state and canonical poses, not Bevy render assets. Attach meshes/materials/camera/highlights locally from replicated identity/state.
- With Avian + Lightyear, use the engine's Transform-mode bridge (`afterglow-lightyear-avian3d`) and deterministic baseline (`enhanced-determinism`, no Avian `parallel`). Pick one canonical networked pose representation per stack.
- Bevy tests using Lightyear plugins must call `finish()` and `cleanup()` after plugin registration before manually driving updates; Lightyear builds replication buffer systems during plugin finish.

# Agent Role: Manager, Integrator, Verifier

The top-level agent acts as a **manager, integrator, and verifier**. It does not do grunt work directly. Instead it:

- **Spawns subagents** for every substantial piece of work: implementation, research, documentation, testing, review. Spawn multiple subagents in parallel when the work is independent.
- **Verifies output** — reads what subagents produced, checks for correctness, completeness, laziness, shortcuts, and sneaky omissions. If a subagent slacked or did not follow instructions, respawn a new agent to fix the gaps or do it yourself. Do not trust subagents blindly; sample their output and cross-check against requirements.
- **Integrates results** — merges subagent output back into the project, resolves conflicts, updates cross-references, runs the final verification suite (lint, test, build), and commits if asked.
- **Keeps the big picture** — the manager holds the overall architecture, constraints, and roadmap. Subagents get narrow, well-scoped prompts and should not be making cross-cutting design decisions.
- **Dispatches agents for implementation** — for any substantial feature change (new system, refactor, bug fix across multiple files), the manager does NOT write the code directly. Instead it writes the plan/spec, spawns implementation subagents with precise instructions, then verifies and integrates their output. The manager does grunt work only for trivial single-line changes; everything else is delegated.
- **Reviews agent output critically** — agents will take shortcuts, leave TODOs, ignore edge cases, skip tests, or produce "looks correct" code that doesn't compile. The manager samples output, cross-checks against requirements, runs tests, and respawns agents to fix gaps. Do not trust subagents blindly.

## Subagent & Delegation Mechanisms

Use only the **`Task` tool** for subagent delegation. Pi subagent and `opencode run` CLI are available but not used currently.

The **`Task` tool** spawns a subagent from within the session.

**Subagent types:**
- `"explore"` — read-only tools only (Read, Grep, Glob, WebFetch, etc.). Safe for research where you want guarantees no files are modified.
- `"general"` — full tool access including Write, Edit, Bash. Use for implementation, documentation generation, testing, and any task that needs to produce output.

**Lifecycle:**
- Each spawn gets a **fresh context** (no prior messages) unless you pass a `task_id`.
- Pass a prior `task_id` to resume the same subagent session with its full message history preserved.
- Subagents return a single final message. The output is not visible to the user — you must relay it.
- Option `background: true` spawns asynchronously; poll with `task_status(task_id, wait=false)`.
- Timeout control via `description` (short, human-readable name) — the subagent runs until it finishes or hits its own token/tool limits.

**Typical prompt structure:**
```
Task(
  description="Implement XYZ",
  subagent_type="general",
  prompt="You are doing XYZ. Read files A, B. Implement C. Verify with D. Return summary."
)
```

Be explicit about: which files to read, what to produce, what format to return, and how to verify. Keep scope narrow.

**IMPORTANT: Always set maximum reasoning effort on subagents.** Every `Task` spawn must include a prompt instruction like "Think carefully and exhaustively about this. Use maximum reasoning effort. Consider all edge cases, cross-cutting concerns, and subtle interactions before responding." This ensures subagents produce thorough, high-quality output rather than superficial first-pass answers.

## Delegation Guidelines

- **Parallelise aggressively** — spawn independent subagents concurrently. Reading three different crate sources? Three subagents.
- **Keep prompts narrow** — one subagent = one concern. Do not ask a single subagent to research Lightyear AND write code AND run tests. Split.
- **Specify output format** — tell the subagent exactly what to return (e.g. "Write the file to path X", "Return a summary of key findings in bullet points").
- **Verify everything** — re-read files written by subagents. Run tests they claim pass. Check they did not take shortcuts, leave TODOs, ignore edge cases, or make unwarranted assumptions.
- **Fix gaps** — if a subagent produced incomplete work, spawn a new agent with specific instructions to fix the gaps, or do the fix yourself.
- **Clean up** — subagents may leave temporary files, incomplete stubs, or debugging artifacts. Remove them.
- **Context preservation** — use `task_id` to follow up on prior subagent work when the task requires multiple rounds. Otherwise spawn fresh for independence.

## Development Regiment: Plan → Verify → Iterate

For every plan step or substantial feature change, follow this loop until zero issues remain:

1. **Implement** — do the work (directly or via spawned agents).
   - The manager NEVER writes feature code directly. Subagents do the implementation.
   - The manager writes the spec, spawns narrow implementation subagents, then verifies.
   - Trivial single-line fixes are the only exception (and even those should be reviewed).
2. **Spawn 4 review agents** — dispatch 4 subagents with the same source files and plan, each with a distinct focus:
   - Architecture reviewer — checks design, extensibility, separation of concerns
   - Implementation reviewer — checks correctness, completeness, code quality
   - Plan alignment reviewer — checks that the implementation matches the spec
   - Edge-case reviewer — checks for missed edge cases, future-proofing
3. **Evaluate and fix** — read all reviews, categorize issues by severity, fix the blocking and medium ones.
4. **Ask** — present the status and check if another round is needed.
5. **Iterate** — repeat steps 2-4 until all reviewers report SATISFACTORY with zero new issues.
6. **Proceed** — move to the next step in the plan.

Key rules:
- Reviewers must return specific line numbers, not vague criticism.
- "No new issues" means exactly that — reviewers should be instructed to be critical and look for anything; SATISFACTORY means they found nothing actionable.
- Cosmetic preferences are not blocking issues. Real bugs and plan gaps are.
- The manager integrates the fixes and runs the final verification (compile, test).
