export type StrokeStabilizerMode = 'off' | 'string' | 'average' | 'exponential' | 'inertia';

const AVERAGE_CAPACITY = 64;
const CATCH_UP_DISTANCE = 0.25;

/** Fixed stroke-position filter. Sample processing does not allocate. */
export class StrokeStabilizer {
  private readonly xs = new Float64Array(AVERAGE_CAPACITY);
  private readonly ys = new Float64Array(AVERAGE_CAPACITY);
  private head = 0;
  private count = 0;
  private sumX = 0;
  private sumY = 0;
  private velocityX = 0;
  private velocityY = 0;
  private lastTime = 0;
  private active = false;
  private firstSample = false;
  private targetX = 0;
  private targetY = 0;

  mode: StrokeStabilizerMode = 'off';
  amount = 20;
  catchUp = true;
  x = 0;
  y = 0;

  configure(mode: StrokeStabilizerMode, amount: number, catchUp: boolean): void {
    this.mode = mode;
    this.amount = Math.max(1, Math.min(100, amount));
    this.catchUp = catchUp;
  }

  begin(x: number, y: number, time: number): void {
    this.active = true;
    this.firstSample = true;
    this.x = x;
    this.y = y;
    this.targetX = x;
    this.targetY = y;
    this.lastTime = time;
    this.velocityX = 0;
    this.velocityY = 0;
    this.head = 0;
    this.count = 1;
    this.sumX = x;
    this.sumY = y;
    this.xs[0] = x;
    this.ys[0] = y;
  }

  end(): void {
    this.active = false;
    this.firstSample = false;
  }

  isActive(): boolean {
    return this.active;
  }

  canCatchUp(): boolean {
    return this.active && this.catchUp &&
      (this.mode === 'average' || this.mode === 'exponential');
  }

  sample(x: number, y: number, time: number, modelUnitsPerPixel = 1): boolean {
    if (!this.active) this.begin(x, y, time);
    this.targetX = x;
    this.targetY = y;

    if (this.firstSample) {
      this.firstSample = false;
      this.lastTime = time;
      return true;
    }

    switch (this.mode) {
      case 'off':
        this.x = x;
        this.y = y;
        break;
      case 'string':
        if (!this.pulledString(x, y, modelUnitsPerPixel)) return false;
        break;
      case 'average':
        this.movingAverage(x, y);
        break;
      case 'exponential':
        this.exponentialAverage(x, y);
        break;
      case 'inertia':
        this.applyInertia(x, y, time);
        break;
    }
    this.lastTime = time;
    return true;
  }

  stepCatchUp(): boolean {
    if (!this.canCatchUp()) return false;
    const dx = this.targetX - this.x;
    const dy = this.targetY - this.y;
    const distanceSquared = dx * dx + dy * dy;
    if (distanceSquared === 0) return false;
    if (distanceSquared <= CATCH_UP_DISTANCE * CATCH_UP_DISTANCE) {
      this.x = this.targetX;
      this.y = this.targetY;
    } else {
      const factor = this.mode === 'exponential' ? 0.38 : 0.22;
      this.x += dx * factor;
      this.y += dy * factor;
    }
    if (this.mode === 'average') this.resetAverageAtCurrentPosition();
    return true;
  }

  finishCatchUp(): boolean {
    if (!this.canCatchUp() || (this.x === this.targetX && this.y === this.targetY)) return false;
    this.x = this.targetX;
    this.y = this.targetY;
    if (this.mode === 'average') this.resetAverageAtCurrentPosition();
    return true;
  }

  private windowSize(): number {
    return 2 + Math.round((this.amount - 1) * (AVERAGE_CAPACITY - 2) / 99);
  }

  private pulledString(x: number, y: number, modelUnitsPerPixel: number): boolean {
    const dx = x - this.x;
    const dy = y - this.y;
    const distance = Math.hypot(dx, dy);
    const length = this.amount * Math.max(0.01, modelUnitsPerPixel);
    if (distance <= length) return false;
    const movement = (distance - length) / distance;
    this.x += dx * movement;
    this.y += dy * movement;
    return true;
  }

  private movingAverage(x: number, y: number): void {
    const window = this.windowSize();
    if (this.count === window) {
      this.sumX -= this.xs[this.head];
      this.sumY -= this.ys[this.head];
      this.xs[this.head] = x;
      this.ys[this.head] = y;
      this.sumX += x;
      this.sumY += y;
      this.head = (this.head + 1) % window;
    } else {
      const index = (this.head + this.count) % AVERAGE_CAPACITY;
      this.xs[index] = x;
      this.ys[index] = y;
      this.sumX += x;
      this.sumY += y;
      this.count++;
    }
    this.x = this.sumX / this.count;
    this.y = this.sumY / this.count;
  }

  private resetAverageAtCurrentPosition(): void {
    this.head = 0;
    this.count = 1;
    this.sumX = this.x;
    this.sumY = this.y;
    this.xs[0] = this.x;
    this.ys[0] = this.y;
  }

  private exponentialAverage(x: number, y: number): void {
    const alpha = 2 / (this.windowSize() + 1);
    this.x += (x - this.x) * alpha;
    this.y += (y - this.y) * alpha;
  }

  private applyInertia(x: number, y: number, time: number): void {
    const seconds = Math.max(0, Math.min(1 / 30, (time - this.lastTime) / 1000));
    const strength = this.amount / 100;
    const omega = 36 - strength * 30;
    const damping = 0.72;
    const steps = Math.ceil(omega * seconds / 0.2);
    const stepTime = seconds / steps;
    for (let step = 0; step < steps; step++) {
      this.velocityX += (omega * omega * (x - this.x) - 2 * damping * omega * this.velocityX) * stepTime;
      this.velocityY += (omega * omega * (y - this.y) - 2 * damping * omega * this.velocityY) * stepTime;
      this.x += this.velocityX * stepTime;
      this.y += this.velocityY * stepTime;
    }
  }
}
