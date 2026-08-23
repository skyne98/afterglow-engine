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

  private interpolatedAxis(values: Float32Array, valid: Uint8Array, position: number): number {
    const current = this.indexAt(position);
    if (valid[current] !== 0) return values[current];

    let before = -1;
    let after = -1;
    for (let p = position - 1; p >= 0; p--) {
      const index = this.indexAt(p);
      if (valid[index] !== 0) {
        before = p;
        break;
      }
    }
    for (let p = position + 1; p < this.count; p++) {
      const index = this.indexAt(p);
      if (valid[index] !== 0) {
        after = p;
        break;
      }
    }
    if (before < 0) return after < 0 ? values[current] : values[this.indexAt(after)];
    if (after < 0) return values[this.indexAt(before)];

    let previous = before;
    for (let p = before - 1; p >= 0; p--) {
      if (valid[this.indexAt(p)] !== 0) {
        previous = p;
        break;
      }
    }
    let next = after;
    for (let p = after + 1; p < this.count; p++) {
      if (valid[this.indexAt(p)] !== 0) {
        next = p;
        break;
      }
    }

    const beforeIndex = this.indexAt(before);
    const afterIndex = this.indexAt(after);
    const span = this.times[afterIndex] - this.times[beforeIndex];
    const t = span > 0
      ? Math.max(0, Math.min(1, (this.times[current] - this.times[beforeIndex]) / span))
      : 0.5;
    return spline4p(
      t,
      values[this.indexAt(previous)],
      values[beforeIndex],
      values[afterIndex],
      values[this.indexAt(next)],
    );
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
      const position = 0;
      const index = this.head;
      if (sink(
        this.times[index], this.xs[index], this.ys[index],
        this.interpolatedAxis(this.pressures, this.pressureValid, position),
        this.interpolatedAxis(this.xtilts, this.tiltValid, position),
        this.interpolatedAxis(this.ytilts, this.tiltValid, position),
        this.interpolatedAxis(this.viewzooms, this.viewValid, position),
        this.viewrotations[index], this.barrelRotations[index],
      ) === false) break;
      this.head = (this.head + 1) % this.capacity;
      this.count--;
      if (this.count > 0 && performance.now() - started >= budgetMs) break;
    }
  }

  drainInterpolated(sink: MotionSink): void {
    const initialCount = this.count;
    let consumed = 0;
    for (let position = 0; position < initialCount; position++) {
      const index = this.indexAt(position);
      if (sink(
        this.times[index], this.xs[index], this.ys[index],
        this.interpolatedAxis(this.pressures, this.pressureValid, position),
        this.interpolatedAxis(this.xtilts, this.tiltValid, position),
        this.interpolatedAxis(this.ytilts, this.tiltValid, position),
        this.interpolatedAxis(this.viewzooms, this.viewValid, position),
        this.viewrotations[index], this.barrelRotations[index],
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
