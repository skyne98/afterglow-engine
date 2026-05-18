# Rules

- Use semver for crate versions
- Use semantic commits (feat, fix, chore, refactor, docs, test, etc.)
- Agent must always maintain a docs/api/ directory with notes describing the fully up-to-date engine API surface per system
- Keep docs/research/ for design notes, benchmarks, architectural investigations, and trade-off analyses
- Keep docs/ROADMAP.md up to date with the current vision
- Write extensive unit and regression tests; do not rely on memory, write tests for everything
- For bug fixes, issue fixes, and review findings, use the regression loop by default: first write or extend a focused test that reproduces the issue, run it and confirm it fails for the expected reason, apply the smallest correct fix, then rerun the test and confirm it passes. If the issue cannot be reproduced with an automated test, document why and use the closest practical verification.
- Before writing tests for an algorithm or system, explicitly identify its edge-case envelope and build tests that box it in: cover normal behavior, boundaries, invalid inputs, ordering/reordering, duplication, deletion/removal, stale state, empty/singleton/maximal cases, and adversarial/security cases where relevant. Do not only test the average path; aim to cover 100% of the edge cases implied by the algorithm.
- Write and extend benchmarks for performance-critical systems, especially rendering, networking, replication, streaming, physics, culling, and persistence; keep benchmark commands documented in docs/api/
- Keep crates/mock-rpg-network-tests as a living integration harness: whenever networking, input, identity, simulation, world streaming, persistence, rollback, or gameplay authority changes, update the mock RPG scenarios so they track the modern engine instead of becoming legacy examples
- Legacy code is bad; delete legacy code, embrace new code and systems
- From time to time, spawn a subagent to look at the code and suggest cleanups — you might have left a mess
- For research-heavy tasks or parallel synthesis, prefer `opencode` subagents with `opencode-go/deepseek-v4-flash`
- When using `opencode` research agents, keep them read-only by default, give them narrow prompts, ask for primary-source links, and merge the findings back into `docs/research/` yourself
- Preferred `opencode` pattern for research agents: `opencode run -m opencode-go/deepseek-v4-flash --pure --dir /path/to/repo "...prompt..."`
- Always clean up temporary files
- KISS and YAGNI
- No files above 500 LOC
- Build system lives in build-system/ — use `bun run <command>` (e.g. `bun run native`, `bun run wasm`, `bun run check`)

# opencode / codex usage

- This agent may be invoked by either `opencode` or `codex` (legacy name). Both use the same binary — `opencode` is the canonical name going forward.
- opencode has a **permission system** that prompts for approval before any non-read tool executes (write, edit, bash, etc.). To bypass all permission prompts (e.g. in CI, batch scripts, or when you fully trust the prompt), pass `--dangerously-skip-permissions`. In session/interactive mode you typically omit this and approve interactively.
- The `--pure` flag disables all external plugins (no MCP servers, no custom tools). Use this for clean, reproducible research runs.

## Delegation patterns (preserve context, avoid reading big files yourself)

- Use the **`Task` tool** (available inside the agent session) to spawn a subagent for research, sweeping codebase scans, or multi-file questions. This preserves your own context window — you don't need to read huge files yourself.
- The Task tool takes `subagent_type`: use `"explore"` for pure read-only questions about the codebase, `"general"` for write tasks or multi-step workflows. Be explicit: tell the subagent exactly which files to read, what question to answer, and what format to return the answer in. Keep the scope narrow.
- Use task subagents for: scanning repos for patterns, cross-referencing types across crates, understanding unfamiliar subsystems, generating summaries of large modules (>200 lines), finding all callers of a function, etc.
- The **`opencode run` CLI** (`opencode run -m <model> --pure --dir <path> "...prompt..."`) is the equivalent for running standalone prompts from a shell. Prefer the Task tool during an active session; use `opencode run` when outside a session or in a script.
- When spawning research agents, keep them read-only by default, give them narrow prompts, ask for primary-source links, and merge the findings back into `docs/research/` yourself.
