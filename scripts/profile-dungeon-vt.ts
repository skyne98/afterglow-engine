#!/usr/bin/env bun

const AGTB_HEADER_BYTES = 40;
const TRACE_RECORD_BYTES = 40;
const NS_PER_SECOND = 1_000_000_000;

const TRACE_NAMES = [
  'frame', 'worker.poll', 'vt.update', 'structural.commands', 'pose.batches',
  'render.prepare', 'game.update', 'render.passes', 'asset.session.open',
  'asset.size', 'asset.read', 'asset.read_bulk', 'rpc.call', 'vt.page_load',
  'vt.bulk_wait', 'asset.bulk_dispatch', 'texture.transcode_queue',
  'texture.transcode', 'vt.upload', 'cache.read', 'cache.write', 'mesh.optimize',
  'vt.feedback_detected', 'vt.scheduler_wait', 'vt.page_published',
] as const;

export interface AgtbHeader {
  sourceId: number;
  epoch: number;
  clockDomain: number;
  flags: number;
  recordCount: number;
  droppedRecords: number;
  ticksPerSecond: number;
}

export interface TraceStageSummary {
  name: string;
  records: number;
  operations: number;
  totalMs: number;
  meanMs: number;
  p50Ms: number;
  p95Ms: number;
  p99Ms: number;
  maxMs: number;
  argument0Total: number;
  statuses: Record<string, number>;
}

function u64(view: DataView, offset: number): number {
  return view.getUint32(offset, true) + view.getUint32(offset + 4, true) * 0x1_0000_0000;
}

function percentile(sorted: readonly number[], fraction: number): number {
  if (sorted.length === 0) return 0;
  return sorted[Math.min(sorted.length - 1, Math.floor((sorted.length - 1) * fraction))] ?? 0;
}

export function validateAgtb(bytes: Uint8Array): AgtbHeader {
  if (bytes.byteLength < AGTB_HEADER_BYTES) throw new Error('AGTB input is shorter than its header');
  if (String.fromCharCode(bytes[0]!, bytes[1]!, bytes[2]!, bytes[3]!) !== 'AGTB')
    throw new Error('AGTB magic mismatch');
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const version = view.getUint16(4, true);
  const headerBytes = view.getUint16(6, true);
  if (version !== 1) throw new Error(`unsupported AGTB version ${version}`);
  if (headerBytes !== AGTB_HEADER_BYTES) throw new Error(`invalid AGTB header length ${headerBytes}`);
  const recordCount = view.getUint32(24, true);
  const expectedBytes = AGTB_HEADER_BYTES + recordCount * TRACE_RECORD_BYTES;
  if (bytes.byteLength !== expectedBytes)
    throw new Error(`AGTB length mismatch: expected ${expectedBytes}, got ${bytes.byteLength}`);
  const ticksPerSecond = u64(view, 32);
  if (ticksPerSecond !== NS_PER_SECOND)
    throw new Error(`browser AGTB tick rate must be ${NS_PER_SECOND}, got ${ticksPerSecond}`);
  return {
    sourceId: view.getUint32(8, true),
    epoch: view.getUint32(12, true),
    clockDomain: view.getUint32(16, true),
    flags: view.getUint32(20, true),
    recordCount,
    droppedRecords: view.getUint32(28, true),
    ticksPerSecond,
  };
}

export function aggregateAgtb(bytes: Uint8Array): {
  header: AgtbHeader;
  unmatchedStarts: number;
  stages: TraceStageSummary[];
  perceptualPriorityBuckets: number[];
  bulkWaitTierStarts: number[];
} {
  const header = validateAgtb(bytes);
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const starts = new Map<string, number[]>();
  const perceptualPriorityBuckets = new Array<number>(25).fill(0);
  const bulkWaitTierStarts = new Array<number>(3).fill(0);
  const raw = TRACE_NAMES.map(name => ({
    name, records: 0, operations: 0, durations: [] as number[], argument0Total: 0,
    statuses: {} as Record<string, number>,
  }));
  for (let index = 0; index < header.recordCount; index++) {
    const base = AGTB_HEADER_BYTES + index * TRACE_RECORD_BYTES;
    const descriptor = view.getUint32(base + 32, true);
    const stage = raw[descriptor];
    if (!stage) continue;
    stage.records++;
    const phase = view.getUint8(base + 36);
    if (descriptor === 22 && phase === 1) {
      const priority = u64(view, base + 16);
      const bucket = Math.floor(priority / 6);
      if (bucket >= 0 && bucket < perceptualPriorityBuckets.length)
        perceptualPriorityBuckets[bucket]++;
    }
    if (descriptor === 14 && phase === 4) {
      const tier = u64(view, base + 24);
      if (tier >= 0 && tier < bulkWaitTierStarts.length) bulkWaitTierStarts[tier]++;
    }
    const correlationLow = view.getUint32(base + 8, true);
    const correlationHigh = view.getUint32(base + 12, true);
    const key = `${descriptor}:${correlationHigh}:${correlationLow}`;
    if (phase === 2 || phase === 4) {
      let stack = starts.get(key);
      if (!stack) { stack = []; starts.set(key, stack); }
      stack.push(u64(view, base));
      continue;
    }
    if (phase !== 3 && phase !== 5) continue;
    const stack = starts.get(key);
    const begin = stack?.pop();
    if (begin !== undefined) {
      stage.operations++;
      stage.durations.push(u64(view, base) - begin);
    }
    stage.argument0Total += u64(view, base + 16);
    if (descriptor === 13 || descriptor === 19 || descriptor === 20 || descriptor === 23) {
      const status = String(u64(view, base + 24));
      stage.statuses[status] = (stage.statuses[status] ?? 0) + 1;
    }
  }
  let unmatchedStarts = 0;
  for (const stack of starts.values()) unmatchedStarts += stack.length;
  const stages = raw.filter(stage => stage.records !== 0).map(stage => {
    stage.durations.sort((left, right) => left - right);
    const totalNs = stage.durations.reduce((sum, value) => sum + value, 0);
    return {
      name: stage.name,
      records: stage.records,
      operations: stage.operations,
      totalMs: totalNs / 1_000_000,
      meanMs: stage.operations === 0 ? 0 : totalNs / stage.operations / 1_000_000,
      p50Ms: percentile(stage.durations, 0.50) / 1_000_000,
      p95Ms: percentile(stage.durations, 0.95) / 1_000_000,
      p99Ms: percentile(stage.durations, 0.99) / 1_000_000,
      maxMs: percentile(stage.durations, 1) / 1_000_000,
      argument0Total: stage.argument0Total,
      statuses: stage.statuses,
    };
  });
  return {
    header, unmatchedStarts, stages, perceptualPriorityBuckets, bulkWaitTierStarts,
  };
}

class CdpSession {
  private nextId = 1;
  private readonly pending = new Map<number, { resolve(value: unknown): void; reject(error: Error): void }>();

  private constructor(private readonly socket: WebSocket) {
    socket.addEventListener('message', event => {
      const message = JSON.parse(String(event.data)) as {
        id?: number;
        error?: unknown;
        result?: { result?: { value?: unknown }; exceptionDetails?: unknown };
      };
      if (message.id === undefined) return;
      const pending = this.pending.get(message.id);
      if (!pending) return;
      this.pending.delete(message.id);
      if (message.error || message.result?.exceptionDetails)
        pending.reject(new Error(JSON.stringify(message.error ?? message.result?.exceptionDetails)));
      else pending.resolve(message.result?.result?.value);
    });
  }

  static async connect(url: string): Promise<CdpSession> {
    const socket = new WebSocket(url);
    await new Promise<void>((resolve, reject) => {
      socket.addEventListener('open', () => resolve(), { once: true });
      socket.addEventListener('error', () => reject(new Error('CDP WebSocket connection failed')), { once: true });
    });
    return new CdpSession(socket);
  }

  evaluate(expression: string): Promise<unknown> {
    const id = this.nextId++;
    this.socket.send(JSON.stringify({
      id,
      method: 'Runtime.evaluate',
      params: { expression, awaitPromise: true, returnByValue: true },
    }));
    return new Promise((resolve, reject) => this.pending.set(id, { resolve, reject }));
  }

  close(): void { this.socket.close(); }
}

interface Options {
  cdp: string;
  scenario: 'traverse' | 'teleport';
  durationMs: number;
  outputPrefix: string;
}

function parseOptions(args: readonly string[]): Options {
  const options: Options = {
    cdp: '127.0.0.1:9333', scenario: 'traverse', durationMs: 4050,
    outputPrefix: `docs/benchmarks/dungeon-vt-profile-${new Date().toISOString().slice(0, 10)}`,
  };
  for (let index = 0; index < args.length; index++) {
    const name = args[index];
    const value = args[index + 1];
    if (name === '--cdp' && value) { options.cdp = value; index++; }
    else if (name === '--scenario' && (value === 'traverse' || value === 'teleport')) {
      options.scenario = value; index++;
    } else if (name === '--duration-ms' && value) { options.durationMs = Number(value); index++; }
    else if (name === '--output-prefix' && value) { options.outputPrefix = value; index++; }
    else throw new Error(`unknown or incomplete argument: ${name ?? '<missing>'}`);
  }
  if (!Number.isFinite(options.durationMs) || options.durationMs < 100)
    throw new Error('--duration-ms must be at least 100');
  return options;
}

function scenarioExpression(options: Options, epoch: number): string {
  const scenario = JSON.stringify(options.scenario);
  return `(async()=>{
    const a=window.__afterglowDungeon;
    if(!a||!a.ready())throw new Error('Dungeon harness unavailable');
    const waitIdle=async(timeout)=>{const end=performance.now()+timeout;while(performance.now()<end){const s=a.telemetry();if(!s.pendingPages&&!s.scheduledRequests&&!s.readyUploads&&!s.activeTranscodes&&!s.queuedTranscodes&&!s.bulkInFlight)return;await new Promise(r=>requestAnimationFrame(r))}throw new Error('VT pipeline did not drain')};
    await waitIdle(15000);
    const before={...a.telemetry()},scenario=${scenario},durationMs=${options.durationMs};
    a.setProgrammatic(true);a.setHudVisible(false);
    if(!a.traceArm(${epoch}))throw new Error('trace arm failed');
    const poses=[[-7.4,-7.4,0],[7.4,-7.4,3.14159],[7.4,7.4,3.14159],[-7.4,7.4,0],[-2.6,-4,-1.57],[2.6,-4,1.57],[-1.6,3,-1.57],[3.6,6,1.57],[-5.5,-5.5,0]];
    const frames=[];let previous=-1,pose=-1,started=performance.now();
    await new Promise(resolve=>{function tick(now){if(previous>=0)frames.push(now-previous);previous=now;const elapsed=now-started;if(scenario==='traverse'){const u=(elapsed%8000)/8000;a.setPose(-7.45+14.9*(u<.5?u*2:(1-u)*2),-7.65,0,0)}else{const next=Math.min(poses.length-1,Math.floor(elapsed/450));if(next!==pose){pose=next;const p=poses[next];a.setPose(p[0],p[1],p[2],0)}}if(elapsed<durationMs)requestAnimationFrame(tick);else resolve()}requestAnimationFrame(tick)});
    await waitIdle(30000);
    const snapshot=a.traceStop();if(!snapshot)throw new Error('trace stop failed');
    const after={...a.telemetry()};a.setPose(-5.5,-5.5,0,0);a.setProgrammatic(false);a.setHudVisible(true);
    frames.sort((x,y)=>x-y);const pct=p=>frames.length?frames[Math.min(frames.length-1,Math.floor((frames.length-1)*p))]:0;
    return {scenario,durationMs:performance.now()-started,frames:frames.length,frameMeanMs:frames.reduce((x,y)=>x+y,0)/frames.length,frameP95Ms:pct(.95),frameP99Ms:pct(.99),frameMaxMs:pct(1),below60:frames.filter(x=>x>1000/60).length,before,after,trace:{count:snapshot.count,dropped:snapshot.dropped,epoch:snapshot.epoch},errors:a.errorCount()};
  })()`;
}

async function readTraceBatch(session: CdpSession): Promise<Uint8Array> {
  const length = Number(await session.evaluate('window.__afterglowDungeon.traceBatch()?.byteLength ?? 0'));
  if (!Number.isInteger(length) || length < AGTB_HEADER_BYTES) throw new Error('frozen trace batch unavailable');
  const output = new Uint8Array(length);
  const chunkBytes = 24 * 1024;
  for (let offset = 0; offset < length; offset += chunkBytes) {
    const end = Math.min(length, offset + chunkBytes);
    const encoded = String(await session.evaluate(
      `(()=>{const b=window.__afterglowDungeon.traceBatch().subarray(${offset},${end});let s='';for(let i=0;i<b.length;i++)s+=String.fromCharCode(b[i]);return btoa(s)})()`,
    ));
    output.set(Uint8Array.fromBase64(encoded), offset);
  }
  return output;
}

async function main(): Promise<void> {
  const options = parseOptions(Bun.argv.slice(2));
  const targets = await (await fetch(`http://${options.cdp}/json/list`)).json() as Array<{
    type: string; url: string; webSocketDebuggerUrl: string;
  }>;
  const target = targets.find(entry => entry.type === 'page' && new URL(entry.url).pathname.endsWith('/dungeon.html'));
  if (!target) throw new Error(`no Dungeon page target on ${options.cdp}`);
  const session = await CdpSession.connect(target.webSocketDebuggerUrl);
  try {
    const environment = await session.evaluate(`(async()=>{const adapter=await navigator.gpu?.requestAdapter();return {isolated:crossOriginIsolated,gpu:!!navigator.gpu,ready:!!window.__afterglowDungeon?.ready(),vendor:adapter?.info?.vendor??'',architecture:adapter?.info?.architecture??'',description:adapter?.info?.description??''}})()`);
    const env = environment as { isolated: boolean; gpu: boolean; ready: boolean; vendor: string; architecture: string; description: string };
    if (!env.isolated || !env.gpu || !env.ready) throw new Error(`invalid Dungeon environment: ${JSON.stringify(env)}`);
    const adapter = `${env.vendor} ${env.architecture} ${env.description}`.toLowerCase();
    if (!env.vendor || /swiftshader|software|llvmpipe|cpu/.test(adapter))
      throw new Error(`software or unidentified WebGPU adapter: ${JSON.stringify(env)}`);
    const epoch = Number(new Date().toISOString().slice(0, 10).replaceAll('-', ''));
    const scenario = await session.evaluate(scenarioExpression(options, epoch)) as Record<string, unknown>;
    const batch = await readTraceBatch(session);
    const aggregate = aggregateAgtb(batch);
    if (aggregate.header.epoch !== epoch) throw new Error(`capture epoch mismatch: ${aggregate.header.epoch} != ${epoch}`);
    if (aggregate.header.droppedRecords !== 0) throw new Error(`trace dropped ${aggregate.header.droppedRecords} records`);
    if (aggregate.unmatchedStarts !== 0) throw new Error(`trace has ${aggregate.unmatchedStarts} unmatched starts`);
    if (scenario.errors !== 0) throw new Error(`Dungeon reported ${scenario.errors} errors`);
    const after = scenario.after as Record<string, number>;
    for (const field of ['pendingPages', 'scheduledRequests', 'readyUploads', 'activeTranscodes', 'queuedTranscodes', 'bulkInFlight']) {
      if ((after[field] ?? 0) !== 0) throw new Error(`final ${field} is ${after[field]}`);
    }
    await Bun.write(`${options.outputPrefix}.agtb`, batch);
    await Bun.write(`${options.outputPrefix}.json`, JSON.stringify({
      capturedAt: new Date().toISOString(), adapter: env, ...scenario, aggregate,
    }, null, 2) + '\n');
    console.log(JSON.stringify({
      agtb: `${options.outputPrefix}.agtb`, json: `${options.outputPrefix}.json`,
      records: aggregate.header.recordCount, scenario: options.scenario,
    }));
  } finally {
    session.close();
  }
}

if (import.meta.main) await main();
