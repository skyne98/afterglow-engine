import { readFile, writeFile } from 'node:fs/promises';

export function memoryTrend(samples: { time: number; processTreePssKiB?: number; values?: string[] }[]) {
  if (samples.length < 18) throw new Error('Insufficient process memory samples');
  const end = samples.at(-1)!.time;
  const tail = samples.filter(sample => sample.time >= end - 180_000);
  const values = tail.map(sample => ({ time: sample.time, bytes: 1024 * (sample.processTreePssKiB ??
    Number(sample.values?.find(value => value.startsWith('VmRSS:'))?.match(/\d+/)?.[0] ?? 0)) }));
  if (values.some(sample => sample.bytes <= 0)) throw new Error('Missing process memory');
  const first = values.filter(sample => sample.time < end - 120_000);
  const last = values.filter(sample => sample.time >= end - 60_000);
  if (first.length < 4 || last.length < 4) throw new Error('Insufficient final memory intervals');
  const before = Math.min(...first.map(sample => sample.bytes));
  const after = Math.min(...last.map(sample => sample.bytes));
  // This measured floor permits GC variation, not unbounded memory growth.
  const tolerance = Math.max(32 * 1024 * 1024, before * 0.05);
  return { before, after, growth: after - before, tolerance, passed: after - before <= tolerance };
}

export function compare(native: any, web: any) {
  for (const result of [native, web]) {
    if (result.width !== 16384 || result.height !== 16384 || result.radius !== 60 || result.elapsedMs < 600_000 || result.strokes <= 256)
      throw new Error('Incomplete 16K ten-minute check');
    if (result.comparison?.length !== 8 || result.comparison.some((row: number[]) => row.length !== 16384))
      throw new Error('Incomplete comparison rows');
    if (result.state?.error || !result.frames?.count) throw new Error('Paint error or missing frame data');
  }
  let differences = 0, maximumDifference = 0;
  for (let row = 0; row < 8; row++) for (let index = 0; index < 16384; index++) {
    const difference = Math.abs(native.comparison[row][index] - web.comparison[row][index]);
    if (difference) differences++;
    maximumDifference = Math.max(maximumDifference, difference);
  }
  const nativeMemory = memoryTrend(native.memorySamples), webMemory = memoryTrend(web.memorySamples);
  const admission = native.state.sharedMemory;
  const admitted = admission && admission.reservedBytes <= admission.limitBytes && admission.peakBytes <= admission.limitBytes;
  const diskBounded = native.state.scratchPeakFileBytes > 0 && native.state.scratchPeakFileBytes <= 64 * 1024 ** 3;
  return { passed: differences === 0 && nativeMemory.passed && webMemory.passed && !!admitted && diskBounded,
    comparedBytes: 8 * 16384, differences, maximumDifference, nativeMemory, webMemory, admitted: !!admitted, diskBounded,
    nativeFrames: native.frames, webFrames: web.frames,
    limits: 'Eight display rows, not all document pixels. Memory floors cover the final three minutes. rAF is not GPU presentation.' };
}

if (import.meta.main) {
  const [nativePath, webPath, output] = process.argv.slice(2);
  if (!nativePath || !webPath || !output) throw new Error('Supply native result, web result, and report paths');
  const native = JSON.parse(await readFile(nativePath, 'utf8')).results[0];
  const web = JSON.parse(await readFile(webPath, 'utf8')).results[0];
  const report = compare(native, web);
  await writeFile(output, JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report));
  if (!report.passed) process.exitCode = 1;
}
