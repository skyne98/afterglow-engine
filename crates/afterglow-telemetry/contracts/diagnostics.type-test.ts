import type {
  Capability, CommandTarget, InspectorClient, SpanSite,
} from './diagnostics.d.ts';

declare const span: SpanSite<{}>;
declare const mixed: () => number | Promise<number>;
declare const thenable: () => PromiseLike<number>;
declare const maybeThenable: () => undefined | PromiseLike<number>;
declare const inspector: InspectorClient;

const value: number = span.run({}, () => 1);
span.run({}, () => undefined);
span.run({}, () => { throw new Error('application error'); });
// @ts-expect-error A synchronous span cannot include an async callback.
span.run({}, async () => 1);
// @ts-expect-error A union with a Promise is not synchronous work.
span.run({}, mixed);
// @ts-expect-error PromiseLike values are not synchronous work.
span.run({}, thenable);
// @ts-expect-error An optional PromiseLike is not synchronous work.
span.run({}, maybeThenable);

const target: CommandTarget = { system: 'build.compiler', generation: 1 };
inspector.debug.invoke(target, 'cancel', {}, { requestId: '1', dryRun: true, timeoutMs: 100 });
// @ts-expect-error A command target must include its registry generation.
inspector.debug.invoke({ system: 'build.compiler' }, 'cancel', {}, { requestId: '2', dryRun: true, timeoutMs: 100 });

const scope: Capability['scope'] = 'application';
void value;
void scope;
