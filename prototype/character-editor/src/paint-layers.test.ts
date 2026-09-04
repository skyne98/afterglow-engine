import { describe, expect, test } from 'bun:test';
import { buildPaintLayerRows, type PaintGroupInfo, type PaintLayerInfo } from './paint-layers.ts';

const layer = (id: number, group = -1): PaintLayerInfo => ({ id, group, active: id === 0, visible: 1, opacity: 1, mode: 0 });
const group = (id: number, parent = -1): PaintGroupInfo => ({ id, parent, alive: true, visible: 1, opacity: 1, mode: 0, passThrough: 0, isolated: 0 });

describe('paint layer rows', () => {
  test('builds group hierarchy and keeps top layers in stack order', () => {
    const rows = buildPaintLayerRows(
      [layer(0), layer(1, 0), layer(2, 1), layer(3)],
      [group(0), group(1, 0)],
    );
    expect(rows.map(({ kind, id, depth }) => `${kind}:${id}:${depth}`)).toEqual([
      'group:0:0',
      'group:1:1',
      'layer:2:2',
      'layer:1:1',
      'layer:3:0',
      'layer:0:0',
    ]);
  });

  test('shows orphan layers and does not repeat cyclic groups', () => {
    const groups = [group(0, 1), group(1, 0)];
    const rows = buildPaintLayerRows([layer(0, 9)], groups);
    expect(rows.filter((row) => row.kind === 'group')).toHaveLength(2);
    expect(rows.at(-1)).toMatchObject({ kind: 'layer', id: 0, depth: 0 });
  });
});
