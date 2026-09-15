import './paint-capture-probe.ts';

const host = globalThis as typeof globalThis & {
  __documentLoaded?: boolean;
};
const wait = (ms: number) => new Promise<void>(resolve => setTimeout(resolve, ms));
async function run() {
  const deadline = performance.now() + 30_000;
  while (!host.__documentLoaded || !(window as typeof window & { __paintCaptureReady?: unknown }).__paintCaptureReady
      || !document.querySelector('.menu-bar [aria-haspopup="menu"]')) {
    if (performance.now() >= deadline) throw new Error('Native menu startup timeout');
    await wait(25);
  }
  await wait(3000);
  for (const trigger of Array.from(document.querySelectorAll<HTMLElement>('.menu-bar [aria-haspopup="menu"]'))) {
    const rect = trigger.getBoundingClientRect();
    console.log('[menu-idle-click]', JSON.stringify({ name: trigger.textContent,
      x: Math.round((rect.left + rect.width / 2) * devicePixelRatio),
      y: Math.round((rect.top + rect.height / 2) * devicePixelRatio) }));
    await wait(1000);
    const menu = document.querySelector<HTMLElement>('[role="menu"]');
    const state = { name: trigger.textContent, expanded: trigger.getAttribute('aria-expanded'),
      menu: menu?.outerHTML.slice(0, 500) ?? null };
    console.log('[menu-idle-state]', JSON.stringify(state));
    if (!menu || state.expanded !== 'true') throw new Error(`Native idle menu did not open: ${JSON.stringify(state)}`);
    console.log('[menu-idle-escape]');
    await wait(1000);
    if (document.querySelector('[role="menu"]')) throw new Error('Native idle menu did not close');
  }
  console.log('[menu-idle] PASS');
}
// Do not hold module evaluation open. This check must run after native readiness.
void run().catch(error => console.error('[menu-idle] FAIL', error instanceof Error ? error.stack : String(error)));
