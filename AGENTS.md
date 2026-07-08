# AGENTS.md — afterglow-engine

## Project direction

**afterglow-engine** is a **web-based game engine**. The rendering foundation
under evaluation is **Three.js with the WebGPU renderer** (with WebGL2 fallback).

See `docs/research/` for the stability/feasibility research that informs this
choice. Research notes are the canonical record of decisions — keep them
up-to-date as the stack evolves.

## Rules

- Use semver for crate versions
- Use semantic commits (feat, fix, chore, refactor, docs, test, etc.)
- Agent must always maintain a docs/api/ directory with notes describing the fully up-to-date engine API surface per system
- Write extensive unit and regression tests; do not rely on memory, write tests for everything
- Legacy code is bad; delete legacy code, embrace new code and systems
- From time to time, spawn a subagent to look at the code and suggest cleanups — you might have left a mess
- Always clean up temporary files
- KISS and YAGNI

## Research

- `docs/research/threejs-webgpu-stability.md` — How stable and usable is the
  Three.js WebGPU renderer? (investigated 2026-07)
