/**
 * Proposed application-independent diagnostics contract, v0.1.0.
 * DESIGN ONLY: this declaration file has no runtime implementation.
 * See docs/implementation/generic-diagnostics-plan.md for decisions and status.
 */

export type Brand<T, N extends string> = T & { readonly __brand: N };
export type Id<K extends string> = Brand<string, K>;
export type SessionId = Id<"session">;
export type ProducerId = Id<"producer">;
export type ClockId = Id<"clock">;
export type CaptureId = Id<"capture">;
export type Revision = Brand<number, "revision">;
export type Detail = "flight" | "trace" | "deep";
export type Mode = "off" | Detail;
export type Outcome = "ok" | "error" | "cancelled" | "timeout" | "abandoned";
export type Privacy = "public" | "sensitive" | "secret";
export type Unit = "ns" | "us" | "ms" | "s" | "bytes" | "count" | "ratio" | "Hz" | "percent" | (string & {});
export type Scalar = string | number | boolean | null;
export type Fields = Readonly<Record<string, Scalar | Ref>>;

export type RefKind = "operation" | "slice" | "event" | "resource" | "epoch" | "queue" | "ticket" | "snapshot" | "artifact" | "track";
/** IDs are opaque in application code; compact producer-local integers on the wire. */
export interface Ref<K extends RefKind = RefKind> {
  readonly kind: K;
  readonly session: SessionId;
  readonly producer: ProducerId;
  readonly id: Id<K>;
  readonly generation: number;
}
export type ResourceRef = Ref<"resource">;
export type OperationRef = Ref<"operation">;
export type EventRef = Ref<"event">;

/** Never substitute Unix wall-clock time for this value. */
export interface Instant {
  readonly clock: ClockId;
  readonly ticks: bigint;
}
export interface TimeRange { readonly start: Instant; readonly end: Instant }
export interface Quality {
  readonly method: "direct" | "sampled" | "estimated" | "bounded";
  readonly provider: string;
  readonly resolutionNs?: number;
  readonly uncertaintyNs?: number;
  readonly sampleProbability?: number;
  readonly notes?: readonly string[];
  readonly valueBounds?: { readonly lower: number; readonly upper: number; readonly unit: Unit };
}
export type MissingReason = "unsupported" | "disabled" | "denied" | "not-captured" | "dropped" | "redacted" | "failed" | "not-applicable";
export type Observation<T> =
  | { readonly state: "value"; readonly value: T; readonly quality: Quality }
  | { readonly state: "missing"; readonly reason: MissingReason; readonly detail?: string };

export interface Context {
  readonly session: SessionId;
  readonly traceId: Id<"trace">;
  readonly parent?: OperationRef;
  readonly correlations: readonly Ref[];
}
/** Carries diagnostic IDs only. It does not carry credentials or arbitrary baggage. */
export interface ContextCarrier {
  readonly version: 1;
  readonly session: string;
  readonly traceId: string;
  readonly parent?: { readonly producer: string; readonly id: string; readonly generation: number };
  readonly correlations: readonly Ref[];
}
export interface ContextApi {
  root(correlations?: readonly Ref[]): Context;
  /** Synchronous dynamic scope only. Do not keep it active across await. */
  current(): Context | undefined;
  inject(context: Context): ContextCarrier;
  extract(carrier: unknown): Result<Context>;
  /** Restores context for the synchronous invocation, not the lifetime of a returned Promise. */
  bind<A extends readonly unknown[], R>(context: Context, fn: (...args: A) => R): (...args: A) => R;
}

export interface Field<T> {
  readonly type: "f64" | "u32" | "i32" | "bool" | "text" | "enum" | "ref";
  /** If absent, resolve from the bootstrap field policy at registration. */
  readonly privacy?: Privacy;
  readonly unit?: Unit;
  readonly maxBytes?: number;
  readonly values?: readonly string[];
  readonly __value?: T;
}
export type Schema = Readonly<Record<string, Field<unknown>>>;
export type Values<S extends Schema> = { readonly [K in keyof S]: S[K] extends Field<infer T> ? T : never };
export interface NumericFieldOptions { readonly unit?: Unit; readonly privacy?: Privacy; readonly min?: number; readonly max?: number }
export declare const field: {
  f64(options?: NumericFieldOptions): Field<number>;
  u32(options?: NumericFieldOptions): Field<number>;
  i32(options?: NumericFieldOptions): Field<number>;
  bool(options?: { readonly privacy?: Privacy }): Field<boolean>;
  text(options: { readonly maxBytes: number; readonly privacy: Privacy }): Field<string>;
  enum<const T extends readonly string[]>(values: T, options?: { readonly privacy?: Privacy }): Field<T[number]>;
  ref<K extends RefKind>(kind: K, options?: { readonly privacy?: Privacy }): Field<Ref<K>>;
};
export interface SourceRef {
  readonly buildId: string;
  readonly moduleId: string;
  readonly sourceId?: string;
  readonly line?: number;
  readonly column?: number;
  readonly symbol?: string;
}
export interface SiteOptions<S extends Schema> {
  readonly fields: S;
  readonly detail?: Detail;
  readonly source?: SourceRef;
  readonly description?: string;
}
export interface ScopeOptions {
  readonly context?: Context;
  readonly correlations?: readonly Ref[];
}
export interface Slice {
  readonly ref: Ref<"slice">;
  /** Idempotent. A scope guard must close the slice on exceptions. */
  end(outcome?: Outcome): void;
}
export interface SpanSite<S extends Schema> {
  readonly name: string;
  readonly enabled: boolean;
  begin(values: Values<S>, options?: ScopeOptions): Slice;
  /** Measures synchronous elapsed time; it does NOT claim CPU running time. */
  run<T>(values: Values<S>, work: (() => T) & (Extract<T, PromiseLike<unknown>> extends never ? unknown : never), options?: ScopeOptions): T;
}
export interface LinkedWork<T> {
  readonly ref: OperationRef;
  readonly result: Promise<T>;
}
export interface WaitOptions {
  readonly kind: "dependency" | "lock" | "io" | "gpu" | "timer" | "backpressure" | "external" | "unknown";
  readonly resource?: ResourceRef;
  readonly deadline?: Instant;
}
export interface Task {
  readonly ref: OperationRef;
  readonly context: Context;
  /** Await interval + a dependency edge; does not infer ready/running state. */
  wait<T>(work: LinkedWork<T>, options: WaitOptions): Promise<T>;
  waitAll<T extends readonly unknown[]>(work: { readonly [K in keyof T]: LinkedWork<T[K]> }, options: WaitOptions): Promise<T>;
  /** Records which work settled first. It does not cancel the other work. */
  waitAny<T>(work: readonly LinkedWork<T>[], options: WaitOptions): Promise<T>;
  /** For an uninstrumented Promise; the upstream cause remains unknown. */
  waitUnknown<T>(work: Promise<T>, options: WaitOptions): Promise<T>;
}
export interface TaskSite<S extends Schema> {
  readonly name: string;
  /** Creates a logical lifetime. Does not imply that this time was spent on a CPU. */
  run<T>(values: Values<S>, work: (task: Task) => Promise<T>, options?: ScopeOptions): Promise<T>;
  /** Same lifecycle, but exposes the operation ID before completion for dependencies. */
  start<T>(values: Values<S>, work: (task: Task) => Promise<T>, options?: ScopeOptions): LinkedWork<T>;
}
export interface EventSite<S extends Schema> {
  emit(values: Values<S>, options?: ScopeOptions): EventRef;
}
export interface LogSite<S extends Schema> {
  emit(values: Values<S>, options?: ScopeOptions & { readonly error?: unknown }): EventRef;
}

export type LabelSchema = Readonly<Record<string, readonly string[]>>;
export type Labels<L extends LabelSchema> = { readonly [K in keyof L]: L[K][number] };
export interface MetricOptions<L extends LabelSchema> {
  readonly unit: Unit;
  readonly privacy?: Privacy;
  readonly labels: L;
  readonly maxSeries: number;
  readonly description?: string;
}
export interface MetricContext {
  readonly context?: Context;
  readonly exemplar?: Ref;
  readonly quality?: Quality;
}
export interface Counter<L extends LabelSchema> { add(delta: number, labels: Labels<L>, options?: MetricContext): void }
export interface UpDownCounter<L extends LabelSchema> { add(delta: number, labels: Labels<L>, options?: MetricContext): void }
export interface Gauge<L extends LabelSchema> { set(value: number, labels: Labels<L>, options?: MetricContext): void }
export interface Histogram<L extends LabelSchema> { observe(value: number, labels: Labels<L>, options?: MetricContext): void }

export type Relation = "spawned" | "happens-before" | "unblocks" | "requires" | "sent-to" | "produced" | "consumed" | "invalidated" | "owns" | "aliases" | "derived-from" | "correlated";
export interface Link {
  readonly from: Ref;
  readonly to: Ref;
  readonly relation: Relation;
  readonly basis: "instrumented" | "inferred";
  readonly evidence?: readonly Ref[];
}
export interface ResourceType<S extends Schema> {
  created(values: Values<S>, options?: ScopeOptions & {
    readonly owner?: ResourceRef;
    readonly backing?: ResourceRef;
    readonly requestedBytes?: number;
  }): ResourceRef;
  /** Each accepted mutation advances the resource revision. */
  changed(ref: ResourceRef, changes: Partial<Values<S>>, options: ScopeOptions & {
    readonly cause: Ref;
    readonly reason: string;
  }): Revision;
  /** Logical release request, not evidence that the driver/allocator freed memory. */
  releaseRequested(ref: ResourceRef, options?: ScopeOptions): void;
  /** Call only when release at the provider's stated layer is observed. */
  released(ref: ResourceRef, options?: ScopeOptions): void;
}
export interface QueueTicket {
  readonly ref: Ref<"ticket">;
  ready(): void;
  started(track: Ref<"track">): void;
  finished(outcome: Outcome): void;
}
export interface QueueProbe {
  readonly ref: Ref<"queue">;
  /** These methods observe a scheduler. They do not implement a scheduler. */
  enqueued(work: OperationRef, options?: ScopeOptions & {
    readonly bytes?: number;
    readonly priority?: number;
    readonly deadline?: Instant;
    readonly ready?: boolean;
  }): QueueTicket;
  depth(value: number): void;
}

export interface InspectRequest {
  readonly target?: ResourceRef;
  readonly cursor?: string;
  readonly maxItems: number;
  readonly maxBytes: number;
  readonly signal: AbortSignal;
}
export interface InspectPage {
  readonly revision: Revision;
  readonly at: Instant;
  readonly consistency: "consistent" | "partial" | "stale";
  readonly truncated: boolean;
  readonly items: readonly { readonly ref?: Ref; readonly values: Fields }[];
  readonly nextCursor?: string;
}
/** Control-plane rejection. The broker converts this to a typed failure/receipt. */
export declare class CommandError extends Error {
  readonly code: "conflict" | "invalid" | "denied" | "timeout" | "cancelled" | "failed";
  constructor(code: CommandError["code"], message: string);
}
export interface CommandReceipt {
  readonly requestId: string;
  readonly status: "accepted" | "applied" | "rejected" | "cancelled" | "unknown";
  readonly revision?: Revision;
  readonly result?: Fields;
}
/** The broker checks both generations before enqueue and at the safe point. */
export interface CommandTarget {
  readonly system: string;
  readonly generation: number;
  readonly resource?: ResourceRef;
}
export interface CommandSpec<S extends Schema, R extends Fields> {
  readonly args: S;
  readonly safePoint: "owner-thread" | "tick-boundary" | "paused";
  readonly description: string;
  execute(args: Values<S>, request: {
    readonly target: CommandTarget;
    readonly expectedRevision?: Revision;
    readonly signal: AbortSignal;
    readonly dryRun: boolean;
  }): Promise<R>;
}
export interface InspectorRegistration {
  readonly id: string;
  readonly dispose: () => void;
}
export interface System {
  readonly id: string;
  span<const S extends Schema>(name: string, options: SiteOptions<S>): SpanSite<S>;
  task<const S extends Schema>(name: string, options: SiteOptions<S>): TaskSite<S>;
  event<const S extends Schema>(name: string, options: SiteOptions<S>): EventSite<S>;
  log<const S extends Schema>(name: string, options: SiteOptions<S> & { readonly level: "debug" | "info" | "warn" | "error" | "fatal" }): LogSite<S>;
  counter<const L extends LabelSchema>(name: string, options: MetricOptions<L>): Counter<L>;
  upDownCounter<const L extends LabelSchema>(name: string, options: MetricOptions<L>): UpDownCounter<L>;
  gauge<const L extends LabelSchema>(name: string, options: MetricOptions<L>): Gauge<L>;
  histogram<const L extends LabelSchema>(name: string, options: MetricOptions<L> & { readonly buckets: readonly number[] }): Histogram<L>;
  resource<const S extends Schema>(name: string, options: SiteOptions<S>): ResourceType<S>;
  queue(name: string, options: { readonly capacity: number; readonly discipline: "fifo" | "priority" | "work-stealing" | "other" }): QueueProbe;
  /** Runs only on an explicit request, never on the event-emission path. */
  inspector(name: string, read: (request: InspectRequest) => Promise<InspectPage>): InspectorRegistration;
  command<const S extends Schema, R extends Fields>(name: string, spec: CommandSpec<S, R>): InspectorRegistration;
  dispose(): void;
}

export type EpochKind = "input" | "simulation" | "render" | "submission" | "presentation" | (string & {});
export interface Epoch {
  readonly ref: Ref<"epoch">;
  readonly context: Context;
  milestone(name: string, options?: { readonly at?: Instant; readonly quality?: Quality }): EventRef;
  end(outcome?: Outcome): void;
}
export interface EpochApi {
  begin(kind: EpochKind, options: {
    readonly sequence: number;
    readonly owner: ResourceRef;
    readonly context?: Context;
    readonly scheduledAt?: Instant;
    readonly deadline?: Instant;
    readonly related?: readonly Ref[];
  }): Epoch;
}

export interface Capability {
  readonly id: string;
  readonly provider: string;
  readonly state: "enabled" | "available" | "unsupported" | "denied" | "disabled" | "failed";
  readonly scope: "application" | "realm" | "process" | "system";
  readonly requirements: readonly string[];
  readonly quality?: Quality;
  readonly reason?: string;
}
export interface BootstrapOptions {
  readonly application: string;
  readonly buildId: string;
  readonly mode: Mode;
  readonly limits: {
    readonly retainedBytes: number;
    readonly targetHistoryMs: number;
    readonly maxRecordBytes: number;
    readonly maxAttributeBytes: number;
    readonly maxMetricSeries: number;
    /** Registered systems, sites, schemas, clocks, and tracks share this limit. */
    readonly maxMetadataEntries: number;
    readonly maxMetadataBytes: number;
    /** Live handles and retained tombstones share this limit. */
    readonly maxReferences: number;
    readonly maxContextReferences: number;
    readonly maxProducers: number;
    readonly maxRecordsPerSecond: number;
    readonly maxBytesPerSecond: number;
    readonly maxArtifacts: number;
    readonly maxArtifactBytes: number;
    readonly maxPendingRequests: number;
    readonly maxRetainedReceipts: number;
  };
  readonly privacy: {
    readonly defaultFieldClass: Privacy;
    readonly retainSensitive: boolean;
    readonly retainSecrets: false;
    readonly payloads: "none" | "explicit-opt-in";
  };
}
export interface Diagnostics {
  readonly clock: { now(): Instant };
  readonly context: ContextApi;
  readonly epochs: EpochApi;
  system(options: { readonly id: string; readonly schemaVersion: number; readonly description?: string }): System;
  link(link: Link): void;
  capabilities(): readonly Capability[];
}
export declare function createDiagnostics(options: BootstrapOptions): Diagnostics;

// Provider surface: platform probes extend records, not the application-facing API.
export interface ClockDescription {
  readonly id: ClockId;
  readonly name: string;
  readonly ticksPerSecond: { readonly numerator: bigint; readonly denominator: bigint };
  readonly resolutionNs: number;
  readonly wrapsAfterTicks?: bigint;
}
export interface ClockMapping {
  readonly source: ClockId;
  readonly target: ClockId;
  readonly sourceAnchor: bigint;
  readonly targetAnchor: bigint;
  readonly rate: number;
  readonly uncertaintyNs: number;
  readonly validFrom: Instant;
  readonly validUntil: Instant;
}
export interface RecordHeader {
  readonly producer: ProducerId;
  readonly sequence: bigint;
  readonly at: Instant;
  readonly schemaId: number;
  readonly schemaVersion: number;
  readonly context?: Context;
  readonly quality: Quality;
}
export interface ProviderRecord {
  readonly header: RecordHeader;
  readonly kind: "slice" | "operation" | "event" | "metric" | "sample" | "resource" | "edge" | "snapshot" | "health";
  readonly payload: Readonly<Record<string, unknown>>;
}
export interface ProviderHost {
  readonly diagnostics: Diagnostics;
  registerClock(clock: ClockDescription): void;
  synchronize(mapping: ClockMapping): void;
  capability(value: Capability): void;
  registerSchema(id: string, version: number, schema: Readonly<Record<string, unknown>>): number;
  /** Slow/control path. Production probes use a generated bounded binary writer. */
  ingest(records: readonly ProviderRecord[]): void;
}
export interface ProbePlugin {
  readonly id: string;
  readonly version: string;
  readonly protocolRange: { readonly minimum: number; readonly maximum: number };
  readonly requiredCapabilities: readonly string[];
  readonly optionalCapabilities: readonly string[];
  probe(host: ProviderHost): Promise<readonly Capability[]>;
  start(host: ProviderHost, mode: Mode, signal: AbortSignal): Promise<void>;
  configure(mode: Mode): Promise<void>;
  checkpoint(signal: AbortSignal): Promise<void>;
  stop(): Promise<void>;
}

// Local inspector API. No roles, tokens, or permission negotiation.
export type Result<T> =
  | { readonly ok: true; readonly value: T }
  | { readonly ok: false; readonly code: "unsupported" | "denied" | "timeout" | "conflict" | "invalid" | "partial" | "cancelled" | "unknown-outcome" | "failed"; readonly message: string };
export interface CaptureRequest {
  readonly mode: Detail;
  readonly preRollMs: number;
  readonly postRollMs: number;
  readonly maxBytes: number;
  readonly reason: string;
  readonly systems?: readonly string[];
}
export interface TriggerSpec {
  readonly id: string;
  readonly when:
    | { readonly kind: "deadline-miss"; readonly epoch: EpochKind; readonly consecutive: number }
    | { readonly kind: "metric"; readonly name: string; readonly comparison: ">" | ">=" | "<"; readonly value: number; readonly windowMs: number }
    | { readonly kind: "event"; readonly name: string }
    | { readonly kind: "heartbeat-missed"; readonly producer: string; readonly timeoutMs: number };
  readonly capture: CaptureRequest;
  readonly cooldownMs: number;
  readonly maximumCaptures: number;
}
export interface CaptureSummary {
  readonly id: CaptureId;
  readonly actualRange: TimeRange;
  readonly requested: CaptureRequest;
  readonly bytes: number;
  readonly lostRecords: Observation<number>;
  readonly missingProviders: readonly string[];
  readonly truncated: boolean;
  readonly debuggerAttached: boolean;
  readonly measurementPerturbations: readonly string[];
}
export interface Artifact {
  readonly ref: Ref<"artifact">;
  readonly mediaType: string;
  readonly byteLength: number;
  readonly hash: string;
}
export interface DiagnosticFinding {
  readonly kind: "observed" | "inferred" | "unknown";
  readonly statement: string;
  readonly evidence: readonly Ref[];
  readonly alternatives: readonly string[];
  readonly missingSignals: readonly string[];
  readonly nextProbe?: { readonly capability: string; readonly reason: string; readonly expectedCost: string };
}
export interface Explanation {
  readonly target: Ref;
  readonly wallMs: Observation<number>;
  readonly criticalPath: readonly Ref[];
  readonly findings: readonly DiagnosticFinding[];
  readonly coverage: {
    readonly missingIntervals: readonly TimeRange[];
    readonly lostRecords: Observation<number>;
    readonly clockUncertaintyNs: Observation<number>;
    readonly unsupportedCapabilities: readonly string[];
  };
}
export interface PauseResult {
  readonly reached: readonly string[];
  readonly pending: readonly string[];
  readonly failed: readonly string[];
  readonly gpuStillRunning: boolean;
}
export interface ReplayManifest {
  readonly scope: "inputs-only" | "controlled-subsystems";
  readonly buildId: string;
  readonly checkpoints: readonly Ref<"snapshot">[];
  readonly unsupportedSources: readonly string[];
  readonly deterministicSystems: readonly string[];
  readonly comparison: "exact" | "tolerance";
}
export interface InspectorClient {
  capabilities(): Promise<Result<readonly Capability[]>>;
  readonly capture: {
    start(request: CaptureRequest): Promise<Result<{ readonly id: CaptureId }>>;
    stop(id: CaptureId): Promise<Result<CaptureSummary>>;
    arm(trigger: TriggerSpec): Promise<Result<void>>;
    disarm(id: string): Promise<Result<void>>;
    export(id: CaptureId, format: "native" | "perfetto" | "chrome-trace" | "otlp"): Promise<Result<Artifact>>;
  };
  readonly analysis: {
    explain(capture: CaptureId, target: Ref, options: { readonly question: "latency" | "work" | "memory" | "correctness"; readonly maxNodes: number }): Promise<Result<Explanation>>;
    /** Read-only, resource-limited query execution on a collector/offline store. */
    query(capture: CaptureId, sql: string, options: { readonly maxRows: number; readonly timeoutMs: number }): Promise<Result<readonly Fields[]>>;
    compare(left: CaptureId, right: CaptureId, options: { readonly metrics: readonly string[]; readonly matchEnvironment: boolean }): Promise<Result<Artifact>>;
  };
  readonly debug: {
    inspect(system: string, inspector: string, request: Omit<InspectRequest, "signal">): Promise<Result<InspectPage>>;
    invoke(target: CommandTarget, command: string, args: Fields, options: { readonly requestId: string; readonly expectedRevision?: Revision; readonly dryRun: boolean; readonly timeoutMs: number }): Promise<Result<CommandReceipt>>;
    commandStatus(requestId: string): Promise<Result<CommandReceipt>>;
    pause(options: { readonly participants: readonly string[]; readonly timeoutMs: number }): Promise<Result<PauseResult>>;
    resume(): Promise<Result<void>>;
    breakOn(spec: { readonly id: string; readonly event: string; readonly equals?: Fields; readonly once: boolean; readonly participants: readonly string[] }): Promise<Result<void>>;
    removeBreakpoint(id: string): Promise<Result<void>>;
    snapshot(options: { readonly providers: readonly string[]; readonly maxBytes: number; readonly timeoutMs: number }): Promise<Result<Artifact>>;
    /** Protocol adapters are negotiated; V8 CDP is not a full Chromium implementation. */
    attachProtocol(protocol: "v8-inspector" | "dap"): Promise<Result<{ readonly channel: string }>>;
  };
  readonly replay: {
    describe(capture: CaptureId): Promise<Result<ReplayManifest>>;
    run(capture: CaptureId, options: { readonly until?: Ref; readonly isolated: true }): Promise<Result<Artifact>>;
  };
  artifact(ref: Ref<"artifact">, options: { readonly offset: number; readonly length: number }): Promise<Result<Uint8Array>>;
  close(): Promise<void>;
}
export interface InspectorTransport {
  request(method: string, args: unknown, signal: AbortSignal): Promise<unknown>;
  close(): Promise<void>;
}
export declare function connectInspector(transport: InspectorTransport, options: {
  readonly protocolVersion: number;
}): Promise<Result<InspectorClient>>;
