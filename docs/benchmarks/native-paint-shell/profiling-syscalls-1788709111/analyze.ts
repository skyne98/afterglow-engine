// Cold evidence analysis. Syscall wall times include tracing and scheduling effects.
const file = Bun.file(process.argv[2] ?? new URL('./syscalls.log', import.meta.url));
if (file.size > 128 * 1024 * 1024) throw new Error('Trace exceeds 128 MiB');
const lines = (await file.text()).trimEnd().split('\n');
const calls = lines.flatMap((text, index) => {
  const match = text.match(/^(\d+\.\d+) (\w+)\(.*<([\d.]+)>$/);
  return match ? [{ line: index + 1, time: match[1], call: match[2]!, ms: Number(match[3]) * 1000, text }] : [];
});
const groups: Record<string, { count: number; totalMs: number; maxMs: number }> = {};
for (const call of calls) {
  const group = groups[call.call] ??= { count: 0, totalMs: 0, maxMs: 0 };
  group.count++;
  group.totalMs += call.ms;
  group.maxMs = Math.max(group.maxMs, call.ms);
}
console.log(JSON.stringify({
  lines: lines.length, parsedCompletedCalls: calls.length, omittedLines: lines.length - calls.length,
  clock: 'strace realtime; no explicit mapping to the DGTL monotonic clock',
  groups, shownLimit: 20, longestCalls: calls.sort((a, b) => b.ms - a.ms).slice(0, 20),
}, null, 2));
