# Rules

- Use semver for crate versions
- Use semantic commits (feat, fix, chore, refactor, docs, test, etc.)
- Agent must always maintain a docs/api/ directory with notes describing the fully up-to-date engine API surface per system
- Keep docs/research/ for design notes, benchmarks, architectural investigations, and trade-off analyses
- Keep docs/ROADMAP.md up to date with the current vision
- Write extensive unit and regression tests; do not rely on memory, write tests for everything
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
