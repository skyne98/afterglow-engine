import { expect, test } from 'bun:test';
import { analyzeHostPauses, type HostRecord } from './analyze-native-paint-capture.ts';
const record = (name: string, phase: number, time: number, id = 0, value = 0): HostRecord => ({
  name, phase, time_ns: String(time * 1e6), correlation: String(id), argument0: String(value), argument1: '0',
});
test('pause analysis clips intervals and keeps unknown state and incomplete intervals explicit', () => {
  const result = analyzeHostPauses([
    record('host.focus', 1, 0, 0, 1),
    record('host.frame', 2, 1, 1),
    record('host.present', 1, 2),
    record('host.frame', 2, 3, 2),
    record('host.present_work', 2, 4, 3),
    record('host.focus', 1, 5, 0, 0),
    record('host.frame', 3, 6, 1),
    record('host.present_work', 3, 7, 3),
    record('host.frame', 3, 8, 2),
    record('host.present', 1, 10),
    record('host.frame', 2, 11, 4),
    record('host.runtime_turn', 3, 12, 5),
  ]);
  expect(result.missing_begins).toBe(1);
  expect(result.missing_ends).toBe(1);
  expect(result.unobserved_measurements).toContain('host.hud_scene');
  expect(result.unobserved_measurements).not.toContain('host.frame');
  expect(result.longest_gaps[0]).toMatchObject({ gap_ms: 8, frame_ms: 6, present_ms: 3, runtime_ms: 0,
    state: { 'host.focus': 1, 'host.occluded': 2, 'host.suspended': 2 },
    state_changes: [{ name: 'host.focus', value: 0, time_ns: '5000000' }],
  });
});
test('pause analysis rejects duplicate starts and reports bounded output', () => {
  const begin = record('host.frame', 2, 0, 1);
  expect(() => analyzeHostPauses([begin, begin])).toThrow('Duplicate');
  const result = analyzeHostPauses(Array.from({ length: 12 }, (_, i) => record('host.present', 1, i * i)));
  expect(result.gap_count).toBe(11);
  expect(result.longest_gaps).toHaveLength(8);
  expect(result.shown_limit).toBe(8);
});
