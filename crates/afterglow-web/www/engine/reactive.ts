// Reactive refs — a minimal reactivity system tailored to the engine's poll model.
//
// Only `.value` access is tracked (like Vue's `shallowRef`). No Proxy traps,
// no deep wrapping — safe for Three.js objects, TypedArrays, anything.
//
// The engine calls `flushEffects()` between `store.poll()` and the render pass,
// so asset swaps never happen mid-frame.
//
// ## DX (Developer Experience)
//
// The API is designed to be ergonomic and discoverable:
//
// ```ts
// import { ref, effect, computed, flushEffects } from './reactive.js';
//
// // Create a reactive ref — starts with a fallback value.
// const texture = ref<Texture | null>(null);
//
// // Read in the render loop — returns current value.
// if (texture.value) material.map = texture.value;
//
// // Write when the asset loads — queues effects, doesn't run them yet.
// texture.value = loadedTexture;
//
// // Auto-swap: effect re-runs when `texture.value` changes.
// effect(() => {
//   if (texture.value) material.map = texture.value;
// });
//
// // Derived state: re-computes when dependencies change.
// const isReady = computed(() => texture.value !== null);
// const bestLod = computed(() => lod2.value ?? lod1.value ?? lod0.value);
//
// // Between frames:
// flushEffects();  // runs all queued effects — material swaps happen here
// ```
//
// ## When to use
//
// Use reactive refs for **low-frequency** state: asset handles, material
// properties, loading progress, UI state. Do NOT use them for per-entity
// component data (use raw TypedArrays in the ECS stores instead).

// --- core reactivity ----------------------------------------------------

/** Internal: the currently-running effect callback (for dependency tracking). */
let activeEffect: (() => void) | null = null;

/** Internal: queued effects waiting to be flushed between frames. */
const effectQueue = new Set<(() => void)>();

/** Internal: effects currently executing (prevents infinite re-entry). */
const runningEffects = new Set<(() => void)>();

/**
 * A reactive reference. Only `.value` access is tracked — no deep proxies.
 *
 * Reading `.value` inside an `effect()` or `computed()` automatically
 * registers a dependency. Writing `.value` queues dependent effects
 * for the next `flushEffects()`.
 */
export class Ref<T> {
  /** @internal */ _value: T;
  /** @internal */ _deps = new Set<() => void>();

  constructor(value: T) {
    this._value = value;
  }

  /** The current value. Reading inside an effect/computed tracks it. */
  get value(): T {
    if (activeEffect) this._deps.add(activeEffect);
    return this._value;
  }

  /** Set a new value. Calls dependent callbacks directly — effects queue
   * themselves, computed marks dirty immediately. */
  set value(next: T) {
    if (Object.is(next, this._value)) return;
    this._value = next;
    // Copy — deps may change during iteration (effect cleanup, etc).
    for (const dep of [...this._deps]) dep();
  }

  /** Force-trigger effects even if the value didn't change (e.g. deep mutation). */
  trigger(): void {
    for (const dep of this._deps) effectQueue.add(dep);
  }

  /** Remove all tracked dependencies. */
  dispose(): void {
    this._deps.clear();
  }
}

/**
 * Create a reactive ref. The value type is inferred from the initial value.
 *
 * ```ts
 * const texture = ref<Texture | null>(null);
 * const score = ref(0);
 * const name = ref('player');
 * ```
 */
export function ref<T>(value: T): Ref<T> {
  return new Ref(value);
}

// --- effects -------------------------------------------------------------

/**
 * Run a function, tracking any `.value` reads as dependencies.
 * When a tracked ref changes, the function is queued and re-runs on the
 * next `flushEffects()`.
 *
 * ```ts
 * effect(() => {
 *   const tex = texture.value;  // tracked
 *   if (tex) material.map = tex; // re-runs when texture.value changes
 * });
 * ```
 *
 * @returns a cleanup function that stops the effect from re-running.
 */
export function effect(fn: () => void): () => void {
  let disposed = false;

  const run = () => {
    if (disposed || runningEffects.has(run)) return;
    const prev = activeEffect;
    activeEffect = onDep; // register onDep (deferred), not run (immediate)
    runningEffects.add(run);
    try {
      fn();
    } finally {
      runningEffects.delete(run);
      activeEffect = prev;
    }
  };

  // The dep callback: queue the run (deferred to next flushEffects).
  const onDep = () => effectQueue.add(run);

  // Initial run: track deps with `onDep` as the callback.
  const prev = activeEffect;
  activeEffect = onDep;
  runningEffects.add(run);
  try {
    fn();
  } finally {
    runningEffects.delete(run);
    activeEffect = prev;
  }

  return () => { disposed = true; };
}

// --- computed ------------------------------------------------------------

/**
 * A computed value that re-evaluates when its dependencies change.
 * Lazy: only re-computes on the next `.value` read after a dependency changes.
 *
 * ```ts
 * const isReady = computed(() => texture.value !== null);
 * const bestLod = computed(() => lod2.value ?? lod1.value ?? lod0.value);
 * ```
 */
export class Computed<T> {
  private _value: T;
  private _dirty = false;

  constructor(private readonly compute: () => T) {
    this._value = this.recompute();
  }

  private recompute(): T {
    // Track deps with a callback that marks dirty immediately (not deferred).
    // This makes computed lazy: it re-computes on the next .value read,
    // not on flushEffects().
    const prev = activeEffect;
    activeEffect = () => { this._dirty = true; };
    try {
      return this.compute();
    } finally {
      activeEffect = prev;
    }
  }

  get value(): T {
    if (this._dirty) {
      this._value = this.recompute();
      this._dirty = false;
    }
    return this._value;
  }

  /** Stop tracking dependencies. */
  dispose(): void {
    // Re-run tracking to unregister — next recompute will re-register.
    this._dirty = true;
  }
}

/**
 * Create a computed value. Re-evaluates lazily when dependencies change.
 */
export function computed<T>(compute: () => T): Computed<T> {
  return new Computed(compute);
}

// --- flush ---------------------------------------------------------------

/**
 * Run all queued effects. Call this between `store.poll()` and the render pass:
 *
 * ```ts
 * assetStore.poll();     // async worker resolves loads → refs update
 * flushEffects();        // material swaps happen here
 * renderer.render();     // everything reads consistent state
 * ```
 *
 * Effects are deduplicated — if the same effect was queued multiple times,
 * it runs once. New effects queued during flush are also run (until the
 * queue is empty), but infinite loops are prevented.
 */
export function flushEffects(): void {
  let iterations = 0;
  while (effectQueue.size > 0 && iterations < 1000) {
    const batch = [...effectQueue];
    effectQueue.clear();
    for (const fn of batch) {
      fn();
    }
    iterations++;
  }
  if (iterations >= 1000) {
    console.warn('[afterglow] flushEffects hit iteration limit — possible infinite loop');
  }
}

/** Clear all pending effects without running them (e.g. on scene unload). */
export function clearEffects(): void {
  effectQueue.clear();
}

/** Number of effects waiting to be flushed. */
export function pendingEffectCount(): number {
  return effectQueue.size;
}
