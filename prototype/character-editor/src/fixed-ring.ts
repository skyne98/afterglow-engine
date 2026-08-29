export class FixedRing<T> {
  readonly capacity: number;
  private readonly values: (T | undefined)[];
  private head = 0;
  private count = 0;

  constructor(capacity: number) {
    if (!Number.isInteger(capacity) || capacity < 1) {
      throw new Error('Invalid ring capacity.');
    }
    this.capacity = capacity;
    this.values = new Array<T | undefined>(capacity);
  }

  get length(): number {
    return this.count;
  }

  push(value: T): boolean {
    if (this.count === this.capacity) return false;
    const index = (this.head + this.count) % this.capacity;
    this.values[index] = value;
    this.count++;
    return true;
  }

  peek(): T | undefined {
    return this.count === 0 ? undefined : this.values[this.head];
  }

  shift(): T | undefined {
    if (this.count === 0) return undefined;
    const value = this.values[this.head];
    this.values[this.head] = undefined;
    this.head = (this.head + 1) % this.capacity;
    this.count--;
    return value;
  }

  clear(): void {
    while (this.count > 0) this.shift();
    this.head = 0;
  }
}
