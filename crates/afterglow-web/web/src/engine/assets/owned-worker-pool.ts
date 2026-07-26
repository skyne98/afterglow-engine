export interface OwnedService {
  close(): void | Promise<void>;
}

/** Fixed bootstrap-owned set of independent services.
 *
 * Creation rolls back in reverse order. Shutdown is idempotent, attempts every
 * service in reverse order, and reports the first failure. The pool has no
 * scheduling or subsystem policy. */
export class OwnedWorkerPool<T extends OwnedService> {
  readonly stats = { closeErrors: 0, closed: false };
  private closed = false;
  private constructor(private readonly owned: T[]) {}

  static async start<T extends OwnedService>(
    count: number,
    create: (index: number) => Promise<T>,
  ): Promise<OwnedWorkerPool<T>> {
    if (!Number.isInteger(count) || count <= 0)
      throw new RangeError('worker pool count must be positive');
    const workers: T[] = [];
    try {
      for (let index = 0; index < count; index++) workers.push(await create(index));
      return new OwnedWorkerPool(workers);
    } catch (error) {
      for (let index = workers.length - 1; index >= 0; index--) {
        try { await workers[index]?.close(); }
        catch (closeError) {
          if (error instanceof Error && error.cause === undefined) error.cause = closeError;
        }
      }
      throw error;
    }
  }

  get workers(): readonly T[] {
    return this.owned;
  }

  get size(): number {
    return this.owned.length;
  }

  async close(): Promise<void> {
    if (this.closed) return;
    this.closed = true;
    let firstError: unknown = null;
    for (let index = this.owned.length - 1; index >= 0; index--) {
      try { await this.owned[index]?.close(); }
      catch (error) {
        this.stats.closeErrors++;
        if (firstError === null) firstError = error;
      }
    }
    this.owned.length = 0;
    this.stats.closed = true;
    if (firstError !== null) throw firstError;
  }
}
