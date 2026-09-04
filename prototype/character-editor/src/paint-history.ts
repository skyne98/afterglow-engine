export const PAINT_HISTORY_MAX_OPERATIONS = 256;

export type PaintHistoryOperation = {
  id: number;
  layer: number;
  side: 0 | 1;
  tiles: number;
};

export class PaintHistoryQueue {
  private operations: PaintHistoryOperation[] = [];
  private cursor = 0;
  private nextId = 1;
  private active: PaintHistoryOperation | null = null;

  begin(layer: number): { operation: PaintHistoryOperation; discarded: PaintHistoryOperation[] } {
    if (this.active) throw new Error('A paint history operation is already active.');
    const discarded = this.operations.splice(this.cursor);
    const operation: PaintHistoryOperation = { id: this.nextId++, layer, side: 0, tiles: 0 };
    this.active = operation;
    return { operation, discarded };
  }

  activeOperation(): PaintHistoryOperation | null {
    return this.active;
  }

  commit(tiles: number): PaintHistoryOperation[] {
    const operation = this.active;
    if (!operation) return [];
    this.active = null;
    if (tiles === 0) return [];
    operation.tiles = tiles;
    this.operations.push(operation);
    this.cursor = this.operations.length;
    if (this.operations.length <= PAINT_HISTORY_MAX_OPERATIONS) return [];
    const discarded = this.operations.splice(0, this.operations.length - PAINT_HISTORY_MAX_OPERATIONS);
    this.cursor = this.operations.length;
    return discarded;
  }

  cancel(): PaintHistoryOperation | null {
    const operation = this.active;
    this.active = null;
    return operation;
  }

  undoTarget(): PaintHistoryOperation | null {
    return this.cursor > 0 ? this.operations[this.cursor - 1] : null;
  }

  redoTarget(): PaintHistoryOperation | null {
    return this.cursor < this.operations.length ? this.operations[this.cursor] : null;
  }

  completeUndo(): void {
    if (this.cursor > 0) this.cursor--;
  }

  completeRedo(): void {
    if (this.cursor < this.operations.length) this.cursor++;
  }

  removeOldest(protectedId = -1): PaintHistoryOperation | null {
    if (this.operations.length === 0 || this.operations[0].id === protectedId) return null;
    const operation = this.operations.shift()!;
    if (this.cursor > 0) this.cursor--;
    return operation;
  }

  reset(): PaintHistoryOperation[] {
    const discarded = this.operations;
    this.operations = [];
    this.cursor = 0;
    this.active = null;
    return discarded;
  }

  canUndo(): boolean {
    return this.cursor > 0;
  }

  canRedo(): boolean {
    return this.cursor < this.operations.length;
  }
}
