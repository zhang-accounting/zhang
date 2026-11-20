// The return paths of the login page and the app URL of the mobile app login handoff, run with Node's built-in runner: pnpm run test
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { appHandoffParams, appReturnTo, appTarget, handoffNeedsConsent, returnPath } from './auth-paths.ts';

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

test("the app's PKCE challenge and state ride along as they are", () => {
  assert.deepEqual(
    appHandoffParams(
      '?return_to=zhang-app%3A%2F%2Fauth%2Fcallback&code_challenge=E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM&code_challenge_method=S256&state=a%26b%20c',
    ),
    { code_challenge: 'E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM', code_challenge_method: 'S256', state: 'a&b c' },
  );
  // an older app
  assert.deepEqual(appHandoffParams('?return_to=zhang-app%3A%2F%2Fauth%2Fcallback'), {});
  assert.deepEqual(appHandoffParams('?code_challenge=&code_challenge_method=&state='), {});
  // the server refuses any method but S256, and a challenge without a method
  assert.deepEqual(appHandoffParams('?code_challenge=abc&code_challenge_method=plain'), { code_challenge: 'abc', code_challenge_method: 'plain' });
  assert.deepEqual(appHandoffParams('?code_challenge=abc'), { code_challenge: 'abc' });
  assert.deepEqual(appHandoffParams('?next=/accounts&code=planted'), {});
});

test('a session the browser already had needs a click before the app gets a code', () => {
  assert.equal(handoffNeedsConsent(false), true);
  // the user just signed in on this page, with the password, a passkey or the first passkey
  assert.equal(handoffNeedsConsent(true), false);
});

test('the consent step names the scheme and host the code goes to', () => {
  assert.equal(appTarget('zhang-app://auth/callback'), 'zhang-app://auth');
  assert.equal(appTarget('ZHANG-APP://auth/callback?x=1#y'), 'zhang-app://auth');
  assert.equal(appTarget('my-app://evil.example.com/cb'), 'my-app://evil.example.com');
  assert.equal(appTarget('my-app:/callback'), 'my-app:');
});

test('the web return path stays on the same origin', () => {
  assert.equal(returnPath('?next=%2Faccounts%3Fq%3D1'), '/accounts?q=1');
  assert.equal(returnPath('?next=//evil.example.com'), '/');
  assert.equal(returnPath('?next=/login'), '/');
  assert.equal(returnPath('?return_to=zhang-app://auth/callback'), '/');
});
