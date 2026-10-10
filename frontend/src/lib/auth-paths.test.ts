// The return paths of the login page and the app URL of the mobile app login handoff, run with Node's built-in runner: pnpm run test
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { appReturnTo, returnPath } from './auth-paths.ts';

const SCHEMES = ['zhang-app'];

test('the app URL is returned to when its scheme is allowed', () => {
  assert.equal(appReturnTo('?return_to=zhang-app%3A%2F%2Fauth%2Fcallback', SCHEMES), 'zhang-app://auth/callback');
  assert.equal(appReturnTo('?return_to=zhang-app://auth/callback?x=1', SCHEMES), 'zhang-app://auth/callback?x=1');
  assert.equal(appReturnTo('?return_to=ZHANG-APP://auth/callback', SCHEMES), 'ZHANG-APP://auth/callback');
  assert.equal(appReturnTo('?return_to=my-app://cb', ['zhang-app', 'my-app']), 'my-app://cb');
});

test('without an allowed app URL the login is a web login', () => {
  assert.equal(appReturnTo('', SCHEMES), null);
  assert.equal(appReturnTo('?next=/accounts', SCHEMES), null);
  assert.equal(appReturnTo('?return_to=', SCHEMES), null);
  assert.equal(appReturnTo('?return_to=https://evil.example.com/', SCHEMES), null);
  assert.equal(appReturnTo('?return_to=javascript:alert(1)', SCHEMES), null);
  assert.equal(appReturnTo('?return_to=/accounts', SCHEMES), null);
  assert.equal(appReturnTo('?return_to=zhang-app://auth/callback', []), null);
  // a server without the handoff
  assert.equal(appReturnTo('?return_to=zhang-app://auth/callback', undefined), null);
});

test('the web return path stays on the same origin', () => {
  assert.equal(returnPath('?next=%2Faccounts%3Fq%3D1'), '/accounts?q=1');
  assert.equal(returnPath('?next=//evil.example.com'), '/');
  assert.equal(returnPath('?next=/login'), '/');
  assert.equal(returnPath('?return_to=zhang-app://auth/callback'), '/');
});
