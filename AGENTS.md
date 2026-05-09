# Rules

- Use semver for crate versions
- Use semantic commits (feat, fix, chore, refactor, docs, test, etc.)
- Agent must always maintain a docs/api/ directory with notes describing the fully up-to-date engine API surface per system
- Keep docs/research/ for design notes, benchmarks, architectural investigations, and trade-off analyses
- Keep docs/ROADMAP.md up to date with the current vision
- Write extensive unit and regression tests; do not rely on memory, write tests for everything
- Legacy code is bad; delete legacy code, embrace new code and systems
- From time to time, spawn a subagent to look at the code and suggest cleanups — you might have left a mess
- Always clean up temporary files
- KISS and YAGNI
- No files above 500 LOC
- Build system lives in build-system/ — use `bun run <command>` (e.g. `bun run native`, `bun run wasm`, `bun run check`)
