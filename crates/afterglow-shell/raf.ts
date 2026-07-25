// Fixed-capacity native requestAnimationFrame queue.
// This file intentionally uses JavaScript-compatible TypeScript because the
// production shell evaluates it directly in V8 without a transpilation step.
(() => {
  const native = globalThis.__afterglowAnimationFrameNative;
  if (native === undefined)
    throw new Error('native animation-frame hooks are unavailable');

  const CAPACITY = 1024;
  const callbacks = new Array(CAPACITY).fill(null);
  const generations = new Uint32Array(CAPACITY);
  const nextFree = new Int32Array(CAPACITY);
  const queue = new Uint16Array(CAPACITY);

  for (let index = 0; index < CAPACITY - 1; index++) nextFree[index] = index + 1;
  nextFree[CAPACITY - 1] = -1;

  let freeHead = 0;
  let queueHead = 0;
  let queueTail = 0;
  let queueCount = 0;
  let activeCount = 0;

  function releaseSlot(slot) {
    nextFree[slot] = freeHead;
    freeHead = slot;
  }

  function decodeId(id) {
    id = Number(id);
    if (!Number.isSafeInteger(id) || id <= 0) return null;
    const encoded = id - 1;
    const slot = encoded % CAPACITY;
    const generation = Math.floor(encoded / CAPACITY);
    if (generation > 0xffffffff) return null;
    return [slot, generation];
  }

  globalThis.requestAnimationFrame = (callback) => {
    if (typeof callback !== 'function')
      throw new TypeError('requestAnimationFrame callback must be a function');
    if (freeHead < 0 || queueCount === CAPACITY) {
      native.overflow();
      throw new RangeError(`requestAnimationFrame capacity ${CAPACITY} exceeded`);
    }

    const slot = freeHead;
    freeHead = nextFree[slot];
    let generation = (generations[slot] + 1) >>> 0;
    if (generation === 0) generation = 1;
    generations[slot] = generation;
    callbacks[slot] = callback;
    queue[queueTail] = slot;
    queueTail = (queueTail + 1) % CAPACITY;
    queueCount++;
    activeCount++;

    // The native side references one aggregate external-op token while
    // activeCount is non-zero. winit admits the batch from about_to_wait.
    native.requested(activeCount);
    return generation * CAPACITY + slot + 1;
  };

  globalThis.cancelAnimationFrame = (id) => {
    const decoded = decodeId(id);
    if (decoded === null) return;
    const slot = decoded[0];
    const generation = decoded[1];
    if (generations[slot] !== generation || callbacks[slot] === null) return;

    // Keep the canceled slot reserved until its queue entry is consumed. This
    // prevents a reused slot from being invoked by the stale entry.
    callbacks[slot] = null;
    activeCount--;
    if (activeCount === 0) native.empty();
  };

  globalThis.__runNativeAnimationFrames = (timestamp = performance.now()) => {
    const batchCount = queueCount;
    let invokedCount = 0;
    let firstError = null;

    // Snapshot batchCount: callbacks requested by a callback are appended to
    // the same ring but are not part of this presentation frame.
    for (let index = 0; index < batchCount; index++) {
      const slot = queue[queueHead];
      queueHead = (queueHead + 1) % CAPACITY;
      queueCount--;
      const callback = callbacks[slot];
      callbacks[slot] = null;
      releaseSlot(slot);

      if (callback === null) continue;
      activeCount--;
      invokedCount++;
      if (activeCount === 0) native.empty();
      try {
        callback(timestamp);
      } catch (error) {
        if (firstError === null) firstError = error;
      }
    }

    native.drained(invokedCount);
    // Browser callback failures are reported after the complete frame batch,
    // so one callback cannot suppress later callbacks in the same frame.
    if (firstError !== null) native.report(firstError);
  };

  globalThis.__nativeAnimationFrameStats = () => ({
    capacity: CAPACITY,
    pending: activeCount,
    queued: queueCount,
  });

  delete globalThis.__afterglowAnimationFrameNative;
})();
