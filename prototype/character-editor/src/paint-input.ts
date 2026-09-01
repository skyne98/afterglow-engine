export type MotionSink = (
  time: number,
  x: number,
  y: number,
  pressure: number,
  xtilt: number,
  ytilt: number,
  viewzoom: number,
  viewrotation: number,
  barrelRotation: number,
) => boolean | void;

/** Fixed input storage for pointer samples. The queue does not allocate. */
export class MotionQueue {
  readonly capacity: number;
  readonly times: Float64Array;
  readonly xs: Float32Array;
  readonly ys: Float32Array;
  readonly pressures: Float32Array;
  readonly pressureValid: Uint8Array;
  readonly xtilts: Float32Array;
  readonly ytilts: Float32Array;
  readonly tiltValid: Uint8Array;
  readonly viewzooms: Float32Array;
  readonly viewValid: Uint8Array;
  readonly viewrotations: Float32Array;
  readonly barrelRotations: Float32Array;
  private head = 0;
  private count = 0;
  private lastTime = 0;
  overflowCount = 0;

  constructor(capacity: number) {
    if (!Number.isInteger(capacity) || capacity < 2) throw new Error('Invalid motion capacity.');
    this.capacity = capacity;
    this.times = new Float64Array(capacity);
    this.xs = new Float32Array(capacity);
    this.ys = new Float32Array(capacity);
    this.pressures = new Float32Array(capacity);
    this.pressureValid = new Uint8Array(capacity);
    this.xtilts = new Float32Array(capacity);
    this.ytilts = new Float32Array(capacity);
    this.tiltValid = new Uint8Array(capacity);
    this.viewzooms = new Float32Array(capacity);
    this.viewValid = new Uint8Array(capacity);
    this.viewrotations = new Float32Array(capacity);
    this.barrelRotations = new Float32Array(capacity);
  }

  get length(): number {
    return this.count;
  }

  peekX(): number {
    return this.count === 0 ? 0 : this.xs[this.head];
  }

  peekY(): number {
    return this.count === 0 ? 0 : this.ys[this.head];
  }

  clear(): void {
    this.head = 0;
    this.count = 0;
    this.lastTime = 0;
  }

  push(
    time: number,
    x: number,
    y: number,
    pressure: number,
    xtilt: number,
    ytilt: number,
    viewzoom: number,
    viewrotation: number,
    barrelRotation: number,
    pressureIsValid = true,
    tiltIsValid = true,
    viewIsValid = true,
  ): boolean {
    let safeTime = Number.isFinite(time) ? time : this.lastTime;
    if (safeTime < this.lastTime) safeTime = this.lastTime;
    this.lastTime = safeTime;
    let accepted = true;
    if (this.count === this.capacity) {
      accepted = false;
      this.overflowCount++;
      // Preserve the newest sample. The pointer handler never queues a
      // button edge here, so dropping the oldest motion is safe.
      this.head = (this.head + 1) % this.capacity;
      this.count--;
    }
    const index = (this.head + this.count) % this.capacity;
    this.times[index] = safeTime;
    this.xs[index] = x;
    this.ys[index] = y;
    this.pressures[index] = pressure;
    this.pressureValid[index] = pressureIsValid ? 1 : 0;
    this.xtilts[index] = xtilt;
    this.ytilts[index] = ytilt;
    this.tiltValid[index] = tiltIsValid ? 1 : 0;
    this.viewzooms[index] = viewzoom;
    this.viewValid[index] = viewIsValid ? 1 : 0;
    this.viewrotations[index] = viewrotation;
    this.barrelRotations[index] = barrelRotation;
    this.count++;
    return accepted;
  }

  drain(sink: MotionSink): void {
    while (this.count > 0) {
      const index = this.head;
      if (sink(
        this.times[index], this.xs[index], this.ys[index], this.pressures[index],
        this.xtilts[index], this.ytilts[index], this.viewzooms[index],
        this.viewrotations[index], this.barrelRotations[index],
      ) === false) break;
      this.head = (this.head + 1) % this.capacity;
      this.count--;
    }
  }

  private indexAt(position: number): number {
    return (this.head + position) % this.capacity;
  }

  private resolveBoundedHeadRun(
    valid: Uint8Array,
    first: Float32Array,
    second?: Float32Array,
  ): void {
    if (this.count === 0 || valid[this.head] !== 0) return;
    let after = 1;
    while (after < this.count && valid[this.indexAt(after)] === 0) after++;
    if (after < this.count) {
      const source = this.indexAt(after);
      const firstValue = first[source];
      const secondValue = second?.[source] ?? 0;
      for (let position = 0; position < after; position++) {
        const index = this.indexAt(position);
        first[index] = firstValue;
        if (second) second[index] = secondValue;
      }
    }
    const end = after < this.count ? after : this.count;
    for (let position = 0; position < end; position++) {
      valid[this.indexAt(position)] = 1;
    }
  }

  private resolveAllMissingRuns(
    valid: Uint8Array,
    first: Float32Array,
    second?: Float32Array,
  ): void {
    let previous = -1;
    let before = -1;
    let position = 0;
    while (position < this.count) {
      const index = this.indexAt(position);
      if (valid[index] !== 0) {
        previous = before < 0 ? position : before;
        before = position;
        position++;
        continue;
      }

      const start = position;
      let after = position + 1;
      while (after < this.count && valid[this.indexAt(after)] === 0) after++;
      let next = after;
      if (after < this.count) {
        next = after + 1;
        while (next < this.count && valid[this.indexAt(next)] === 0) next++;
        if (next >= this.count) next = after;
      }

      for (let missing = start; missing < after; missing++) {
        const missingIndex = this.indexAt(missing);
        if (before < 0) {
          if (after < this.count) {
            const afterIndex = this.indexAt(after);
            first[missingIndex] = first[afterIndex];
            if (second) second[missingIndex] = second[afterIndex];
          }
        } else if (after >= this.count) {
          const beforeIndex = this.indexAt(before);
          first[missingIndex] = first[beforeIndex];
          if (second) second[missingIndex] = second[beforeIndex];
        } else {
          const beforeIndex = this.indexAt(before);
          const afterIndex = this.indexAt(after);
          const span = this.times[afterIndex] - this.times[beforeIndex];
          const t = span > 0
            ? Math.max(0, Math.min(1,
              (this.times[missingIndex] - this.times[beforeIndex]) / span))
            : 0.5;
          first[missingIndex] = spline4p(
            t,
            first[this.indexAt(previous)], first[beforeIndex],
            first[afterIndex], first[this.indexAt(next)],
          );
          if (second) {
            second[missingIndex] = spline4p(
              t,
              second[this.indexAt(previous)], second[beforeIndex],
              second[afterIndex], second[this.indexAt(next)],
            );
          }
        }
        valid[missingIndex] = 1;
      }
      position = after;
    }
  }

  /**
   * Drain interpolated samples until either the queue is empty or `budgetMs`
   * has elapsed. Remaining samples stay queued for the next call, so no input
   * is lost when a burst of pointer moves arrives faster than the browser can
   * process the brush engine.
   */
  drainInterpolatedBounded(sink: MotionSink, budgetMs: number): void {
    const started = performance.now();
    while (this.count > 0) {
      this.resolveBoundedHeadRun(this.pressureValid, this.pressures);
      this.resolveBoundedHeadRun(this.tiltValid, this.xtilts, this.ytilts);
      this.resolveBoundedHeadRun(this.viewValid, this.viewzooms);
      const index = this.head;
      if (sink(
        this.times[index], this.xs[index], this.ys[index],
        this.pressures[index], this.xtilts[index], this.ytilts[index],
        this.viewzooms[index], this.viewrotations[index],
        this.barrelRotations[index],
      ) === false) break;
      this.head = (this.head + 1) % this.capacity;
      this.count--;
      if (this.count > 0 && performance.now() - started >= budgetMs) break;
    }
  }

  drainInterpolated(sink: MotionSink): void {
    this.resolveAllMissingRuns(this.pressureValid, this.pressures);
    this.resolveAllMissingRuns(this.tiltValid, this.xtilts, this.ytilts);
    this.resolveAllMissingRuns(this.viewValid, this.viewzooms);
    const initialCount = this.count;
    let consumed = 0;
    for (let position = 0; position < initialCount; position++) {
      const index = this.indexAt(position);
      if (sink(
        this.times[index], this.xs[index], this.ys[index],
        this.pressures[index], this.xtilts[index], this.ytilts[index],
        this.viewzooms[index], this.viewrotations[index],
        this.barrelRotations[index],
      ) === false) break;
      consumed++;
    }
    this.head = (this.head + consumed) % this.capacity;
    this.count -= consumed;
  }
}

/** MyPaint's four-point cubic interpolation for one input axis. */
export function spline4p(
  t: number,
  previous: number,
  first: number,
  second: number,
  next: number,
): number {
  const t2 = t * t;
  const t3 = t2 * t;
  return 0.5 * (
    (2 * first) +
    (-previous + second) * t +
    (2 * previous - 5 * first + 4 * second - next) * t2 +
    (-previous + 3 * first - 3 * second + next) * t3
  );
}
