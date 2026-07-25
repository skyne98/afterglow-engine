# AGENTS.md — Runtime Development Rules

These rules are mandatory for all work on the afterglow-shell crate.
They exist because every shortcut taken here has cost more time than it saved.

## 1. Never replace, stub, or modify client/example code

The example code (`examples/*.html`, their `<script type="module">`, and the
`three/addons/*` modules) **is the test**. It is known-good — it runs in Chrome
and produces the reference screenshots we diff against.

- **Never** intercept an import in the module loader to return a fake/stub module.
- **Never** patch the example's source (no `replacen` on the extracted module to
  inject `await renderer.init()` or anything else). The example must run verbatim.
- **Never** replace an addon (Inspector, OrbitControls, GLTFLoader, etc.) with a
  no-op or partial stub. A no-op stub silently breaks the example's own logic
  (e.g. an OrbitControls stub that skips `controls.update()` breaks the camera
  framing the example already does correctly).

If you find yourself wanting to stub something "to dodge a hang," stop — you are
about to create a fake test that proves nothing. The only legitimate response to
a hang is rule 2.

**Corollary — the comparison is only legitimate if the real code ran unmodified.**
A render that matches the reference after stubbing OrbitControls is meaningless.

## 2. The fix for any hang is to implement the missing environment piece

Every hang so far has been one specific thing the runtime environment is missing —
a DOM method, a global, a WebGPU entry point, an event API. The example/addon code
is correct; the environment is incomplete.

When something hangs:
1. Instrument with logs to find the **exact** statement that blocks (step-by-step,
   as was done to find the Inspector eval hang, the `copyExternalImageToTexture`
   gap, etc.).
2. Identify the **environment** piece it needs (e.g. a real `addEventListener`,
   a real 2D canvas context, `queue.copyExternalImageToTexture`, `GPUFeatureName`,
   a proper `fetch`/`createImageBitmap`).
3. **Implement that piece properly** in the shim/runtime so the real code runs
   unmodified.

Never implement the missing piece by editing the client code. Implement it in the
host (Rust ops) or the browser-shim layer (`browser_shim.js`) that the code runs
against.

## 3. No workarounds, no shortcuts, no "good enough for now"

- No `// TODO: real impl` stubs left in to unblock the next thing.
- No compensating hacks (e.g. manually calling `camera.lookAt` because a stub
  skipped it). If a hack is needed, the real code is broken — fix the root cause.
- No commented-out code, no dead diagnostic branches left in production paths.
- Diagnostics (instrumentation logs, `__renderer` exposure, module dumps) are
  temporary — remove them once the cause is found and fixed.

## 4. Clean code

- Keep the browser-shim layer (`browser_shim.js`) and the host ops
  (`examples/browser_test.rs`) minimal and well-structured.
- One concern per op/shim. Document *why* each shim exists (which real API it
  backs, and that it's an environment piece, not a client-code replacement).
- If a shim grows into a partial reimplementation of a spec API, that's fine —
  but it must implement the spec behavior correctly, not a no-op that happens to
  not crash.

## 5. V8 + wgpu is a real WebGPU stack — treat it as such

three.js runs on this stack. The example code is deterministic and known-good.
Differences vs Chrome's reference are **not** "different stacks, can't match" —
they are specific, findable causes (a missing environment API, a wrong shim, an
unimplemented WebGPU method). "Computers are deterministic" applies: every mismatch
has one specific root cause. Find it; don't hand-wave it.

---

## Historical context (what these rules replace)

The stub-for-OrbitControls hack (a no-op class that skipped `controls.update()`)
silently broke camera framing and was then "fixed" by manually adding
`camera.lookAt` to the stub — compensating for a self-inflicted bug. The
stub-for-Inspector hack (intercepting the import) avoided a real DOM-UI eval hang
but set the bad pattern that spread to OrbitControls. The `init(); → await
renderer.init();` source patch modified the example to gate the host. All of these
are forbidden by the rules above and must be removed in favor of proper
environment implementations.
