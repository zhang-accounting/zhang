// Tests of the ledger fetch layer, run with Node's built-in runner (Node >= 23.6 strips the types): pnpm run test.
// The hooks render into happy-dom; the modules load after it is registered, so React finds a DOM.
import { GlobalRegistrator } from '@happy-dom/global-registrator';
import assert from 'node:assert/strict';
import { after, test } from 'node:test';

GlobalRegistrator.register();
after(() => GlobalRegistrator.unregister());
// lets `act` flush effects and updates outside a test framework that sets it
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const { act, createElement } = await import('react');
const { createRoot } = await import('react-dom/client');
const { createStore, Provider } = await import('jotai');
const { ledgerChangedAtom, useLedgerQuery } = await import('./ledger.ts');

type Shown = { value?: string; firstLoad: boolean; refreshing: boolean };

/** Mounts a page that shows one ledger query, and records what it rendered. */
async function mountPage(query: () => Promise<string>, deps: unknown[] = []) {
  const store = createStore();
  const renders: Shown[] = [];
  function Page() {
    const { value, firstLoad, refreshing } = useLedgerQuery(async () => ({ data: { data: await query() } }), deps);
    renders.push({ value, firstLoad, refreshing });
    return null;
  }
  const root = createRoot(document.createElement('div'));
  await act(async () => root.render(createElement(Provider, { store }, createElement(Page))));
  return { store, renders, shown: () => renders[renders.length - 1], unmount: () => act(() => root.unmount()) };
}

test('a page follows a ledger reload instead of keeping the data it loaded first', async () => {
  // the account page of the stale-page bug: the header showed 970.00 after the server had reloaded to 870.00
  let balance = '970.00';
  const page = await mountPage(async () => balance);
  assert.deepEqual(page.shown(), { value: '970.00', firstLoad: false, refreshing: false });

  balance = '870.00'; // a transaction is appended to the ledger file and the server reloads it
  await act(async () => page.store.set(ledgerChangedAtom)); // what SSE `Reload` and every write in the app do
  assert.deepEqual(page.shown(), { value: '870.00', firstLoad: false, refreshing: false });
  await page.unmount();
});

test('a reload keeps the previous value on screen while the new one loads', async () => {
  let answer: Promise<string> = Promise.resolve('first');
  const page = await mountPage(() => answer);
  assert.equal(page.renders[0].firstLoad, true);
  assert.deepEqual(page.shown(), { value: 'first', firstLoad: false, refreshing: false });

  let respond: (value: string) => void = () => {};
  answer = new Promise((resolve) => (respond = resolve));
  await act(async () => page.store.set(ledgerChangedAtom));
  assert.deepEqual(page.shown(), { value: 'first', firstLoad: false, refreshing: true });

  await act(async () => respond('second'));
  assert.deepEqual(page.shown(), { value: 'second', firstLoad: false, refreshing: false });
  await page.unmount();
});

test('a page queries once per ledger revision, not on every render', async () => {
  let calls = 0;
  const page = await mountPage(async () => String(++calls), ['Assets:Bank']);
  assert.equal(calls, 1);
  await act(async () => page.store.set(ledgerChangedAtom));
  await act(async () => page.store.set(ledgerChangedAtom));
  assert.equal(calls, 3);
  assert.equal(page.shown().value, '3');
  await page.unmount();
});
