import { describe, expect, test } from 'bun:test';
import { buildPaintLayerRows, samePaintLayerState, type PaintGroupInfo, type PaintLayerInfo } from './paint-layers.ts';

const layer = (id: number, group = -1): PaintLayerInfo => ({ id, group, active: id === 0, visible: 1, opacity: 1, mode: 0 });
const group = (id: number, parent = -1): PaintGroupInfo => ({ id, parent, alive: true, visible: 1, opacity: 1, mode: 0, passThrough: 0, isolated: 0 });

describe('paint layer rows', () => {
  test('retains the panel for equal snapshots and detects every displayed field', () => {
    const state = { activeLayer: 0, layers: [layer(0), layer(1)], groups: [group(0), group(1)] };
    expect(samePaintLayerState(null, state)).toBe(false);
    expect(samePaintLayerState(state, structuredClone(state))).toBe(true);
    expect(samePaintLayerState(state, { ...state, activeLayer: 1 })).toBe(false);
    for (const key of ['layers', 'groups'] as const) {
      const shorter = structuredClone(state);
      shorter[key].pop();
      expect(samePaintLayerState(state, shorter)).toBe(false);
      const reordered = structuredClone(state);
      reordered[key].reverse();
      expect(samePaintLayerState(state, reordered)).toBe(false);
      for (const field of Object.keys(state[key][0])) {
        const changed = structuredClone(state);
        const item = changed[key][0] as unknown as Record<string, number | boolean>;
        item[field] = typeof item[field] === 'boolean' ? !item[field] : Number(item[field]) + 1;
        expect(samePaintLayerState(state, changed)).toBe(false);
      }
    }
  });
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
