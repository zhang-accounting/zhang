// Component test of the mobile app login handoff, run with Node's built-in runner: pnpm run test. Vite loads the component
// (TSX and `@/` imports) as the app build does, and it renders into happy-dom with the English strings. `fetch` records the
// requests; `POST /api/auth/app/code` answers with a code.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { after, test } from 'node:test';
import { createServer } from 'vite';

const vite = await createServer({ appType: 'custom', logLevel: 'error', server: { middlewareMode: true, hmr: false, ws: false } });
const { GlobalRegistrator } = await import('@happy-dom/global-registrator');
GlobalRegistrator.register({ url: 'http://localhost:8000/login' });
after(async () => {
  await vite.close();
  await GlobalRegistrator.unregister();
});
// lets `act` flush effects and updates outside a test framework that sets it
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const { act, createElement } = await import('react');
const { createRoot } = await import('react-dom/client');
const { MemoryRouter } = await import('react-router');
const { getDefaultStore } = await import('jotai');
const i18next = (await import('i18next')).default;
const { initReactI18next } = await import('react-i18next');
const { AppHandoff } = await vite.ssrLoadModule('/src/layout/AppHandoff.tsx');
const { signedInForAppAtom } = await vite.ssrLoadModule('/src/states/auth.ts');

const translation = JSON.parse(await readFile(new URL('../../public/locales/en/translation.json', import.meta.url), 'utf8'));
await i18next.use(initReactI18next).init({ lng: 'en', resources: { en: { translation } }, interpolation: { escapeValue: false } });

const RETURN_TO = 'zhang-app://auth/callback';
const PARAMS = { code_challenge: 'E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM', code_challenge_method: 'S256', state: 'st' };

/** The requests `fetch` was asked for, as `METHOD path` with the JSON body. */
const requests: { request: string; body: unknown }[] = [];
globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit) => {
  const url = new URL(String(input), 'http://localhost:8000');
  requests.push({ request: `${init?.method ?? 'GET'} ${url.pathname}`, body: init?.body ? JSON.parse(String(init.body)) : undefined });
  // a redirect to a page of the same origin: happy-dom follows `location.replace` without leaving the test
  return new Response(JSON.stringify({ data: { code: 'c0de', redirect: 'http://localhost:8000/returned' } }), {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
  });
}) as typeof fetch;

const codeRequests = () => requests.filter((r) => r.request === 'POST /api/auth/app/code');

/** Lets the effects, the request and its answer run. */
const settle = () => act(() => new Promise<void>((resolve) => setTimeout(resolve, 50)));

async function render(needsConsent: boolean) {
  requests.length = 0;
  const container = document.createElement('div');
  document.body.replaceChildren(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(
      createElement(
        MemoryRouter,
        { initialEntries: ['/login'] },
        createElement(AppHandoff, { returnTo: RETURN_TO, params: PARAMS, user: 'admin', needsConsent }),
      ),
    );
  });
  await settle();
  return { container, unmount: () => act(() => root.unmount()) };
}

function button(container: HTMLElement, label: string) {
  const found = [...container.querySelectorAll('button')].find((b) => b.textContent?.trim() === label);
  assert.ok(found, `a "${label}" button`);
  return found;
}

test('an existing browser session: no code until "Continue" is clicked', async () => {
  getDefaultStore().set(signedInForAppAtom, null);
  const { container, unmount } = await render(true);
  assert.match(container.textContent ?? '', /Sign in to the app as admin\?/);
  assert.match(container.textContent ?? '', /zhang-app:\/\/auth/);
  await settle();
  assert.deepEqual(codeRequests(), [], 'nothing is requested before the click');

  await act(async () => button(container, 'Continue').click());
  await settle();
  assert.deepEqual(codeRequests(), [{ request: 'POST /api/auth/app/code', body: { return_to: RETURN_TO, ...PARAMS } }]);
  await unmount();
});

test('"Cancel" requests nothing', async () => {
  const { container, unmount } = await render(true);
  await act(async () => button(container, 'Cancel').click());
  await settle();
  assert.match(container.textContent ?? '', /The app was not signed in/);
  assert.deepEqual(codeRequests(), []);
  await unmount();
});

test('right after a sign-in for this app URL the code is created at once, and that sign-in is used up', async () => {
  getDefaultStore().set(signedInForAppAtom, RETURN_TO);
  const { unmount } = await render(false);
  assert.equal(codeRequests().length, 1);
  assert.equal(getDefaultStore().get(signedInForAppAtom), null, 'the next code needs the click again');
  await unmount();
});
