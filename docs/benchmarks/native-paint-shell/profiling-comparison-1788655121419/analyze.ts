// Cold analysis of the retained captures through the public CLI.
const root = import.meta.dirname;
const cli = new URL('../../../../target/release/afterglow-collector', import.meta.url).pathname;
const reports = [];
for (const run of ['capture-1', 'capture-2', 'capture-3']) {
  const directory = `${root}/${run}`;
  const files = Array.from(new Bun.Glob('*.dgtl').scanSync(directory));
  if (files.length !== 1) throw new Error('Expected one capture file');
  const result = Bun.spawnSync([cli, 'records', `${directory}/${files[0]}`, '0', '-', '10000']);
  if (result.exitCode) throw new Error(result.stderr.toString());
  const data = JSON.parse(result.stdout.toString());
  if (data.next_offset !== null) throw new Error('Incomplete record query');
  const pending = new Map<string, { start: bigint; worker: string; method: string }>();
  const spans: { start: bigint; end: bigint; ms: number; worker: string; method: string; bytes: string }[] = [];
  const gaps: { start: bigint; end: bigint; ms: number }[] = [];
  let previous: bigint | undefined;
  for (const record of data.records) {
    const time = BigInt(record.time_ns);
    if (record.name === 'host.present') {
      if (previous !== undefined) gaps.push({ start: previous, end: time, ms: Number(time - previous) / 1e6 });
      previous = time;
    }
    if (record.name !== 'rpc.round_trip') continue;
    if (record.phase === 4) {
      if (pending.has(record.correlation)) throw new Error('Duplicate RPC begin');
      pending.set(record.correlation, { start: time, worker: record.argument0, method: record.argument1 });
    } else if (record.phase === 5) {
      const begin = pending.get(record.correlation);
      if (!begin || time < begin.start) throw new Error('Invalid RPC interval');
      spans.push({ ...begin, end: time, ms: Number(time - begin.start) / 1e6, bytes: record.argument0 });
      pending.delete(record.correlation);
    }
  }
  if (pending.size) throw new Error('Incomplete RPC intervals');
  reports.push({ run, records: data.returned, pending: pending.size,
    longestGaps: gaps.sort((a, b) => b.ms - a.ms).slice(0, 8).map(gap => ({
      start_ms: Number(gap.start) / 1e6, end_ms: Number(gap.end) / 1e6, gap_ms: gap.ms,
      overlapping_rpc: spans.filter(span => span.start < gap.end && span.end > gap.start)
        .map(span => ({ method: span.method, worker: span.worker, ms: span.ms, bytes: span.bytes })),
    })), rpc: { count: spans.length, max_ms: Math.max(...spans.map(span => span.ms)) } });
}
await Bun.write(`${root}/host-gap-analysis.json`, JSON.stringify({ reports }, null, 2) + '\n');
console.log(JSON.stringify(reports.map(report => ({ run: report.run, rpc: report.rpc, largest_gap_ms: report.longestGaps[0]?.gap_ms }))));
