import { describe, expect, test } from 'bun:test';
import { PAINT_HISTORY_MAX_OPERATIONS, PaintHistoryQueue } from './paint-history.ts';

describe('PaintHistoryQueue', () => {
  test('removes oldest operations and keeps the newest operations undoable', () => {
    const queue = new PaintHistoryQueue();
    let firstRemoved = 0;
    for (let layer = 0; layer <= PAINT_HISTORY_MAX_OPERATIONS; layer++) {
      queue.begin(layer);
      const removed = queue.commit(1);
      if (removed.length > 0) firstRemoved = removed[0].id;
    }
    expect(firstRemoved).toBe(1);
    let undoCount = 0;
    while (queue.undoTarget()) {
      queue.completeUndo();
      undoCount++;
    }
    expect(undoCount).toBe(PAINT_HISTORY_MAX_OPERATIONS);
  });

  test('deletes redo operations when a new operation starts', () => {
    const queue = new PaintHistoryQueue();
    queue.begin(0); queue.commit(2);
    queue.begin(1); queue.commit(3);
    queue.completeUndo();
    const { discarded } = queue.begin(2);
    expect(discarded).toHaveLength(1);
    expect(queue.commit(4)).toHaveLength(0);
    expect(queue.canRedo()).toBe(false);
  });
});
