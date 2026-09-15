// Cold analysis through the public CLI. Intervals are wall time, not CPU time.
export interface HostRecord {
  name: string;
  phase: number;
  time_ns: string;
  correlation: string;
  argument0: string;
  argument1: string;
}
interface Interval { start: bigint; end: bigint; name: string; argument0: string; argument1: string }
const ms = (ns: bigint) => Number(ns) / 1e6;
const stateNames = ['host.focus', 'host.occluded', 'host.suspended'];
export function analyzeHostPauses(records: readonly HostRecord[]) {
  const pending = new Map<string, HostRecord>();
  const intervals: Interval[] = [], gaps: { start: bigint; end: bigint }[] = [];
  let previous: bigint | undefined, missingBegins = 0;
  for (const record of records) {
    const time = BigInt(record.time_ns);
    if (record.name === 'host.present') {
      if (previous !== undefined) {
        if (time < previous) throw new Error('Non-monotonic presentation records');
        gaps.push({ start: previous, end: time });
      }
      previous = time;
    }
    const key = `${record.name}:${record.correlation}`;
    if (record.phase === 2 || record.phase === 4) {
      if (pending.has(key)) throw new Error('Duplicate interval begin');
      pending.set(key, record);
    } else if (record.phase === 3 || record.phase === 5) {
      const begin = pending.get(key);
      if (!begin) { missingBegins++; continue; }
      const start = BigInt(begin.time_ns);
      if (time < start) throw new Error('Negative interval');
      intervals.push({ start, end: time, name: record.name, argument0: begin.argument0, argument1: begin.argument1 });
      pending.delete(key);
    }
  }
  function overlap(name: string, start: bigint, end: bigint) {
    const ranges = intervals.filter(i => i.name === name && i.start < end && i.end > start)
      .map(i => ({ start: i.start < start ? start : i.start, end: i.end > end ? end : i.end }))
      .sort((a, b) => a.start < b.start ? -1 : a.start > b.start ? 1 : 0);
    let total = 0n, previousEnd = start;
    for (const range of ranges) {
      const from = range.start > previousEnd ? range.start : previousEnd;
      if (range.end > from) total += range.end - from;
      if (range.end > previousEnd) previousEnd = range.end;
    }
    return ms(total);
  }
  return {
    records: records.length, intervals: intervals.length, missing_begins: missingBegins, missing_ends: pending.size,
    unobserved_measurements: ['host.frame', 'host.runtime_turn', 'host.present_work', 'host.hud_scene', 'host.hud_composite', 'host.surface_present', 'host.redraw_request', ...stateNames]
      .filter(name => !records.some(record => record.name === name)),
    gap_count: gaps.length, shown_limit: 8,
    longest_gaps: gaps.sort((a, b) => ms(b.end - b.start) - ms(a.end - a.start)).slice(0, 8).map(gap => {
      const state: Record<string, number> = Object.fromEntries(stateNames.map(name => [name, 2]));
      const stateChanges = [];
      for (const record of records) {
        if (!stateNames.includes(record.name)) continue;
        const time = BigInt(record.time_ns);
        if (time <= gap.start) state[record.name] = Number(record.argument0);
        else if (time <= gap.end) stateChanges.push({ name: record.name, value: Number(record.argument0), time_ns: record.time_ns });
      }
      return {
        start_ns: gap.start.toString(), end_ns: gap.end.toString(), gap_ms: ms(gap.end - gap.start),
        frame_ms: overlap('host.frame', gap.start, gap.end),
        runtime_ms: overlap('host.runtime_turn', gap.start, gap.end),
        present_ms: overlap('host.present_work', gap.start, gap.end),
        hud_scene_ms: overlap('host.hud_scene', gap.start, gap.end),
        hud_composite_ms: overlap('host.hud_composite', gap.start, gap.end),
        surface_present_ms: overlap('host.surface_present', gap.start, gap.end),
        state, state_changes: stateChanges,
        redraw_requests: records.filter(r => r.name === 'host.redraw_request' && BigInt(r.time_ns) > gap.start && BigInt(r.time_ns) <= gap.end).map(r => r.time_ns),
        overlapping_rpc: intervals.filter(i => i.name === 'rpc.round_trip' && i.start < gap.end && i.end > gap.start)
          .map(i => ({ worker: i.argument0, method: i.argument1, duration_ms: ms(i.end - i.start) })),
      };
    }),
  };
}

if (import.meta.main) {
  if (process.argv.length !== 3) throw new Error('Expected one DGTL file');
  const records: HostRecord[] = [];
  let offset = 0;
  for (;;) {
    const result = Bun.spawnSync(['./target/release/afterglow-collector', 'records', process.argv[2]!, '0', '-', '10000', String(offset)]);
    if (result.exitCode) throw new Error(result.stderr.toString());
    const page = JSON.parse(result.stdout.toString());
    records.push(...page.records);
    if (page.next_offset === null) break;
    if (records.length >= 100_000 || page.next_offset <= offset) throw new Error('Analysis record limit reached');
    offset = page.next_offset;
  }
  console.log(JSON.stringify(analyzeHostPauses(records), null, 2));
}
