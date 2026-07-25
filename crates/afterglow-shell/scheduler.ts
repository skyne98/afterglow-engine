// Deliberate Prioritized Task Scheduling subset for the native shell.
// Only Scheduler.yield() is exposed; postTask/TaskController are unsupported.
// JavaScript-compatible TypeScript: production evaluates this directly in V8.
(() => {
  const native = globalThis.__afterglowSchedulerNative;
  if (native === undefined)
    throw new Error('native scheduler hooks are unavailable');

  const constructionToken = Symbol('afterglow Scheduler construction');
  let installedScheduler = null;

  class Scheduler {
    constructor(token) {
      if (token !== constructionToken) throw new TypeError('Illegal constructor');
    }
  }

  const yieldContinuation = function () {
    if (this !== installedScheduler) throw new TypeError('Illegal invocation');
    return new Promise((resolve) => native.defer(resolve));
  };
  Object.defineProperty(yieldContinuation, 'name', { value: 'yield', configurable: true });
  Object.defineProperty(Scheduler.prototype, 'yield', {
    value: yieldContinuation,
    writable: true,
    enumerable: true,
    configurable: true,
  });
  Object.defineProperty(Scheduler.prototype, Symbol.toStringTag, {
    value: 'Scheduler',
    configurable: true,
  });

  installedScheduler = new Scheduler(constructionToken);
  let currentScheduler = installedScheduler;
  Object.defineProperty(globalThis, 'Scheduler', {
    value: Scheduler,
    writable: true,
    configurable: true,
  });
  Object.defineProperty(globalThis, 'scheduler', {
    get: () => currentScheduler,
    set: (value) => { currentScheduler = value; },
    enumerable: true,
    configurable: true,
  });

  delete globalThis.__afterglowSchedulerNative;
})();
